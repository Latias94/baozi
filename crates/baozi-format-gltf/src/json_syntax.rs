use crate::GltfFailure;
use crate::source_map::{
    GltfSourceMap, SourceEntityId, SourceEntityKind, SourceExtensionKind, SourcePayload,
    SourcePayloadKind, SourcePayloadSite, SourceRange,
};
use baozi_import::{
    ImportStorageClass, ImportWorkClass, ImportWorkGroup, ImportWorkScope, MeteredVec,
    ParseExecution, ParseFailure, ParseResult,
};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;

#[cfg(test)]
#[path = "json_syntax/control_tests.rs"]
mod control_tests;

const UTF8_BOM: &[u8; 3] = &[0xef, 0xbb, 0xbf];
const SCAN_CHUNK_BYTES: usize = 4 * 1024;
const DUPLICATE_POLL_COMPARISONS: u64 = baozi_import::MAX_GEOMETRY_ACTIONS_BETWEEN_POLLS as u64;
const MAX_JSON_NESTING_DEPTH: usize = 256;

type ScanResult<T> = Result<T, ScanAbort>;

// A parser limit must leave the active DecodeWork scope before it can publish the
// corresponding Tokens or Records ledger failure without inventing decode progress.
enum ScanAbort {
    Failure(ParseFailure),
    LedgerLimit {
        class: ImportWorkClass,
        attempted: u64,
    },
}

impl From<ParseFailure> for ScanAbort {
    fn from(failure: ParseFailure) -> Self {
        Self::Failure(failure)
    }
}

pub(crate) fn admit<'scope>(
    ctx: &mut ParseExecution<'_, 'scope>,
    bytes: &[u8],
    raw_base: u64,
    is_glb: bool,
) -> ParseResult<(GltfSourceMap<'scope>, JsonIndex<'scope>)> {
    ctx.enforce_resource_limit(
        "gltf-decoded-utf8-bytes",
        to_u64(bytes.len()),
        ctx.limits().max_decoded_utf8_bytes,
    )?;

    let mut scanner = JsonScanner::new(ctx, bytes, raw_base, is_glb);
    let scan_result = ctx.with_work_group(
        [(ImportWorkClass::DecodeWork, to_u64(bytes.len()))],
        |ctx, work| scanner.scan(work, ctx),
    );
    if let Err(abort) = scan_result {
        drop(scanner);
        return match abort {
            ScanAbort::Failure(failure) => Err(failure),
            ScanAbort::LedgerLimit { class, attempted } => {
                publish_ledger_limit(ctx, class, attempted)
            }
        };
    }

    let JsonScanner {
        frames,
        mut keys,
        stats,
        mut source_map,
        index,
        ..
    } = scanner;
    debug_assert!(frames.is_empty());
    debug_assert!(stats.strings <= stats.tokens);
    debug_assert!(stats.numbers <= stats.tokens);
    drop(frames);

    ctx.with_parent_work_group(
        [
            (ImportWorkClass::Tokens, stats.tokens),
            (ImportWorkClass::Records, stats.containers),
        ],
        |ctx, _parent| validate_duplicate_keys(ctx, bytes, raw_base, &mut keys),
    )?;
    debug_assert_eq!(stats.containers, stats.objects + stats.arrays);
    drop(keys);
    source_map.finish(ctx)?;
    Ok((source_map, index))
}

pub(crate) type ValueId = usize;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValueKind {
    Object,
    Array,
    String,
    Number,
    Bool,
    Null,
}

#[derive(Clone, Copy)]
pub(crate) struct IndexedValue {
    pub(crate) kind: ValueKind,
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) key_start: usize,
    pub(crate) key_end: usize,
    pub(crate) first_child: Option<ValueId>,
    last_child: Option<ValueId>,
    pub(crate) next_sibling: Option<ValueId>,
    pub(crate) child_count: usize,
}

/// Structural storage only: unknown values and strings retain borrowed raw ranges.
pub(crate) struct JsonIndex<'scope> {
    pub(crate) values: MeteredVec<'scope, IndexedValue>,
}

fn publish_ledger_limit<T>(
    ctx: &mut ParseExecution<'_, '_>,
    class: ImportWorkClass,
    attempted: u64,
) -> ParseResult<T> {
    let result: ParseResult<()> =
        ctx.with_work_group([(class, attempted)], |_ctx, work| work.complete_all());
    match result {
        Err(failure) => Err(failure),
        Ok(()) => Err(ctx.format_failure(GltfFailure::internal(
            "JSON scanner predicted a ledger limit that the session accepted",
        ))),
    }
}

#[derive(Clone, Copy)]
struct JsonStats {
    tokens: u64,
    containers: u64,
    objects: u64,
    arrays: u64,
    strings: u64,
    numbers: u64,
}

impl JsonStats {
    const fn new() -> Self {
        Self {
            tokens: 0,
            containers: 0,
            objects: 0,
            arrays: 0,
            strings: 0,
            numbers: 0,
        }
    }
}

#[derive(Clone, Copy)]
struct JsonKey {
    object_id: u64,
    raw_start: usize,
    raw_end: usize,
    content_start: usize,
    content_end: usize,
    // The digest partitions candidates only. Decoded scalar comparison remains authoritative.
    digest: [u8; 32],
}

#[derive(Clone, Copy)]
struct JsonFrame {
    value_id: ValueId,
    state: FrameState,
    object_id: u64,
    raw_start: usize,
    next_ordinal: usize,
    member: SourceMember,
    member_key_start: usize,
    member_key_end: usize,
    source: SourceContext,
}

#[derive(Clone, Copy)]
enum SourceContext {
    Other,
    Entity {
        id: SourceEntityId,
        map_index: usize,
    },
    EntityArray {
        kind: SourceEntityKind,
        parent_index: Option<usize>,
    },
    Property {
        owner: SourceEntityId,
        site: SourcePayloadSite,
    },
    Payload {
        payload_index: usize,
    },
    Extensions {
        payload_index: usize,
        owner: SourceEntityId,
        site: SourcePayloadSite,
    },
    ExtensionProperty {
        payload_index: usize,
        owner: SourceEntityId,
        site: SourcePayloadSite,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SourceMember {
    Other,
    Scenes,
    Nodes,
    Meshes,
    Primitives,
    Materials,
    Textures,
    Images,
    Samplers,
    Skins,
    Buffers,
    BufferViews,
    Accessors,
    Animations,
    Channels,
    Cameras,
    Extensions,
    Extras,
    Asset,
    PbrMetallicRoughness,
    BaseColorTexture,
    MetallicRoughnessTexture,
    NormalTexture,
    OcclusionTexture,
    EmissiveTexture,
    Sparse,
    Indices,
    Values,
    Target,
    Orthographic,
    Perspective,
    KhrTextureTransform,
    KhrMaterialsUnlit,
    KhrMaterialsEmissiveStrength,
    EmissiveStrength,
    KhrMaterialsClearcoat,
    ClearcoatFactor,
    ClearcoatRoughnessFactor,
    ClearcoatTexture,
    ClearcoatRoughnessTexture,
    ClearcoatNormalTexture,
    KhrMaterialsSheen,
    SheenColorFactor,
    SheenRoughnessFactor,
    SheenColorTexture,
    SheenRoughnessTexture,
    Index,
    TexCoord,
    Offset,
    Rotation,
    Scale,
    KhrLightsPunctual,
    Lights,
    Light,
    Spot,
    Name,
    Type,
    Color,
    Intensity,
    Range,
    InnerConeAngle,
    OuterConeAngle,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FrameState {
    ArrayValueOrEnd,
    ArrayValue,
    ArrayCommaOrEnd,
    ObjectKeyOrEnd,
    ObjectKey,
    ObjectColon,
    ObjectValue,
    ObjectCommaOrEnd,
}

struct JsonScanner<'scope, 'bytes> {
    bytes: &'bytes [u8],
    raw_base: u64,
    is_glb: bool,
    cursor: usize,
    polled_cursor: usize,
    root_started: bool,
    root_end: Option<usize>,
    frames: MeteredVec<'scope, JsonFrame>,
    keys: MeteredVec<'scope, JsonKey>,
    source_map: GltfSourceMap<'scope>,
    index: JsonIndex<'scope>,
    stats: JsonStats,
}

impl<'scope, 'bytes> JsonScanner<'scope, 'bytes> {
    fn new(
        ctx: &mut ParseExecution<'_, 'scope>,
        bytes: &'bytes [u8],
        raw_base: u64,
        is_glb: bool,
    ) -> Self {
        Self {
            bytes,
            raw_base,
            is_glb,
            cursor: 0,
            polled_cursor: 0,
            root_started: false,
            root_end: None,
            frames: ctx.metered_vec(
                ImportStorageClass::FormatNormalization,
                "gltf-json-frame-stack",
            ),
            keys: ctx.metered_vec(
                ImportStorageClass::FormatNormalization,
                "gltf-json-object-keys",
            ),
            source_map: GltfSourceMap::new(ctx),
            index: JsonIndex {
                values: ctx.metered_vec(
                    ImportStorageClass::FormatNormalization,
                    "gltf-json-structural-index",
                ),
            },
            stats: JsonStats::new(),
        }
    }

    fn scan(
        &mut self,
        work: &mut ImportWorkScope<'_, 1>,
        ctx: &mut ParseExecution<'_, 'scope>,
    ) -> ScanResult<()> {
        if self.bytes.starts_with(UTF8_BOM) {
            self.consume(work, UTF8_BOM.len(), ctx)?;
            self.flush(work, ctx)?;
            return Err(ctx
                .format_failure(GltfFailure::forbidden_bom(self.raw_base))
                .into());
        }

        while self.root_end.is_none() {
            self.skip_whitespace(work, ctx)?;
            if let Some(frame) = self.frames.as_slice().last().copied() {
                self.scan_frame(work, frame, ctx)?;
            } else if !self.root_started {
                if self.cursor == self.bytes.len() {
                    return self.syntax_error(
                        work,
                        "json-document",
                        self.cursor,
                        0,
                        "glTF JSON is missing a root value",
                        ctx,
                    );
                }
                self.root_started = true;
                self.scan_value(work, None, ctx)?;
                if self.frames.is_empty() {
                    self.root_end = Some(self.cursor);
                }
            } else {
                let byte_offset = self.absolute_offset(self.cursor, ctx)?;
                return Err(ctx
                    .format_failure(GltfFailure::invalid_json(
                        "json-syntax-map",
                        byte_offset,
                        0,
                        "glTF JSON root completion state is inconsistent",
                    ))
                    .into());
            }
        }

        self.scan_trailing(work, ctx)?;
        self.flush(work, ctx)
    }

    fn scan_frame(
        &mut self,
        work: &mut ImportWorkScope<'_, 1>,
        frame: JsonFrame,
        ctx: &mut ParseExecution<'_, 'scope>,
    ) -> ScanResult<()> {
        match frame.state {
            FrameState::ArrayValueOrEnd if self.peek() == Some(b']') => {
                self.close_container(work, b']', ctx)
            }
            FrameState::ArrayValueOrEnd | FrameState::ArrayValue => {
                if self.cursor == self.bytes.len() {
                    return self.syntax_error(
                        work,
                        "json-array",
                        self.cursor,
                        0,
                        "glTF JSON array is missing a value",
                        ctx,
                    );
                }
                self.set_frame_state(FrameState::ArrayCommaOrEnd, ctx)?;
                let next_ordinal = frame
                    .next_ordinal
                    .checked_add(1)
                    .ok_or_else(|| ctx.allocation_failure("gltf-json-array-ordinal"))?;
                self.active_frame_mut(ctx)?.next_ordinal = next_ordinal;
                self.scan_value(work, Some(frame), ctx)
            }
            FrameState::ArrayCommaOrEnd => match self.peek() {
                Some(b',') => {
                    self.count_token(work, ctx)?;
                    self.consume(work, 1, ctx)?;
                    self.set_frame_state(FrameState::ArrayValue, ctx)
                }
                Some(b']') => self.close_container(work, b']', ctx),
                Some(_) => self.syntax_error_current(
                    work,
                    "json-array",
                    self.cursor,
                    "glTF JSON array expects a comma or closing bracket",
                    ctx,
                ),
                None => self.syntax_error(
                    work,
                    "json-array",
                    self.cursor,
                    0,
                    "glTF JSON array is not closed",
                    ctx,
                ),
            },
            FrameState::ObjectKeyOrEnd if self.peek() == Some(b'}') => {
                self.close_container(work, b'}', ctx)
            }
            FrameState::ObjectKeyOrEnd | FrameState::ObjectKey => {
                if self.peek() != Some(b'"') {
                    let length = self.current_scalar_length(work, ctx)?;
                    return self.syntax_error(
                        work,
                        "json-object",
                        self.cursor,
                        length,
                        "glTF JSON object expects a quoted object key",
                        ctx,
                    );
                }
                let key = self.scan_string(work, true, ctx)?;
                let member = source_member(self.bytes, key.content_start, key.content_end);
                ctx.poll()?;
                self.keys.try_push(
                    ctx,
                    JsonKey {
                        object_id: frame.object_id,
                        raw_start: key.raw_start,
                        raw_end: key.raw_end,
                        content_start: key.content_start,
                        content_end: key.content_end,
                        digest: key.digest,
                    },
                )?;
                let frame = self.active_frame_mut(ctx)?;
                frame.member = member;
                frame.member_key_start = key.raw_start;
                frame.member_key_end = key.raw_end;
                frame.state = FrameState::ObjectColon;
                Ok(())
            }
            FrameState::ObjectColon => match self.peek() {
                Some(b':') => {
                    self.count_token(work, ctx)?;
                    self.consume(work, 1, ctx)?;
                    self.set_frame_state(FrameState::ObjectValue, ctx)
                }
                Some(_) => self.syntax_error_current(
                    work,
                    "json-object",
                    self.cursor,
                    "glTF JSON object key is missing a colon",
                    ctx,
                ),
                None => self.syntax_error(
                    work,
                    "json-object",
                    self.cursor,
                    0,
                    "glTF JSON object key is missing a colon",
                    ctx,
                ),
            },
            FrameState::ObjectValue => {
                if self.cursor == self.bytes.len() {
                    return self.syntax_error(
                        work,
                        "json-object",
                        self.cursor,
                        0,
                        "glTF JSON object key is missing a value",
                        ctx,
                    );
                }
                self.set_frame_state(FrameState::ObjectCommaOrEnd, ctx)?;
                self.scan_value(work, Some(frame), ctx)
            }
            FrameState::ObjectCommaOrEnd => match self.peek() {
                Some(b',') => {
                    self.count_token(work, ctx)?;
                    self.consume(work, 1, ctx)?;
                    self.set_frame_state(FrameState::ObjectKey, ctx)
                }
                Some(b'}') => self.close_container(work, b'}', ctx),
                Some(_) => self.syntax_error_current(
                    work,
                    "json-object",
                    self.cursor,
                    "glTF JSON object expects a comma or closing brace",
                    ctx,
                ),
                None => self.syntax_error(
                    work,
                    "json-object",
                    self.cursor,
                    0,
                    "glTF JSON object is not closed",
                    ctx,
                ),
            },
        }
    }

    fn scan_value(
        &mut self,
        work: &mut ImportWorkScope<'_, 1>,
        parent: Option<JsonFrame>,
        ctx: &mut ParseExecution<'_, 'scope>,
    ) -> ScanResult<()> {
        let raw_start = self.cursor;
        let source = self.value_source(parent, ctx)?;
        let kind = match self.peek() {
            Some(b'{') => ValueKind::Object,
            Some(b'[') => ValueKind::Array,
            Some(b'"') => ValueKind::String,
            Some(b't' | b'f') => ValueKind::Bool,
            Some(b'n') => ValueKind::Null,
            _ => ValueKind::Number,
        };
        let value_id = self.index.values.len();
        self.index.values.try_push(
            ctx,
            IndexedValue {
                kind,
                start: raw_start,
                end: raw_start,
                key_start: parent.map_or(0, |frame| frame.member_key_start),
                key_end: parent.map_or(0, |frame| frame.member_key_end),
                first_child: None,
                last_child: None,
                next_sibling: None,
                child_count: 0,
            },
        )?;
        if let Some(parent) = parent {
            let previous = self.index.values.as_slice()[parent.value_id].last_child;
            if let Some(previous) = previous {
                self.index.values.as_mut_slice()[previous].next_sibling = Some(value_id);
            }
            let entry = &mut self.index.values.as_mut_slice()[parent.value_id];
            entry.first_child.get_or_insert(value_id);
            entry.last_child = Some(value_id);
            entry.child_count += 1;
        }
        match self.peek() {
            Some(b'{') => {
                return self.open_container(
                    work,
                    FrameState::ObjectKeyOrEnd,
                    source,
                    value_id,
                    ctx,
                );
            }
            Some(b'[') => {
                return self.open_container(
                    work,
                    FrameState::ArrayValueOrEnd,
                    source,
                    value_id,
                    ctx,
                );
            }
            Some(b'"') => self.scan_string(work, false, ctx).map(|_| ()),
            Some(b't') => self.scan_literal(work, b"true", ctx),
            Some(b'f') => self.scan_literal(work, b"false", ctx),
            Some(b'n') => self.scan_literal(work, b"null", ctx),
            Some(b'-' | b'0'..=b'9') => self.scan_number(work, ctx),
            Some(byte) if byte >= 0x80 => {
                let length = self.valid_utf8_scalar_width(work, ctx)?;
                self.syntax_error(
                    work,
                    "json-document",
                    self.cursor,
                    length,
                    "glTF JSON value begins with an unexpected Unicode scalar",
                    ctx,
                )
            }
            Some(_) => self.syntax_error(
                work,
                "json-document",
                self.cursor,
                1,
                "glTF JSON value begins with an unexpected byte",
                ctx,
            ),
            None => self.syntax_error(
                work,
                "json-document",
                self.cursor,
                0,
                "glTF JSON is missing a value",
                ctx,
            ),
        }?;
        self.index.values.as_mut_slice()[value_id].end = self.cursor;
        self.finish_source_range(source, raw_start, ctx)
    }

    fn value_source(
        &mut self,
        parent: Option<JsonFrame>,
        ctx: &mut ParseExecution<'_, 'scope>,
    ) -> ScanResult<SourceContext> {
        let first = self.peek();
        match parent {
            None if first == Some(b'{') => self.begin_source_entity(
                SourceEntityId {
                    kind: SourceEntityKind::Document,
                    parent_index: None,
                    source_index: 0,
                },
                ctx,
            ),
            Some(
                frame @ JsonFrame {
                    source: SourceContext::Entity { id, .. },
                    ..
                },
            ) => {
                if matches!(
                    frame.member,
                    SourceMember::Extensions | SourceMember::Extras
                ) {
                    return self.begin_source_payload(id, SourcePayloadSite::Entity, frame, ctx);
                }
                if id.kind == SourceEntityKind::Light
                    && !matches!(
                        frame.member,
                        SourceMember::Name
                            | SourceMember::Type
                            | SourceMember::Color
                            | SourceMember::Intensity
                            | SourceMember::Range
                            | SourceMember::Spot
                    )
                {
                    return self.begin_source_payload(id, SourcePayloadSite::Entity, frame, ctx);
                }
                if first == Some(b'[') {
                    return Ok(source_entity_array(id, frame.member)
                        .map_or(SourceContext::Other, |(kind, parent_index)| {
                            SourceContext::EntityArray { kind, parent_index }
                        }));
                }
                Ok(source_property(
                    id,
                    SourcePayloadSite::Entity,
                    frame.member,
                    first,
                ))
            }
            Some(
                frame @ JsonFrame {
                    source: SourceContext::Property { owner, site },
                    ..
                },
            ) => {
                let unsupported = match site {
                    SourcePayloadSite::LightSpot => !matches!(
                        frame.member,
                        SourceMember::InnerConeAngle | SourceMember::OuterConeAngle
                    ),
                    SourcePayloadSite::ClearcoatTexture
                    | SourcePayloadSite::ClearcoatRoughnessTexture
                    | SourcePayloadSite::SheenColorTexture
                    | SourcePayloadSite::SheenRoughnessTexture => {
                        !matches!(frame.member, SourceMember::Index | SourceMember::TexCoord)
                    }
                    SourcePayloadSite::ClearcoatNormalTexture => !matches!(
                        frame.member,
                        SourceMember::Index | SourceMember::TexCoord | SourceMember::Scale
                    ),
                    _ => false,
                };
                if matches!(
                    frame.member,
                    SourceMember::Extensions | SourceMember::Extras
                ) || unsupported
                {
                    return self.begin_source_payload(owner, site, frame, ctx);
                }
                Ok(source_property(owner, site, frame.member, first))
            }
            Some(
                frame @ JsonFrame {
                    source: SourceContext::Extensions { owner, site, .. },
                    ..
                },
            ) => self.begin_source_payload(owner, site, frame, ctx),
            Some(
                frame @ JsonFrame {
                    source: SourceContext::ExtensionProperty { owner, site, .. },
                    ..
                },
            ) => {
                if site == SourcePayloadSite::DocumentPunctualLights
                    && frame.member == SourceMember::Lights
                    && first == Some(b'[')
                {
                    return Ok(SourceContext::EntityArray {
                        kind: SourceEntityKind::Light,
                        parent_index: None,
                    });
                }
                let supported = match site {
                    SourcePayloadSite::MaterialUnlit => false,
                    SourcePayloadSite::MaterialEmissiveStrength => {
                        frame.member == SourceMember::EmissiveStrength
                    }
                    SourcePayloadSite::MaterialClearcoat => matches!(
                        frame.member,
                        SourceMember::ClearcoatFactor
                            | SourceMember::ClearcoatRoughnessFactor
                            | SourceMember::ClearcoatTexture
                            | SourceMember::ClearcoatRoughnessTexture
                            | SourceMember::ClearcoatNormalTexture
                    ),
                    SourcePayloadSite::MaterialSheen => matches!(
                        frame.member,
                        SourceMember::SheenColorFactor
                            | SourceMember::SheenRoughnessFactor
                            | SourceMember::SheenColorTexture
                            | SourceMember::SheenRoughnessTexture
                    ),
                    SourcePayloadSite::ClearcoatTextureTransform
                    | SourcePayloadSite::ClearcoatRoughnessTextureTransform
                    | SourcePayloadSite::ClearcoatNormalTextureTransform
                    | SourcePayloadSite::SheenColorTextureTransform
                    | SourcePayloadSite::SheenRoughnessTextureTransform => matches!(
                        frame.member,
                        SourceMember::TexCoord
                            | SourceMember::Offset
                            | SourceMember::Rotation
                            | SourceMember::Scale
                    ),
                    SourcePayloadSite::DocumentPunctualLights => {
                        frame.member == SourceMember::Lights
                    }
                    SourcePayloadSite::NodePunctualLight => frame.member == SourceMember::Light,
                    _ => true,
                };
                if matches!(
                    frame.member,
                    SourceMember::Extras | SourceMember::Extensions
                ) || !supported
                {
                    self.begin_source_payload(owner, site, frame, ctx)
                } else {
                    Ok(source_property(owner, site, frame.member, first))
                }
            }
            Some(JsonFrame {
                source: SourceContext::EntityArray { kind, parent_index },
                next_ordinal,
                ..
            }) if first == Some(b'{') => self.begin_source_entity(
                SourceEntityId {
                    kind,
                    parent_index,
                    source_index: next_ordinal,
                },
                ctx,
            ),
            _ => Ok(SourceContext::Other),
        }
    }

    fn begin_source_entity(
        &mut self,
        id: SourceEntityId,
        ctx: &mut ParseExecution<'_, 'scope>,
    ) -> ScanResult<SourceContext> {
        let start = self.source_range(self.cursor, ctx)?;
        let map_index = self.source_map.begin_entity(ctx, id, start)?;
        Ok(SourceContext::Entity { id, map_index })
    }

    fn source_range(
        &self,
        raw_start: usize,
        ctx: &mut ParseExecution<'_, 'scope>,
    ) -> ParseResult<SourceRange> {
        SourceRange::from_json_offsets(self.raw_base, raw_start, self.cursor)
            .ok_or_else(|| ctx.allocation_failure("gltf-source-byte-range"))
    }

    fn begin_source_payload(
        &mut self,
        owner: SourceEntityId,
        site: SourcePayloadSite,
        frame: JsonFrame,
        ctx: &mut ParseExecution<'_, 'scope>,
    ) -> ScanResult<SourceContext> {
        let kind = if matches!(frame.source, SourceContext::Extensions { .. }) {
            let extension = match frame.member {
                SourceMember::KhrTextureTransform => SourceExtensionKind::KhrTextureTransform,
                SourceMember::KhrMaterialsUnlit => SourceExtensionKind::KhrMaterialsUnlit,
                SourceMember::KhrMaterialsEmissiveStrength => {
                    SourceExtensionKind::KhrMaterialsEmissiveStrength
                }
                SourceMember::KhrMaterialsClearcoat => SourceExtensionKind::KhrMaterialsClearcoat,
                SourceMember::KhrMaterialsSheen => SourceExtensionKind::KhrMaterialsSheen,
                SourceMember::KhrLightsPunctual => SourceExtensionKind::KhrLightsPunctual,
                _ => SourceExtensionKind::Other,
            };
            SourcePayloadKind::Extension(extension)
        } else if frame.member == SourceMember::Extensions {
            SourcePayloadKind::Extensions
        } else if frame.member == SourceMember::Extras {
            SourcePayloadKind::Extras
        } else {
            SourcePayloadKind::ExtensionProperty
        };
        let key_range = SourceRange::from_json_offsets(
            self.raw_base,
            frame.member_key_start,
            frame.member_key_end,
        )
        .ok_or_else(|| ctx.allocation_failure("gltf-source-key-range"))?;
        let payload_index = {
            let argument_0 = SourcePayload {
                owner,
                site,
                kind,
                key_range,
                member_range: self.source_range(frame.member_key_start, ctx)?,
                value_range: self.source_range(self.cursor, ctx)?,
            };
            self.source_map.begin_payload(ctx, argument_0)
        }?;
        if self.peek() != Some(b'{') {
            return Ok(SourceContext::Payload { payload_index });
        }
        if kind == SourcePayloadKind::Extensions {
            return Ok(SourceContext::Extensions {
                payload_index,
                owner,
                site,
            });
        }
        let site = match (kind, owner.kind, site) {
            (
                SourcePayloadKind::Extension(SourceExtensionKind::KhrLightsPunctual),
                SourceEntityKind::Document,
                SourcePayloadSite::Entity,
            ) => SourcePayloadSite::DocumentPunctualLights,
            (
                SourcePayloadKind::Extension(SourceExtensionKind::KhrLightsPunctual),
                SourceEntityKind::Node,
                SourcePayloadSite::Entity,
            ) => SourcePayloadSite::NodePunctualLight,
            (
                SourcePayloadKind::Extension(SourceExtensionKind::KhrMaterialsUnlit),
                SourceEntityKind::Material,
                SourcePayloadSite::Entity,
            ) => SourcePayloadSite::MaterialUnlit,
            (
                SourcePayloadKind::Extension(SourceExtensionKind::KhrMaterialsEmissiveStrength),
                SourceEntityKind::Material,
                SourcePayloadSite::Entity,
            ) => SourcePayloadSite::MaterialEmissiveStrength,
            (
                SourcePayloadKind::Extension(SourceExtensionKind::KhrMaterialsClearcoat),
                SourceEntityKind::Material,
                SourcePayloadSite::Entity,
            ) => SourcePayloadSite::MaterialClearcoat,
            (
                SourcePayloadKind::Extension(SourceExtensionKind::KhrMaterialsSheen),
                SourceEntityKind::Material,
                SourcePayloadSite::Entity,
            ) => SourcePayloadSite::MaterialSheen,
            (
                SourcePayloadKind::Extension(SourceExtensionKind::KhrTextureTransform),
                _,
                SourcePayloadSite::BaseColorTexture,
            ) => SourcePayloadSite::BaseColorTextureTransform,
            (
                SourcePayloadKind::Extension(SourceExtensionKind::KhrTextureTransform),
                _,
                SourcePayloadSite::MetallicRoughnessTexture,
            ) => SourcePayloadSite::MetallicRoughnessTextureTransform,
            (
                SourcePayloadKind::Extension(SourceExtensionKind::KhrTextureTransform),
                _,
                SourcePayloadSite::NormalTexture,
            ) => SourcePayloadSite::NormalTextureTransform,
            (
                SourcePayloadKind::Extension(SourceExtensionKind::KhrTextureTransform),
                _,
                SourcePayloadSite::OcclusionTexture,
            ) => SourcePayloadSite::OcclusionTextureTransform,
            (
                SourcePayloadKind::Extension(SourceExtensionKind::KhrTextureTransform),
                _,
                SourcePayloadSite::EmissiveTexture,
            ) => SourcePayloadSite::EmissiveTextureTransform,
            (
                SourcePayloadKind::Extension(SourceExtensionKind::KhrTextureTransform),
                SourceEntityKind::Material,
                SourcePayloadSite::ClearcoatTexture,
            ) => SourcePayloadSite::ClearcoatTextureTransform,
            (
                SourcePayloadKind::Extension(SourceExtensionKind::KhrTextureTransform),
                SourceEntityKind::Material,
                SourcePayloadSite::ClearcoatRoughnessTexture,
            ) => SourcePayloadSite::ClearcoatRoughnessTextureTransform,
            (
                SourcePayloadKind::Extension(SourceExtensionKind::KhrTextureTransform),
                SourceEntityKind::Material,
                SourcePayloadSite::ClearcoatNormalTexture,
            ) => SourcePayloadSite::ClearcoatNormalTextureTransform,
            (
                SourcePayloadKind::Extension(SourceExtensionKind::KhrTextureTransform),
                SourceEntityKind::Material,
                SourcePayloadSite::SheenColorTexture,
            ) => SourcePayloadSite::SheenColorTextureTransform,
            (
                SourcePayloadKind::Extension(SourceExtensionKind::KhrTextureTransform),
                SourceEntityKind::Material,
                SourcePayloadSite::SheenRoughnessTexture,
            ) => SourcePayloadSite::SheenRoughnessTextureTransform,
            _ => return Ok(SourceContext::Payload { payload_index }),
        };
        Ok(SourceContext::ExtensionProperty {
            payload_index,
            owner,
            site,
        })
    }

    fn finish_source_range(
        &mut self,
        source: SourceContext,
        raw_start: usize,
        ctx: &mut ParseExecution<'_, 'scope>,
    ) -> ScanResult<()> {
        match source {
            SourceContext::Entity { map_index, .. } => {
                let range = self.source_range(raw_start, ctx)?;
                let entity = self.source_map.entity_mut(map_index).ok_or_else(|| {
                    ctx.format_failure(GltfFailure::internal(
                        "JSON source range refers to a missing glTF entity",
                    ))
                })?;
                entity.range = range;
            }
            SourceContext::Payload { payload_index }
            | SourceContext::Extensions { payload_index, .. }
            | SourceContext::ExtensionProperty { payload_index, .. } => {
                let range = self.source_range(raw_start, ctx)?;
                let raw_end = self.absolute_offset(self.cursor, ctx)?;
                let payload = self.source_map.payload_mut(payload_index).ok_or_else(|| {
                    ctx.format_failure(GltfFailure::internal(
                        "JSON source range refers to a missing glTF payload",
                    ))
                })?;
                payload.value_range = range;
                payload.member_range.byte_length = raw_end
                    .checked_sub(payload.member_range.byte_offset)
                    .ok_or_else(|| ctx.allocation_failure("gltf-source-member-range"))?;
            }
            SourceContext::Other
            | SourceContext::EntityArray { .. }
            | SourceContext::Property { .. } => {}
        }
        Ok(())
    }

    fn open_container(
        &mut self,
        work: &mut ImportWorkScope<'_, 1>,
        state: FrameState,
        source: SourceContext,
        value_id: ValueId,
        ctx: &mut ParseExecution<'_, 'scope>,
    ) -> ScanResult<()> {
        let depth = self
            .frames
            .len()
            .checked_add(1)
            .ok_or_else(|| ctx.allocation_failure("gltf-json-nesting-depth"))?;
        self.enforce_usize_limit(
            work,
            "gltf-json-nesting-depth",
            depth,
            MAX_JSON_NESTING_DEPTH,
            ctx,
        )?;
        self.count_token(work, ctx)?;

        let object_id = if state == FrameState::ObjectKeyOrEnd {
            let objects = self.increment_counter(self.stats.objects, "gltf-json-objects", ctx)?;
            self.stats.objects = objects;
            objects
        } else {
            self.stats.arrays =
                self.increment_counter(self.stats.arrays, "gltf-json-arrays", ctx)?;
            0
        };
        let containers =
            self.increment_counter(self.stats.containers, "gltf-json-containers", ctx)?;
        let record_limit = to_u64(ctx.limits().max_records);
        if containers > record_limit {
            self.flush(work, ctx)?;
            return Err(ScanAbort::LedgerLimit {
                class: ImportWorkClass::Records,
                attempted: containers,
            });
        }
        self.stats.containers = containers;

        ctx.poll()?;
        self.frames.try_push(
            ctx,
            JsonFrame {
                value_id,
                state,
                object_id,
                raw_start: self.cursor,
                next_ordinal: 0,
                member: SourceMember::Other,
                member_key_start: 0,
                member_key_end: 0,
                source,
            },
        )?;
        self.consume(work, 1, ctx)
    }

    fn close_container(
        &mut self,
        work: &mut ImportWorkScope<'_, 1>,
        expected: u8,
        ctx: &mut ParseExecution<'_, 'scope>,
    ) -> ScanResult<()> {
        if self.peek() != Some(expected) {
            let byte_offset = self.absolute_offset(self.cursor, ctx)?;
            return Err(ctx
                .format_failure(GltfFailure::invalid_json(
                    "json-syntax-map",
                    byte_offset,
                    0,
                    "glTF JSON container close state is inconsistent",
                ))
                .into());
        }
        self.count_token(work, ctx)?;
        self.consume(work, 1, ctx)?;
        let Some(closed) = self.frames.pop() else {
            let byte_offset = self.absolute_offset(self.cursor, ctx)?;
            return Err(ctx
                .format_failure(GltfFailure::invalid_json(
                    "json-syntax-map",
                    byte_offset,
                    0,
                    "glTF JSON container stack underflowed",
                ))
                .into());
        };
        self.finish_source_range(closed.source, closed.raw_start, ctx)?;
        self.index.values.as_mut_slice()[closed.value_id].end = self.cursor;
        if self.frames.is_empty() {
            self.root_end = Some(self.cursor);
        }
        Ok(())
    }

    fn scan_string(
        &mut self,
        work: &mut ImportWorkScope<'_, 1>,
        hash_key: bool,
        ctx: &mut ParseExecution<'_, 'scope>,
    ) -> ScanResult<ScannedString> {
        self.count_token(work, ctx)?;
        self.stats.strings =
            self.increment_counter(self.stats.strings, "gltf-json-strings", ctx)?;
        let raw_start = self.cursor;
        self.consume(work, 1, ctx)?;
        let content_start = self.cursor;
        let mut hasher = hash_key.then(Sha256::new);

        loop {
            let Some(byte) = self.peek() else {
                return self.syntax_error(
                    work,
                    "json-string",
                    raw_start,
                    self.cursor - raw_start,
                    "glTF JSON string is not closed",
                    ctx,
                );
            };
            match byte {
                b'"' => {
                    let content_end = self.cursor;
                    self.consume(work, 1, ctx)?;
                    let digest = hasher.map_or([0; 32], |hasher| hasher.finalize().into());
                    return Ok(ScannedString {
                        raw_start,
                        raw_end: self.cursor,
                        content_start,
                        content_end,
                        digest,
                    });
                }
                0x00..=0x1f => {
                    return self.syntax_error(
                        work,
                        "json-string",
                        self.cursor,
                        1,
                        "glTF JSON string contains an unescaped control character",
                        ctx,
                    );
                }
                b'\\' => {
                    let escape_start = self.cursor;
                    self.consume_string_bytes(work, content_start, 1, ctx)?;
                    let Some(escape) = self.peek() else {
                        return self.syntax_error(
                            work,
                            "json-string",
                            escape_start,
                            self.cursor - escape_start,
                            "glTF JSON string ends inside an escape",
                            ctx,
                        );
                    };
                    match escape {
                        b'"' | b'\\' | b'/' => {
                            self.consume_string_bytes(work, content_start, 1, ctx)?;
                            hash_scalar(&mut hasher, u32::from(escape));
                        }
                        b'b' | b'f' | b'n' | b'r' | b't' => {
                            self.consume_string_bytes(work, content_start, 1, ctx)?;
                            let scalar = match escape {
                                b'b' => 0x08,
                                b'f' => 0x0c,
                                b'n' => 0x0a,
                                b'r' => 0x0d,
                                _ => 0x09,
                            };
                            hash_scalar(&mut hasher, scalar);
                        }
                        b'u' => {
                            self.consume_string_bytes(work, content_start, 1, ctx)?;
                            let first =
                                self.scan_hex_quad(work, content_start, escape_start, ctx)?;
                            let scalar = if (0xd800..=0xdbff).contains(&first) {
                                if self.peek() != Some(b'\\')
                                    || self.bytes.get(self.cursor + 1) != Some(&b'u')
                                {
                                    return self.syntax_error(
                                        work,
                                        "json-string",
                                        escape_start,
                                        self.cursor - escape_start,
                                        "glTF JSON high surrogate is not paired",
                                        ctx,
                                    );
                                }
                                let low_start = self.cursor;
                                self.consume_string_bytes(work, content_start, 2, ctx)?;
                                let low =
                                    self.scan_hex_quad(work, content_start, low_start, ctx)?;
                                if !(0xdc00..=0xdfff).contains(&low) {
                                    return self.syntax_error(
                                        work,
                                        "json-string",
                                        low_start,
                                        self.cursor - low_start,
                                        "glTF JSON high surrogate has an invalid low surrogate",
                                        ctx,
                                    );
                                }
                                0x1_0000
                                    + ((u32::from(first) - 0xd800) << 10)
                                    + (u32::from(low) - 0xdc00)
                            } else if (0xdc00..=0xdfff).contains(&first) {
                                return self.syntax_error(
                                    work,
                                    "json-string",
                                    escape_start,
                                    self.cursor - escape_start,
                                    "glTF JSON contains an unpaired low surrogate",
                                    ctx,
                                );
                            } else {
                                u32::from(first)
                            };
                            hash_scalar(&mut hasher, scalar);
                        }
                        _ => {
                            return self.syntax_error_current(
                                work,
                                "json-string",
                                self.cursor,
                                "glTF JSON string contains an invalid escape",
                                ctx,
                            );
                        }
                    }
                }
                0x20..=0x7f => {
                    self.consume_string_bytes(work, content_start, 1, ctx)?;
                    if let Some(hasher) = hasher.as_mut() {
                        hasher.update([byte]);
                        #[cfg(test)]
                        control_tests::key_bytes_hashed(self.cursor, hasher);
                    }
                }
                _ => {
                    let width = self.valid_utf8_scalar_width(work, ctx)?;
                    self.enforce_string_bytes(work, content_start, width, ctx)?;
                    if let Some(hasher) = hasher.as_mut() {
                        hasher.update(&self.bytes[self.cursor..self.cursor + width]);
                        #[cfg(test)]
                        control_tests::key_bytes_hashed(self.cursor + width, hasher);
                    }
                    self.consume(work, width, ctx)?;
                }
            }
        }
    }

    fn scan_hex_quad(
        &mut self,
        work: &mut ImportWorkScope<'_, 1>,
        content_start: usize,
        escape_start: usize,
        ctx: &mut ParseExecution<'_, 'scope>,
    ) -> ScanResult<u16> {
        let mut value = 0_u16;
        for _ in 0..4 {
            let Some(byte) = self.peek() else {
                return self.syntax_error(
                    work,
                    "json-string",
                    escape_start,
                    self.cursor - escape_start,
                    "glTF JSON Unicode escape is truncated",
                    ctx,
                );
            };
            let Some(digit) = hex_value(byte) else {
                return self.syntax_error_current(
                    work,
                    "json-string",
                    self.cursor,
                    "glTF JSON Unicode escape contains a non-hex digit",
                    ctx,
                );
            };
            self.consume_string_bytes(work, content_start, 1, ctx)?;
            value = (value << 4) | u16::from(digit);
        }
        Ok(value)
    }

    fn scan_number(
        &mut self,
        work: &mut ImportWorkScope<'_, 1>,
        ctx: &mut ParseExecution<'_, 'scope>,
    ) -> ScanResult<()> {
        self.count_token(work, ctx)?;
        self.stats.numbers =
            self.increment_counter(self.stats.numbers, "gltf-json-numbers", ctx)?;
        let start = self.cursor;

        if self.peek() == Some(b'-') {
            self.consume_number_byte(work, start, ctx)?;
        }
        match self.peek() {
            Some(b'0') => {
                self.consume_number_byte(work, start, ctx)?;
                if self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                    return self.syntax_error(
                        work,
                        "json-number",
                        self.cursor,
                        1,
                        "glTF JSON number has a leading zero",
                        ctx,
                    );
                }
            }
            Some(b'1'..=b'9') => {
                while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                    self.consume_number_byte(work, start, ctx)?;
                }
            }
            Some(_) => {
                return self.syntax_error_current(
                    work,
                    "json-number",
                    self.cursor,
                    "glTF JSON number is missing an integer digit",
                    ctx,
                );
            }
            None => {
                return self.syntax_error(
                    work,
                    "json-number",
                    self.cursor,
                    0,
                    "glTF JSON number is missing an integer digit",
                    ctx,
                );
            }
        }

        if self.peek() == Some(b'.') {
            self.consume_number_byte(work, start, ctx)?;
            if !self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                return self.syntax_error(
                    work,
                    "json-number",
                    self.cursor,
                    usize::from(self.cursor < self.bytes.len()),
                    "glTF JSON number fraction is missing a digit",
                    ctx,
                );
            }
            while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                self.consume_number_byte(work, start, ctx)?;
            }
        }

        if self.peek().is_some_and(|byte| matches!(byte, b'e' | b'E')) {
            self.consume_number_byte(work, start, ctx)?;
            if self.peek().is_some_and(|byte| matches!(byte, b'+' | b'-')) {
                self.consume_number_byte(work, start, ctx)?;
            }
            if !self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                return self.syntax_error(
                    work,
                    "json-number",
                    self.cursor,
                    usize::from(self.cursor < self.bytes.len()),
                    "glTF JSON number exponent is missing a digit",
                    ctx,
                );
            }
            while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                self.consume_number_byte(work, start, ctx)?;
            }
        }

        if self.peek().is_some_and(|byte| !is_value_delimiter(byte)) {
            return self.syntax_error_current(
                work,
                "json-number",
                self.cursor,
                "glTF JSON number has an invalid terminator",
                ctx,
            );
        }
        Ok(())
    }

    fn scan_literal(
        &mut self,
        work: &mut ImportWorkScope<'_, 1>,
        literal: &[u8],
        ctx: &mut ParseExecution<'_, 'scope>,
    ) -> ScanResult<()> {
        self.count_token(work, ctx)?;
        let start = self.cursor;
        for expected in literal {
            let attempted = self
                .cursor
                .checked_add(1)
                .and_then(|end| end.checked_sub(start))
                .ok_or_else(|| ctx.allocation_failure("gltf-json-literal-bytes"))?;
            self.enforce_usize_limit(
                work,
                "gltf-json-literal-bytes",
                attempted,
                ctx.limits().max_token_bytes,
                ctx,
            )?;
            match self.peek() {
                Some(actual) if actual == *expected => self.consume(work, 1, ctx)?,
                Some(_) => {
                    return self.syntax_error_current(
                        work,
                        "json-literal",
                        self.cursor,
                        "glTF JSON literal is malformed",
                        ctx,
                    );
                }
                None => {
                    return self.syntax_error(
                        work,
                        "json-literal",
                        start,
                        self.cursor - start,
                        "glTF JSON literal is truncated",
                        ctx,
                    );
                }
            }
        }
        if self.peek().is_some_and(|byte| !is_value_delimiter(byte)) {
            return self.syntax_error_current(
                work,
                "json-literal",
                self.cursor,
                "glTF JSON literal has an invalid terminator",
                ctx,
            );
        }
        Ok(())
    }

    fn scan_trailing(
        &mut self,
        work: &mut ImportWorkScope<'_, 1>,
        ctx: &mut ParseExecution<'_, 'scope>,
    ) -> ScanResult<()> {
        self.skip_whitespace(work, ctx)?;
        if self.cursor == self.bytes.len() {
            return Ok(());
        }
        let length = self.current_scalar_length(work, ctx)?;
        let (subject, reason) = if self.is_glb {
            (
                "json-padding",
                "GLB JSON padding contains non-whitespace content",
            )
        } else {
            (
                "json-document",
                "glTF JSON contains trailing content after the root value",
            )
        };
        self.syntax_error(work, subject, self.cursor, length, reason, ctx)
    }

    fn skip_whitespace(
        &mut self,
        work: &mut ImportWorkScope<'_, 1>,
        ctx: &mut ParseExecution<'_, 'scope>,
    ) -> ScanResult<()> {
        while self.peek().is_some_and(is_json_whitespace) {
            self.consume(work, 1, ctx)?;
        }
        Ok(())
    }

    fn count_token(
        &mut self,
        work: &mut ImportWorkScope<'_, 1>,
        ctx: &mut ParseExecution<'_, 'scope>,
    ) -> ScanResult<()> {
        let attempted = self
            .stats
            .tokens
            .checked_add(1)
            .ok_or_else(|| ctx.allocation_failure("gltf-json-token-count"))?;
        if attempted > ctx.limits().max_tokens {
            self.flush(work, ctx)?;
            return Err(ScanAbort::LedgerLimit {
                class: ImportWorkClass::Tokens,
                attempted,
            });
        }
        self.stats.tokens = attempted;
        Ok(())
    }

    fn increment_counter(
        &self,
        current: u64,
        resource: &'static str,
        ctx: &mut ParseExecution<'_, 'scope>,
    ) -> ScanResult<u64> {
        let attempted = current
            .checked_add(1)
            .ok_or_else(|| ctx.allocation_failure(resource))?;
        Ok(attempted)
    }

    fn consume_string_bytes(
        &mut self,
        work: &mut ImportWorkScope<'_, 1>,
        content_start: usize,
        amount: usize,
        ctx: &mut ParseExecution<'_, 'scope>,
    ) -> ScanResult<()> {
        self.enforce_string_bytes(work, content_start, amount, ctx)?;
        self.consume(work, amount, ctx)
    }

    fn enforce_string_bytes(
        &mut self,
        work: &mut ImportWorkScope<'_, 1>,
        content_start: usize,
        amount: usize,
        ctx: &mut ParseExecution<'_, 'scope>,
    ) -> ScanResult<()> {
        let end = self
            .cursor
            .checked_add(amount)
            .ok_or_else(|| ctx.allocation_failure("gltf-json-string-bytes"))?;
        let attempted = end
            .checked_sub(content_start)
            .ok_or_else(|| ctx.allocation_failure("gltf-json-string-bytes"))?;
        self.enforce_usize_limit(
            work,
            "gltf-json-string-bytes",
            attempted,
            ctx.limits().max_token_bytes,
            ctx,
        )
    }

    fn consume_number_byte(
        &mut self,
        work: &mut ImportWorkScope<'_, 1>,
        start: usize,
        ctx: &mut ParseExecution<'_, 'scope>,
    ) -> ScanResult<()> {
        let attempted = self
            .cursor
            .checked_add(1)
            .and_then(|end| end.checked_sub(start))
            .ok_or_else(|| ctx.allocation_failure("gltf-json-number-bytes"))?;
        self.enforce_usize_limit(
            work,
            "gltf-json-number-bytes",
            attempted,
            ctx.limits().max_token_bytes,
            ctx,
        )?;
        self.consume(work, 1, ctx)
    }

    fn enforce_usize_limit(
        &mut self,
        work: &mut ImportWorkScope<'_, 1>,
        resource: &'static str,
        attempted: usize,
        limit: usize,
        ctx: &mut ParseExecution<'_, 'scope>,
    ) -> ScanResult<()> {
        if attempted > limit {
            self.flush(work, ctx)?;
            return Err(ctx.resource_limit_exceeded(resource, to_u64(limit)).into());
        }
        Ok(())
    }

    fn set_frame_state(
        &mut self,
        state: FrameState,
        ctx: &mut ParseExecution<'_, 'scope>,
    ) -> ScanResult<()> {
        self.active_frame_mut(ctx)?.state = state;
        Ok(())
    }

    fn active_frame_mut(
        &mut self,
        ctx: &mut ParseExecution<'_, 'scope>,
    ) -> ScanResult<&mut JsonFrame> {
        let byte_offset = self.absolute_offset(self.cursor, ctx)?;
        let Some(frame) = self.frames.as_mut_slice().last_mut() else {
            return Err(ctx
                .format_failure(GltfFailure::invalid_json(
                    "json-syntax-map",
                    byte_offset,
                    0,
                    "glTF JSON frame state update has no active frame",
                ))
                .into());
        };
        Ok(frame)
    }

    fn current_scalar_length(
        &mut self,
        work: &mut ImportWorkScope<'_, 1>,
        ctx: &mut ParseExecution<'_, 'scope>,
    ) -> ScanResult<usize> {
        match self.peek() {
            Some(byte) if byte >= 0x80 => self.valid_utf8_scalar_width(work, ctx),
            Some(_) => Ok(1),
            None => Ok(0),
        }
    }

    fn valid_utf8_scalar_width(
        &mut self,
        work: &mut ImportWorkScope<'_, 1>,
        ctx: &mut ParseExecution<'_, 'scope>,
    ) -> ScanResult<usize> {
        let lead = self.bytes[self.cursor];
        let width = match lead {
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => 1,
        };
        // Poll before classifying or hashing this indivisible raw unit.
        if self.cursor - self.polled_cursor + width > SCAN_CHUNK_BYTES {
            self.flush(work, ctx)?;
        }
        let end = self.cursor.saturating_add(width).min(self.bytes.len());
        let encoded = &self.bytes[self.cursor..end];
        match std::str::from_utf8(encoded) {
            Ok(value) if value.chars().count() == 1 => Ok(width),
            Ok(_) => {
                let byte_offset = self.absolute_offset(self.cursor, ctx)?;
                Err(ctx
                    .format_failure(GltfFailure::invalid_json(
                        "json-syntax-map",
                        byte_offset,
                        to_u64(encoded.len()),
                        "glTF JSON UTF-8 scalar probe was ambiguous",
                    ))
                    .into())
            }
            Err(source) => {
                let byte_length = source.error_len().unwrap_or(encoded.len()).max(1);
                self.flush(work, ctx)?;
                let byte_offset = self.absolute_offset(self.cursor, ctx)?;
                Err(ctx
                    .format_failure(GltfFailure::invalid_utf8(
                        source,
                        byte_offset,
                        to_u64(byte_length),
                    ))
                    .into())
            }
        }
    }

    fn syntax_error<T>(
        &mut self,
        work: &mut ImportWorkScope<'_, 1>,
        subject: &'static str,
        offset: usize,
        length: usize,
        reason: &'static str,
        ctx: &mut ParseExecution<'_, 'scope>,
    ) -> ScanResult<T> {
        // UTF-8 conformance owns precedence over grammar errors anywhere in the admitted surface.
        self.scan_remaining_utf8(work, ctx)?;
        self.flush(work, ctx)?;
        let byte_offset = self.absolute_offset(offset, ctx)?;
        Err(ctx
            .format_failure(GltfFailure::invalid_json(
                subject,
                byte_offset,
                to_u64(length),
                reason,
            ))
            .into())
    }

    fn syntax_error_current<T>(
        &mut self,
        work: &mut ImportWorkScope<'_, 1>,
        subject: &'static str,
        offset: usize,
        reason: &'static str,
        ctx: &mut ParseExecution<'_, 'scope>,
    ) -> ScanResult<T> {
        let length = self.current_scalar_length(work, ctx)?;
        self.syntax_error(work, subject, offset, length, reason, ctx)
    }

    fn scan_remaining_utf8(
        &mut self,
        work: &mut ImportWorkScope<'_, 1>,
        ctx: &mut ParseExecution<'_, 'scope>,
    ) -> ScanResult<()> {
        while self.cursor < self.bytes.len() {
            let start = self.cursor;
            let lead = self.bytes[start];
            let width = match lead {
                0x00..=0x7f => 1,
                0xc2..=0xdf => 2,
                0xe0..=0xef => 3,
                0xf0..=0xf4 => 4,
                _ => 1,
            };
            let end = start.saturating_add(width).min(self.bytes.len());
            let encoded = &self.bytes[start..end];
            match std::str::from_utf8(encoded) {
                Ok(value) if value.chars().count() == 1 => self.consume(work, width, ctx)?,
                Ok(_) => {
                    let byte_offset = self.absolute_offset(start, ctx)?;
                    return Err(ctx
                        .format_failure(GltfFailure::invalid_json(
                            "json-syntax-map",
                            byte_offset,
                            to_u64(encoded.len()),
                            "glTF JSON UTF-8 scalar probe was ambiguous",
                        ))
                        .into());
                }
                Err(source) => {
                    let byte_length = source.error_len().unwrap_or(encoded.len()).max(1);
                    self.cursor = start;
                    self.flush(work, ctx)?;
                    let byte_offset = self.absolute_offset(start, ctx)?;
                    return Err(ctx
                        .format_failure(GltfFailure::invalid_utf8(
                            source,
                            byte_offset,
                            to_u64(byte_length),
                        ))
                        .into());
                }
            }
        }
        Ok(())
    }

    fn consume(
        &mut self,
        work: &mut ImportWorkScope<'_, 1>,
        amount: usize,
        ctx: &mut ParseExecution<'_, 'scope>,
    ) -> ScanResult<()> {
        if self.cursor - self.polled_cursor + amount > SCAN_CHUNK_BYTES {
            self.flush(work, ctx)?;
        }
        self.cursor = self
            .cursor
            .checked_add(amount)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| ctx.allocation_failure("gltf-json-scan-offset"))?;
        work.complete([to_u64(amount)])?;
        if self.cursor - self.polled_cursor >= SCAN_CHUNK_BYTES {
            self.flush(work, ctx)?;
        }
        Ok(())
    }

    fn flush(
        &mut self,
        _work: &mut ImportWorkScope<'_, 1>,
        ctx: &mut ParseExecution<'_, 'scope>,
    ) -> ScanResult<()> {
        ctx.poll()?;
        self.polled_cursor = self.cursor;
        Ok(())
    }

    fn absolute_offset(
        &self,
        relative: usize,
        ctx: &mut ParseExecution<'_, 'scope>,
    ) -> ParseResult<u64> {
        self.raw_base
            .checked_add(
                u64::try_from(relative)
                    .map_err(|_| ctx.allocation_failure("gltf-json-byte-offset"))?,
            )
            .ok_or_else(|| ctx.allocation_failure("gltf-json-byte-offset"))
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.cursor).copied()
    }
}

struct ScannedString {
    raw_start: usize,
    raw_end: usize,
    content_start: usize,
    content_end: usize,
    digest: [u8; 32],
}

/// Only glTF table and payload names are retained; arbitrary keys never allocate path strings.
/// The extra decode is bounded by the longest recognized ASCII name, even for escaped keys.
fn source_member(bytes: &[u8], content_start: usize, content_end: usize) -> SourceMember {
    let mut scalars = DecodedScalars::new(bytes, content_start, content_end);
    let mut name = [0_u8; crate::document::KHR_MATERIALS_EMISSIVE_STRENGTH.len()];
    let mut length = 0;
    loop {
        match scalars.next_scalar() {
            Ok(None) => break,
            Ok(Some(scalar)) if scalar <= 0x7f && length < name.len() => {
                name[length] = scalar as u8;
                length += 1;
            }
            _ => return SourceMember::Other,
        }
    }
    match &name[..length] {
        b"scenes" => SourceMember::Scenes,
        b"nodes" => SourceMember::Nodes,
        b"meshes" => SourceMember::Meshes,
        b"primitives" => SourceMember::Primitives,
        b"materials" => SourceMember::Materials,
        b"textures" => SourceMember::Textures,
        b"images" => SourceMember::Images,
        b"samplers" => SourceMember::Samplers,
        b"skins" => SourceMember::Skins,
        b"buffers" => SourceMember::Buffers,
        b"bufferViews" => SourceMember::BufferViews,
        b"accessors" => SourceMember::Accessors,
        b"animations" => SourceMember::Animations,
        b"channels" => SourceMember::Channels,
        b"cameras" => SourceMember::Cameras,
        b"extensions" => SourceMember::Extensions,
        b"extras" => SourceMember::Extras,
        b"asset" => SourceMember::Asset,
        b"pbrMetallicRoughness" => SourceMember::PbrMetallicRoughness,
        b"baseColorTexture" => SourceMember::BaseColorTexture,
        b"metallicRoughnessTexture" => SourceMember::MetallicRoughnessTexture,
        b"normalTexture" => SourceMember::NormalTexture,
        b"occlusionTexture" => SourceMember::OcclusionTexture,
        b"emissiveTexture" => SourceMember::EmissiveTexture,
        b"sparse" => SourceMember::Sparse,
        b"indices" => SourceMember::Indices,
        b"values" => SourceMember::Values,
        b"target" => SourceMember::Target,
        b"orthographic" => SourceMember::Orthographic,
        b"perspective" => SourceMember::Perspective,
        b"KHR_texture_transform" => SourceMember::KhrTextureTransform,
        b"KHR_materials_unlit" => SourceMember::KhrMaterialsUnlit,
        b"KHR_materials_emissive_strength" => SourceMember::KhrMaterialsEmissiveStrength,
        b"emissiveStrength" => SourceMember::EmissiveStrength,
        b"KHR_materials_clearcoat" => SourceMember::KhrMaterialsClearcoat,
        b"clearcoatFactor" => SourceMember::ClearcoatFactor,
        b"clearcoatRoughnessFactor" => SourceMember::ClearcoatRoughnessFactor,
        b"clearcoatTexture" => SourceMember::ClearcoatTexture,
        b"clearcoatRoughnessTexture" => SourceMember::ClearcoatRoughnessTexture,
        b"clearcoatNormalTexture" => SourceMember::ClearcoatNormalTexture,
        b"KHR_materials_sheen" => SourceMember::KhrMaterialsSheen,
        b"sheenColorFactor" => SourceMember::SheenColorFactor,
        b"sheenRoughnessFactor" => SourceMember::SheenRoughnessFactor,
        b"sheenColorTexture" => SourceMember::SheenColorTexture,
        b"sheenRoughnessTexture" => SourceMember::SheenRoughnessTexture,
        b"index" => SourceMember::Index,
        b"texCoord" => SourceMember::TexCoord,
        b"offset" => SourceMember::Offset,
        b"rotation" => SourceMember::Rotation,
        b"scale" => SourceMember::Scale,
        b"KHR_lights_punctual" => SourceMember::KhrLightsPunctual,
        b"lights" => SourceMember::Lights,
        b"light" => SourceMember::Light,
        b"spot" => SourceMember::Spot,
        b"name" => SourceMember::Name,
        b"type" => SourceMember::Type,
        b"color" => SourceMember::Color,
        b"intensity" => SourceMember::Intensity,
        b"range" => SourceMember::Range,
        b"innerConeAngle" => SourceMember::InnerConeAngle,
        b"outerConeAngle" => SourceMember::OuterConeAngle,
        _ => SourceMember::Other,
    }
}

/// These are concrete schema property objects, not paths through arbitrary extension JSON.
fn source_property(
    owner: SourceEntityId,
    parent_site: SourcePayloadSite,
    member: SourceMember,
    first: Option<u8>,
) -> SourceContext {
    if first != Some(b'{') {
        return SourceContext::Other;
    }
    let site = match (owner.kind, parent_site, member) {
        (SourceEntityKind::Document, SourcePayloadSite::Entity, SourceMember::Asset) => {
            SourcePayloadSite::Asset
        }
        (SourceEntityKind::Material, SourcePayloadSite::Entity, member) => match member {
            SourceMember::PbrMetallicRoughness => SourcePayloadSite::MaterialPbrMetallicRoughness,
            SourceMember::NormalTexture => SourcePayloadSite::NormalTexture,
            SourceMember::OcclusionTexture => SourcePayloadSite::OcclusionTexture,
            SourceMember::EmissiveTexture => SourcePayloadSite::EmissiveTexture,
            _ => return SourceContext::Other,
        },
        (SourceEntityKind::Material, SourcePayloadSite::MaterialPbrMetallicRoughness, member) => {
            match member {
                SourceMember::BaseColorTexture => SourcePayloadSite::BaseColorTexture,
                SourceMember::MetallicRoughnessTexture => {
                    SourcePayloadSite::MetallicRoughnessTexture
                }
                _ => return SourceContext::Other,
            }
        }
        (SourceEntityKind::Material, SourcePayloadSite::MaterialClearcoat, member) => {
            match member {
                SourceMember::ClearcoatTexture => SourcePayloadSite::ClearcoatTexture,
                SourceMember::ClearcoatRoughnessTexture => {
                    SourcePayloadSite::ClearcoatRoughnessTexture
                }
                SourceMember::ClearcoatNormalTexture => SourcePayloadSite::ClearcoatNormalTexture,
                _ => return SourceContext::Other,
            }
        }
        (SourceEntityKind::Material, SourcePayloadSite::MaterialSheen, member) => match member {
            SourceMember::SheenColorTexture => SourcePayloadSite::SheenColorTexture,
            SourceMember::SheenRoughnessTexture => SourcePayloadSite::SheenRoughnessTexture,
            _ => return SourceContext::Other,
        },
        (SourceEntityKind::Accessor, SourcePayloadSite::Entity, SourceMember::Sparse) => {
            SourcePayloadSite::AccessorSparse
        }
        (SourceEntityKind::Accessor, SourcePayloadSite::AccessorSparse, SourceMember::Indices) => {
            SourcePayloadSite::AccessorSparseIndices
        }
        (SourceEntityKind::Accessor, SourcePayloadSite::AccessorSparse, SourceMember::Values) => {
            SourcePayloadSite::AccessorSparseValues
        }
        (SourceEntityKind::AnimationChannel, SourcePayloadSite::Entity, SourceMember::Target) => {
            SourcePayloadSite::AnimationChannelTarget
        }
        (SourceEntityKind::Camera, SourcePayloadSite::Entity, SourceMember::Orthographic) => {
            SourcePayloadSite::CameraOrthographic
        }
        (SourceEntityKind::Camera, SourcePayloadSite::Entity, SourceMember::Perspective) => {
            SourcePayloadSite::CameraPerspective
        }
        (SourceEntityKind::Light, SourcePayloadSite::Entity, SourceMember::Spot) => {
            SourcePayloadSite::LightSpot
        }
        _ => return SourceContext::Other,
    };
    SourceContext::Property { owner, site }
}

/// A table is recognized only at its schema owner, never by a matching key at an arbitrary depth.
fn source_entity_array(
    owner: SourceEntityId,
    member: SourceMember,
) -> Option<(SourceEntityKind, Option<usize>)> {
    let kind = match (owner.kind, member) {
        (SourceEntityKind::Document, SourceMember::Scenes) => SourceEntityKind::Scene,
        (SourceEntityKind::Document, SourceMember::Nodes) => SourceEntityKind::Node,
        (SourceEntityKind::Document, SourceMember::Meshes) => SourceEntityKind::Mesh,
        (SourceEntityKind::Document, SourceMember::Materials) => SourceEntityKind::Material,
        (SourceEntityKind::Document, SourceMember::Textures) => SourceEntityKind::Texture,
        (SourceEntityKind::Document, SourceMember::Images) => SourceEntityKind::Image,
        (SourceEntityKind::Document, SourceMember::Samplers) => SourceEntityKind::Sampler,
        (SourceEntityKind::Document, SourceMember::Skins) => SourceEntityKind::Skin,
        (SourceEntityKind::Document, SourceMember::Buffers) => SourceEntityKind::Buffer,
        (SourceEntityKind::Document, SourceMember::BufferViews) => SourceEntityKind::BufferView,
        (SourceEntityKind::Document, SourceMember::Accessors) => SourceEntityKind::Accessor,
        (SourceEntityKind::Document, SourceMember::Animations) => SourceEntityKind::Animation,
        (SourceEntityKind::Document, SourceMember::Cameras) => SourceEntityKind::Camera,
        (SourceEntityKind::Mesh, SourceMember::Primitives) => {
            return Some((SourceEntityKind::Primitive, Some(owner.source_index)));
        }
        (SourceEntityKind::Animation, SourceMember::Channels) => {
            return Some((SourceEntityKind::AnimationChannel, Some(owner.source_index)));
        }
        (SourceEntityKind::Animation, SourceMember::Samplers) => {
            return Some((SourceEntityKind::AnimationSampler, Some(owner.source_index)));
        }
        _ => return None,
    };
    Some((kind, None))
}

fn validate_duplicate_keys<'region>(
    ctx: &mut ParseExecution<'region, '_>,
    bytes: &[u8],
    raw_base: u64,
    keys: &mut MeteredVec<'_, JsonKey>,
) -> ParseResult<()> {
    if keys.len() > 1 {
        ctx.with_work_group(
            [(ImportWorkClass::WorkUnits, 0)],
            |ctx, work| -> ParseResult<()> {
                let mut comparisons = 0_u64;
                keys.try_sort_unstable_by(|left, right| -> Result<Ordering, ParseFailure> {
                    if comparisons.is_multiple_of(DUPLICATE_POLL_COMPARISONS) {
                        ctx.poll()?;
                    }
                    work.replenish(1)?;
                    let ordering = left
                        .object_id
                        .cmp(&right.object_id)
                        .then_with(|| left.digest.cmp(&right.digest))
                        .then_with(|| left.raw_start.cmp(&right.raw_start));
                    work.complete([1])?;
                    comparisons += 1;
                    Ok(ordering)
                })?;
                Ok(())
            },
        )?;
    }

    let comparison_work = ctx.bind_work_group([ImportWorkClass::WorkUnits])?;
    let keys = keys.as_slice();
    let mut group_start = 0usize;
    while group_start < keys.len() {
        ctx.poll()?;
        let group_end = ctx.with_work(&comparison_work, [0], |ctx, work| -> ParseResult<_> {
            let mut end = group_start + 1;
            while end < keys.len() {
                if (end - group_start - 1)
                    .is_multiple_of(baozi_import::MAX_GEOMETRY_ACTIONS_BETWEEN_POLLS)
                {
                    ctx.poll()?;
                }
                work.replenish(1)?;
                let same = keys[end].object_id == keys[group_start].object_id
                    && keys[end].digest == keys[group_start].digest;
                work.complete([1])?;
                if !same {
                    break;
                }
                end += 1;
            }
            Ok(end)
        })?;

        for left in group_start..group_end {
            for right in left + 1..group_end {
                if decoded_keys_equal(ctx, &comparison_work, bytes, &keys[left], &keys[right])? {
                    let duplicate = keys[right];
                    let byte_offset = raw_base
                        .checked_add(to_u64(duplicate.raw_start))
                        .ok_or_else(|| ctx.allocation_failure("gltf-json-byte-offset"))?;
                    let byte_length = duplicate
                        .raw_end
                        .checked_sub(duplicate.raw_start)
                        .map(to_u64)
                        .ok_or_else(|| ctx.allocation_failure("gltf-json-key-range"))?;
                    return Err(ctx.format_failure(GltfFailure::invalid_json(
                        "json-object-key",
                        byte_offset,
                        byte_length,
                        "glTF JSON object contains a duplicate key",
                    )));
                }
            }
        }
        group_start = group_end;
    }

    Ok(())
}

fn decoded_keys_equal<'region>(
    ctx: &mut ParseExecution<'region, '_>,
    comparison_work: &ImportWorkGroup<'region, 1>,
    bytes: &[u8],
    left: &JsonKey,
    right: &JsonKey,
) -> ParseResult<bool> {
    ctx.with_work(comparison_work, [0], |ctx, work| -> ParseResult<bool> {
        let mut left = DecodedScalars::new(bytes, left.content_start, left.content_end);
        let mut right = DecodedScalars::new(bytes, right.content_start, right.content_end);
        let mut raw_since_poll = SCAN_CHUNK_BYTES;
        let equal = loop {
            // Two escaped surrogate pairs consume at most 24 raw bytes.
            if raw_since_poll + 24 > SCAN_CHUNK_BYTES {
                ctx.poll()?;
                raw_since_poll = 0;
            }
            work.replenish(1)?;
            let before = (left.cursor, right.cursor);
            let compared = (left.next_scalar(), right.next_scalar());
            work.complete([1])?;
            raw_since_poll += left.cursor - before.0 + right.cursor - before.1;
            match compared {
                (Ok(Some(left)), Ok(Some(right))) if left == right => {}
                (Ok(None), Ok(None)) => break true,
                (Ok(_), Ok(_)) => break false,
                _ => {
                    return Err(ctx.format_failure(GltfFailure::invalid_json(
                        "json-syntax-map",
                        0,
                        0,
                        "validated JSON key could not be decoded during comparison",
                    )));
                }
            }
        };
        Ok(equal)
    })
}

pub(crate) struct DecodedScalars<'bytes> {
    bytes: &'bytes [u8],
    cursor: usize,
    end: usize,
}

impl<'bytes> DecodedScalars<'bytes> {
    pub(crate) const fn new(bytes: &'bytes [u8], cursor: usize, end: usize) -> Self {
        Self { bytes, cursor, end }
    }

    pub(crate) const fn position(&self) -> usize {
        self.cursor
    }

    pub(crate) fn next_scalar(&mut self) -> Result<Option<u32>, ()> {
        if self.cursor == self.end {
            return Ok(None);
        }
        let byte = *self.bytes.get(self.cursor).ok_or(())?;
        if byte == b'\\' {
            let escape = *self.bytes.get(self.cursor + 1).ok_or(())?;
            self.cursor += 2;
            let scalar = match escape {
                b'"' | b'\\' | b'/' => u32::from(escape),
                b'b' => 0x08,
                b'f' => 0x0c,
                b'n' => 0x0a,
                b'r' => 0x0d,
                b't' => 0x09,
                b'u' => {
                    let first = self.read_hex_quad()?;
                    if (0xd800..=0xdbff).contains(&first) {
                        if self.bytes.get(self.cursor) != Some(&b'\\')
                            || self.bytes.get(self.cursor + 1) != Some(&b'u')
                        {
                            return Err(());
                        }
                        self.cursor += 2;
                        let low = self.read_hex_quad()?;
                        if !(0xdc00..=0xdfff).contains(&low) {
                            return Err(());
                        }
                        0x1_0000 + ((u32::from(first) - 0xd800) << 10) + (u32::from(low) - 0xdc00)
                    } else if (0xdc00..=0xdfff).contains(&first) {
                        return Err(());
                    } else {
                        u32::from(first)
                    }
                }
                _ => return Err(()),
            };
            return Ok(Some(scalar));
        }

        if byte < 0x80 {
            self.cursor += 1;
            return Ok(Some(u32::from(byte)));
        }
        let width = match byte {
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => return Err(()),
        };
        let scalar_end = self.cursor.checked_add(width).ok_or(())?;
        if scalar_end > self.end {
            return Err(());
        }
        let encoded = std::str::from_utf8(&self.bytes[self.cursor..scalar_end]).map_err(|_| ())?;
        let scalar = encoded.chars().next().ok_or(())? as u32;
        self.cursor = scalar_end;
        Ok(Some(scalar))
    }

    fn read_hex_quad(&mut self) -> Result<u16, ()> {
        let mut value = 0_u16;
        for _ in 0..4 {
            let digit = hex_value(*self.bytes.get(self.cursor).ok_or(())?).ok_or(())?;
            self.cursor += 1;
            value = (value << 4) | u16::from(digit);
        }
        Ok(value)
    }
}

fn hash_scalar(hasher: &mut Option<Sha256>, scalar: u32) {
    let Some(hasher) = hasher.as_mut() else {
        return;
    };
    let Some(character) = char::from_u32(scalar) else {
        return;
    };
    let mut encoded = [0_u8; 4];
    hasher.update(character.encode_utf8(&mut encoded).as_bytes());
}

const fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

const fn is_json_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n')
}

const fn is_value_delimiter(byte: u8) -> bool {
    is_json_whitespace(byte) || matches!(byte, b',' | b']' | b'}')
}

fn to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use baozi_import::{
        DetectionOutcome, DetectionResult, DetectionScope, Format, FormatDescriptor,
        FormatSelection, ImportConfigOverlay, ImportOperationControl, ImportRequest,
        ImportRequestLayer, OptionCompileLimits, Probe, ReadConfidence, ReadHint,
        RegistryBuildLimits, RegistryBuilder,
    };
    use baozi_io::AssetPath;
    use std::error::Error;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering as AtomicOrdering},
    };

    type TestResult = Result<(), Box<dyn Error>>;
    type Inspect = fn(&mut ParseExecution<'_, '_>, &GltfSourceMap<'_>);

    struct InspectAdmission {
        inspect: Inspect,
        inspected: Arc<AtomicBool>,
    }

    impl Format for InspectAdmission {
        type Options = baozi_import::NoFormatOptions;

        fn compile_defaults(
            &self,
            scope: &mut baozi_import::RegistryBuildScope<'_>,
        ) -> std::result::Result<
            baozi_import::CompiledOptions<Self::Options>,
            baozi_import::OptionCompileError,
        > {
            Self::Options::compile_defaults(scope)
        }

        fn descriptor(&self) -> FormatDescriptor {
            crate::format_descriptor()
        }

        fn detect(
            &self,
            _: &Probe<'_>,
            _: &ReadHint,
            _: &DetectionScope<'_>,
        ) -> DetectionResult<DetectionOutcome> {
            Ok(DetectionOutcome::Match(ReadConfidence::Certain))
        }

        fn parse(
            &self,
            _: &Self::Options,
            context: &mut baozi_import::ParseContext<'_>,
        ) -> ParseResult<()> {
            context.with_execution(|scope| {
                let map = GltfSourceMap::from_primary_for_test(scope)?;
                (self.inspect)(scope, &map);
                self.inspected.store(true, AtomicOrdering::SeqCst);
                // The test stops at admission, before either backend decode or scene construction.
                Err(scope.format_failure(GltfFailure::internal("source map inspection completed")))
            })
        }
    }

    pub(super) fn admission_plan(
        inspect: Inspect,
        inspected: Arc<AtomicBool>,
    ) -> Result<baozi_import::ImportPlan, Box<dyn Error>> {
        let mut registry = RegistryBuilder::new();
        registry.register(InspectAdmission { inspect, inspected })?;
        let request = registry.freeze(RegistryBuildLimits::default())?.plan(
            &ImportRequest::from_layers(
                ImportRequestLayer::default(),
                ImportRequestLayer::new(ImportConfigOverlay::default().with_format_selection(
                    FormatSelection::Forced(baozi_import::authoring::FormatId::new(
                        crate::FORMAT_ID,
                    )?),
                )),
            ),
            OptionCompileLimits::default(),
        )?;
        Ok(request)
    }

    fn inspect_admission(bytes: &[u8], inspect: Inspect) -> TestResult {
        let inspected = Arc::new(AtomicBool::new(false));
        let request = admission_plan(inspect, Arc::clone(&inspected))?;
        let result = request.read_bytes(
            AssetPath::new("mapping.gltf")?,
            bytes,
            ImportOperationControl::default(),
        );
        assert!(
            inspected.load(AtomicOrdering::SeqCst),
            "admission failed before inspection"
        );
        let failure = result
            .expect_err("inspection must stop before scene construction")
            .into_operation_failure()?;
        assert_eq!(failure.ledger().current_live_bytes(), 0);
        Ok(())
    }

    fn id(
        kind: SourceEntityKind,
        parent_index: Option<usize>,
        source_index: usize,
    ) -> SourceEntityId {
        SourceEntityId {
            kind,
            parent_index,
            source_index,
        }
    }

    fn assert_raw(scope: &mut ParseExecution<'_, '_>, range: SourceRange, expected: &str) {
        let raw = range.raw_range();
        let start = usize::try_from(raw.start).expect("fixture offset fits usize");
        let end = usize::try_from(raw.end).expect("fixture offset fits usize");
        assert_eq!(
            scope.primary_bytes().get(start..end),
            Some(expected.as_bytes())
        );
    }

    pub(super) fn glb(json: &[u8]) -> Vec<u8> {
        let json_length = (json.len() + 3) & !3;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"glTF");
        bytes.extend_from_slice(&2_u32.to_le_bytes());
        bytes.extend_from_slice(&u32::try_from(20 + json_length).unwrap().to_le_bytes());
        bytes.extend_from_slice(&u32::try_from(json_length).unwrap().to_le_bytes());
        bytes.extend_from_slice(b"JSON");
        bytes.extend_from_slice(json);
        bytes.resize(20 + json_length, b' ');
        bytes
    }

    #[test]
    fn raw_ranges_preserve_unicode_escaped_keys_and_glb_offsets() -> TestResult {
        let json = " \r\n{\"asset\":{\"version\":\"2.0\"},\"n\\u006fdes\":[{\"name\":\"包子🦀\",\"e\\u0078tras\": [\"原始\",{\"nodes\":[{}]}]}]}\n";
        fn inspect(scope: &mut ParseExecution<'_, '_>, map: &GltfSourceMap<'_>) {
            let base = if scope.primary_bytes().starts_with(b"glTF") {
                20
            } else {
                0
            };
            let document = map.get(id(SourceEntityKind::Document, None, 0)).unwrap();
            assert_eq!(document.range.byte_offset, base + 3);
            assert_raw(
                scope,
                document.range,
                "{\"asset\":{\"version\":\"2.0\"},\"n\\u006fdes\":[{\"name\":\"包子🦀\",\"e\\u0078tras\": [\"原始\",{\"nodes\":[{}]}]}]}",
            );
            let node = map.get(id(SourceEntityKind::Node, None, 0)).unwrap();
            assert_raw(
                scope,
                node.range,
                "{\"name\":\"包子🦀\",\"e\\u0078tras\": [\"原始\",{\"nodes\":[{}]}]}",
            );
            let payloads = map.payloads_for(node.id);
            assert_eq!(payloads.len(), 1);
            assert_eq!(payloads[0].kind, SourcePayloadKind::Extras);
            assert_raw(scope, payloads[0].key_range, "\"e\\u0078tras\"");
            assert_raw(
                scope,
                payloads[0].member_range,
                "\"e\\u0078tras\": [\"原始\",{\"nodes\":[{}]}]",
            );
            assert_raw(
                scope,
                payloads[0].value_range,
                "[\"原始\",{\"nodes\":[{}]}]",
            );
            assert_eq!(
                map.entities().len(),
                2,
                "payload contents must not create entities"
            );
        }
        inspect_admission(json.as_bytes(), inspect)?;
        inspect_admission(&glb(json.as_bytes()), inspect)
    }

    #[test]
    fn indexed_entities_keep_local_ordinals_and_missing_ranges() -> TestResult {
        let json = br#"{
            "scenes":[{"name":"scene"}], "nodes":[null,{"name":"node"},[{}]],
            "meshes":[{"primitives":[{"name":"m0p0"},{"name":"m0p1"}]},
                      {"primitives":[{"name":"m1p0"}]}],
            "materials":[{"name":"material"}], "textures":[{"name":"texture"}],
            "images":[{"name":"image"}], "samplers":[{"name":"sampler"}],
            "skins":[{"name":"skin"}], "buffers":[{"name":"buffer"}],
            "bufferViews":[{"name":"view"}], "accessors":[{"name":"accessor"}],
            "animations":[{"channels":[],"samplers":[{"name":"a0s0"}]},
                          {"channels":[{"name":"a1c0"}],"samplers":[{"name":"a1s0"}]}],
            "cameras":[{"name":"camera"}], "extras":{"meshes":[{}]}
        }"#;
        inspect_admission(json, |scope, map| {
            let top_level = [
                (SourceEntityKind::Scene, "scene"),
                (SourceEntityKind::Material, "material"),
                (SourceEntityKind::Texture, "texture"),
                (SourceEntityKind::Image, "image"),
                (SourceEntityKind::Sampler, "sampler"),
                (SourceEntityKind::Skin, "skin"),
                (SourceEntityKind::Buffer, "buffer"),
                (SourceEntityKind::BufferView, "view"),
                (SourceEntityKind::Accessor, "accessor"),
                (SourceEntityKind::Camera, "camera"),
            ];
            for (kind, name) in top_level {
                assert_raw(
                    scope,
                    map.get(id(kind, None, 0)).unwrap().range,
                    &format!(r#"{{"name":"{name}"}}"#),
                );
            }
            assert_raw(
                scope,
                map.get(id(SourceEntityKind::Node, None, 1)).unwrap().range,
                r#"{"name":"node"}"#,
            );
            assert!(map.get(id(SourceEntityKind::Node, None, 0)).is_none());
            assert!(map.get(id(SourceEntityKind::Node, None, 2)).is_none());
            for (kind, parent, index, name) in [
                (SourceEntityKind::Primitive, 0, 0, "m0p0"),
                (SourceEntityKind::Primitive, 0, 1, "m0p1"),
                (SourceEntityKind::Primitive, 1, 0, "m1p0"),
                (SourceEntityKind::AnimationSampler, 0, 0, "a0s0"),
                (SourceEntityKind::AnimationSampler, 1, 0, "a1s0"),
                (SourceEntityKind::AnimationChannel, 1, 0, "a1c0"),
            ] {
                assert_raw(
                    scope,
                    map.get(id(kind, Some(parent), index)).unwrap().range,
                    &format!(r#"{{"name":"{name}"}}"#),
                );
            }
            assert!(
                map.get(id(SourceEntityKind::AnimationChannel, Some(0), 0))
                    .is_none()
            );
            assert!(
                map.get(id(SourceEntityKind::Primitive, Some(2), 0))
                    .is_none()
            );
            assert_eq!(map.entities().len(), 22);
        })
    }

    #[test]
    fn payloads_retain_owners_sites_unknown_names_and_all_texture_binding_locations() -> TestResult
    {
        let json = br#"{
          "asset":{"extras":null},
          "materials":[{
            "extensions":{"KHR_texture_transform":{"extras":{"nodes":[{}]}}},
            "pbrMetallicRoughness":{
              "extras":true,
              "baseColorTexture":{"extensions":{"KHR_texture_transform":{"texCoord":0,"extras":null},"VENDOR_\u0078":[]}},
              "metallicRoughnessTexture":{"extensions":{"KHR_texture_transform":{"texCoord":1}}}
            },
            "normalTexture":{"extensions":{"KHR_texture_transform":{"texCoord":2}}},
            "occlusionTexture":{"extensions":{"KHR_texture_transform":{"texCoord":3}}},
            "emissiveTexture":{"e\u0078tras":"raw", "extensions":{"KHR_texture_\u0074ransform":{"texCoord":4}}}
          }],
          "nodes":[{"extras":false,"extensions":{}},{"extensions":{"extras":0}}],
          "accessors":[{"sparse":{"extras":[],"indices":{"extras":2},"values":{"extensions":{"VENDOR_sparse":{}}}}}],
          "animations":[{"channels":[{"target":{"extras":3}}],"samplers":[{"extras":4}]}],
          "cameras":[{"perspective":{"extras":5}}]
        }"#;
        inspect_admission(json, |scope, map| {
            let material = id(SourceEntityKind::Material, None, 0);
            let payloads = map.payloads_for(material);
            for (site, tex_coord) in [
                (SourcePayloadSite::BaseColorTexture, 0),
                (SourcePayloadSite::MetallicRoughnessTexture, 1),
                (SourcePayloadSite::NormalTexture, 2),
                (SourcePayloadSite::OcclusionTexture, 3),
                (SourcePayloadSite::EmissiveTexture, 4),
            ] {
                let payload = payloads
                    .iter()
                    .find(|payload| {
                        payload.site == site
                            && payload.kind
                                == SourcePayloadKind::Extension(
                                    SourceExtensionKind::KhrTextureTransform,
                                )
                    })
                    .unwrap();
                let expected = if tex_coord == 0 {
                    r#"{"texCoord":0,"extras":null}"#.to_owned()
                } else {
                    format!(r#"{{"texCoord":{tex_coord}}}"#)
                };
                assert_raw(scope, payload.value_range, &expected);
            }
            let unknown = payloads
                .iter()
                .find(|payload| {
                    payload.kind == SourcePayloadKind::Extension(SourceExtensionKind::Other)
                })
                .unwrap();
            assert_eq!(unknown.site, SourcePayloadSite::BaseColorTexture);
            assert_raw(scope, unknown.key_range, r#""VENDOR_\u0078""#);
            assert_raw(scope, unknown.value_range, "[]");
            let unsupported_site = payloads
                .iter()
                .find(|payload| {
                    payload.site == SourcePayloadSite::Entity
                        && payload.kind
                            == SourcePayloadKind::Extension(
                                SourceExtensionKind::KhrTextureTransform,
                            )
                })
                .unwrap();
            assert_raw(
                scope,
                unsupported_site.value_range,
                r#"{"extras":{"nodes":[{}]}}"#,
            );
            assert_eq!(
                map.entities()
                    .iter()
                    .filter(|entity| entity.id.kind == SourceEntityKind::Node)
                    .count(),
                2
            );
            let node_payloads = map.payloads_for(id(SourceEntityKind::Node, None, 1));
            assert_eq!(node_payloads.len(), 2);
            assert_eq!(
                node_payloads[1].kind,
                SourcePayloadKind::Extension(SourceExtensionKind::Other)
            );
            assert_raw(scope, node_payloads[1].key_range, r#""extras""#);
            for (owner, site) in [
                (
                    id(SourceEntityKind::Document, None, 0),
                    SourcePayloadSite::Asset,
                ),
                (material, SourcePayloadSite::MaterialPbrMetallicRoughness),
                (material, SourcePayloadSite::EmissiveTexture),
                (material, SourcePayloadSite::BaseColorTextureTransform),
                (
                    id(SourceEntityKind::Accessor, None, 0),
                    SourcePayloadSite::AccessorSparse,
                ),
                (
                    id(SourceEntityKind::Accessor, None, 0),
                    SourcePayloadSite::AccessorSparseIndices,
                ),
                (
                    id(SourceEntityKind::AnimationChannel, Some(0), 0),
                    SourcePayloadSite::AnimationChannelTarget,
                ),
                (
                    id(SourceEntityKind::AnimationSampler, Some(0), 0),
                    SourcePayloadSite::Entity,
                ),
                (
                    id(SourceEntityKind::Camera, None, 0),
                    SourcePayloadSite::CameraPerspective,
                ),
            ] {
                assert!(
                    map.payloads_for(owner)
                        .iter()
                        .any(|payload| payload.site == site
                            && payload.kind == SourcePayloadKind::Extras),
                    "missing payload at {owner:?}/{site:?}"
                );
            }
            assert!(
                map.payloads_for(id(SourceEntityKind::Node, None, 99))
                    .is_empty()
            );
        })
    }

    #[test]
    fn material_unlit_shapes_keep_raw_ranges_and_glb_offsets() -> TestResult {
        let json = r#"{
            "materials":[
                {"extensions":{"KHR_materials_unlit" : {}}},
                {"extensions":{"KHR_materials_unlit" : {"extras":null}}},
                {"extensions":{"KHR_materials_unlit" : {"extras":7,"extensions":{}}}},
                {"extensions":{"KHR_materials_unlit" : {"extras":"包子🦀","extensions":null}}},
                {"extensions":{"KHR_materials_unlit" : {"extensions":false}}},
                {"extensions":{"KHR_materials_unlit" : null}},
                {"extensions":{"KHR_materials_unlit" : 7}},
                {"extensions":{"KHR_materials_unlit" : true}},
                {"extensions":{"KHR_materials_unlit" : "raw"}},
                {"extensions":{"KHR_materials_unlit" : [{"extras":false,"extensions":{"KHR_materials_unlit":{}}}]}}
            ]
        }"#;
        fn inspect(scope: &mut ParseExecution<'_, '_>, map: &GltfSourceMap<'_>) {
            let expected = [
                ("{}", None, None),
                (r#"{"extras":null}"#, Some("null"), None),
                (r#"{"extras":7,"extensions":{}}"#, Some("7"), Some("{}")),
                (
                    r#"{"extras":"包子🦀","extensions":null}"#,
                    Some(r#""包子🦀""#),
                    Some("null"),
                ),
                (r#"{"extensions":false}"#, None, Some("false")),
                ("null", None, None),
                ("7", None, None),
                ("true", None, None),
                (r#""raw""#, None, None),
                (
                    r#"[{"extras":false,"extensions":{"KHR_materials_unlit":{}}}]"#,
                    None,
                    None,
                ),
            ];
            assert_eq!(map.entities().len(), expected.len() + 1);
            for (index, (value, extras, extensions)) in expected.into_iter().enumerate() {
                let owner = id(SourceEntityKind::Material, None, index);
                let payloads = map.payloads_for(owner);
                assert_eq!(
                    payloads.len(),
                    2 + usize::from(extras.is_some()) + usize::from(extensions.is_some())
                );
                let unlit = payloads
                    .iter()
                    .find(|payload| {
                        payload.kind
                            == SourcePayloadKind::Extension(SourceExtensionKind::KhrMaterialsUnlit)
                    })
                    .unwrap();
                assert_eq!(unlit.site, SourcePayloadSite::Entity);
                assert_raw(scope, unlit.key_range, r#""KHR_materials_unlit""#);
                assert_raw(
                    scope,
                    unlit.member_range,
                    &format!(r#""KHR_materials_unlit" : {value}"#),
                );
                assert_raw(scope, unlit.value_range, value);
                for (kind, value) in [
                    (SourcePayloadKind::Extras, extras),
                    (SourcePayloadKind::Extensions, extensions),
                ] {
                    if let Some(value) = value {
                        let payload = payloads
                            .iter()
                            .find(|payload| {
                                payload.site == SourcePayloadSite::MaterialUnlit
                                    && payload.kind == kind
                            })
                            .unwrap();
                        assert_raw(scope, payload.value_range, value);
                    }
                }
            }
        }
        inspect_admission(json.as_bytes(), inspect)?;
        inspect_admission(&glb(json.as_bytes()), inspect)
    }

    #[test]
    fn material_unlit_nested_extensions_keep_sites_and_texture_transform_ranges() -> TestResult {
        let json = br#"{
            "materials":[{
                "extensions":{"KHR_materials_unlit":{
                    "extras":{"nodes":[{}]},
                    "extensions":{
                        "VENDOR_nested":{"extras":0},
                        "KHR_texture_transform":{"extras":1},
                        "KHR_materials_unlit":{"extras":2}
                    }
                }},
                "pbrMetallicRoughness":{"baseColorTexture":{"extensions":{
                    "KHR_texture_transform":{"texCoord":1,"offset":[1,2],"rotation":0.5,"scale":[2,2],"extras":null,"extensions":{"VENDOR_transform":0}}
                }}}
            }]
        }"#;
        inspect_admission(json, |scope, map| {
            assert_eq!(map.entities().len(), 2);
            let payloads = map.payloads_for(id(SourceEntityKind::Material, None, 0));
            assert_eq!(payloads.len(), 12);
            assert_eq!(
                payloads
                    .iter()
                    .filter(|payload| payload.site == SourcePayloadSite::MaterialUnlit)
                    .count(),
                5
            );
            for (extension, key, value) in [
                (
                    SourceExtensionKind::Other,
                    "VENDOR_nested",
                    r#"{"extras":0}"#,
                ),
                (
                    SourceExtensionKind::KhrTextureTransform,
                    "KHR_texture_transform",
                    r#"{"extras":1}"#,
                ),
                (
                    SourceExtensionKind::KhrMaterialsUnlit,
                    "KHR_materials_unlit",
                    r#"{"extras":2}"#,
                ),
            ] {
                let payload = payloads
                    .iter()
                    .find(|payload| {
                        payload.site == SourcePayloadSite::MaterialUnlit
                            && payload.kind == SourcePayloadKind::Extension(extension)
                    })
                    .unwrap();
                assert_raw(scope, payload.key_range, &format!(r#""{key}""#));
                assert_raw(scope, payload.value_range, value);
            }
            let transform = payloads
                .iter()
                .find(|payload| {
                    payload.site == SourcePayloadSite::BaseColorTexture
                        && payload.kind
                            == SourcePayloadKind::Extension(
                                SourceExtensionKind::KhrTextureTransform,
                            )
                })
                .unwrap();
            let value = r#"{"texCoord":1,"offset":[1,2],"rotation":0.5,"scale":[2,2],"extras":null,"extensions":{"VENDOR_transform":0}}"#;
            assert_raw(scope, transform.key_range, r#""KHR_texture_transform""#);
            assert_raw(scope, transform.value_range, value);
            assert_raw(
                scope,
                transform.member_range,
                &format!(r#""KHR_texture_transform":{value}"#),
            );
            for (site, kind, value) in [
                (
                    SourcePayloadSite::MaterialUnlit,
                    SourcePayloadKind::Extras,
                    r#"{"nodes":[{}]}"#,
                ),
                (
                    SourcePayloadSite::BaseColorTextureTransform,
                    SourcePayloadKind::Extras,
                    "null",
                ),
                (
                    SourcePayloadSite::BaseColorTextureTransform,
                    SourcePayloadKind::Extensions,
                    r#"{"VENDOR_transform":0}"#,
                ),
                (
                    SourcePayloadSite::BaseColorTextureTransform,
                    SourcePayloadKind::Extension(SourceExtensionKind::Other),
                    "0",
                ),
            ] {
                let payload = payloads
                    .iter()
                    .find(|payload| payload.site == site && payload.kind == kind)
                    .unwrap();
                assert_raw(scope, payload.value_range, value);
            }
            assert!(
                payloads
                    .iter()
                    .all(|payload| payload.kind != SourcePayloadKind::ExtensionProperty)
            );
        })
    }

    #[test]
    fn escaped_material_unlit_keys_preserve_raw_member_ranges() -> TestResult {
        let json = r#"{
            "ma\u0074erials":[{
                "e\u0078tensions":{
                    "KHR_materials_un\u006cit" : {
                        "e\u0078tras":"包子🦀",
                        "e\u0078tensions":{},
                        "un\u006bnown" : [null, {"extras":0}]
                    }
                }
            }]
        }"#;
        fn inspect(scope: &mut ParseExecution<'_, '_>, map: &GltfSourceMap<'_>) {
            assert_eq!(map.entities().len(), 2);
            let payloads = map.payloads_for(id(SourceEntityKind::Material, None, 0));
            assert_eq!(payloads.len(), 5);
            for (site, kind, key, member, value) in [
                (
                    SourcePayloadSite::MaterialUnlit,
                    SourcePayloadKind::Extras,
                    r#""e\u0078tras""#,
                    r#""e\u0078tras":"包子🦀""#,
                    r#""包子🦀""#,
                ),
                (
                    SourcePayloadSite::MaterialUnlit,
                    SourcePayloadKind::Extensions,
                    r#""e\u0078tensions""#,
                    r#""e\u0078tensions":{}"#,
                    "{}",
                ),
                (
                    SourcePayloadSite::MaterialUnlit,
                    SourcePayloadKind::ExtensionProperty,
                    r#""un\u006bnown""#,
                    r#""un\u006bnown" : [null, {"extras":0}]"#,
                    r#"[null, {"extras":0}]"#,
                ),
            ] {
                let payload = payloads
                    .iter()
                    .find(|payload| payload.site == site && payload.kind == kind)
                    .unwrap();
                assert_raw(scope, payload.key_range, key);
                assert_raw(scope, payload.member_range, member);
                assert_raw(scope, payload.value_range, value);
            }
            let unlit = payloads
                .iter()
                .find(|payload| {
                    payload.kind
                        == SourcePayloadKind::Extension(SourceExtensionKind::KhrMaterialsUnlit)
                })
                .unwrap();
            assert_eq!(unlit.site, SourcePayloadSite::Entity);
            assert_raw(scope, unlit.key_range, r#""KHR_materials_un\u006cit""#);
            let value = r#"{
                        "e\u0078tras":"包子🦀",
                        "e\u0078tensions":{},
                        "un\u006bnown" : [null, {"extras":0}]
                    }"#;
            assert_raw(scope, unlit.value_range, value);
            assert_raw(
                scope,
                unlit.member_range,
                &format!(r#""KHR_materials_un\u006cit" : {value}"#),
            );
        }
        inspect_admission(json.as_bytes(), inspect)?;
        inspect_admission(&glb(json.as_bytes()), inspect)
    }

    #[test]
    fn material_unlit_unknown_direct_properties_remain_observable_and_opaque() -> TestResult {
        let json = br#"{
            "materials":[{"extensions":{"KHR_materials_unlit":{
                "nodes":[{}],
                "normalTexture":{"extras":1,"extensions":{"KHR_materials_unlit":{}}},
                "KHR_materials_unlit":{},
                "nullable":null,
                "enabled":true,
                "number":7,
                "label":"raw"
            }}}]
        }"#;
        inspect_admission(json, |scope, map| {
            assert_eq!(map.entities().len(), 2);
            assert!(map.get(id(SourceEntityKind::Node, None, 0)).is_none());
            let payloads = map.payloads_for(id(SourceEntityKind::Material, None, 0));
            let expected = [
                ("nodes", "[{}]"),
                (
                    "normalTexture",
                    r#"{"extras":1,"extensions":{"KHR_materials_unlit":{}}}"#,
                ),
                ("KHR_materials_unlit", "{}"),
                ("nullable", "null"),
                ("enabled", "true"),
                ("number", "7"),
                ("label", r#""raw""#),
            ];
            assert_eq!(payloads.len(), expected.len() + 2);
            let properties = payloads
                .iter()
                .filter(|payload| payload.site == SourcePayloadSite::MaterialUnlit);
            assert_eq!(properties.clone().count(), expected.len());
            for (payload, (key, value)) in properties.zip(expected) {
                assert_eq!(payload.kind, SourcePayloadKind::ExtensionProperty);
                assert_raw(scope, payload.key_range, &format!(r#""{key}""#));
                assert_raw(scope, payload.member_range, &format!(r#""{key}":{value}"#));
                assert_raw(scope, payload.value_range, value);
            }
        })
    }

    #[test]
    fn material_unlit_wrong_owners_and_sites_keep_raw_payloads() -> TestResult {
        let json = br#"{
            "extensions":{"KHR_materials_unlit":{"extras":0}},
            "asset":{"extensions":{"KHR_materials_unlit":{"extras":1}}},
            "nodes":[{"extensions":{"KHR_materials_unlit":{"extras":2}}}],
            "textures":[{"extensions":{"KHR_materials_unlit":{"extras":3}}}],
            "materials":[{
                "pbrMetallicRoughness":{
                    "extensions":{"KHR_materials_unlit":{"extras":4}},
                    "baseColorTexture":{"extensions":{
                        "KHR_materials_unlit":{"extras":5},
                        "KHR_texture_transform":{"extensions":{"KHR_materials_unlit":{"extras":6}}}
                    }}
                },
                "normalTexture":{"extensions":{"KHR_materials_unlit":{"extras":7}}},
                "extensions":{"KHR_materials_unlit":{"extensions":{"KHR_materials_unlit":{"extras":8}}}}
            }]
        }"#;
        inspect_admission(json, |scope, map| {
            let document = id(SourceEntityKind::Document, None, 0);
            let material = id(SourceEntityKind::Material, None, 0);
            for (owner, site, marker) in [
                (document, SourcePayloadSite::Entity, 0),
                (document, SourcePayloadSite::Asset, 1),
                (
                    id(SourceEntityKind::Node, None, 0),
                    SourcePayloadSite::Entity,
                    2,
                ),
                (
                    id(SourceEntityKind::Texture, None, 0),
                    SourcePayloadSite::Entity,
                    3,
                ),
                (material, SourcePayloadSite::MaterialPbrMetallicRoughness, 4),
                (material, SourcePayloadSite::BaseColorTexture, 5),
                (material, SourcePayloadSite::BaseColorTextureTransform, 6),
                (material, SourcePayloadSite::NormalTexture, 7),
                (material, SourcePayloadSite::MaterialUnlit, 8),
            ] {
                let payload = map
                    .payloads_for(owner)
                    .iter()
                    .find(|payload| {
                        payload.site == site
                            && payload.kind
                                == SourcePayloadKind::Extension(
                                    SourceExtensionKind::KhrMaterialsUnlit,
                                )
                    })
                    .unwrap();
                assert_raw(scope, payload.key_range, r#""KHR_materials_unlit""#);
                let value = format!(r#"{{"extras":{marker}}}"#);
                assert_raw(scope, payload.value_range, &value);
                assert_raw(
                    scope,
                    payload.member_range,
                    &format!(r#""KHR_materials_unlit":{value}"#),
                );
            }
            assert_eq!(
                map.payloads()
                    .iter()
                    .filter(|payload| {
                        payload.kind
                            == SourcePayloadKind::Extension(SourceExtensionKind::KhrMaterialsUnlit)
                    })
                    .count(),
                10
            );
            assert!(map.payloads().iter().all(|payload| {
                !matches!(
                    payload.kind,
                    SourcePayloadKind::Extras | SourcePayloadKind::ExtensionProperty
                )
            }));
        })
    }

    #[test]
    fn unlit_keys_inside_extras_do_not_create_semantic_objects() -> TestResult {
        let json = br#"{
            "extras":{"materials":[{"extensions":{"KHR_materials_unlit":{}}}]},
            "materials":[{
                "extras":{"extensions":{"KHR_materials_unlit":{"extras":null}}},
                "extensions":{"KHR_materials_unlit":{
                    "extras":{"nodes":[{}],"materials":[{}],"extensions":{"KHR_materials_unlit":{}},"KHR_materials_unlit":{"extras":null}}
                }}
            }]
        }"#;
        inspect_admission(json, |scope, map| {
            assert_eq!(map.entities().len(), 2);
            assert!(map.get(id(SourceEntityKind::Node, None, 0)).is_none());
            assert!(map.get(id(SourceEntityKind::Material, None, 1)).is_none());
            assert_eq!(map.payloads().len(), 5);
            let payloads = map.payloads_for(id(SourceEntityKind::Material, None, 0));
            assert_eq!(payloads.len(), 4);
            let extras = payloads
                .iter()
                .find(|payload| payload.site == SourcePayloadSite::MaterialUnlit)
                .unwrap();
            assert_eq!(extras.kind, SourcePayloadKind::Extras);
            assert_raw(
                scope,
                extras.value_range,
                r#"{"nodes":[{}],"materials":[{}],"extensions":{"KHR_materials_unlit":{}},"KHR_materials_unlit":{"extras":null}}"#,
            );
        })
    }

    #[test]
    fn clearcoat_sheen_nested_sites_keep_exact_ranges_and_material_ordinals() -> TestResult {
        let json = br#"{
            "materials":[null,{
                "extras":{"parent":true},
                "exte\u006esions":{
                    "KHR_materials_clearc\u006fat" : {
                        "clearcoatFac\u0074or":0,"clearcoatRoughnessFactor":0.25,
                        "extr\u0061s" : {"material":"\u0063","nodes":[{}],"extensions":{"KHR_materials_sheen":{"sheenColorTexture":{"extras":1}}}},
                        "clearcoatText\u0075re":{"index":0,"texCo\u006frd":1,"extr\u0061s" : [0],"extensions":{
                            "KHR_texture_tran\u0073form" : {"texCoord":1,"offset":[1,2],"rotation":0.5,"scale":[2,3],"extr\u0061s" : { "t" : 0 }}
                        }},
                        "clearcoatRoughnessText\u0075re":{"index":1,"extr\u0061s" : {"slot":1},"extensions":{
                            "KHR_texture_tran\u0073form" : {"extr\u0061s" : [1, 2]}
                        }},
                        "clearcoatNormalText\u0075re":{"index":2,"scale":-2,"extr\u0061s" : false,"extensions":{
                            "KHR_texture_tran\u0073form" : {"extr\u0061s" : null}
                        }}
                    },
                    "KHR_materials_sh\u0065en" : {
                        "sheenColorFac\u0074or":[0,0.5,1],"sheenRoughnessFactor":0.75,
                        "extr\u0061s" : null,
                        "sheenColorText\u0075re":{"index":3,"extr\u0061s" : "sheen","extensions":{
                            "KHR_texture_tran\u0073form" : {"extr\u0061s" : true}
                        }},
                        "sheenRoughnessText\u0075re":{"index":4,"extr\u0061s" : 7,"extensions":{
                            "KHR_texture_tran\u0073form" : {"extr\u0061s" : "tail"}
                        }}
                    }
                }
            }]
        }"#;
        fn inspect(scope: &mut ParseExecution<'_, '_>, map: &GltfSourceMap<'_>) {
            use SourcePayloadSite as Site;
            assert_eq!(map.entities().len(), 2);
            assert!(map.get(id(SourceEntityKind::Material, None, 0)).is_none());
            let owner = id(SourceEntityKind::Material, None, 1);
            assert!(map.get(owner).is_some());
            let payloads = map.payloads_for(owner);
            assert_eq!(payloads.len(), 26);
            assert_eq!(map.payloads().len(), payloads.len());
            assert!(payloads.iter().all(|payload| {
                payload.owner == owner && payload.kind != SourcePayloadKind::ExtensionProperty
            }));
            for (kind, raw_key) in [
                (
                    SourceExtensionKind::KhrMaterialsClearcoat,
                    r#""KHR_materials_clearc\u006fat""#,
                ),
                (
                    SourceExtensionKind::KhrMaterialsSheen,
                    r#""KHR_materials_sh\u0065en""#,
                ),
            ] {
                let payload = payloads
                    .iter()
                    .find(|payload| payload.kind == SourcePayloadKind::Extension(kind))
                    .unwrap();
                assert_eq!(payload.site, Site::Entity);
                assert_raw(scope, payload.key_range, raw_key);
                assert_eq!(
                    payload.value_range.byte_offset,
                    payload.member_range.byte_offset + raw_key.len() as u64 + 3,
                );
            }
            for (site, value) in [
                (
                    Site::MaterialClearcoat,
                    r#"{"material":"\u0063","nodes":[{}],"extensions":{"KHR_materials_sheen":{"sheenColorTexture":{"extras":1}}}}"#,
                ),
                (Site::MaterialSheen, "null"),
                (Site::ClearcoatTexture, "[0]"),
                (Site::ClearcoatRoughnessTexture, r#"{"slot":1}"#),
                (Site::ClearcoatNormalTexture, "false"),
                (Site::SheenColorTexture, r#""sheen""#),
                (Site::SheenRoughnessTexture, "7"),
                (Site::ClearcoatTextureTransform, "{ \"t\" : 0 }"),
                (Site::ClearcoatRoughnessTextureTransform, "[1, 2]"),
                (Site::ClearcoatNormalTextureTransform, "null"),
                (Site::SheenColorTextureTransform, "true"),
                (Site::SheenRoughnessTextureTransform, r#""tail""#),
            ] {
                let payload = payloads
                    .iter()
                    .find(|payload| {
                        payload.site == site && payload.kind == SourcePayloadKind::Extras
                    })
                    .unwrap();
                assert_raw(scope, payload.key_range, r#""extr\u0061s""#);
                assert_raw(
                    scope,
                    payload.member_range,
                    &format!(r#""extr\u0061s" : {value}"#),
                );
                assert_raw(scope, payload.value_range, value);
            }
            for (site, value) in [
                (
                    Site::ClearcoatTexture,
                    r#"{"texCoord":1,"offset":[1,2],"rotation":0.5,"scale":[2,3],"extr\u0061s" : { "t" : 0 }}"#,
                ),
                (
                    Site::ClearcoatRoughnessTexture,
                    r#"{"extr\u0061s" : [1, 2]}"#,
                ),
                (Site::ClearcoatNormalTexture, r#"{"extr\u0061s" : null}"#),
                (Site::SheenColorTexture, r#"{"extr\u0061s" : true}"#),
                (Site::SheenRoughnessTexture, r#"{"extr\u0061s" : "tail"}"#),
            ] {
                let payload = payloads
                    .iter()
                    .find(|payload| {
                        payload.site == site
                            && payload.kind
                                == SourcePayloadKind::Extension(
                                    SourceExtensionKind::KhrTextureTransform,
                                )
                    })
                    .unwrap();
                assert_raw(scope, payload.key_range, r#""KHR_texture_tran\u0073form""#);
                assert_raw(
                    scope,
                    payload.member_range,
                    &format!(r#""KHR_texture_tran\u0073form" : {value}"#),
                );
                assert_raw(scope, payload.value_range, value);
            }
        }
        inspect_admission(json, inspect)?;
        inspect_admission(&glb(json), inspect)
    }

    #[test]
    fn clearcoat_sheen_payload_shapes_keep_null_and_scalar_ranges() -> TestResult {
        let json = br#"{"materials":[
            {"extensions":{"KHR_materials_clearcoat" : {}}},
            {"extensions":{"KHR_materials_clearcoat" : null}},
            {"extensions":{"KHR_materials_clearcoat" : 7}},
            {"extensions":{"KHR_materials_clearcoat" : false}},
            {"extensions":{"KHR_materials_clearcoat" : "raw"}},
            {"extensions":{"KHR_materials_clearcoat" : [{"extras":1,"clearcoatTexture":{"extras":2}}]}},
            {"extensions":{"KHR_materials_sheen" : {}}},
            {"extensions":{"KHR_materials_sheen" : null}},
            {"extensions":{"KHR_materials_sheen" : 7}},
            {"extensions":{"KHR_materials_sheen" : false}},
            {"extensions":{"KHR_materials_sheen" : "raw"}},
            {"extensions":{"KHR_materials_sheen" : [{"extras":1,"sheenColorTexture":{"extras":2}}]}},
            {"extensions":{
                "KHR_materials_clearcoat":{
                    "clearcoatTexture":{"extensions":{"KHR_texture_transform":null}},
                    "clearcoatRoughnessTexture":{"extensions":{"KHR_texture_transform":false}},
                    "clearcoatNormalTexture":{"extensions":{"KHR_texture_transform":7}}
                },
                "KHR_materials_sheen":{
                    "sheenColorTexture":{"extensions":{"KHR_texture_transform":"raw"}},
                    "sheenRoughnessTexture":{"extensions":{"KHR_texture_transform":[{"extras":1}]}}
                }
            }}
        ]}"#;
        fn inspect(scope: &mut ParseExecution<'_, '_>, map: &GltfSourceMap<'_>) {
            assert_eq!(map.entities().len(), 14);
            assert_eq!(map.payloads().len(), 37);
            for (start, kind, name, array) in [
                (
                    0,
                    SourceExtensionKind::KhrMaterialsClearcoat,
                    "KHR_materials_clearcoat",
                    r#"[{"extras":1,"clearcoatTexture":{"extras":2}}]"#,
                ),
                (
                    6,
                    SourceExtensionKind::KhrMaterialsSheen,
                    "KHR_materials_sheen",
                    r#"[{"extras":1,"sheenColorTexture":{"extras":2}}]"#,
                ),
            ] {
                for (index, value) in ["{}", "null", "7", "false", r#""raw""#, array]
                    .into_iter()
                    .enumerate()
                {
                    let owner = id(SourceEntityKind::Material, None, start + index);
                    let payloads = map.payloads_for(owner);
                    assert_eq!(payloads.len(), 2);
                    let payload = payloads
                        .iter()
                        .find(|payload| payload.kind == SourcePayloadKind::Extension(kind))
                        .unwrap();
                    assert_eq!(payload.site, SourcePayloadSite::Entity);
                    assert_raw(scope, payload.key_range, &format!(r#""{name}""#));
                    assert_raw(
                        scope,
                        payload.member_range,
                        &format!(r#""{name}" : {value}"#),
                    );
                    assert_raw(scope, payload.value_range, value);
                }
            }
            let payloads = map.payloads_for(id(SourceEntityKind::Material, None, 12));
            for (site, value) in [
                (SourcePayloadSite::ClearcoatTexture, "null"),
                (SourcePayloadSite::ClearcoatRoughnessTexture, "false"),
                (SourcePayloadSite::ClearcoatNormalTexture, "7"),
                (SourcePayloadSite::SheenColorTexture, r#""raw""#),
                (
                    SourcePayloadSite::SheenRoughnessTexture,
                    r#"[{"extras":1}]"#,
                ),
            ] {
                let payload = payloads
                    .iter()
                    .find(|payload| {
                        payload.site == site
                            && payload.kind
                                == SourcePayloadKind::Extension(
                                    SourceExtensionKind::KhrTextureTransform,
                                )
                    })
                    .unwrap();
                assert_raw(scope, payload.key_range, r#""KHR_texture_transform""#);
                assert_raw(
                    scope,
                    payload.member_range,
                    &format!(r#""KHR_texture_transform":{value}"#),
                );
                assert_raw(scope, payload.value_range, value);
            }
            assert!(map.payloads().iter().all(|payload| {
                payload.kind != SourcePayloadKind::Extras
                    && payload.kind != SourcePayloadKind::ExtensionProperty
            }));
        }
        inspect_admission(json, inspect)?;
        inspect_admission(&glb(json), inspect)
    }

    #[test]
    fn clearcoat_sheen_unknown_direct_properties_keep_opaque_value_ranges() -> TestResult {
        let json = br#"{"materials":[{"extensions":{
            "KHR_materials_clearcoat":{
                "clearcoatFactor":0.5,"clearcoatRoughnessFactor":0.25,
                "sheenColorFactor" : [0,1,0],
                "clearcoatRoughnessTextureExtended" : {"extras":0,"materials":[{}]},
                "clearcoatTexture":{"index":0,"scale" : -1,"extensions":{"KHR_texture_transform":{"index" : 7}}},
                "clearcoatRoughnessTexture":{"index":1,"unexpected" : [{"extras":1}],"extensions":{"KHR_texture_transform":{"unexpected" : null}}},
                "clearcoatNormalTexture":{"index":2,"scale":-2,"strength" : false,"extensions":{"KHR_texture_transform":{"unexpected" : {"nodes":[{}],"extras":2}}}}
            },
            "KHR_materials_sheen":{
                "sheenColorFactor":[0,0.5,1],"sheenRoughnessFactor":0,
                "clearcoatFactor" : null,
                "sheenColorTexture":{"index":3,"unexpected" : "raw","extensions":{"KHR_texture_transform":{"unexpected" : [true,false]}}},
                "sheenRoughnessTexture":{"index":4,"unexpected" : 8,"extensions":{"KHR_texture_transform":{"unexpected" : true}}}
            }
        }}]}"#;
        fn inspect(scope: &mut ParseExecution<'_, '_>, map: &GltfSourceMap<'_>) {
            use SourcePayloadSite as Site;
            assert_eq!(map.entities().len(), 2);
            let payloads = map.payloads_for(id(SourceEntityKind::Material, None, 0));
            let properties = payloads
                .iter()
                .filter(|payload| payload.kind == SourcePayloadKind::ExtensionProperty);
            let expected = [
                (Site::MaterialClearcoat, "sheenColorFactor", "[0,1,0]"),
                (
                    Site::MaterialClearcoat,
                    "clearcoatRoughnessTextureExtended",
                    r#"{"extras":0,"materials":[{}]}"#,
                ),
                (Site::MaterialSheen, "clearcoatFactor", "null"),
                (Site::ClearcoatTexture, "scale", "-1"),
                (
                    Site::ClearcoatRoughnessTexture,
                    "unexpected",
                    r#"[{"extras":1}]"#,
                ),
                (Site::ClearcoatNormalTexture, "strength", "false"),
                (Site::SheenColorTexture, "unexpected", r#""raw""#),
                (Site::SheenRoughnessTexture, "unexpected", "8"),
                (Site::ClearcoatTextureTransform, "index", "7"),
                (
                    Site::ClearcoatRoughnessTextureTransform,
                    "unexpected",
                    "null",
                ),
                (
                    Site::ClearcoatNormalTextureTransform,
                    "unexpected",
                    r#"{"nodes":[{}],"extras":2}"#,
                ),
                (
                    Site::SheenColorTextureTransform,
                    "unexpected",
                    "[true,false]",
                ),
                (Site::SheenRoughnessTextureTransform, "unexpected", "true"),
            ];
            assert_eq!(properties.clone().count(), expected.len());
            for (payload, (site, key, value)) in properties.zip(expected) {
                assert_eq!(payload.site, site);
                assert_raw(scope, payload.key_range, &format!(r#""{key}""#));
                assert_raw(
                    scope,
                    payload.member_range,
                    &format!(r#""{key}" : {value}"#),
                );
                assert_raw(scope, payload.value_range, value);
            }
            assert!(
                payloads
                    .iter()
                    .all(|payload| payload.kind != SourcePayloadKind::Extras)
            );
        }
        inspect_admission(json, inspect)?;
        inspect_admission(&glb(json), inspect)
    }

    #[test]
    fn clearcoat_sheen_wrong_sites_keep_raw_payloads_without_child_sites() -> TestResult {
        let json = br#"{
            "extensions":{"KHR_materials_clearc\u006fat":{"extras":0}},
            "asset":{"extensions":{"KHR_materials_sheen":{"extras":1}}},
            "nodes":[{"extensions":{"KHR_materials_clearcoat":{"extras":2}}}],
            "textures":[{"extensions":{"KHR_materials_sheen":{"extras":3}}}],
            "materials":[{
                "pbrMetallicRoughness":{"extensions":{"KHR_materials_clearcoat":{"extras":4}}},
                "normalTexture":{"extensions":{"KHR_materials_sheen":{"extras":5}}},
                "extensions":{
                    "KHR_materials_clearcoat":{
                        "clearcoatTexture":{"extensions":{
                            "KHR_materials_sheen":{"extras":6},
                            "KHR_texture_transform":{"extensions":{"KHR_materials_clearcoat":{"extras":7}},"extras":8}
                        }},
                        "extensions":{
                            "KHR_materials_clearcoat":{"extras":9},
                            "KHR_texture_transform":{"extras":10}
                        }
                    },
                    "KHR_materials_sheen":{"extensions":{"KHR_materials_sheen":{"extras":11}}}
                }
            }]
        }"#;
        fn inspect(scope: &mut ParseExecution<'_, '_>, map: &GltfSourceMap<'_>) {
            use SourceExtensionKind as Extension;
            use SourcePayloadSite as Site;
            let document = id(SourceEntityKind::Document, None, 0);
            let material = id(SourceEntityKind::Material, None, 0);
            assert_eq!(map.entities().len(), 4);
            for (owner, site, kind, name, marker) in [
                (
                    document,
                    Site::Entity,
                    Extension::KhrMaterialsClearcoat,
                    r#"KHR_materials_clearc\u006fat"#,
                    0,
                ),
                (
                    document,
                    Site::Asset,
                    Extension::KhrMaterialsSheen,
                    "KHR_materials_sheen",
                    1,
                ),
                (
                    id(SourceEntityKind::Node, None, 0),
                    Site::Entity,
                    Extension::KhrMaterialsClearcoat,
                    "KHR_materials_clearcoat",
                    2,
                ),
                (
                    id(SourceEntityKind::Texture, None, 0),
                    Site::Entity,
                    Extension::KhrMaterialsSheen,
                    "KHR_materials_sheen",
                    3,
                ),
                (
                    material,
                    Site::MaterialPbrMetallicRoughness,
                    Extension::KhrMaterialsClearcoat,
                    "KHR_materials_clearcoat",
                    4,
                ),
                (
                    material,
                    Site::NormalTexture,
                    Extension::KhrMaterialsSheen,
                    "KHR_materials_sheen",
                    5,
                ),
                (
                    material,
                    Site::ClearcoatTexture,
                    Extension::KhrMaterialsSheen,
                    "KHR_materials_sheen",
                    6,
                ),
                (
                    material,
                    Site::ClearcoatTextureTransform,
                    Extension::KhrMaterialsClearcoat,
                    "KHR_materials_clearcoat",
                    7,
                ),
                (
                    material,
                    Site::MaterialClearcoat,
                    Extension::KhrMaterialsClearcoat,
                    "KHR_materials_clearcoat",
                    9,
                ),
                (
                    material,
                    Site::MaterialClearcoat,
                    Extension::KhrTextureTransform,
                    "KHR_texture_transform",
                    10,
                ),
                (
                    material,
                    Site::MaterialSheen,
                    Extension::KhrMaterialsSheen,
                    "KHR_materials_sheen",
                    11,
                ),
            ] {
                let payload = map
                    .payloads_for(owner)
                    .iter()
                    .find(|payload| {
                        payload.site == site && payload.kind == SourcePayloadKind::Extension(kind)
                    })
                    .unwrap();
                let value = format!(r#"{{"extras":{marker}}}"#);
                assert_raw(scope, payload.key_range, &format!(r#""{name}""#));
                assert_raw(scope, payload.member_range, &format!(r#""{name}":{value}"#));
                assert_raw(scope, payload.value_range, &value);
            }
            assert_eq!(
                map.payloads()
                    .iter()
                    .filter(|payload| matches!(payload.kind, SourcePayloadKind::Extension(_)))
                    .count(),
                14,
            );
            let extras = map
                .payloads()
                .iter()
                .filter(|payload| payload.kind == SourcePayloadKind::Extras)
                .collect::<Vec<_>>();
            assert_eq!(extras.len(), 1);
            assert_eq!(extras[0].owner, material);
            assert_eq!(extras[0].site, Site::ClearcoatTextureTransform);
            assert_raw(scope, extras[0].value_range, "8");
        }
        inspect_admission(json, inspect)?;
        inspect_admission(&glb(json), inspect)
    }

    #[test]
    fn clearcoat_sheen_recognition_respects_decoded_key_and_token_bounds() {
        for (raw, member) in [
            (
                r#"KHR_materials_clearc\u006fat"#,
                SourceMember::KhrMaterialsClearcoat,
            ),
            (
                r#"KHR_materials_sh\u0065en"#,
                SourceMember::KhrMaterialsSheen,
            ),
            (r#"clearcoatFac\u0074or"#, SourceMember::ClearcoatFactor),
            (
                r#"clearcoatRoughnessFac\u0074or"#,
                SourceMember::ClearcoatRoughnessFactor,
            ),
            (r#"clearcoatText\u0075re"#, SourceMember::ClearcoatTexture),
            (
                r#"clearcoatRoughnessText\u0075re"#,
                SourceMember::ClearcoatRoughnessTexture,
            ),
            (
                r#"clearcoatNormalText\u0075re"#,
                SourceMember::ClearcoatNormalTexture,
            ),
            (r#"sheenColorFac\u0074or"#, SourceMember::SheenColorFactor),
            (
                r#"sheenRoughnessFac\u0074or"#,
                SourceMember::SheenRoughnessFactor,
            ),
            (r#"sheenColorText\u0075re"#, SourceMember::SheenColorTexture),
            (
                r#"sheenRoughnessText\u0075re"#,
                SourceMember::SheenRoughnessTexture,
            ),
        ] {
            let adjacent = format!("!{raw}suffix");
            assert_eq!(source_member(adjacent.as_bytes(), 1, 1 + raw.len()), member);
            assert_eq!(
                source_member(adjacent.as_bytes(), 1, adjacent.len()),
                SourceMember::Other,
            );
        }
        for raw in [
            "KHR_materials_clearcoatExtra",
            "KHR_materials_sheenExtra",
            "clearcoatRoughnessTextureExtended",
            r#"clearcoatTexture\u0058"#,
            r#"sheenColorTexture\u00e9"#,
            "clearcoat",
            "sheen",
        ] {
            assert_eq!(
                source_member(raw.as_bytes(), 0, raw.len()),
                SourceMember::Other,
            );
        }
    }

    #[test]
    fn empty_objects_and_arrays_do_not_fabricate_source_entities() -> TestResult {
        inspect_admission(b"{}", |scope, map| {
            assert_eq!(map.entities().len(), 1);
            assert_raw(scope, map.entities()[0].range, "{}");
            assert!(map.payloads().is_empty());
        })?;
        inspect_admission(
            br#"{"scenes":[],"nodes":[],"meshes":[],"animations":[]}"#,
            |_, map| {
                assert_eq!(map.entities().len(), 1);
                assert!(map.get(id(SourceEntityKind::Scene, None, 0)).is_none());
                assert!(map.payloads().is_empty());
            },
        )?;
        inspect_admission(b"[]", |_, map| {
            assert!(map.entities().is_empty());
            assert!(map.get(id(SourceEntityKind::Document, None, 0)).is_none());
        })
    }

    #[test]
    fn punctual_lights_inventory_and_unknown_sites_use_only_their_schema_owners() -> TestResult {
        let json = br#"{
            "extensions":{
                "KHR_lights_punct\u0075al":{"li\u0067hts":[{
                    "type":"spot","unknown":[{"lights":[{}]}],
                    "sp\u006ft":{"innerConeAngle":0,"outerConeAngle":1,"unknown":false,"extras":null}
                }],"unknown":true,"extras":[]},
                "VENDOR_data":{"lights":[{}],"extensions":{"KHR_lights_punctual":{"lights":[{}]}}}
            },
            "nodes":[{"extensions":{"KHR_lights_punctual":{"light":0,"unknown":17,"extras":{}}}}],
            "materials":[{"extensions":{"KHR_lights_punctual":{"lights":[{}]}}}],
            "extras":{"extensions":{"KHR_lights_punctual":{"lights":[{}]}}}
        }"#;
        fn inspect(scope: &mut ParseExecution<'_, '_>, map: &GltfSourceMap<'_>) {
            let lights = map
                .entities()
                .iter()
                .filter(|entity| entity.id.kind == SourceEntityKind::Light)
                .collect::<Vec<_>>();
            assert_eq!(lights.len(), 1);
            assert_eq!(lights[0].id, id(SourceEntityKind::Light, None, 0));
            for (owner, site, raw) in [
                (
                    id(SourceEntityKind::Document, None, 0),
                    SourcePayloadSite::DocumentPunctualLights,
                    "true",
                ),
                (
                    id(SourceEntityKind::Node, None, 0),
                    SourcePayloadSite::NodePunctualLight,
                    "17",
                ),
                (
                    id(SourceEntityKind::Light, None, 0),
                    SourcePayloadSite::Entity,
                    r#"[{"lights":[{}]}]"#,
                ),
                (
                    id(SourceEntityKind::Light, None, 0),
                    SourcePayloadSite::LightSpot,
                    "false",
                ),
            ] {
                let payload = map
                    .payloads_for(owner)
                    .iter()
                    .find(|payload| {
                        payload.site == site && payload.kind == SourcePayloadKind::ExtensionProperty
                    })
                    .unwrap();
                assert_raw(scope, payload.key_range, r#""unknown""#);
                assert_raw(scope, payload.value_range, raw);
            }
            for (owner, site, raw) in [
                (
                    id(SourceEntityKind::Document, None, 0),
                    SourcePayloadSite::DocumentPunctualLights,
                    "[]",
                ),
                (
                    id(SourceEntityKind::Node, None, 0),
                    SourcePayloadSite::NodePunctualLight,
                    "{}",
                ),
                (
                    id(SourceEntityKind::Light, None, 0),
                    SourcePayloadSite::LightSpot,
                    "null",
                ),
            ] {
                let payload = map
                    .payloads_for(owner)
                    .iter()
                    .find(|payload| {
                        payload.site == site && payload.kind == SourcePayloadKind::Extras
                    })
                    .unwrap();
                assert_raw(scope, payload.value_range, raw);
            }
        }
        inspect_admission(json, inspect)?;
        inspect_admission(&glb(json), inspect)
    }

    #[test]
    fn raw_range_construction_rejects_reversed_or_overflowing_offsets() {
        assert_eq!(
            SourceRange::from_json_offsets(20, 3, 9)
                .unwrap()
                .raw_range(),
            23..29
        );
        assert!(SourceRange::from_json_offsets(0, 9, 3).is_none());
        assert!(SourceRange::from_json_offsets(u64::MAX, 0, 1).is_none());
    }
}
