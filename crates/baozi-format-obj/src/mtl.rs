use crate::allocation::{LiveStorage, append_string_chunked, usize_to_u64 as usize_amount};
use crate::material_library_role;
use crate::parser::{self, TextDecodeFailure};
use baozi_core::{AlphaMode, Color, MetadataMap, MetadataValue};
use baozi_import::{
    DependencySnapshot, DiagnosticCode, DiagnosticSeverity, ImportProgressPhase,
    ImportStorageClass, ImportWorkClass, ImportWorkPermit, ParseResult, ParseScope,
};
use baozi_io::AssetPath;
use std::cmp::Ordering;
use std::fmt;

const INTERRUPTIBLE_WORK_CHUNK: u64 = 64;
const MAX_NUMERIC_LITERAL_BYTES: usize = 128;

pub(crate) struct ComparisonWork<'scope, 'session> {
    scope: &'scope ParseScope<'session>,
    permit: Option<ImportWorkPermit<'scope, 'session>>,
    remaining: u64,
}

impl<'scope, 'session> ComparisonWork<'scope, 'session> {
    fn new(scope: &'scope ParseScope<'session>) -> Self {
        Self {
            scope,
            permit: None,
            remaining: 0,
        }
    }

    fn step(&mut self) -> ParseResult<()> {
        if self.remaining == 0 {
            if let Some(permit) = self.permit.take() {
                permit.complete()?;
            }
            self.permit = Some(begin_comparison_work(self.scope)?);
            self.remaining = INTERRUPTIBLE_WORK_CHUNK;
        }
        self.remaining -= 1;
        Ok(())
    }

    pub(crate) fn compare_str(&mut self, left: &str, right: &str) -> ParseResult<Ordering> {
        for (left, right) in left.as_bytes().iter().zip(right.as_bytes()) {
            self.step()?;
            match left.cmp(right) {
                Ordering::Equal => {}
                ordering => return Ok(ordering),
            }
        }
        self.step()?;
        Ok(left.len().cmp(&right.len()))
    }

    fn finish(mut self) -> ParseResult<()> {
        if let Some(permit) = self.permit.take() {
            permit.complete()?;
        }
        Ok(())
    }
}

pub(crate) fn interruptible_sort_by<'scope, 'session, T>(
    scope: &'scope ParseScope<'session>,
    values: &mut [T],
    mut compare: impl FnMut(&mut ComparisonWork<'scope, 'session>, &T, &T) -> ParseResult<Ordering>,
) -> ParseResult<()> {
    if values.len() <= 1 {
        return Ok(());
    }

    let mut work = ComparisonWork::new(scope);
    let mut start = values.len() / 2;
    while start > 0 {
        start -= 1;
        sift_down(values, start, values.len(), &mut work, &mut compare)?;
    }
    let mut end = values.len();
    while end > 1 {
        end -= 1;
        values.swap(0, end);
        sift_down(values, 0, end, &mut work, &mut compare)?;
    }
    work.finish()
}

pub(crate) fn interruptible_dedup_by<'scope, 'session, T>(
    scope: &'scope ParseScope<'session>,
    values: &mut Vec<T>,
    mut equivalent: impl FnMut(&mut ComparisonWork<'scope, 'session>, &T, &T) -> ParseResult<bool>,
) -> ParseResult<()> {
    if values.len() <= 1 {
        return Ok(());
    }

    let mut work = ComparisonWork::new(scope);
    let mut write = 1usize;
    for read in 1..values.len() {
        work.step()?;
        if equivalent(&mut work, &values[write - 1], &values[read])? {
            continue;
        }
        if write != read {
            values.swap(write, read);
        }
        write += 1;
    }
    work.finish()?;
    while values.len() > write {
        let drop_count = (values.len() - write).min(INTERRUPTIBLE_WORK_CHUNK as usize);
        let permit = scope.begin_work(
            ImportWorkClass::WorkUnits,
            usize_amount(scope, drop_count, "obj-dedup-drop-work")?,
        )?;
        values.truncate(values.len() - drop_count);
        permit.complete()?;
    }
    Ok(())
}

pub(crate) fn interruptible_binary_search_by<'scope, 'session, T>(
    scope: &'scope ParseScope<'session>,
    values: &[T],
    mut compare: impl FnMut(&mut ComparisonWork<'scope, 'session>, &T) -> ParseResult<Ordering>,
) -> ParseResult<Option<usize>> {
    let mut work = ComparisonWork::new(scope);
    let mut left = 0usize;
    let mut right = values.len();
    let mut found = None;
    while left < right {
        let middle = left + (right - left) / 2;
        work.step()?;
        match compare(&mut work, &values[middle])? {
            Ordering::Less => left = middle + 1,
            Ordering::Greater => right = middle,
            Ordering::Equal => {
                found = Some(middle);
                break;
            }
        }
    }
    work.finish()?;
    Ok(found)
}

pub(crate) fn for_work_chunks(
    scope: &ParseScope<'_>,
    len: usize,
    resource: &'static str,
    mut visit: impl FnMut(std::ops::Range<usize>) -> ParseResult<()>,
) -> ParseResult<()> {
    let mut start = 0usize;
    while start < len {
        let end = start
            .checked_add(INTERRUPTIBLE_WORK_CHUNK as usize)
            .map_or(len, |end| end.min(len));
        let permit = scope.begin_work(
            ImportWorkClass::WorkUnits,
            usize_amount(scope, end - start, resource)?,
        )?;
        visit(start..end)?;
        permit.complete()?;
        start = end;
    }
    Ok(())
}

pub(crate) fn interruptible_str_eq(
    scope: &ParseScope<'_>,
    left: &str,
    right: &str,
    resource: &'static str,
) -> ParseResult<bool> {
    if left.len() != right.len() {
        return Ok(false);
    }
    let left = left.as_bytes();
    let right = right.as_bytes();
    let mut equal = true;
    for_work_chunks(scope, left.len(), resource, |range| {
        if left[range.clone()] != right[range] {
            equal = false;
        }
        Ok(())
    })?;
    Ok(equal)
}

fn begin_comparison_work<'scope, 'session>(
    scope: &'scope ParseScope<'session>,
) -> ParseResult<ImportWorkPermit<'scope, 'session>> {
    scope.begin_work(ImportWorkClass::WorkUnits, INTERRUPTIBLE_WORK_CHUNK)
}

fn sift_down<'scope, 'session, T>(
    values: &mut [T],
    mut root: usize,
    end: usize,
    work: &mut ComparisonWork<'scope, 'session>,
    compare: &mut impl FnMut(&mut ComparisonWork<'scope, 'session>, &T, &T) -> ParseResult<Ordering>,
) -> ParseResult<()> {
    while root < end / 2 {
        let child = root * 2 + 1;
        let mut greater = child;
        if child + 1 < end {
            work.step()?;
            if compare(work, &values[child], &values[child + 1])? == Ordering::Less {
                greater = child + 1;
            }
        }
        work.step()?;
        if compare(work, &values[root], &values[greater])? != Ordering::Less {
            return Ok(());
        }
        values.swap(root, greater);
        root = greater;
    }
    Ok(())
}

pub(crate) struct MaterialLibrary<'scope> {
    materials: Vec<ParsedMaterial<'scope>>,
    next_order: usize,
    storage: LiveStorage<'scope>,
}

pub(crate) struct ParsedMaterial<'scope> {
    pub name: String,
    order: usize,
    pub base_color: Color,
    pub emissive: Color,
    pub alpha_mode: AlphaMode,
    pub metadata: MetadataMap,
    pub diffuse_texture: Option<ParsedTexture<'scope>>,
    retired_metadata_strings: Vec<String>,
    storage: LiveStorage<'scope>,
}

pub(crate) struct ParsedTexture<'scope> {
    pub uri: String,
    pub source_key: &'static str,
    _storage: LiveStorage<'scope>,
}

impl<'scope> ParsedMaterial<'scope> {
    fn new(name: String, order: usize, storage: LiveStorage<'scope>) -> Self {
        Self {
            name,
            order,
            base_color: Color::WHITE,
            emissive: Color::linear_rgba(0.0, 0.0, 0.0, 1.0),
            alpha_mode: AlphaMode::Opaque,
            metadata: MetadataMap::new(),
            diffuse_texture: None,
            retired_metadata_strings: Vec::new(),
            storage,
        }
    }
}

pub(crate) fn load_libraries<'scope>(
    scope: &'scope ParseScope<'_>,
    mtllibs: &[String],
) -> ParseResult<MaterialLibrary<'scope>> {
    let role = material_library_role();
    let mut library = MaterialLibrary {
        materials: Vec::new(),
        next_order: 0,
        storage: LiveStorage::new(ImportStorageClass::FormatNormalization),
    };
    for mtllib in mtllibs {
        let outcome = scope.optional_dependency(&role, mtllib)?;
        if let Some(snapshot) = outcome.snapshot() {
            let text = match parser::decode_utf8(scope, snapshot.bytes(), "obj-mtl-text") {
                Ok(text) => text,
                Err(TextDecodeFailure::Scope(failure)) => return Err(failure),
                Err(TextDecodeFailure::InvalidUtf8 {
                    source,
                    base_offset,
                }) => {
                    let failure = crate::ObjFailure::invalid_utf8_at(source, base_offset)
                        .ok_or_else(|| {
                            scope.allocation_failure("obj-mtl-invalid-utf8-byte-offset")
                        })?;
                    return Err(scope.format_failure(failure));
                }
            };
            parse_mtl(scope, snapshot, &text, &mut library)?;
            drop(text);
            continue;
        }

        let Some(evidence) = outcome.unavailable_evidence() else {
            return Err(scope.format_failure(crate::ObjFailure::internal(
                "optional-dependency-outcome-without-evidence",
            )));
        };
        push_warning(
            scope,
            None,
            "obj.mtl_unavailable",
            format_args!(
                "optional material library '{}' is unavailable; owner={:?}, cache={:?}",
                evidence.normalized_path(),
                evidence.owner(),
                evidence.cache_disposition()
            ),
        )?;
    }
    finalize_materials(scope, &mut library)?;
    Ok(library)
}

impl<'scope> MaterialLibrary<'scope> {
    pub(crate) fn find<'library>(
        &'library self,
        scope: &ParseScope<'_>,
        name: &str,
    ) -> ParseResult<Option<&'library ParsedMaterial<'scope>>> {
        let index = interruptible_binary_search_by(scope, &self.materials, |work, material| {
            work.compare_str(&material.name, name)
        })?;
        Ok(index.map(|index| &self.materials[index]))
    }
}

fn parse_mtl<'scope>(
    scope: &'scope ParseScope<'_>,
    snapshot: &DependencySnapshot<'_>,
    text: &parser::DecodedText<'_>,
    library: &mut MaterialLibrary<'scope>,
) -> ParseResult<()> {
    let path = snapshot.identity().path();
    let mut current = None;

    parser::scan_lines(scope, text, |decoded_line, line_number| {
        let record_permit = scope.begin_work(ImportWorkClass::Records, 1)?;

        let raw_line = decoded_line.text();
        let tokens = tokenize_mtl(scope, &decoded_line)?;
        let keyword = tokens.first().map(|token| token.text);
        let allowed_value = match keyword {
            Some("newmtl") => trailing_mtl_token_range(&tokens),
            Some("map_Kd") => recovery_map_path_range(scope, &tokens[1..])?,
            _ => None,
        };
        parser::authorize_line_recoveries(
            scope,
            &decoded_line,
            allowed_value,
            decoded_line.comment_start(),
            || snapshot.diagnostic_source(),
        )?;
        let Some(keyword) = keyword else {
            return record_permit.complete();
        };

        match keyword {
            "newmtl" => {
                flush_material(scope, library, current.take())?;
                let name = trailing_text_after_keyword(raw_line, &tokens).unwrap_or_default();
                if name.is_empty() {
                    push_dependency_warning(
                        scope,
                        snapshot,
                        Some((line_number, 1)),
                        "obj.mtl_empty_name",
                        format_args!("MTL newmtl has no material name"),
                    )?;
                } else if name.len() > parser::max_text_value_bytes(scope) {
                    push_dependency_warning(
                        scope,
                        snapshot,
                        Some((line_number, 1)),
                        "obj.mtl_name_limit",
                        format_args!("MTL material name exceeded max_string_bytes"),
                    )?;
                } else {
                    let order = library.next_order;
                    library.next_order = library
                        .next_order
                        .checked_add(1)
                        .ok_or_else(|| scope.allocation_failure("obj-mtl-material-order"))?;
                    let mut storage = LiveStorage::new(ImportStorageClass::FormatNormalization);
                    let name = owned_string(scope, &mut storage, name, "obj-mtl-material-name")?;
                    current = Some(ParsedMaterial::new(name, order, storage));
                }
            }
            "Kd" => {
                if let Some(material) = current.as_mut()
                    && let Some(color) = parse_color(scope, snapshot, &tokens, line_number, "Kd")?
                {
                    material.base_color.r = color.r;
                    material.base_color.g = color.g;
                    material.base_color.b = color.b;
                }
            }
            "d" => {
                if let Some(material) = current.as_mut()
                    && let Some(alpha) = parse_scalar(scope, snapshot, &tokens, line_number, "d")?
                {
                    material.base_color.a = alpha;
                    if alpha < 1.0 {
                        material.alpha_mode = AlphaMode::Blend;
                    }
                }
            }
            "Tr" => {
                if let Some(material) = current.as_mut()
                    && let Some(transparency) =
                        parse_scalar(scope, snapshot, &tokens, line_number, "Tr")?
                {
                    let alpha = 1.0 - transparency;
                    material.base_color.a = alpha;
                    if alpha < 1.0 {
                        material.alpha_mode = AlphaMode::Blend;
                    }
                }
            }
            "Ke" => {
                if let Some(material) = current.as_mut()
                    && let Some(color) = parse_color(scope, snapshot, &tokens, line_number, "Ke")?
                {
                    material.emissive = color;
                }
            }
            "Ns" | "Ni" => {
                if let Some(material) = current.as_mut()
                    && let Some(value) =
                        parse_scalar(scope, snapshot, &tokens, line_number, keyword)?
                {
                    let metadata_key = if keyword == "Ns" { "obj:ns" } else { "obj:ni" };
                    insert_metadata(
                        scope,
                        material,
                        metadata_key,
                        MetadataValue::F64(value as f64),
                    )?;
                }
            }
            "illum" => {
                if let Some(material) = current.as_mut()
                    && let Some(value) = parse_i64(scope, snapshot, &tokens, line_number, "illum")?
                {
                    insert_metadata(scope, material, "obj:illum", MetadataValue::I64(value))?;
                }
            }
            "Ka" | "Ks" => {
                if let Some(material) = current.as_mut()
                    && let Some(value) = parse_vec_metadata(
                        scope,
                        &mut material.storage,
                        snapshot,
                        &tokens,
                        line_number,
                        keyword,
                    )?
                {
                    let metadata_key = if keyword == "Ka" { "obj:ka" } else { "obj:ks" };
                    insert_metadata(scope, material, metadata_key, MetadataValue::String(value))?;
                }
            }
            "map_Kd" => {
                if let Some(material) = current.as_mut() {
                    match texture_path_from_map(scope, snapshot, &tokens[1..], line_number)? {
                        TexturePathOutcome::Value {
                            path: texture_path,
                            storage: texture_path_storage,
                        } => {
                            let path_work = path
                                .as_str()
                                .len()
                                .checked_add(texture_path.len())
                                .ok_or_else(|| {
                                    scope.allocation_failure("obj-mtl-texture-path-work")
                                })?;
                            let resolution_work = scope.begin_work(
                                ImportWorkClass::WorkUnits,
                                usize_amount(scope, path_work, "obj-mtl-texture-path-work")?,
                            )?;
                            let mut resolution_storage =
                                LiveStorage::new(ImportStorageClass::FormatNormalization);
                            resolution_storage.reserve_bytes(
                                scope,
                                path_resolution_allowance(scope, path, &texture_path)?,
                                "obj-mtl-texture-path-resolution",
                            )?;
                            let resolved = path.resolve_from_file(&texture_path);
                            resolution_work.complete()?;
                            match resolved {
                                Ok(resolved) => {
                                    if resolved.as_str().len() > parser::max_text_value_bytes(scope)
                                    {
                                        push_dependency_warning(
                                            scope,
                                            snapshot,
                                            Some((line_number, 1)),
                                            "obj.mtl_texture_path_limit",
                                            format_args!(
                                                "resolved MTL map_Kd texture URI exceeded max_string_bytes"
                                            ),
                                        )?;
                                    } else {
                                        let mut texture_storage = LiveStorage::new(
                                            ImportStorageClass::FormatNormalization,
                                        );
                                        let uri = owned_string(
                                            scope,
                                            &mut texture_storage,
                                            resolved.as_str(),
                                            "obj-mtl-texture-uri",
                                        )?;
                                        material.diffuse_texture = Some(ParsedTexture {
                                            uri,
                                            source_key: "map_Kd",
                                            _storage: texture_storage,
                                        });
                                    }
                                }
                                Err(error) => push_dependency_warning(
                                    scope,
                                    snapshot,
                                    Some((line_number, 1)),
                                    "obj.mtl_texture_resolve_failed",
                                    format_args!(
                                        "MTL map_Kd texture could not be resolved: {error}"
                                    ),
                                )?,
                            }
                            drop(resolution_storage);
                            drop(texture_path);
                            drop(texture_path_storage);
                        }
                        TexturePathOutcome::TooLong => push_dependency_warning(
                            scope,
                            snapshot,
                            Some((line_number, 1)),
                            "obj.mtl_texture_path_limit",
                            format_args!("MTL map_Kd texture path exceeded max_string_bytes"),
                        )?,
                        TexturePathOutcome::Missing => push_dependency_warning(
                            scope,
                            snapshot,
                            Some((line_number, 1)),
                            "obj.mtl_texture_missing",
                            format_args!("MTL map_Kd has no texture path"),
                        )?,
                    }
                }
            }
            keyword => push_dependency_warning(
                scope,
                snapshot,
                Some((line_number, 1)),
                "obj.mtl_unknown_statement",
                format_args!("MTL statement '{keyword}' is not recognized"),
            )?,
        }
        record_permit.complete()
    })?;

    flush_material(scope, library, current)
}

fn flush_material<'scope>(
    scope: &'scope ParseScope<'_>,
    library: &mut MaterialLibrary<'scope>,
    material: Option<ParsedMaterial<'scope>>,
) -> ParseResult<()> {
    let Some(material) = material else {
        return Ok(());
    };
    library
        .storage
        .reserve_vec(scope, &mut library.materials, 1, "obj-mtl-materials")?;
    library.materials.push(material);
    Ok(())
}

fn finalize_materials(
    scope: &ParseScope<'_>,
    library: &mut MaterialLibrary<'_>,
) -> ParseResult<()> {
    if library.materials.len() <= 1 {
        return Ok(());
    }

    interruptible_sort_by(scope, &mut library.materials, |work, left, right| {
        Ok(work
            .compare_str(&left.name, &right.name)?
            .then_with(|| right.order.cmp(&left.order)))
    })?;
    interruptible_dedup_by(scope, &mut library.materials, |work, previous, current| {
        let duplicate = work.compare_str(&previous.name, &current.name)? == Ordering::Equal;
        if duplicate {
            push_warning(
                scope,
                None,
                "obj.mtl_duplicate_material",
                format_args!(
                    "MTL material '{}' was redefined; using the last definition",
                    previous.name
                ),
            )?;
        }
        Ok(duplicate)
    })
}

fn insert_metadata<'scope>(
    scope: &'scope ParseScope<'_>,
    material: &mut ParsedMaterial<'scope>,
    key: &'static str,
    value: MetadataValue,
) -> ParseResult<()> {
    if let Some(existing) = material.metadata.get_mut(key) {
        let previous = std::mem::replace(existing, value);
        if let MetadataValue::String(previous) = previous {
            material.storage.reserve_vec(
                scope,
                &mut material.retired_metadata_strings,
                1,
                "obj-mtl-retired-metadata-strings",
            )?;
            material.retired_metadata_strings.push(previous);
        }
        return Ok(());
    }

    let key = owned_string(scope, &mut material.storage, key, "obj-mtl-metadata-key")?;
    material.storage.insert_ordered_map(
        scope,
        &mut material.metadata,
        key,
        value,
        "obj-mtl-metadata-map",
    )?;
    Ok(())
}

#[derive(Clone, Copy)]
struct MtlToken<'line> {
    text: &'line str,
    start: usize,
    end: usize,
}

struct MtlTokenBuffer<'scope, 'line> {
    values: Vec<MtlToken<'line>>,
    storage: LiveStorage<'scope>,
}

impl<'line> std::ops::Deref for MtlTokenBuffer<'_, 'line> {
    type Target = [MtlToken<'line>];

    fn deref(&self) -> &Self::Target {
        &self.values
    }
}

fn tokenize_mtl<'scope, 'line>(
    scope: &'scope ParseScope<'_>,
    decoded_line: &'line parser::DecodedLine<'_>,
) -> ParseResult<MtlTokenBuffer<'scope, 'line>> {
    let line = decoded_line.text();
    let mut tokens = MtlTokenBuffer {
        values: Vec::new(),
        storage: LiveStorage::new(ImportStorageClass::FormatNormalization),
    };
    parser::scan_token_ranges(
        scope,
        line,
        "obj-mtl-token-scan-work",
        |_, _| Ok(()),
        |start, end| {
            let token_permit = scope.begin_work(ImportWorkClass::Tokens, 1)?;
            let token_bytes = scope.begin_work(
                ImportWorkClass::TokenBytes,
                usize_amount(scope, end - start, "obj-mtl-token-bytes")?,
            )?;
            tokens
                .storage
                .reserve_vec(scope, &mut tokens.values, 1, "obj-mtl-tokens")?;
            tokens.values.push(MtlToken {
                text: &line[start..end],
                start,
                end,
            });
            token_bytes.complete()?;
            token_permit.complete()
        },
    )?;
    Ok(tokens)
}

fn parse_color(
    scope: &ParseScope<'_>,
    snapshot: &DependencySnapshot<'_>,
    tokens: &[MtlToken<'_>],
    line_number: u32,
    keyword: &str,
) -> ParseResult<Option<Color>> {
    if tokens.len() < 4 {
        push_dependency_warning(
            scope,
            snapshot,
            Some((line_number, 1)),
            "obj.mtl_bad_color",
            format_args!("MTL {keyword} requires three components"),
        )?;
        return Ok(None);
    }
    let Some(red) = parse_f32_warning(scope, snapshot, tokens[1].text, line_number, keyword)?
    else {
        return Ok(None);
    };
    let Some(green) = parse_f32_warning(scope, snapshot, tokens[2].text, line_number, keyword)?
    else {
        return Ok(None);
    };
    let Some(blue) = parse_f32_warning(scope, snapshot, tokens[3].text, line_number, keyword)?
    else {
        return Ok(None);
    };
    Ok(Some(Color::linear_rgba(red, green, blue, 1.0)))
}

fn parse_scalar(
    scope: &ParseScope<'_>,
    snapshot: &DependencySnapshot<'_>,
    tokens: &[MtlToken<'_>],
    line_number: u32,
    keyword: &str,
) -> ParseResult<Option<f32>> {
    if tokens.len() < 2 {
        push_dependency_warning(
            scope,
            snapshot,
            Some((line_number, 1)),
            "obj.mtl_bad_scalar",
            format_args!("MTL {keyword} requires one value"),
        )?;
        return Ok(None);
    }
    parse_f32_warning(scope, snapshot, tokens[1].text, line_number, keyword)
}

fn parse_i64(
    scope: &ParseScope<'_>,
    snapshot: &DependencySnapshot<'_>,
    tokens: &[MtlToken<'_>],
    line_number: u32,
    keyword: &str,
) -> ParseResult<Option<i64>> {
    if tokens.len() < 2 {
        push_dependency_warning(
            scope,
            snapshot,
            Some((line_number, 1)),
            "obj.mtl_bad_integer",
            format_args!("MTL {keyword} requires one integer"),
        )?;
        return Ok(None);
    }
    if tokens[1].text.len() > MAX_NUMERIC_LITERAL_BYTES {
        push_dependency_warning(
            scope,
            snapshot,
            Some((line_number, 1)),
            "obj.mtl_bad_integer",
            format_args!("MTL {keyword} integer is too long"),
        )?;
        return Ok(None);
    }
    match tokens[1].text.parse::<i64>() {
        Ok(value) => Ok(Some(value)),
        Err(_) => {
            push_dependency_warning(
                scope,
                snapshot,
                Some((line_number, 1)),
                "obj.mtl_bad_integer",
                format_args!("invalid MTL {keyword} integer"),
            )?;
            Ok(None)
        }
    }
}

fn parse_vec_metadata<'scope>(
    scope: &'scope ParseScope<'_>,
    storage: &mut LiveStorage<'scope>,
    snapshot: &DependencySnapshot<'_>,
    tokens: &[MtlToken<'_>],
    line_number: u32,
    keyword: &str,
) -> ParseResult<Option<String>> {
    let Some(color) = parse_color(scope, snapshot, tokens, line_number, keyword)? else {
        return Ok(None);
    };
    owned_format(
        scope,
        storage,
        format_args!("{} {} {}", color.r, color.g, color.b),
    )
    .map(Some)
}

fn parse_f32_warning(
    scope: &ParseScope<'_>,
    snapshot: &DependencySnapshot<'_>,
    text: &str,
    line_number: u32,
    keyword: &str,
) -> ParseResult<Option<f32>> {
    if text.len() > MAX_NUMERIC_LITERAL_BYTES {
        push_dependency_warning(
            scope,
            snapshot,
            Some((line_number, 1)),
            "obj.mtl_bad_float",
            format_args!("MTL {keyword} float is too long"),
        )?;
        return Ok(None);
    }
    let Ok(value) = text.parse::<f32>() else {
        push_dependency_warning(
            scope,
            snapshot,
            Some((line_number, 1)),
            "obj.mtl_bad_float",
            format_args!("invalid MTL {keyword} float"),
        )?;
        return Ok(None);
    };
    if !value.is_finite() {
        push_dependency_warning(
            scope,
            snapshot,
            Some((line_number, 1)),
            "obj.mtl_bad_float",
            format_args!("MTL {keyword} float is non-finite"),
        )?;
        return Ok(None);
    }
    Ok(Some(value))
}

fn texture_path_from_map<'scope>(
    scope: &'scope ParseScope<'_>,
    snapshot: &DependencySnapshot<'_>,
    tokens: &[MtlToken<'_>],
    line_number: u32,
) -> ParseResult<TexturePathOutcome<'scope>> {
    let mut index = 0usize;
    while index < tokens.len() {
        scope.poll()?;
        let token = tokens[index].text;
        if token.starts_with('-') {
            if !is_known_map_option(token) {
                push_dependency_warning(
                    scope,
                    snapshot,
                    Some((line_number, 1)),
                    "obj.mtl_unsupported_texture_option",
                    format_args!("MTL map_Kd option '{token}' is not supported"),
                )?;
            }
            index = index.saturating_add(1 + map_option_arity(token, &tokens[index + 1..]));
        } else {
            let mut length = 0usize;
            for chunk in tokens[index..].chunks(64) {
                scope.poll()?;
                for token in chunk {
                    length = length
                        .checked_add(token.text.len())
                        .and_then(|length| length.checked_add(1))
                        .ok_or_else(|| scope.allocation_failure("obj-mtl-texture-path-length"))?;
                }
            }
            let length = length.saturating_sub(1);
            if length > parser::max_text_value_bytes(scope) {
                return Ok(TexturePathOutcome::TooLong);
            }
            let string_permit = scope.begin_work(
                ImportWorkClass::StringBytes,
                usize_amount(scope, length, "obj-mtl-texture-path-bytes")?,
            )?;
            let mut storage = LiveStorage::new(ImportStorageClass::FormatNormalization);
            let mut path = storage.allocate_string(scope, length, "obj-mtl-texture-path")?;
            let mut first = true;
            for chunk in tokens[index..].chunks(64) {
                scope.poll()?;
                for token in chunk {
                    if !first {
                        path.push(' ');
                    }
                    append_string_chunked(scope, &mut path, token.text)?;
                    first = false;
                }
            }
            string_permit.complete()?;
            return Ok(TexturePathOutcome::Value { path, storage });
        }
    }
    Ok(TexturePathOutcome::Missing)
}

enum TexturePathOutcome<'scope> {
    Missing,
    TooLong,
    Value {
        path: String,
        storage: LiveStorage<'scope>,
    },
}

fn path_resolution_allowance(
    scope: &ParseScope<'_>,
    base: &AssetPath,
    reference: &str,
) -> ParseResult<usize> {
    let source_bytes = base
        .as_str()
        .len()
        .checked_add(reference.len())
        .ok_or_else(|| scope.allocation_failure("obj-mtl-texture-path-resolution"))?;
    let mut component_count = 2usize;
    for bytes in [base.as_str().as_bytes(), reference.as_bytes()] {
        for chunk in bytes.chunks(parser::SCAN_CHUNK_BYTES) {
            scope.poll()?;
            for byte in chunk {
                if *byte == b'/' || *byte == b'\\' {
                    component_count = component_count.checked_add(1).ok_or_else(|| {
                        scope.allocation_failure("obj-mtl-texture-path-resolution")
                    })?;
                }
            }
        }
    }
    let component_storage = std::mem::size_of::<String>()
        .checked_add(std::mem::size_of::<&str>())
        .and_then(|bytes| bytes.checked_mul(component_count))
        .ok_or_else(|| scope.allocation_failure("obj-mtl-texture-path-resolution"))?;
    source_bytes
        .checked_mul(6)
        .and_then(|bytes| bytes.checked_add(component_storage))
        .ok_or_else(|| scope.allocation_failure("obj-mtl-texture-path-resolution"))
}

fn owned_string<'scope>(
    scope: &'scope ParseScope<'_>,
    storage: &mut LiveStorage<'scope>,
    value: &str,
    resource: &'static str,
) -> ParseResult<String> {
    parser::enforce_text_value_limit(scope, value.len(), resource)?;
    let string_permit = scope.begin_work(
        ImportWorkClass::StringBytes,
        usize_amount(scope, value.len(), resource)?,
    )?;
    let output = storage.copy_string(scope, value, resource)?;
    string_permit.complete()?;
    Ok(output)
}

fn owned_format<'scope>(
    scope: &'scope ParseScope<'_>,
    storage: &mut LiveStorage<'scope>,
    arguments: fmt::Arguments<'_>,
) -> ParseResult<String> {
    use fmt::Write as _;

    let mut counter = FormatByteCounter::default();
    counter
        .write_fmt(arguments)
        .map_err(|_| scope.allocation_failure("obj-mtl-formatted-string-length"))?;
    parser::enforce_text_value_limit(scope, counter.bytes, "obj-mtl-formatted-string-bytes")?;
    let string_permit = scope.begin_work(
        ImportWorkClass::StringBytes,
        usize_amount(scope, counter.bytes, "obj-mtl-formatted-string-bytes")?,
    )?;
    let mut output = storage.allocate_string(scope, counter.bytes, "obj-mtl-formatted-string")?;
    output
        .write_fmt(arguments)
        .map_err(|_| scope.allocation_failure("obj-mtl-formatted-string"))?;
    string_permit.complete()?;
    Ok(output)
}

#[derive(Default)]
struct FormatByteCounter {
    bytes: usize,
}

impl fmt::Write for FormatByteCounter {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        self.bytes = self.bytes.checked_add(value.len()).ok_or(fmt::Error)?;
        Ok(())
    }
}

fn is_known_map_option(option: &str) -> bool {
    matches!(
        option,
        "-s" | "-o" | "-t" | "-mm" | "-bm" | "-boost" | "-clamp" | "-type" | "-texres"
    )
}

fn map_option_arity(option: &str, following: &[MtlToken<'_>]) -> usize {
    match option {
        "-s" | "-o" | "-t" => numeric_option_arity(following, 3),
        "-mm" => numeric_option_arity(following, 2),
        "-bm" | "-boost" | "-clamp" | "-type" | "-texres" => 1,
        _ => 1,
    }
}

fn numeric_option_arity(tokens: &[MtlToken<'_>], max: usize) -> usize {
    tokens
        .iter()
        .take(max)
        .take_while(|token| {
            token.text.len() <= MAX_NUMERIC_LITERAL_BYTES && token.text.parse::<f32>().is_ok()
        })
        .count()
}

fn trailing_text_after_keyword<'line>(
    line: &'line str,
    tokens: &[MtlToken<'line>],
) -> Option<&'line str> {
    let first_value = tokens.get(1)?;
    let last_value = tokens.last()?;
    line.get(first_value.start..last_value.end)
}

fn trailing_mtl_token_range(tokens: &[MtlToken<'_>]) -> Option<std::ops::Range<usize>> {
    let first_value = tokens.get(1)?;
    let last_value = tokens.last()?;
    Some(first_value.start..last_value.end)
}

fn recovery_map_path_range(
    scope: &ParseScope<'_>,
    tokens: &[MtlToken<'_>],
) -> ParseResult<Option<std::ops::Range<usize>>> {
    let mut index = 0usize;
    while let Some(token) = tokens.get(index) {
        scope.poll()?;
        if !token.text.starts_with('-') {
            let last = tokens.last().ok_or_else(|| {
                scope.format_failure(crate::ObjFailure::internal("missing-map-path-token"))
            })?;
            return Ok(Some(token.start..last.end));
        }

        let following = tokens.get(index + 1..).unwrap_or_default();
        let consumed = match map_option_recovery_arity(token.text) {
            MapOptionRecoveryArity::Numeric { minimum, maximum } => {
                if following.len() < minimum
                    || following.iter().take(minimum).any(|value| {
                        value.text.len() > MAX_NUMERIC_LITERAL_BYTES
                            || value.text.parse::<f32>().is_err()
                    })
                {
                    return Ok(None);
                }
                numeric_option_arity(following, maximum).max(minimum)
            }
            MapOptionRecoveryArity::Opaque(count) => {
                if following.len() < count {
                    return Ok(None);
                }
                count
            }
        };
        index = index
            .checked_add(1)
            .and_then(|value| value.checked_add(consumed))
            .ok_or_else(|| scope.allocation_failure("obj-mtl-map-option-index"))?;
    }
    Ok(None)
}

enum MapOptionRecoveryArity {
    Numeric { minimum: usize, maximum: usize },
    Opaque(usize),
}

fn map_option_recovery_arity(option: &str) -> MapOptionRecoveryArity {
    match option {
        "-s" | "-o" | "-t" => MapOptionRecoveryArity::Numeric {
            minimum: 1,
            maximum: 3,
        },
        "-mm" => MapOptionRecoveryArity::Numeric {
            minimum: 2,
            maximum: 2,
        },
        "-bm" | "-boost" | "-texres" => MapOptionRecoveryArity::Numeric {
            minimum: 1,
            maximum: 1,
        },
        _ => MapOptionRecoveryArity::Opaque(1),
    }
}

pub(crate) fn push_warning(
    scope: &ParseScope<'_>,
    location: Option<(u32, u32)>,
    code: &'static str,
    message: fmt::Arguments<'_>,
) -> ParseResult<()> {
    let code = DiagnosticCode::new(code).expect("static OBJ diagnostic code must be valid");
    match location {
        Some((line, column)) => scope.emit_diagnostic(
            DiagnosticSeverity::Warning,
            code,
            format_args!("line {line}, column {column}: {message}"),
        ),
        None => scope.emit_diagnostic(DiagnosticSeverity::Warning, code, message),
    }
}

fn push_dependency_warning(
    scope: &ParseScope<'_>,
    snapshot: &DependencySnapshot<'_>,
    location: Option<(u32, u32)>,
    code: &'static str,
    message: fmt::Arguments<'_>,
) -> ParseResult<()> {
    let code = DiagnosticCode::new(code).expect("static OBJ diagnostic code must be valid");
    match location {
        Some((line, column)) => scope.emit_diagnostic_with(
            DiagnosticSeverity::Warning,
            code,
            ImportProgressPhase::Decode,
            snapshot.diagnostic_source(),
            None,
            None,
            |_| {},
            format_args!("line {line}, column {column}: {message}"),
        ),
        None => scope.emit_diagnostic_with(
            DiagnosticSeverity::Warning,
            code,
            ImportProgressPhase::Decode,
            snapshot.diagnostic_source(),
            None,
            None,
            |_| {},
            message,
        ),
    }
}
