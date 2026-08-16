use crate::material_library_role;
use crate::parser::{self, TextDecodeFailure};
use baozi_core::{AlphaMode, Color};
use baozi_import::{
    DependencySnapshot, DiagnosticCode, DiagnosticSeverity, ImportProgressPhase,
    ImportStorageClass, ImportWorkClass, ImportWorkScope, MeteredOrderedMap, MeteredString,
    MeteredVec, ParseFailure, ParseResult, ParseScope,
};
use baozi_io::AssetPath;
use std::cmp::Ordering;
use std::fmt;

use parser::usize_amount;

const INTERRUPTIBLE_WORK_CHUNK: u64 = 64;
const MAX_NUMERIC_LITERAL_BYTES: usize = 128;

pub(crate) struct ComparisonWork<'meter, 'work, 'session> {
    work: &'meter mut ImportWorkScope<'work, 'session, 1>,
    admitted: bool,
    remaining: u64,
}

impl<'meter, 'work, 'session> ComparisonWork<'meter, 'work, 'session> {
    fn new(work: &'meter mut ImportWorkScope<'work, 'session, 1>) -> Self {
        Self {
            work,
            admitted: false,
            remaining: 0,
        }
    }

    fn step(&mut self) -> ParseResult<()> {
        if self.remaining == 0 {
            if self.admitted {
                self.work.complete_all()?;
            }
            self.work.replenish(INTERRUPTIBLE_WORK_CHUNK)?;
            self.admitted = true;
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

    fn finish(self) -> ParseResult<()> {
        if self.admitted {
            self.work.complete_all()?;
        }
        Ok(())
    }
}

pub(crate) fn interruptible_sort_by<'scope, 'session, T>(
    scope: &'scope ParseScope<'session>,
    values: &mut MeteredVec<'_, T>,
    mut compare: impl for<'meter, 'work> FnMut(
        &mut ComparisonWork<'meter, 'work, 'session>,
        &T,
        &T,
    ) -> ParseResult<Ordering>,
) -> ParseResult<()> {
    if values.len() <= 1 {
        return Ok(());
    }

    scope.with_work(ImportWorkClass::WorkUnits, 0, |ledger_work| {
        let mut work = ComparisonWork::new(ledger_work);
        let result: Result<(), ParseFailure> = values.try_sort_unstable_by(|left, right| {
            work.step()?;
            compare(&mut work, left, right)
        });
        result?;
        work.finish()
    })
}

pub(crate) fn interruptible_dedup_by<'scope, 'session, T>(
    scope: &'scope ParseScope<'session>,
    values: &mut MeteredVec<'_, T>,
    mut equivalent: impl for<'meter, 'work> FnMut(
        &mut ComparisonWork<'meter, 'work, 'session>,
        &T,
        &T,
    ) -> ParseResult<bool>,
) -> ParseResult<()> {
    if values.len() <= 1 {
        return Ok(());
    }

    let previous_len = values.len();
    let retained_len = scope.with_work(ImportWorkClass::WorkUnits, 0, |ledger_work| {
        let mut work = ComparisonWork::new(ledger_work);
        let result: Result<usize, ParseFailure> =
            values.try_partition_dedup_by(|previous, current| {
                work.step()?;
                equivalent(&mut work, previous, current)
            });
        let retained_len = result?;
        work.finish()?;
        Ok(retained_len)
    })?;
    for_work_chunks(
        scope,
        previous_len - retained_len,
        "obj-dedup-drop-work",
        |range| {
            values.truncate(values.len() - range.len());
            Ok(())
        },
    )
}

pub(crate) fn interruptible_binary_search_by<'scope, 'session, T>(
    scope: &'scope ParseScope<'session>,
    values: &[T],
    mut compare: impl for<'meter, 'work> FnMut(
        &mut ComparisonWork<'meter, 'work, 'session>,
        &T,
    ) -> ParseResult<Ordering>,
) -> ParseResult<Option<usize>> {
    if values.is_empty() {
        return Ok(None);
    }

    scope.with_work(ImportWorkClass::WorkUnits, 0, |ledger_work| {
        let mut work = ComparisonWork::new(ledger_work);
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
    })
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
        scope.with_work(
            ImportWorkClass::WorkUnits,
            usize_amount(scope, end - start, resource)?,
            |work| {
                visit(start..end)?;
                work.complete_all()
            },
        )?;
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

pub(crate) struct MaterialLibrary<'scope> {
    materials: MeteredVec<'scope, ParsedMaterial<'scope>>,
    next_order: usize,
}

pub(crate) struct ParsedMaterial<'scope> {
    pub name: MeteredString<'scope>,
    order: usize,
    pub base_color: Color,
    pub emissive: Color,
    pub alpha_mode: AlphaMode,
    pub metadata: ParsedMetadata<'scope>,
    pub diffuse_texture: Option<ParsedTexture<'scope>>,
}

pub(crate) struct ParsedTexture<'scope> {
    pub uri: MeteredString<'scope>,
    pub source_key: &'static str,
}

pub(crate) struct ParsedMetadata<'scope> {
    entries: MeteredOrderedMap<'scope, ParsedString<'scope>, ParsedMetadataValue<'scope>>,
}

struct ParsedString<'scope>(MeteredString<'scope>);

pub(crate) enum ParsedMetadataValue<'scope> {
    I64(i64),
    F64(f64),
    String(MeteredString<'scope>),
}

impl PartialEq for ParsedString<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.0.as_str() == other.0.as_str()
    }
}

impl Eq for ParsedString<'_> {}

impl PartialOrd for ParsedString<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ParsedString<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.as_str().cmp(other.0.as_str())
    }
}

impl<'scope> ParsedMaterial<'scope> {
    fn new(scope: &'scope ParseScope<'_>, name: MeteredString<'scope>, order: usize) -> Self {
        Self {
            name,
            order,
            base_color: Color::WHITE,
            emissive: Color::linear_rgba(0.0, 0.0, 0.0, 1.0),
            alpha_mode: AlphaMode::Opaque,
            metadata: ParsedMetadata {
                entries: scope.metered_ordered_map(
                    ImportStorageClass::FormatNormalization,
                    "obj-mtl-metadata-entries",
                ),
            },
            diffuse_texture: None,
        }
    }
}

impl ParsedMetadata<'_> {
    pub(crate) fn iter(&self) -> impl ExactSizeIterator<Item = (&str, &ParsedMetadataValue<'_>)> {
        self.entries
            .iter()
            .map(|(key, value)| (key.0.as_str(), value))
    }
}

pub(crate) fn load_libraries<'scope>(
    scope: &'scope ParseScope<'_>,
    mtllibs: &[MeteredString<'scope>],
) -> ParseResult<MaterialLibrary<'scope>> {
    let role = material_library_role();
    let mut library = MaterialLibrary {
        materials: scope.metered_vec(ImportStorageClass::FormatNormalization, "obj-mtl-materials"),
        next_order: 0,
    };
    for mtllib in mtllibs {
        let outcome = scope.optional_dependency(&role, mtllib.as_str())?;
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
        let index =
            interruptible_binary_search_by(scope, self.materials.as_slice(), |work, material| {
                work.compare_str(material.name.as_str(), name)
            })?;
        Ok(index.map(|index| &self.materials.as_slice()[index]))
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
        // A decoded line becomes a consumed record as soon as this parser admits it for
        // processing. Child token, string, and resolution work starts only after that admission.
        let raw_line = scope.with_work(ImportWorkClass::Records, 1, |work| {
            let raw_line = decoded_line.text();
            work.complete_all()?;
            Ok(raw_line)
        })?;
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
            return Ok(());
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
                    let name = owned_string(scope, name, "obj-mtl-material-name")?;
                    current = Some(ParsedMaterial::new(scope, name, order));
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
                        ParsedMetadataValue::F64(value as f64),
                    )?;
                }
            }
            "illum" => {
                if let Some(material) = current.as_mut()
                    && let Some(value) = parse_i64(scope, snapshot, &tokens, line_number, "illum")?
                {
                    insert_metadata(
                        scope,
                        material,
                        "obj:illum",
                        ParsedMetadataValue::I64(value),
                    )?;
                }
            }
            "Ka" | "Ks" => {
                if let Some(material) = current.as_mut()
                    && let Some(value) =
                        parse_vec_metadata(scope, snapshot, &tokens, line_number, keyword)?
                {
                    let metadata_key = if keyword == "Ka" { "obj:ka" } else { "obj:ks" };
                    insert_metadata(
                        scope,
                        material,
                        metadata_key,
                        ParsedMetadataValue::String(value),
                    )?;
                }
            }
            "map_Kd" => {
                if let Some(material) = current.as_mut() {
                    match texture_path_from_map(scope, snapshot, &tokens[1..], line_number)? {
                        TexturePathOutcome::Value(texture_path) => {
                            let path_work = path
                                .as_str()
                                .len()
                                .checked_add(texture_path.len())
                                .ok_or_else(|| {
                                    scope.allocation_failure("obj-mtl-texture-path-work")
                                })?;
                            let (resolved, resolution_allowance) = scope.with_work(
                                ImportWorkClass::WorkUnits,
                                usize_amount(scope, path_work, "obj-mtl-texture-path-work")?,
                                |work| {
                                    let resolution_allowance = scope.metered_opaque_allowance(
                                        ImportStorageClass::FormatNormalization,
                                        usize_amount(
                                            scope,
                                            path_resolution_allowance(
                                                scope,
                                                path,
                                                texture_path.as_str(),
                                            )?,
                                            "obj-mtl-texture-path-resolution",
                                        )?,
                                    )?;
                                    let resolved = path.resolve_from_file(texture_path.as_str());
                                    work.complete_all()?;
                                    Ok((resolved, resolution_allowance))
                                },
                            )?;
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
                                        let uri = owned_string(
                                            scope,
                                            resolved.as_str(),
                                            "obj-mtl-texture-uri",
                                        )?;
                                        material.diffuse_texture = Some(ParsedTexture {
                                            uri,
                                            source_key: "map_Kd",
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
                            drop(resolution_allowance);
                            drop(texture_path);
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
        Ok(())
    })?;

    flush_material(scope, library, current)
}

fn flush_material<'scope>(
    _scope: &'scope ParseScope<'_>,
    library: &mut MaterialLibrary<'scope>,
    material: Option<ParsedMaterial<'scope>>,
) -> ParseResult<()> {
    let Some(material) = material else {
        return Ok(());
    };
    library.materials.try_push(material)
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
            .compare_str(left.name.as_str(), right.name.as_str())?
            .then_with(|| right.order.cmp(&left.order)))
    })?;
    interruptible_dedup_by(scope, &mut library.materials, |work, previous, current| {
        let duplicate =
            work.compare_str(previous.name.as_str(), current.name.as_str())? == Ordering::Equal;
        if duplicate {
            push_warning(
                scope,
                None,
                "obj.mtl_duplicate_material",
                format_args!(
                    "MTL material '{}' was redefined; using the last definition",
                    previous.name.as_str()
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
    value: ParsedMetadataValue<'scope>,
) -> ParseResult<()> {
    let key = ParsedString(owned_string(scope, key, "obj-mtl-metadata-key")?);
    drop(material.metadata.entries.try_insert(key, value)?);
    Ok(())
}

#[derive(Clone, Copy)]
struct MtlToken<'line> {
    text: &'line str,
    start: usize,
    end: usize,
}

struct MtlTokenBuffer<'scope, 'line> {
    values: MeteredVec<'scope, MtlToken<'line>>,
}

impl<'line> std::ops::Deref for MtlTokenBuffer<'_, 'line> {
    type Target = [MtlToken<'line>];

    fn deref(&self) -> &Self::Target {
        self.values.as_slice()
    }
}

fn tokenize_mtl<'scope, 'line>(
    scope: &'scope ParseScope<'_>,
    decoded_line: &'line parser::DecodedLine<'_>,
) -> ParseResult<MtlTokenBuffer<'scope, 'line>> {
    let line = decoded_line.text();
    let mut tokens = MtlTokenBuffer {
        values: scope.metered_vec(ImportStorageClass::FormatNormalization, "obj-mtl-tokens"),
    };
    parser::scan_token_ranges(
        scope,
        line,
        "obj-mtl-token-scan-work",
        |_, _| Ok(()),
        |start, end| {
            scope.with_work_group(
                [
                    (ImportWorkClass::Tokens, 1),
                    (
                        ImportWorkClass::TokenBytes,
                        usize_amount(scope, end - start, "obj-mtl-token-bytes")?,
                    ),
                ],
                |work| {
                    tokens.values.try_push(MtlToken {
                        text: &line[start..end],
                        start,
                        end,
                    })?;
                    work.complete_all()
                },
            )
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
    snapshot: &DependencySnapshot<'_>,
    tokens: &[MtlToken<'_>],
    line_number: u32,
    keyword: &str,
) -> ParseResult<Option<MeteredString<'scope>>> {
    let Some(color) = parse_color(scope, snapshot, tokens, line_number, keyword)? else {
        return Ok(None);
    };
    owned_format(scope, format_args!("{} {} {}", color.r, color.g, color.b)).map(Some)
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
            return scope.with_work(
                ImportWorkClass::StringBytes,
                usize_amount(scope, length, "obj-mtl-texture-path-bytes")?,
                |work| {
                    let mut path = scope.metered_string(
                        ImportStorageClass::FormatNormalization,
                        "obj-mtl-texture-path",
                    );
                    path.try_reserve(length)?;
                    let mut first = true;
                    for chunk in tokens[index..].chunks(64) {
                        work.poll()?;
                        for token in chunk {
                            if !first {
                                path.try_push_char_reserved(' ')?;
                            }
                            if token.text.len() <= parser::SCAN_CHUNK_BYTES {
                                path.try_push_str_reserved(token.text)?;
                            } else {
                                path.try_push_str(token.text)?;
                            }
                            first = false;
                        }
                    }
                    work.complete_all()?;
                    Ok(TexturePathOutcome::Value(path))
                },
            );
        }
    }
    Ok(TexturePathOutcome::Missing)
}

enum TexturePathOutcome<'scope> {
    Missing,
    TooLong,
    Value(MeteredString<'scope>),
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
    value: &str,
    resource: &'static str,
) -> ParseResult<MeteredString<'scope>> {
    parser::enforce_text_value_limit(scope, value.len(), resource)?;
    scope.with_work(
        ImportWorkClass::StringBytes,
        usize_amount(scope, value.len(), resource)?,
        |work| {
            let mut output =
                scope.metered_string(ImportStorageClass::FormatNormalization, resource);
            output.try_push_str(value)?;
            work.complete_all()?;
            Ok(output)
        },
    )
}

fn owned_format<'scope>(
    scope: &'scope ParseScope<'_>,
    arguments: fmt::Arguments<'_>,
) -> ParseResult<MeteredString<'scope>> {
    use fmt::Write as _;

    let mut counter = FormatByteCounter::default();
    counter
        .write_fmt(arguments)
        .map_err(|_| scope.allocation_failure("obj-mtl-formatted-string-length"))?;
    parser::enforce_text_value_limit(scope, counter.bytes, "obj-mtl-formatted-string-bytes")?;
    scope.with_work(
        ImportWorkClass::StringBytes,
        usize_amount(scope, counter.bytes, "obj-mtl-formatted-string-bytes")?,
        |work| {
            let mut output = scope.metered_string(
                ImportStorageClass::FormatNormalization,
                "obj-mtl-formatted-string",
            );
            output.try_reserve(counter.bytes)?;
            output.try_write_fmt(arguments)?;
            work.complete_all()?;
            Ok(output)
        },
    )
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
