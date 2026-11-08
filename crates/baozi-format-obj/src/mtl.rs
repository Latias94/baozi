use crate::parser;
use crate::{ObjImportOptions, material_library_role};
use baozi_import::authoring::{
    Color, SamplerState, TextureAddressMode, TextureTransform, Vec2, Vec3,
};
use baozi_import::{
    ConformedTextInput, DependencyOrigin, DependencySnapshot, DiagnosticCode, DiagnosticSeverity,
    ImportProgressPhase, ImportStorageClass, ImportWorkClass, MeteredOrderedMap, MeteredString,
    MeteredVec, ParseChildren, ParseFailure, ParseParser, ParseResult, ParseRoot, ParseServices,
    SourceScopeEntityKind, TextInputLine,
};
use std::cmp::Ordering;
use std::fmt;

use parser::usize_amount;

const INTERRUPTIBLE_WORK_CHUNK: u64 = 64;
const MAX_NUMERIC_LITERAL_BYTES: usize = 128;

pub(crate) struct ComparisonWork<'meter, 'parser, 'frame, 'ctx, 'op, 'session> {
    work: &'meter mut ParseParser<'parser, 'frame, 'ctx, 'op, 'session, 1, true>,
    leaves_until_poll: u64,
}

impl<'meter, 'parser, 'frame, 'ctx, 'op, 'session>
    ComparisonWork<'meter, 'parser, 'frame, 'ctx, 'op, 'session>
{
    fn new(work: &'meter mut ParseParser<'parser, 'frame, 'ctx, 'op, 'session, 1, true>) -> Self {
        Self {
            work,
            leaves_until_poll: INTERRUPTIBLE_WORK_CHUNK,
        }
    }

    pub(crate) fn run_leaf<T>(&mut self, compare: impl FnOnce() -> T) -> ParseResult<T> {
        // Admit only the next bounded comparison, then publish its actual completion.
        // Polling is batched independently so a short tail never consumes unused work.
        self.work.replenish(1)?;
        let value = self.work.run([1], || Ok::<_, ParseFailure>(compare()))?;
        self.leaves_until_poll -= 1;
        if self.leaves_until_poll == 0 {
            self.work.poll()?;
            self.leaves_until_poll = INTERRUPTIBLE_WORK_CHUNK;
        }
        Ok(value)
    }

    pub(crate) fn compare_str(&mut self, left: &str, right: &str) -> ParseResult<Ordering> {
        for (left, right) in left.as_bytes().iter().zip(right.as_bytes()) {
            match self.run_leaf(|| left.cmp(right))? {
                Ordering::Equal => {}
                ordering => return Ok(ordering),
            }
        }
        self.run_leaf(|| left.len().cmp(&right.len()))
    }
}

pub(crate) fn interruptible_sort_by<'op, 'session, T>(
    children: &mut ParseChildren<'_, 'op, 'session>,
    values: &mut MeteredVec<'_, T>,
    mut compare: impl FnMut(
        &mut ComparisonWork<'_, '_, '_, '_, 'op, 'session>,
        &T,
        &T,
    ) -> ParseResult<Ordering>,
) -> ParseResult<()> {
    if values.len() <= 1 {
        return Ok(());
    }
    let comparison_work = children.bind_group([ImportWorkClass::WorkUnits])?;
    children.child(&comparison_work, [0], |ledger_work| {
        let mut work = ComparisonWork::new(ledger_work);
        values.try_sort_unstable_by(|left, right| compare(&mut work, left, right))
    })
}

pub(crate) fn interruptible_dedup_by<'op, 'session, T>(
    children: &mut ParseChildren<'_, 'op, 'session>,
    values: &mut MeteredVec<'_, T>,
    mut equivalent: impl FnMut(
        &mut ComparisonWork<'_, '_, '_, '_, 'op, 'session>,
        &T,
        &T,
    ) -> ParseResult<bool>,
) -> ParseResult<()> {
    if values.len() <= 1 {
        return Ok(());
    }
    let previous_len = values.len();
    let comparison_work = children.bind_group([ImportWorkClass::WorkUnits])?;
    let retained_len = children.child(&comparison_work, [0], |ledger_work| {
        let mut work = ComparisonWork::new(ledger_work);
        let retained_len = values
            .try_partition_dedup_by(|previous, current| equivalent(&mut work, previous, current))?;
        Ok::<_, ParseFailure>(retained_len)
    })?;
    for_work_chunks(
        children,
        previous_len - retained_len,
        "obj-dedup-drop-work",
        |work, range| {
            work.run([range.len() as u64], || {
                values.truncate(values.len() - range.len());
                Ok::<_, ParseFailure>(())
            })
        },
    )
}

pub(crate) fn interruptible_binary_search_by<'op, 'session, T>(
    children: &mut ParseChildren<'_, 'op, 'session>,
    values: &[T],
    mut compare: impl FnMut(
        &mut ComparisonWork<'_, '_, '_, '_, 'op, 'session>,
        &T,
    ) -> ParseResult<Ordering>,
) -> ParseResult<Option<usize>> {
    if values.is_empty() {
        return Ok(None);
    }
    let comparison_work = children.bind_group([ImportWorkClass::WorkUnits])?;
    children.child(&comparison_work, [0], |ledger_work| {
        let mut work = ComparisonWork::new(ledger_work);
        let mut left = 0usize;
        let mut right = values.len();
        let mut found = None;
        while left < right {
            let middle = left + (right - left) / 2;
            match compare(&mut work, &values[middle])? {
                Ordering::Less => left = middle + 1,
                Ordering::Greater => right = middle,
                Ordering::Equal => {
                    found = Some(middle);
                    break;
                }
            }
        }
        Ok(found)
    })
}

// Visitors publish completed items or atomic ranges before any later fallible work.
pub(crate) fn for_work_chunks<'op, 'session>(
    children: &mut ParseChildren<'_, 'op, 'session>,
    len: usize,
    resource: &'static str,
    mut visit: impl FnMut(
        &mut ParseParser<'_, '_, '_, 'op, 'session, 1, true>,
        std::ops::Range<usize>,
    ) -> ParseResult<()>,
) -> ParseResult<()> {
    let work_group = children.bind_group([ImportWorkClass::WorkUnits])?;
    let mut start = 0usize;
    while start < len {
        let end = start
            .checked_add(INTERRUPTIBLE_WORK_CHUNK as usize)
            .map_or(len, |end| end.min(len));
        let amount = usize_amount(&children.services()?, end - start, resource)?;
        children.child(&work_group, [amount], |work| visit(work, start..end))?;
        start = end;
    }
    Ok(())
}

pub(crate) fn interruptible_str_eq(
    children: &mut ParseChildren<'_, '_, '_>,
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
    for_work_chunks(children, left.len(), resource, |work, range| {
        work.run([range.len() as u64], || {
            if left[range.clone()] != right[range] {
                equal = false;
            }
            Ok::<_, ParseFailure>(())
        })
    })?;
    Ok(equal)
}

pub(crate) struct MaterialLibrary<'session> {
    materials: MeteredVec<'session, ParsedMaterial<'session>>,
    origins: MeteredVec<'session, DependencyOrigin<'session>>,
    next_order: usize,
}

pub(crate) struct ParsedMaterial<'session> {
    pub name: MeteredString<'session>,
    order: usize,
    pub base_color: Color,
    pub ambient: Vec3,
    pub specular: Vec3,
    pub emissive: Color,
    pub metadata: ParsedMetadata<'session>,
    pub diffuse_texture: Option<ParsedTexture<'session>>,
}

pub(crate) struct ParsedTexture<'session> {
    pub reference: MeteredString<'session>,
    pub origin: usize,
    pub transform: TextureTransform,
    pub sampler: SamplerState,
}

pub(crate) struct ParsedMetadata<'session> {
    entries: MeteredOrderedMap<'session, ParsedString<'session>, ParsedMetadataValue<'session>>,
}

struct ParsedString<'session>(MeteredString<'session>);

#[allow(
    clippy::large_enum_variant,
    reason = "text metadata keeps its admitted session buffer and lease together without a second allocation"
)]
pub(crate) enum ParsedMetadataValue<'session> {
    I64(i64),
    F64(f64),
    String(MeteredString<'session>),
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

impl<'session> ParsedMaterial<'session> {
    fn new(
        exec: &mut ParseServices<'_, '_, 'session>,
        name: MeteredString<'session>,
        order: usize,
    ) -> Self {
        Self {
            name,
            order,
            base_color: Color::WHITE,
            ambient: Vec3::ZERO,
            specular: Vec3::ZERO,
            emissive: Color::linear_rgba(0.0, 0.0, 0.0, 1.0),
            metadata: ParsedMetadata {
                entries: exec.metered_ordered_map(
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

pub(crate) fn load_libraries<'session>(
    root: &mut ParseRoot<'_, '_, 'session>,
    options: &ObjImportOptions,
    mtllibs: &[MeteredString<'session>],
) -> ParseResult<MaterialLibrary<'session>> {
    let role = material_library_role();
    let exec = &mut root.services()?;
    let mut library = MaterialLibrary {
        materials: exec.metered_vec(ImportStorageClass::FormatNormalization, "obj-mtl-materials"),
        origins: exec.metered_vec(
            ImportStorageClass::FormatNormalization,
            "obj-mtl-resource-origins",
        ),
        next_order: 0,
    };
    library.origins.try_reserve(exec, mtllibs.len())?;
    let origin_root = exec.root_origin();
    let token_work = parser::TokenWorkLayouts::bind(&mut root.children()?)?;
    for mtllib in mtllibs {
        let outcome = root.dependency(&origin_root, &role, mtllib.as_str())?;
        match outcome.into_result() {
            Ok(snapshot) => {
                let origin = library.origins.len();
                let dependency_origin = snapshot.origin();
                let raw_limit = root.services()?.limits().max_dependency_asset_bytes;
                parser::with_obj_dependency_text_limit(
                    &mut root.children()?,
                    options,
                    snapshot.diagnostic_source(),
                    "obj-mtl-text",
                    raw_limit,
                    |exec, text, recoveries| {
                        parse_mtl(
                            exec,
                            &token_work,
                            &snapshot,
                            text,
                            recoveries,
                            origin,
                            &mut library,
                        )
                    },
                )?;
                drop(snapshot);
                library.origins.try_push_reserved(dependency_origin)?;
            }
            Err(evidence) => push_warning(
                &mut root.services()?,
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
    finalize_materials(&mut root.children()?, &mut library)?;
    Ok(library)
}

impl<'session> MaterialLibrary<'session> {
    pub(crate) fn find<'library>(
        &'library self,
        children: &mut ParseChildren<'_, '_, '_>,
        name: &str,
    ) -> ParseResult<Option<&'library ParsedMaterial<'session>>> {
        let index = interruptible_binary_search_by(
            children,
            self.materials.as_slice(),
            |work, material| work.compare_str(material.name.as_str(), name),
        )?;
        Ok(index.map(|index| &self.materials.as_slice()[index]))
    }

    pub(crate) fn origin(&self, index: usize) -> Option<&DependencyOrigin<'session>> {
        self.origins.as_slice().get(index)
    }
}

fn parse_mtl<'op, 'session>(
    children: &mut ParseChildren<'_, 'op, 'session>,
    token_work: &parser::TokenWorkLayouts<'op>,
    snapshot: &DependencySnapshot<'_>,
    text: &ConformedTextInput<'_>,
    recoveries: &mut baozi_import::TextInputRecoveryTracker<'_, 'session>,
    origin: usize,
    library: &mut MaterialLibrary<'session>,
) -> ParseResult<()> {
    let mut current = None;
    let mut tokens = MtlTokenBuffer {
        values: children
            .services()?
            .metered_vec(ImportStorageClass::FormatNormalization, "obj-mtl-tokens"),
        comment_start: None,
    };
    let mut scan_scratch = parser::TokenScanScratch::new();

    let record_work = children.bind_group([ImportWorkClass::Records])?;
    for decoded_line in text.lines() {
        let line_number = u32::try_from(decoded_line.number()).map_err(|_| {
            children
                .error_view()
                .resource_limit_exceeded("line-count", u64::from(u32::MAX))
        })?;
        // A decoded line becomes a consumed record as soon as this parser admits it for
        // processing. Child token, string, and resolution work starts only after that admission.
        let raw_line = children.child(&record_work, [1], |work| {
            work.run([1], || Ok::<_, ParseFailure>(decoded_line.text()))
        })?;
        tokenize_mtl(
            children,
            token_work,
            &mut scan_scratch,
            &decoded_line,
            &mut tokens,
        )?;
        let exec = &mut children.services()?;
        let keyword = tokens.first().map(|token| token.text);
        let allowed_value = match keyword {
            Some("newmtl") => trailing_mtl_token_range(&tokens),
            Some("map_Kd") => recovery_map_path_range(exec, &tokens[1..])?,
            _ => None,
        };
        parser::authorize_line_recoveries(
            exec,
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
                flush_material(exec, library, current.take())?;
                let name = trailing_text_after_keyword(raw_line, &tokens).unwrap_or_default();
                if name.is_empty() {
                    push_dependency_warning(
                        exec,
                        snapshot,
                        Some((line_number, 1)),
                        "obj.mtl_empty_name",
                        format_args!("MTL newmtl has no material name"),
                    )?;
                } else if name.len() > parser::max_text_value_bytes(exec) {
                    push_dependency_warning(
                        exec,
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
                        .ok_or_else(|| exec.allocation_failure("obj-mtl-material-order"))?;
                    let name = owned_string(children, name, "obj-mtl-material-name")?;
                    current = Some(ParsedMaterial::new(&mut children.services()?, name, order));
                }
            }
            "Kd" => {
                if let Some(material) = current.as_mut()
                    && let Some(color) = parse_color(exec, snapshot, &tokens, line_number, "Kd")?
                {
                    material.base_color.r = color.r;
                    material.base_color.g = color.g;
                    material.base_color.b = color.b;
                }
            }
            "d" => {
                if let Some(material) = current.as_mut()
                    && let Some(alpha) = parse_scalar(exec, snapshot, &tokens, line_number, "d")?
                {
                    material.base_color.a = alpha;
                }
            }
            "Tr" => {
                if let Some(material) = current.as_mut()
                    && let Some(transparency) =
                        parse_scalar(exec, snapshot, &tokens, line_number, "Tr")?
                {
                    material.base_color.a = 1.0 - transparency;
                }
            }
            "Ke" => {
                if let Some(material) = current.as_mut()
                    && let Some(color) = parse_color(exec, snapshot, &tokens, line_number, "Ke")?
                {
                    material.emissive = color;
                }
            }
            "Ns" | "Ni" => {
                if let Some(material) = current.as_mut()
                    && let Some(value) =
                        parse_scalar(exec, snapshot, &tokens, line_number, keyword)?
                {
                    let metadata_key = if keyword == "Ns" { "obj:ns" } else { "obj:ni" };
                    insert_metadata(
                        children,
                        material,
                        metadata_key,
                        ParsedMetadataValue::F64(value as f64),
                    )?;
                }
            }
            "illum" => {
                if let Some(material) = current.as_mut()
                    && let Some(value) = parse_i64(exec, snapshot, &tokens, line_number, "illum")?
                {
                    insert_metadata(
                        children,
                        material,
                        "obj:illum",
                        ParsedMetadataValue::I64(value),
                    )?;
                }
            }
            "Ka" | "Ks" => {
                if let Some(material) = current.as_mut() {
                    let color = parse_legacy_color(exec, &tokens, line_number)?;
                    let metadata_key = if keyword == "Ka" { "obj:ka" } else { "obj:ks" };
                    let value = owned_string(
                        children,
                        trailing_text_after_keyword(raw_line, &tokens).unwrap_or_default(),
                        "obj-mtl-color-source",
                    )?;
                    insert_metadata(
                        children,
                        material,
                        metadata_key,
                        ParsedMetadataValue::String(value),
                    )?;
                    let mut factor = Vec3::new(color.r, color.g, color.b);
                    if color.r < 0.0 || color.g < 0.0 || color.b < 0.0 {
                        record_material_loss(
                            children,
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
                let arguments = parse_map_arguments(exec, &tokens[1..], line_number)?;
                if let Some(material) = current.as_mut() {
                    let (transform, sampler) = map_texture_options(
                        children,
                        snapshot,
                        &tokens[1..],
                        &arguments,
                        line_number,
                    )?;
                    let path = &tokens[1 + arguments.path_index..];
                    match texture_path_from_tokens(children, raw_line, path)? {
                        TexturePathOutcome::Value(texture_path) => {
                            let source = owned_string(
                                children,
                                trailing_text_after_keyword(raw_line, &tokens).unwrap_or_default(),
                                "obj-mtl-map-source",
                            )?;
                            insert_metadata(
                                children,
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
                            &mut children.services()?,
                            snapshot,
                            Some((line_number, 1)),
                            "obj.mtl_texture_path_limit",
                            format_args!("MTL map_Kd texture path exceeded max_string_bytes"),
                        )?,
                    }
                }
            }
            keyword => push_dependency_warning(
                exec,
                snapshot,
                Some((line_number, 1)),
                "obj.mtl_unknown_statement",
                format_args!("MTL statement '{keyword}' is not recognized"),
            )?,
        }
    }

    drop(tokens);
    flush_material(&mut children.services()?, library, current)
}

fn flush_material<'session>(
    exec: &mut ParseServices<'_, '_, 'session>,
    library: &mut MaterialLibrary<'session>,
    material: Option<ParsedMaterial<'session>>,
) -> ParseResult<()> {
    let Some(material) = material else {
        return Ok(());
    };
    library.materials.try_push(exec, material)
}

fn finalize_materials(
    children: &mut ParseChildren<'_, '_, '_>,
    library: &mut MaterialLibrary<'_>,
) -> ParseResult<()> {
    if library.materials.len() <= 1 {
        return Ok(());
    }

    interruptible_sort_by(
        children,
        &mut library.materials,
        |work, left, right| match work.compare_str(left.name.as_str(), right.name.as_str())? {
            Ordering::Equal => work.run_leaf(|| right.order.cmp(&left.order)),
            ordering => Ok(ordering),
        },
    )?;
    interruptible_dedup_by(
        children,
        &mut library.materials,
        |work, previous, current| {
            let duplicate =
                work.compare_str(previous.name.as_str(), current.name.as_str())? == Ordering::Equal;
            if duplicate {
                push_warning(
                    &mut work.work.services()?,
                    None,
                    "obj.mtl_duplicate_material",
                    format_args!(
                        "MTL material '{}' was redefined; using the last definition",
                        previous.name.as_str()
                    ),
                )?;
            }
            Ok(duplicate)
        },
    )
}

fn insert_metadata<'session>(
    children: &mut ParseChildren<'_, '_, 'session>,
    material: &mut ParsedMaterial<'session>,
    key: &'static str,
    value: ParsedMetadataValue<'session>,
) -> ParseResult<()> {
    let key = ParsedString(owned_string(children, key, "obj-mtl-metadata-key")?);
    drop(
        material
            .metadata
            .entries
            .try_insert(&mut children.services()?, key, value)?,
    );
    Ok(())
}

#[derive(Clone, Copy)]
struct MtlToken<'input> {
    text: &'input str,
    start: usize,
    end: usize,
    column: u32,
}

struct MtlTokenBuffer<'session, 'input> {
    values: MeteredVec<'session, MtlToken<'input>>,
    comment_start: Option<usize>,
}

impl<'input> std::ops::Deref for MtlTokenBuffer<'_, 'input> {
    type Target = [MtlToken<'input>];

    fn deref(&self) -> &Self::Target {
        self.values.as_slice()
    }
}

fn tokenize_mtl<'op, 'session, 'input>(
    children: &mut ParseChildren<'_, 'op, 'session>,
    token_work: &parser::TokenWorkLayouts<'op>,
    scan_scratch: &mut parser::TokenScanScratch,
    decoded_line: &TextInputLine<'input, '_>,
    tokens: &mut MtlTokenBuffer<'session, 'input>,
) -> ParseResult<()> {
    tokens.values.truncate(0);
    tokens.comment_start = None;
    let line = decoded_line.text();
    tokens.comment_start = parser::scan_token_ranges(
        children,
        &token_work.scan,
        scan_scratch,
        line,
        "obj-mtl-token-scan-work",
        |_, _| Ok(()),
        |children, start, end| {
            let amounts = [
                1,
                usize_amount(&children.services()?, end - start, "obj-mtl-token-bytes")?,
            ];
            let errors = children.error_view();
            children.run_piece_with_storage::<_, crate::ObjFailure, _>(
                &token_work.tokens,
                amounts,
                |mut storage| {
                    let column = start
                        .checked_add(1)
                        .and_then(|column| u32::try_from(column).ok())
                        .ok_or_else(|| errors.resource_limit_exceeded("line-columns", u64::MAX))?;
                    storage.push_vec(
                        &mut tokens.values,
                        MtlToken {
                            text: &line[start..end],
                            start,
                            end,
                            column,
                        },
                    )?;
                    Ok(((), amounts))
                },
            )
        },
    )?;
    Ok(())
}

fn parse_color(
    exec: &mut ParseServices<'_, '_, '_>,
    snapshot: &DependencySnapshot<'_>,
    tokens: &[MtlToken<'_>],
    line_number: u32,
    keyword: &str,
) -> ParseResult<Option<Color>> {
    if tokens.len() < 4 {
        push_dependency_warning(
            exec,
            snapshot,
            Some((line_number, 1)),
            "obj.mtl_bad_color",
            format_args!("MTL {keyword} requires three components"),
        )?;
        return Ok(None);
    }
    let Some(red) = parse_f32_warning(exec, snapshot, tokens[1].text, line_number, keyword)? else {
        return Ok(None);
    };
    let Some(green) = parse_f32_warning(exec, snapshot, tokens[2].text, line_number, keyword)?
    else {
        return Ok(None);
    };
    let Some(blue) = parse_f32_warning(exec, snapshot, tokens[3].text, line_number, keyword)?
    else {
        return Ok(None);
    };
    Ok(Some(Color::linear_rgba(red, green, blue, 1.0)))
}

fn parse_scalar(
    exec: &mut ParseServices<'_, '_, '_>,
    snapshot: &DependencySnapshot<'_>,
    tokens: &[MtlToken<'_>],
    line_number: u32,
    keyword: &str,
) -> ParseResult<Option<f32>> {
    if tokens.len() < 2 {
        push_dependency_warning(
            exec,
            snapshot,
            Some((line_number, 1)),
            "obj.mtl_bad_scalar",
            format_args!("MTL {keyword} requires one value"),
        )?;
        return Ok(None);
    }
    parse_f32_warning(exec, snapshot, tokens[1].text, line_number, keyword)
}

fn parse_i64(
    exec: &mut ParseServices<'_, '_, '_>,
    snapshot: &DependencySnapshot<'_>,
    tokens: &[MtlToken<'_>],
    line_number: u32,
    keyword: &str,
) -> ParseResult<Option<i64>> {
    if tokens.len() < 2 {
        push_dependency_warning(
            exec,
            snapshot,
            Some((line_number, 1)),
            "obj.mtl_bad_integer",
            format_args!("MTL {keyword} requires one integer"),
        )?;
        return Ok(None);
    }
    if tokens[1].text.len() > MAX_NUMERIC_LITERAL_BYTES {
        push_dependency_warning(
            exec,
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
                exec,
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
    exec: &mut ParseServices<'_, '_, '_>,
    tokens: &[MtlToken<'_>],
    line_number: u32,
) -> ParseResult<Color> {
    if !(2..=4).contains(&tokens.len()) {
        return Err(map_syntax_failure(
            exec,
            tokens[0],
            line_number,
            "legacy-color-component-count",
        ));
    }
    let red = parse_required_map_number(exec, tokens[1], line_number)?;
    let green = match tokens.get(2) {
        Some(token) => parse_required_map_number(exec, *token, line_number)?,
        None => red,
    };
    let blue = match tokens.get(3) {
        Some(token) => parse_required_map_number(exec, *token, line_number)?,
        None => red,
    };
    Ok(Color::linear_rgba(red, green, blue, 1.0))
}

fn parse_f32_warning(
    exec: &mut ParseServices<'_, '_, '_>,
    snapshot: &DependencySnapshot<'_>,
    text: &str,
    line_number: u32,
    keyword: &str,
) -> ParseResult<Option<f32>> {
    if text.len() > MAX_NUMERIC_LITERAL_BYTES {
        push_dependency_warning(
            exec,
            snapshot,
            Some((line_number, 1)),
            "obj.mtl_bad_float",
            format_args!("MTL {keyword} float is too long"),
        )?;
        return Ok(None);
    }
    let Ok(value) = text.parse::<f32>() else {
        push_dependency_warning(
            exec,
            snapshot,
            Some((line_number, 1)),
            "obj.mtl_bad_float",
            format_args!("invalid MTL {keyword} float"),
        )?;
        return Ok(None);
    };
    if !value.is_finite() {
        push_dependency_warning(
            exec,
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
    exec: &mut ParseServices<'_, '_, '_>,
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
        exec.poll()?;
        // A stray negative number is an extra operand, not an unknown map option.
        // Numeric filenames can be disambiguated with a prefix such as ./.
        if option.text.parse::<f32>().is_ok() {
            return Err(map_syntax_failure(
                exec,
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
                        exec,
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
                    *value = parse_required_map_number(exec, *token, line_number)?;
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
                            exec,
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
                    map_syntax_failure(exec, option, line_number, "missing-option-operand")
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
                        enforce_map_numeric_limit(exec, operand.text)?;
                        operand.text.parse::<u32>().is_ok_and(|value| value > 0)
                    }
                    _ => false,
                };
                if !valid {
                    return Err(map_syntax_failure(
                        exec,
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
                        exec,
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
    Err(exec.format_failure(crate::ObjFailure::statement(
        "map_Kd",
        "missing-texture-path",
        line_number,
        1,
    )))
}

fn enforce_map_numeric_limit(exec: &mut ParseServices<'_, '_, '_>, text: &str) -> ParseResult<()> {
    exec.enforce_resource_limit(
        "obj-mtl-number-bytes",
        usize_amount(exec, text.len(), "obj-mtl-number-bytes")?,
        MAX_NUMERIC_LITERAL_BYTES as u64,
    )
}

fn parse_required_map_number(
    exec: &mut ParseServices<'_, '_, '_>,
    token: MtlToken<'_>,
    line_number: u32,
) -> ParseResult<f32> {
    enforce_map_numeric_limit(exec, token.text)?;
    let value = token
        .text
        .parse::<f32>()
        .map_err(|_| map_syntax_failure(exec, token, line_number, "invalid-numeric-operand"))?;
    if !value.is_finite() {
        return Err(map_syntax_failure(
            exec,
            token,
            line_number,
            "non-finite-numeric-operand",
        ));
    }
    Ok(value)
}

fn map_syntax_failure(
    exec: &mut ParseServices<'_, '_, '_>,
    token: MtlToken<'_>,
    line_number: u32,
    reason: &'static str,
) -> ParseFailure {
    exec.format_failure(crate::ObjFailure::statement(
        "mtl-material",
        reason,
        line_number,
        token.column,
    ))
}

fn map_texture_options(
    children: &mut ParseChildren<'_, '_, '_>,
    snapshot: &DependencySnapshot<'_>,
    tokens: &[MtlToken<'_>],
    arguments: &MapArguments,
    line_number: u32,
) -> ParseResult<(TextureTransform, SamplerState)> {
    let mut index = 0;
    while index < arguments.path_index {
        let exec = &mut children.services()?;
        exec.poll()?;
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
                children,
                snapshot,
                line_number,
                option,
                code,
                "unsupported-option",
            )?;
        } else if !is_known_map_option(option.text) {
            push_dependency_warning(
                &mut children.services()?,
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
                children,
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
    let transform =
        TextureTransform::try_new(offset, 0.0, scale).map_err(|_| match children.services() {
            Ok(mut exec) => {
                exec.format_failure(crate::ObjFailure::internal("invalid-texture-transform"))
            }
            Err(cause) => cause,
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
    children: &mut ParseChildren<'_, '_, '_>,
    snapshot: &DependencySnapshot<'_>,
    line_number: u32,
    token: MtlToken<'_>,
    code: &'static str,
    reason: &'static str,
) -> ParseResult<()> {
    let source = snapshot.diagnostic_source();
    let path = owned_format(
        children,
        format_args!(
            "/dependencies/{}/records/{line_number}/{}",
            source.path().as_str(),
            token.text,
        ),
    )?;
    let code = DiagnosticCode::new(code).expect("static OBJ diagnostic code must be valid");
    children.services()?.record_source_semantic_loss(
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

fn texture_path_from_tokens<'session>(
    children: &mut ParseChildren<'_, '_, 'session>,
    raw_line: &str,
    tokens: &[MtlToken<'_>],
) -> ParseResult<TexturePathOutcome<'session>> {
    let exec = &mut children.services()?;
    let first = tokens.first().ok_or_else(|| {
        exec.format_failure(crate::ObjFailure::internal("missing-map-path-token"))
    })?;
    let last = tokens.last().ok_or_else(|| {
        exec.format_failure(crate::ObjFailure::internal("missing-map-path-token"))
    })?;
    let path = &raw_line[first.start..last.end];
    if path.len() > parser::max_text_value_bytes(exec) {
        return Ok(TexturePathOutcome::TooLong);
    }
    owned_string(children, path, "obj-mtl-texture-path").map(TexturePathOutcome::Value)
}

#[allow(
    clippy::large_enum_variant,
    reason = "the metered string retains its reservation inline; boxing would add unmetered allocation"
)]
enum TexturePathOutcome<'session> {
    TooLong,
    Value(MeteredString<'session>),
}

fn owned_string<'session>(
    children: &mut ParseChildren<'_, '_, 'session>,
    value: &str,
    resource: &'static str,
) -> ParseResult<MeteredString<'session>> {
    parser::enforce_text_value_limit(&children.services()?, value.len(), resource)?;
    let string_work = children.bind_group([ImportWorkClass::StringBytes])?;
    let amount = usize_amount(&children.services()?, value.len(), resource)?;
    children.child(&string_work, [amount], |work| {
        work.run_returned_with_storage::<_, crate::ObjFailure>(|mut storage| {
            let mut output =
                storage.metered_string(ImportStorageClass::FormatNormalization, resource);
            storage.append_string(&mut output, value)?;
            Ok((output, [amount]))
        })
    })
}

fn owned_format<'session>(
    children: &mut ParseChildren<'_, '_, 'session>,
    arguments: fmt::Arguments<'_>,
) -> ParseResult<MeteredString<'session>> {
    use fmt::Write as _;

    let mut counter = FormatByteCounter::default();
    counter.write_fmt(arguments).map_err(|_| {
        children
            .error_view()
            .allocation_failure("obj-mtl-formatted-string-length")
    })?;
    parser::enforce_text_value_limit(
        &children.services()?,
        counter.bytes,
        "obj-mtl-formatted-string-bytes",
    )?;
    let string_work = children.bind_group([ImportWorkClass::StringBytes])?;
    let amount = usize_amount(
        &children.services()?,
        counter.bytes,
        "obj-mtl-formatted-string-bytes",
    )?;
    children.child(&string_work, [amount], |work| {
        work.run_returned_with_storage::<_, crate::ObjFailure>(|mut storage| {
            let mut output = storage.metered_string(
                ImportStorageClass::FormatNormalization,
                "obj-mtl-formatted-string",
            );
            storage.reserve_string(&mut output, counter.bytes)?;
            storage.write_string_fmt(&mut output, arguments)?;
            Ok((output, [amount]))
        })
    })
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
    exec: &mut ParseServices<'_, '_, '_>,
    tokens: &[MtlToken<'_>],
) -> ParseResult<Option<std::ops::Range<usize>>> {
    let mut index = 0usize;
    while let Some(token) = tokens.get(index) {
        exec.poll()?;
        if !token.text.starts_with('-') {
            let last = tokens.last().ok_or_else(|| {
                exec.format_failure(crate::ObjFailure::internal("missing-map-path-token"))
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
            .ok_or_else(|| exec.allocation_failure("obj-mtl-map-option-index"))?;
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
    exec: &mut ParseServices<'_, '_, '_>,
    location: Option<(u32, u32)>,
    code: &'static str,
    message: fmt::Arguments<'_>,
) -> ParseResult<()> {
    let code = DiagnosticCode::new(code).expect("static OBJ diagnostic code must be valid");
    match location {
        Some((line, column)) => exec.emit_diagnostic(
            DiagnosticSeverity::Warning,
            code,
            format_args!("line {line}, column {column}: {message}"),
        ),
        None => exec.emit_diagnostic(DiagnosticSeverity::Warning, code, message),
    }
}

fn push_dependency_warning(
    exec: &mut ParseServices<'_, '_, '_>,
    snapshot: &DependencySnapshot<'_>,
    location: Option<(u32, u32)>,
    code: &'static str,
    message: fmt::Arguments<'_>,
) -> ParseResult<()> {
    let code = DiagnosticCode::new(code).expect("static OBJ diagnostic code must be valid");
    match location {
        Some((line, column)) => exec.emit_diagnostic_with(
            DiagnosticSeverity::Warning,
            code,
            ImportProgressPhase::Decode,
            snapshot.diagnostic_source(),
            None,
            None,
            |_| {},
            format_args!("line {line}, column {column}: {message}"),
        ),
        None => exec.emit_diagnostic_with(
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

#[cfg(test)]
#[path = "comparison_tests.rs"]
mod comparison_tests;
