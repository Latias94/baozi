use crate::material_library_role;
use crate::parser;
use baozi_import::authoring::{
    Color, SamplerState, TextureAddressMode, TextureTransform, Vec2, Vec3,
};
use baozi_import::{
    ConformedTextInput, DependencyOrigin, DependencySnapshot, DiagnosticCode, DiagnosticSeverity,
    ImportProgressPhase, ImportStorageClass, ImportWorkClass, ImportWorkScope, MeteredOrderedMap,
    MeteredString, MeteredVec, ParseFailure, ParseResult, ParseScope, SourceScopeEntityKind,
    TextInputLine,
};
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
    origins: MeteredVec<'scope, DependencyOrigin<'scope>>,
    next_order: usize,
}

pub(crate) struct ParsedMaterial<'scope> {
    pub name: MeteredString<'scope>,
    order: usize,
    pub base_color: Color,
    pub ambient: Vec3,
    pub specular: Vec3,
    pub emissive: Color,
    pub alpha_mode: ParsedAlphaMode,
    pub metadata: ParsedMetadata<'scope>,
    pub diffuse_texture: Option<ParsedTexture<'scope>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParsedAlphaMode {
    Opaque,
    Blend,
}

pub(crate) struct ParsedTexture<'scope> {
    pub reference: MeteredString<'scope>,
    pub origin: usize,
    pub transform: TextureTransform,
    pub sampler: SamplerState,
}

pub(crate) struct ParsedMetadata<'scope> {
    entries: MeteredOrderedMap<'scope, ParsedString<'scope>, ParsedMetadataValue<'scope>>,
}

struct ParsedString<'scope>(MeteredString<'scope>);

#[allow(
    clippy::large_enum_variant,
    reason = "text metadata keeps its admitted session buffer and lease together without a second allocation"
)]
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
            ambient: Vec3::ZERO,
            specular: Vec3::ZERO,
            emissive: Color::linear_rgba(0.0, 0.0, 0.0, 1.0),
            alpha_mode: ParsedAlphaMode::Opaque,
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
        origins: scope.metered_vec(
            ImportStorageClass::FormatNormalization,
            "obj-mtl-resource-origins",
        ),
        next_order: 0,
    };
    library.origins.try_reserve(mtllibs.len())?;
    let root = scope.root_origin();
    for mtllib in mtllibs {
        let outcome = scope.dependency(&root, &role, mtllib.as_str())?;
        match outcome.into_result() {
            Ok(snapshot) => {
                let origin = library.origins.len();
                let dependency_origin = snapshot.origin();
                parser::with_obj_dependency_text_limit(
                    scope,
                    snapshot.diagnostic_source(),
                    "obj-mtl-text",
                    scope.limits().max_dependency_asset_bytes,
                    |text, recoveries| {
                        parse_mtl(scope, &snapshot, text, recoveries, origin, &mut library)
                    },
                )?;
                drop(snapshot);
                library.origins.try_push_reserved(dependency_origin)?;
            }
            Err(evidence) => push_warning(
                scope,
                None,
                "obj.mtl_unavailable",
                format_args!(
                    "optional material library '{}' is unavailable; owner={:?}, cache={:?}",
                    evidence.normalized_path(),
                    evidence.owner(),
                    evidence.cache_disposition()
                ),
            )?,
        }
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

    pub(crate) fn origin(&self, index: usize) -> Option<&DependencyOrigin<'scope>> {
        self.origins.as_slice().get(index)
    }
}

fn parse_mtl<'scope>(
    scope: &'scope ParseScope<'_>,
    snapshot: &DependencySnapshot<'_>,
    text: &ConformedTextInput<'_>,
    recoveries: &mut baozi_import::TextInputRecoveryTracker<'_, '_>,
    origin: usize,
    library: &mut MaterialLibrary<'scope>,
) -> ParseResult<()> {
    let mut current = None;

    for decoded_line in text.lines() {
        let line_number = u32::try_from(decoded_line.number())
            .map_err(|_| scope.resource_limit_exceeded("line-count", u64::from(u32::MAX)))?;
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
            recoveries,
            &decoded_line,
            allowed_value,
            tokens.comment_start,
        )?;
        let Some(keyword) = keyword else {
            continue;
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
                        material.alpha_mode = ParsedAlphaMode::Blend;
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
                        material.alpha_mode = ParsedAlphaMode::Blend;
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
                if let Some(material) = current.as_mut() {
                    let color = parse_legacy_color(scope, &tokens, line_number)?;
                    let metadata_key = if keyword == "Ka" { "obj:ka" } else { "obj:ks" };
                    let value = owned_string(
                        scope,
                        trailing_text_after_keyword(raw_line, &tokens).unwrap_or_default(),
                        "obj-mtl-color-source",
                    )?;
                    insert_metadata(
                        scope,
                        material,
                        metadata_key,
                        ParsedMetadataValue::String(value),
                    )?;
                    let mut factor = Vec3::new(color.r, color.g, color.b);
                    if color.r < 0.0 || color.g < 0.0 || color.b < 0.0 {
                        record_material_loss(
                            scope,
                            snapshot,
                            line_number,
                            tokens[0],
                            if keyword == "Ka" {
                                "obj.mtl_ambient_omitted"
                            } else {
                                "obj.mtl_specular_omitted"
                            },
                            "negative-legacy-factor",
                        )?;
                        factor = Vec3::ZERO;
                    }
                    if keyword == "Ka" {
                        material.ambient = factor;
                    } else {
                        material.specular = factor;
                    }
                }
            }
            "map_Kd" => {
                // Validate the complete option prefix before consulting omission authority.
                let arguments = parse_map_arguments(scope, &tokens[1..], line_number)?;
                if let Some(material) = current.as_mut() {
                    let (transform, sampler) = map_texture_options(
                        scope,
                        snapshot,
                        &tokens[1..],
                        &arguments,
                        line_number,
                    )?;
                    let path = &tokens[1 + arguments.path_index..];
                    match texture_path_from_tokens(scope, raw_line, path)? {
                        TexturePathOutcome::Value(texture_path) => {
                            let source = owned_string(
                                scope,
                                trailing_text_after_keyword(raw_line, &tokens).unwrap_or_default(),
                                "obj-mtl-map-source",
                            )?;
                            insert_metadata(
                                scope,
                                material,
                                "obj:map_kd",
                                ParsedMetadataValue::String(source),
                            )?;
                            material.diffuse_texture = Some(ParsedTexture {
                                reference: texture_path,
                                origin,
                                transform,
                                sampler,
                            });
                        }
                        TexturePathOutcome::TooLong => push_dependency_warning(
                            scope,
                            snapshot,
                            Some((line_number, 1)),
                            "obj.mtl_texture_path_limit",
                            format_args!("MTL map_Kd texture path exceeded max_string_bytes"),
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
    }

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
    column: u32,
}

struct MtlTokenBuffer<'scope, 'line> {
    values: MeteredVec<'scope, MtlToken<'line>>,
    comment_start: Option<usize>,
}

impl<'line> std::ops::Deref for MtlTokenBuffer<'_, 'line> {
    type Target = [MtlToken<'line>];

    fn deref(&self) -> &Self::Target {
        self.values.as_slice()
    }
}

fn tokenize_mtl<'scope, 'line>(
    scope: &'scope ParseScope<'_>,
    decoded_line: &'line TextInputLine<'_, '_>,
) -> ParseResult<MtlTokenBuffer<'scope, 'line>> {
    let line = decoded_line.text();
    let mut tokens = MtlTokenBuffer {
        values: scope.metered_vec(ImportStorageClass::FormatNormalization, "obj-mtl-tokens"),
        comment_start: None,
    };
    tokens.comment_start = parser::scan_token_ranges(
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
                    let column = start
                        .checked_add(1)
                        .and_then(|column| u32::try_from(column).ok())
                        .ok_or_else(|| scope.resource_limit_exceeded("line-columns", u64::MAX))?;
                    tokens.values.try_push(MtlToken {
                        text: &line[start..end],
                        start,
                        end,
                        column,
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

fn parse_legacy_color(
    scope: &ParseScope<'_>,
    tokens: &[MtlToken<'_>],
    line_number: u32,
) -> ParseResult<Color> {
    if !(2..=4).contains(&tokens.len()) {
        return Err(map_syntax_failure(
            scope,
            tokens[0],
            line_number,
            "legacy-color-component-count",
        ));
    }
    let red = parse_required_map_number(scope, tokens[1], line_number)?;
    let green = match tokens.get(2) {
        Some(token) => parse_required_map_number(scope, *token, line_number)?,
        None => red,
    };
    let blue = match tokens.get(3) {
        Some(token) => parse_required_map_number(scope, *token, line_number)?,
        None => red,
    };
    Ok(Color::linear_rgba(red, green, blue, 1.0))
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

struct MapArguments {
    offset: [f32; 3],
    scale: [f32; 3],
    clamp: bool,
    offset_index: Option<usize>,
    scale_index: Option<usize>,
    path_index: usize,
}

fn parse_map_arguments(
    scope: &ParseScope<'_>,
    tokens: &[MtlToken<'_>],
    line_number: u32,
) -> ParseResult<MapArguments> {
    let mut arguments = MapArguments {
        offset: [0.0; 3],
        scale: [1.0; 3],
        clamp: false,
        offset_index: None,
        scale_index: None,
        path_index: 0,
    };
    let mut index = 0;
    while let Some(option) = tokens.get(index).copied() {
        scope.poll()?;
        // A stray negative number is an extra operand, not an unknown map option.
        // Numeric filenames can be disambiguated with a prefix such as ./.
        if option.text.parse::<f32>().is_ok() {
            return Err(map_syntax_failure(
                scope,
                option,
                line_number,
                "unexpected-numeric-operand",
            ));
        }
        if !option.text.starts_with('-') {
            arguments.path_index = index;
            return Ok(arguments);
        }
        let following = &tokens[index + 1..];
        let consumed = match option.text {
            "-o" | "-s" | "-t" | "-mm" | "-bm" | "-boost" => {
                let (minimum, maximum) = match option.text {
                    "-o" | "-s" | "-t" => (1, 3),
                    "-mm" => (2, 2),
                    _ => (1, 1),
                };
                if following.len() < minimum {
                    return Err(map_syntax_failure(
                        scope,
                        option,
                        line_number,
                        "missing-option-operand",
                    ));
                }
                let mut values = if option.text == "-s" {
                    [1.0; 3]
                } else {
                    [0.0; 3]
                };
                let count = numeric_option_arity(following, maximum).max(minimum);
                for (value, token) in values.iter_mut().zip(following).take(count) {
                    *value = parse_required_map_number(scope, *token, line_number)?;
                }
                match option.text {
                    "-o" => {
                        arguments.offset = values;
                        arguments.offset_index = Some(index);
                    }
                    "-s" => {
                        arguments.scale = values;
                        arguments.scale_index = Some(index);
                    }
                    "-boost" if values[0] < 0.0 => {
                        return Err(map_syntax_failure(
                            scope,
                            option,
                            line_number,
                            "negative-boost",
                        ));
                    }
                    _ => {}
                }
                count
            }
            "-clamp" | "-type" | "-texres" => {
                let operand = following.first().copied().ok_or_else(|| {
                    map_syntax_failure(scope, option, line_number, "missing-option-operand")
                })?;
                let valid = match option.text {
                    "-clamp" => {
                        arguments.clamp = operand.text == "on";
                        matches!(operand.text, "on" | "off")
                    }
                    "-type" => matches!(
                        operand.text,
                        "sphere"
                            | "cube_top"
                            | "cube_bottom"
                            | "cube_front"
                            | "cube_back"
                            | "cube_left"
                            | "cube_right"
                    ),
                    "-texres" => {
                        enforce_map_numeric_limit(scope, operand.text)?;
                        operand.text.parse::<u32>().is_ok_and(|value| value > 0)
                    }
                    _ => false,
                };
                if !valid {
                    return Err(map_syntax_failure(
                        scope,
                        operand,
                        line_number,
                        "invalid-option-operand",
                    ));
                }
                1
            }
            _ => {
                // Unknown options retain the existing warning-only, one-operand profile.
                // The finite recognized branch list below is governed by semantic loss.
                if following
                    .first()
                    .is_none_or(|token| token.text.starts_with('-'))
                {
                    return Err(map_syntax_failure(
                        scope,
                        option,
                        line_number,
                        "missing-option-operand",
                    ));
                }
                1
            }
        };
        index += 1 + consumed;
    }
    Err(scope.format_failure(crate::ObjFailure::statement(
        "map_Kd",
        "missing-texture-path",
        line_number,
        1,
    )))
}

fn enforce_map_numeric_limit(scope: &ParseScope<'_>, text: &str) -> ParseResult<()> {
    scope.enforce_resource_limit(
        "obj-mtl-number-bytes",
        usize_amount(scope, text.len(), "obj-mtl-number-bytes")?,
        MAX_NUMERIC_LITERAL_BYTES as u64,
    )
}

fn parse_required_map_number(
    scope: &ParseScope<'_>,
    token: MtlToken<'_>,
    line_number: u32,
) -> ParseResult<f32> {
    enforce_map_numeric_limit(scope, token.text)?;
    let value = token
        .text
        .parse::<f32>()
        .map_err(|_| map_syntax_failure(scope, token, line_number, "invalid-numeric-operand"))?;
    if !value.is_finite() {
        return Err(map_syntax_failure(
            scope,
            token,
            line_number,
            "non-finite-numeric-operand",
        ));
    }
    Ok(value)
}

fn map_syntax_failure(
    scope: &ParseScope<'_>,
    token: MtlToken<'_>,
    line_number: u32,
    reason: &'static str,
) -> ParseFailure {
    scope.format_failure(crate::ObjFailure::statement(
        "mtl-material",
        reason,
        line_number,
        token.column,
    ))
}

fn map_texture_options(
    scope: &ParseScope<'_>,
    snapshot: &DependencySnapshot<'_>,
    tokens: &[MtlToken<'_>],
    arguments: &MapArguments,
    line_number: u32,
) -> ParseResult<(TextureTransform, SamplerState)> {
    let mut index = 0;
    while index < arguments.path_index {
        scope.poll()?;
        let option = tokens[index];
        let code = match option.text {
            "-t" => Some("obj.mtl_texture_turbulence_omitted"),
            "-mm" => Some("obj.mtl_texture_range_omitted"),
            "-bm" => Some("obj.mtl_texture_bump_multiplier_omitted"),
            "-boost" => Some("obj.mtl_texture_boost_omitted"),
            "-type" => Some("obj.mtl_texture_type_omitted"),
            "-texres" => Some("obj.mtl_texture_resolution_omitted"),
            _ => None,
        };
        if let Some(code) = code {
            record_material_loss(
                scope,
                snapshot,
                line_number,
                option,
                code,
                "unsupported-option",
            )?;
        } else if !is_known_map_option(option.text) {
            push_dependency_warning(
                scope,
                snapshot,
                Some((line_number, option.column)),
                "obj.mtl_unsupported_texture_option",
                format_args!("MTL map_Kd option '{}' is not supported", option.text),
            )?;
        }
        index += 1 + map_option_arity(option.text, &tokens[index + 1..]);
    }

    for (nonneutral, option_index, code) in [
        (
            arguments.offset[2] != 0.0,
            arguments.offset_index,
            "obj.mtl_texture_offset_w_omitted",
        ),
        (
            arguments.scale[2] != 1.0,
            arguments.scale_index,
            "obj.mtl_texture_scale_w_omitted",
        ),
    ] {
        if nonneutral && let Some(index) = option_index {
            record_material_loss(
                scope,
                snapshot,
                line_number,
                tokens[index],
                code,
                "nonrepresentable-third-component",
            )?;
        }
    }

    // Baozi OBJ/MTL UV Sampling Profile: q = offset + scale * p. Retain authored UV
    // components directly; the original manual does not uniquely specify this equation.
    let scale = Vec2::new(arguments.scale[0], arguments.scale[1]);
    let offset = Vec2::new(arguments.offset[0], arguments.offset[1]);
    let transform = TextureTransform::try_new(offset, 0.0, scale).map_err(|_| {
        scope.format_failure(crate::ObjFailure::internal("invalid-texture-transform"))
    })?;
    let default = SamplerState::CANONICAL_DEFAULT;
    let address = if arguments.clamp {
        // The manual leaves the underlying material unchanged outside the single map copy.
        TextureAddressMode::Decal
    } else {
        TextureAddressMode::Repeat
    };
    let sampler = SamplerState::new(
        default.minification(),
        default.magnification(),
        default.mipmap(),
        address,
        address,
        default.address_w(),
    );
    Ok((transform, sampler))
}

fn record_material_loss(
    scope: &ParseScope<'_>,
    snapshot: &DependencySnapshot<'_>,
    line_number: u32,
    token: MtlToken<'_>,
    code: &'static str,
    reason: &'static str,
) -> ParseResult<()> {
    let source = snapshot.diagnostic_source();
    let path = owned_format(
        scope,
        format_args!(
            "/dependencies/{}/records/{line_number}/{}",
            source.path().as_str(),
            token.text,
        ),
    )?;
    let code = DiagnosticCode::new(code).expect("static OBJ diagnostic code must be valid");
    scope.record_source_semantic_loss(
        code,
        SourceScopeEntityKind::Material,
        path.as_str(),
        |details| {
            details.string("dependency", source.path().as_str());
            details.unsigned("line", u64::from(line_number));
            details.unsigned("column", u64::from(token.column));
            details.string("option", token.text);
            details.string("reason", reason);
        },
        format_args!(
            "MTL '{}' at line {line_number} has unrepresented '{}' semantics ({reason}); omission requires explicit source semantic loss authorization",
            source.path().as_str(), token.text,
        ),
    )
}

fn texture_path_from_tokens<'scope>(
    scope: &'scope ParseScope<'_>,
    raw_line: &str,
    tokens: &[MtlToken<'_>],
) -> ParseResult<TexturePathOutcome<'scope>> {
    let first = tokens.first().ok_or_else(|| {
        scope.format_failure(crate::ObjFailure::internal("missing-map-path-token"))
    })?;
    let last = tokens.last().ok_or_else(|| {
        scope.format_failure(crate::ObjFailure::internal("missing-map-path-token"))
    })?;
    let path = &raw_line[first.start..last.end];
    if path.len() > parser::max_text_value_bytes(scope) {
        return Ok(TexturePathOutcome::TooLong);
    }
    owned_string(scope, path, "obj-mtl-texture-path").map(TexturePathOutcome::Value)
}

#[allow(
    clippy::large_enum_variant,
    reason = "the metered string retains its reservation inline; boxing would add unmetered allocation"
)]
enum TexturePathOutcome<'scope> {
    TooLong,
    Value(MeteredString<'scope>),
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
