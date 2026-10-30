use crate::GltfFailure;
use crate::source_map::{
    GltfSourceMap, SourceEntityId, SourceEntityKind, SourceExtensionKind, SourcePayload,
    SourcePayloadKind, SourcePayloadSite, SourceRange,
};
use baozi_import::{
    ImportStorageClass, ImportWorkClass, ImportWorkScope, MeteredVec, ParseFailure, ParseResult,
    ParseScope,
};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;

const UTF8_BOM: &[u8; 3] = &[0xef, 0xbb, 0xbf];
const SCAN_CHUNK_BYTES: usize = 4 * 1024;
const DUPLICATE_POLL_COMPARISONS: u64 = 4 * 1024;
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
    ctx: &'scope ParseScope<'_>,
    bytes: &[u8],
    raw_base: u64,
    is_glb: bool,
) -> ParseResult<GltfSourceMap<'scope>> {
    ctx.enforce_resource_limit(
        "gltf-decoded-utf8-bytes",
        to_u64(bytes.len()),
        ctx.limits().max_decoded_utf8_bytes,
    )?;

    let mut scanner = JsonScanner::new(ctx, bytes, raw_base, is_glb);
    let scan_result = ctx.with_work(ImportWorkClass::DecodeWork, to_u64(bytes.len()), |work| {
        scanner.scan(work)
    });
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
        || validate_duplicate_keys(ctx, bytes, raw_base, &mut keys),
    )?;
    debug_assert_eq!(stats.containers, stats.objects + stats.arrays);
    drop(keys);
    source_map.finish(ctx)?;
    Ok(source_map)
}

fn publish_ledger_limit<T>(
    ctx: &ParseScope<'_>,
    class: ImportWorkClass,
    attempted: u64,
) -> ParseResult<T> {
    let result: ParseResult<()> = ctx.with_work(class, attempted, |work| work.complete_all());
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
    TextureTransform {
        payload_index: usize,
        owner: SourceEntityId,
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

struct JsonScanner<'scope, 'session, 'bytes> {
    ctx: &'scope ParseScope<'session>,
    bytes: &'bytes [u8],
    raw_base: u64,
    is_glb: bool,
    cursor: usize,
    accounted_cursor: usize,
    root_started: bool,
    root_end: Option<usize>,
    frames: MeteredVec<'scope, JsonFrame>,
    keys: MeteredVec<'scope, JsonKey>,
    source_map: GltfSourceMap<'scope>,
    stats: JsonStats,
}

impl<'scope, 'session, 'bytes> JsonScanner<'scope, 'session, 'bytes> {
    fn new(
        ctx: &'scope ParseScope<'session>,
        bytes: &'bytes [u8],
        raw_base: u64,
        is_glb: bool,
    ) -> Self {
        Self {
            ctx,
            bytes,
            raw_base,
            is_glb,
            cursor: 0,
            accounted_cursor: 0,
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
            stats: JsonStats::new(),
        }
    }

    fn scan(&mut self, work: &mut ImportWorkScope<'_, 'session, 1>) -> ScanResult<()> {
        if self.bytes.starts_with(UTF8_BOM) {
            self.consume(work, UTF8_BOM.len())?;
            self.flush(work)?;
            return Err(self
                .ctx
                .format_failure(GltfFailure::forbidden_bom(self.raw_base))
                .into());
        }

        while self.root_end.is_none() {
            self.skip_whitespace(work)?;
            if let Some(frame) = self.frames.as_slice().last().copied() {
                self.scan_frame(work, frame)?;
            } else if !self.root_started {
                if self.cursor == self.bytes.len() {
                    return self.syntax_error(
                        work,
                        "json-document",
                        self.cursor,
                        0,
                        "glTF JSON is missing a root value",
                    );
                }
                self.root_started = true;
                self.scan_value(work, None)?;
                if self.frames.is_empty() {
                    self.root_end = Some(self.cursor);
                }
            } else {
                return Err(self
                    .ctx
                    .format_failure(GltfFailure::invalid_json(
                        "json-syntax-map",
                        self.absolute_offset(self.cursor)?,
                        0,
                        "glTF JSON root completion state is inconsistent",
                    ))
                    .into());
            }
        }

        self.scan_trailing(work)?;
        self.flush(work)
    }

    fn scan_frame(
        &mut self,
        work: &mut ImportWorkScope<'_, 'session, 1>,
        frame: JsonFrame,
    ) -> ScanResult<()> {
        match frame.state {
            FrameState::ArrayValueOrEnd if self.peek() == Some(b']') => {
                self.close_container(work, b']')
            }
            FrameState::ArrayValueOrEnd | FrameState::ArrayValue => {
                if self.cursor == self.bytes.len() {
                    return self.syntax_error(
                        work,
                        "json-array",
                        self.cursor,
                        0,
                        "glTF JSON array is missing a value",
                    );
                }
                self.set_frame_state(FrameState::ArrayCommaOrEnd)?;
                let next_ordinal = frame
                    .next_ordinal
                    .checked_add(1)
                    .ok_or_else(|| self.ctx.allocation_failure("gltf-json-array-ordinal"))?;
                self.active_frame_mut()?.next_ordinal = next_ordinal;
                self.scan_value(work, Some(frame))
            }
            FrameState::ArrayCommaOrEnd => match self.peek() {
                Some(b',') => {
                    self.count_token(work)?;
                    self.consume(work, 1)?;
                    self.set_frame_state(FrameState::ArrayValue)
                }
                Some(b']') => self.close_container(work, b']'),
                Some(_) => self.syntax_error_current(
                    work,
                    "json-array",
                    self.cursor,
                    "glTF JSON array expects a comma or closing bracket",
                ),
                None => self.syntax_error(
                    work,
                    "json-array",
                    self.cursor,
                    0,
                    "glTF JSON array is not closed",
                ),
            },
            FrameState::ObjectKeyOrEnd if self.peek() == Some(b'}') => {
                self.close_container(work, b'}')
            }
            FrameState::ObjectKeyOrEnd | FrameState::ObjectKey => {
                if self.peek() != Some(b'"') {
                    let length = self.current_scalar_length(work)?;
                    return self.syntax_error(
                        work,
                        "json-object",
                        self.cursor,
                        length,
                        "glTF JSON object expects a quoted object key",
                    );
                }
                let key = self.scan_string(work, true)?;
                let member = source_member(self.bytes, key.content_start, key.content_end);
                work.poll()?;
                self.keys.try_push(JsonKey {
                    object_id: frame.object_id,
                    raw_start: key.raw_start,
                    raw_end: key.raw_end,
                    content_start: key.content_start,
                    content_end: key.content_end,
                    digest: key.digest,
                })?;
                let frame = self.active_frame_mut()?;
                frame.member = member;
                frame.member_key_start = key.raw_start;
                frame.member_key_end = key.raw_end;
                frame.state = FrameState::ObjectColon;
                Ok(())
            }
            FrameState::ObjectColon => match self.peek() {
                Some(b':') => {
                    self.count_token(work)?;
                    self.consume(work, 1)?;
                    self.set_frame_state(FrameState::ObjectValue)
                }
                Some(_) => self.syntax_error_current(
                    work,
                    "json-object",
                    self.cursor,
                    "glTF JSON object key is missing a colon",
                ),
                None => self.syntax_error(
                    work,
                    "json-object",
                    self.cursor,
                    0,
                    "glTF JSON object key is missing a colon",
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
                    );
                }
                self.set_frame_state(FrameState::ObjectCommaOrEnd)?;
                self.scan_value(work, Some(frame))
            }
            FrameState::ObjectCommaOrEnd => match self.peek() {
                Some(b',') => {
                    self.count_token(work)?;
                    self.consume(work, 1)?;
                    self.set_frame_state(FrameState::ObjectKey)
                }
                Some(b'}') => self.close_container(work, b'}'),
                Some(_) => self.syntax_error_current(
                    work,
                    "json-object",
                    self.cursor,
                    "glTF JSON object expects a comma or closing brace",
                ),
                None => self.syntax_error(
                    work,
                    "json-object",
                    self.cursor,
                    0,
                    "glTF JSON object is not closed",
                ),
            },
        }
    }

    fn scan_value(
        &mut self,
        work: &mut ImportWorkScope<'_, 'session, 1>,
        parent: Option<JsonFrame>,
    ) -> ScanResult<()> {
        let raw_start = self.cursor;
        let source = self.value_source(parent)?;
        match self.peek() {
            Some(b'{') => return self.open_container(work, FrameState::ObjectKeyOrEnd, source),
            Some(b'[') => return self.open_container(work, FrameState::ArrayValueOrEnd, source),
            Some(b'"') => self.scan_string(work, false).map(|_| ()),
            Some(b't') => self.scan_literal(work, b"true"),
            Some(b'f') => self.scan_literal(work, b"false"),
            Some(b'n') => self.scan_literal(work, b"null"),
            Some(b'-' | b'0'..=b'9') => self.scan_number(work),
            Some(byte) if byte >= 0x80 => {
                let length = self.valid_utf8_scalar_width(work)?;
                self.syntax_error(
                    work,
                    "json-document",
                    self.cursor,
                    length,
                    "glTF JSON value begins with an unexpected Unicode scalar",
                )
            }
            Some(_) => self.syntax_error(
                work,
                "json-document",
                self.cursor,
                1,
                "glTF JSON value begins with an unexpected byte",
            ),
            None => self.syntax_error(
                work,
                "json-document",
                self.cursor,
                0,
                "glTF JSON is missing a value",
            ),
        }?;
        self.finish_source_range(source, raw_start)
    }

    fn value_source(&mut self, parent: Option<JsonFrame>) -> ScanResult<SourceContext> {
        let first = self.peek();
        match parent {
            None if first == Some(b'{') => self.begin_source_entity(SourceEntityId {
                kind: SourceEntityKind::Document,
                parent_index: None,
                source_index: 0,
            }),
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
                    return self.begin_source_payload(id, SourcePayloadSite::Entity, frame);
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
                if matches!(
                    frame.member,
                    SourceMember::Extensions | SourceMember::Extras
                ) {
                    return self.begin_source_payload(owner, site, frame);
                }
                Ok(source_property(owner, site, frame.member, first))
            }
            Some(
                frame @ JsonFrame {
                    source: SourceContext::Extensions { owner, site, .. },
                    ..
                },
            ) => self.begin_source_payload(owner, site, frame),
            Some(
                frame @ JsonFrame {
                    source: SourceContext::TextureTransform { owner, .. },
                    ..
                },
            ) if matches!(
                frame.member,
                SourceMember::Extras | SourceMember::Extensions
            ) =>
            {
                self.begin_source_payload(owner, SourcePayloadSite::TextureTransform, frame)
            }
            Some(JsonFrame {
                source: SourceContext::EntityArray { kind, parent_index },
                next_ordinal,
                ..
            }) if first == Some(b'{') => self.begin_source_entity(SourceEntityId {
                kind,
                parent_index,
                source_index: next_ordinal,
            }),
            _ => Ok(SourceContext::Other),
        }
    }

    fn begin_source_entity(&mut self, id: SourceEntityId) -> ScanResult<SourceContext> {
        let start = self.source_range(self.cursor)?;
        let map_index = self.source_map.begin_entity(id, start)?;
        Ok(SourceContext::Entity { id, map_index })
    }

    fn source_range(&self, raw_start: usize) -> ParseResult<SourceRange> {
        SourceRange::from_json_offsets(self.raw_base, raw_start, self.cursor)
            .ok_or_else(|| self.ctx.allocation_failure("gltf-source-byte-range"))
    }

    fn begin_source_payload(
        &mut self,
        owner: SourceEntityId,
        site: SourcePayloadSite,
        frame: JsonFrame,
    ) -> ScanResult<SourceContext> {
        let kind = if matches!(frame.source, SourceContext::Extensions { .. }) {
            let extension = if frame.member == SourceMember::KhrTextureTransform {
                SourceExtensionKind::KhrTextureTransform
            } else {
                SourceExtensionKind::Other
            };
            SourcePayloadKind::Extension(extension)
        } else if frame.member == SourceMember::Extensions {
            SourcePayloadKind::Extensions
        } else {
            SourcePayloadKind::Extras
        };
        let key_range = SourceRange::from_json_offsets(
            self.raw_base,
            frame.member_key_start,
            frame.member_key_end,
        )
        .ok_or_else(|| self.ctx.allocation_failure("gltf-source-key-range"))?;
        let payload_index = self.source_map.begin_payload(SourcePayload {
            owner,
            site,
            kind,
            key_range,
            member_range: self.source_range(frame.member_key_start)?,
            value_range: self.source_range(self.cursor)?,
        })?;
        if kind == SourcePayloadKind::Extensions && self.peek() == Some(b'{') {
            Ok(SourceContext::Extensions {
                payload_index,
                owner,
                site,
            })
        } else if kind == SourcePayloadKind::Extension(SourceExtensionKind::KhrTextureTransform)
            && matches!(
                site,
                SourcePayloadSite::BaseColorTexture
                    | SourcePayloadSite::MetallicRoughnessTexture
                    | SourcePayloadSite::NormalTexture
                    | SourcePayloadSite::OcclusionTexture
                    | SourcePayloadSite::EmissiveTexture
            )
            && self.peek() == Some(b'{')
        {
            Ok(SourceContext::TextureTransform {
                payload_index,
                owner,
            })
        } else {
            Ok(SourceContext::Payload { payload_index })
        }
    }

    fn finish_source_range(&mut self, source: SourceContext, raw_start: usize) -> ScanResult<()> {
        match source {
            SourceContext::Entity { map_index, .. } => {
                let range = self.source_range(raw_start)?;
                let entity = self.source_map.entity_mut(map_index).ok_or_else(|| {
                    self.ctx.format_failure(GltfFailure::internal(
                        "JSON source range refers to a missing glTF entity",
                    ))
                })?;
                entity.range = range;
            }
            SourceContext::Payload { payload_index }
            | SourceContext::Extensions { payload_index, .. }
            | SourceContext::TextureTransform { payload_index, .. } => {
                let range = self.source_range(raw_start)?;
                let raw_end = self.absolute_offset(self.cursor)?;
                let payload = self.source_map.payload_mut(payload_index).ok_or_else(|| {
                    self.ctx.format_failure(GltfFailure::internal(
                        "JSON source range refers to a missing glTF payload",
                    ))
                })?;
                payload.value_range = range;
                payload.member_range.byte_length = raw_end
                    .checked_sub(payload.member_range.byte_offset)
                    .ok_or_else(|| self.ctx.allocation_failure("gltf-source-member-range"))?;
            }
            SourceContext::Other
            | SourceContext::EntityArray { .. }
            | SourceContext::Property { .. } => {}
        }
        Ok(())
    }

    fn open_container(
        &mut self,
        work: &mut ImportWorkScope<'_, 'session, 1>,
        state: FrameState,
        source: SourceContext,
    ) -> ScanResult<()> {
        let depth = self
            .frames
            .len()
            .checked_add(1)
            .ok_or_else(|| self.ctx.allocation_failure("gltf-json-nesting-depth"))?;
        self.enforce_usize_limit(
            work,
            "gltf-json-nesting-depth",
            depth,
            MAX_JSON_NESTING_DEPTH,
        )?;
        self.count_token(work)?;

        let object_id = if state == FrameState::ObjectKeyOrEnd {
            let objects = self.increment_counter(self.stats.objects, "gltf-json-objects")?;
            self.stats.objects = objects;
            objects
        } else {
            self.stats.arrays = self.increment_counter(self.stats.arrays, "gltf-json-arrays")?;
            0
        };
        let containers = self.increment_counter(self.stats.containers, "gltf-json-containers")?;
        let record_limit = to_u64(self.ctx.limits().max_records);
        if containers > record_limit {
            self.flush(work)?;
            return Err(ScanAbort::LedgerLimit {
                class: ImportWorkClass::Records,
                attempted: containers,
            });
        }
        self.stats.containers = containers;

        work.poll()?;
        self.frames.try_push(JsonFrame {
            state,
            object_id,
            raw_start: self.cursor,
            next_ordinal: 0,
            member: SourceMember::Other,
            member_key_start: 0,
            member_key_end: 0,
            source,
        })?;
        self.consume(work, 1)
    }

    fn close_container(
        &mut self,
        work: &mut ImportWorkScope<'_, 'session, 1>,
        expected: u8,
    ) -> ScanResult<()> {
        if self.peek() != Some(expected) {
            return Err(self
                .ctx
                .format_failure(GltfFailure::invalid_json(
                    "json-syntax-map",
                    self.absolute_offset(self.cursor)?,
                    0,
                    "glTF JSON container close state is inconsistent",
                ))
                .into());
        }
        self.count_token(work)?;
        self.consume(work, 1)?;
        let Some(closed) = self.frames.pop() else {
            return Err(self
                .ctx
                .format_failure(GltfFailure::invalid_json(
                    "json-syntax-map",
                    self.absolute_offset(self.cursor)?,
                    0,
                    "glTF JSON container stack underflowed",
                ))
                .into());
        };
        self.finish_source_range(closed.source, closed.raw_start)?;
        if self.frames.is_empty() {
            self.root_end = Some(self.cursor);
        }
        Ok(())
    }

    fn scan_string(
        &mut self,
        work: &mut ImportWorkScope<'_, 'session, 1>,
        hash_key: bool,
    ) -> ScanResult<ScannedString> {
        self.count_token(work)?;
        self.stats.strings = self.increment_counter(self.stats.strings, "gltf-json-strings")?;
        let raw_start = self.cursor;
        self.consume(work, 1)?;
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
                );
            };
            match byte {
                b'"' => {
                    let content_end = self.cursor;
                    self.consume(work, 1)?;
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
                    );
                }
                b'\\' => {
                    let escape_start = self.cursor;
                    self.consume_string_bytes(work, content_start, 1)?;
                    let Some(escape) = self.peek() else {
                        return self.syntax_error(
                            work,
                            "json-string",
                            escape_start,
                            self.cursor - escape_start,
                            "glTF JSON string ends inside an escape",
                        );
                    };
                    match escape {
                        b'"' | b'\\' | b'/' => {
                            self.consume_string_bytes(work, content_start, 1)?;
                            hash_scalar(&mut hasher, u32::from(escape));
                        }
                        b'b' | b'f' | b'n' | b'r' | b't' => {
                            self.consume_string_bytes(work, content_start, 1)?;
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
                            self.consume_string_bytes(work, content_start, 1)?;
                            let first = self.scan_hex_quad(work, content_start, escape_start)?;
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
                                    );
                                }
                                let low_start = self.cursor;
                                self.consume_string_bytes(work, content_start, 2)?;
                                let low = self.scan_hex_quad(work, content_start, low_start)?;
                                if !(0xdc00..=0xdfff).contains(&low) {
                                    return self.syntax_error(
                                        work,
                                        "json-string",
                                        low_start,
                                        self.cursor - low_start,
                                        "glTF JSON high surrogate has an invalid low surrogate",
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
                            );
                        }
                    }
                }
                0x20..=0x7f => {
                    self.consume_string_bytes(work, content_start, 1)?;
                    if let Some(hasher) = hasher.as_mut() {
                        hasher.update([byte]);
                    }
                }
                _ => {
                    let width = self.valid_utf8_scalar_width(work)?;
                    self.enforce_string_bytes(work, content_start, width)?;
                    if let Some(hasher) = hasher.as_mut() {
                        hasher.update(&self.bytes[self.cursor..self.cursor + width]);
                    }
                    self.consume(work, width)?;
                }
            }
        }
    }

    fn scan_hex_quad(
        &mut self,
        work: &mut ImportWorkScope<'_, 'session, 1>,
        content_start: usize,
        escape_start: usize,
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
                );
            };
            let Some(digit) = hex_value(byte) else {
                return self.syntax_error_current(
                    work,
                    "json-string",
                    self.cursor,
                    "glTF JSON Unicode escape contains a non-hex digit",
                );
            };
            self.consume_string_bytes(work, content_start, 1)?;
            value = (value << 4) | u16::from(digit);
        }
        Ok(value)
    }

    fn scan_number(&mut self, work: &mut ImportWorkScope<'_, 'session, 1>) -> ScanResult<()> {
        self.count_token(work)?;
        self.stats.numbers = self.increment_counter(self.stats.numbers, "gltf-json-numbers")?;
        let start = self.cursor;

        if self.peek() == Some(b'-') {
            self.consume_number_byte(work, start)?;
        }
        match self.peek() {
            Some(b'0') => {
                self.consume_number_byte(work, start)?;
                if self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                    return self.syntax_error(
                        work,
                        "json-number",
                        self.cursor,
                        1,
                        "glTF JSON number has a leading zero",
                    );
                }
            }
            Some(b'1'..=b'9') => {
                while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                    self.consume_number_byte(work, start)?;
                }
            }
            Some(_) => {
                return self.syntax_error_current(
                    work,
                    "json-number",
                    self.cursor,
                    "glTF JSON number is missing an integer digit",
                );
            }
            None => {
                return self.syntax_error(
                    work,
                    "json-number",
                    self.cursor,
                    0,
                    "glTF JSON number is missing an integer digit",
                );
            }
        }

        if self.peek() == Some(b'.') {
            self.consume_number_byte(work, start)?;
            if !self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                return self.syntax_error(
                    work,
                    "json-number",
                    self.cursor,
                    usize::from(self.cursor < self.bytes.len()),
                    "glTF JSON number fraction is missing a digit",
                );
            }
            while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                self.consume_number_byte(work, start)?;
            }
        }

        if self.peek().is_some_and(|byte| matches!(byte, b'e' | b'E')) {
            self.consume_number_byte(work, start)?;
            if self.peek().is_some_and(|byte| matches!(byte, b'+' | b'-')) {
                self.consume_number_byte(work, start)?;
            }
            if !self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                return self.syntax_error(
                    work,
                    "json-number",
                    self.cursor,
                    usize::from(self.cursor < self.bytes.len()),
                    "glTF JSON number exponent is missing a digit",
                );
            }
            while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                self.consume_number_byte(work, start)?;
            }
        }

        if self.peek().is_some_and(|byte| !is_value_delimiter(byte)) {
            return self.syntax_error_current(
                work,
                "json-number",
                self.cursor,
                "glTF JSON number has an invalid terminator",
            );
        }
        Ok(())
    }

    fn scan_literal(
        &mut self,
        work: &mut ImportWorkScope<'_, 'session, 1>,
        literal: &[u8],
    ) -> ScanResult<()> {
        self.count_token(work)?;
        let start = self.cursor;
        for expected in literal {
            let attempted = self
                .cursor
                .checked_add(1)
                .and_then(|end| end.checked_sub(start))
                .ok_or_else(|| self.ctx.allocation_failure("gltf-json-literal-bytes"))?;
            self.enforce_usize_limit(
                work,
                "gltf-json-literal-bytes",
                attempted,
                self.ctx.limits().max_token_bytes,
            )?;
            match self.peek() {
                Some(actual) if actual == *expected => self.consume(work, 1)?,
                Some(_) => {
                    return self.syntax_error_current(
                        work,
                        "json-literal",
                        self.cursor,
                        "glTF JSON literal is malformed",
                    );
                }
                None => {
                    return self.syntax_error(
                        work,
                        "json-literal",
                        start,
                        self.cursor - start,
                        "glTF JSON literal is truncated",
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
            );
        }
        Ok(())
    }

    fn scan_trailing(&mut self, work: &mut ImportWorkScope<'_, 'session, 1>) -> ScanResult<()> {
        self.skip_whitespace(work)?;
        if self.cursor == self.bytes.len() {
            return Ok(());
        }
        let length = self.current_scalar_length(work)?;
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
        self.syntax_error(work, subject, self.cursor, length, reason)
    }

    fn skip_whitespace(&mut self, work: &mut ImportWorkScope<'_, 'session, 1>) -> ScanResult<()> {
        while self.peek().is_some_and(is_json_whitespace) {
            self.consume(work, 1)?;
        }
        Ok(())
    }

    fn count_token(&mut self, work: &mut ImportWorkScope<'_, 'session, 1>) -> ScanResult<()> {
        let attempted = self
            .stats
            .tokens
            .checked_add(1)
            .ok_or_else(|| self.ctx.allocation_failure("gltf-json-token-count"))?;
        if attempted > self.ctx.limits().max_tokens {
            self.flush(work)?;
            return Err(ScanAbort::LedgerLimit {
                class: ImportWorkClass::Tokens,
                attempted,
            });
        }
        self.stats.tokens = attempted;
        Ok(())
    }

    fn increment_counter(&self, current: u64, resource: &'static str) -> ScanResult<u64> {
        let attempted = current
            .checked_add(1)
            .ok_or_else(|| self.ctx.allocation_failure(resource))?;
        Ok(attempted)
    }

    fn consume_string_bytes(
        &mut self,
        work: &mut ImportWorkScope<'_, 'session, 1>,
        content_start: usize,
        amount: usize,
    ) -> ScanResult<()> {
        self.enforce_string_bytes(work, content_start, amount)?;
        self.consume(work, amount)
    }

    fn enforce_string_bytes(
        &mut self,
        work: &mut ImportWorkScope<'_, 'session, 1>,
        content_start: usize,
        amount: usize,
    ) -> ScanResult<()> {
        let end = self
            .cursor
            .checked_add(amount)
            .ok_or_else(|| self.ctx.allocation_failure("gltf-json-string-bytes"))?;
        let attempted = end
            .checked_sub(content_start)
            .ok_or_else(|| self.ctx.allocation_failure("gltf-json-string-bytes"))?;
        self.enforce_usize_limit(
            work,
            "gltf-json-string-bytes",
            attempted,
            self.ctx.limits().max_token_bytes,
        )
    }

    fn consume_number_byte(
        &mut self,
        work: &mut ImportWorkScope<'_, 'session, 1>,
        start: usize,
    ) -> ScanResult<()> {
        let attempted = self
            .cursor
            .checked_add(1)
            .and_then(|end| end.checked_sub(start))
            .ok_or_else(|| self.ctx.allocation_failure("gltf-json-number-bytes"))?;
        self.enforce_usize_limit(
            work,
            "gltf-json-number-bytes",
            attempted,
            self.ctx.limits().max_token_bytes,
        )?;
        self.consume(work, 1)
    }

    fn enforce_usize_limit(
        &mut self,
        work: &mut ImportWorkScope<'_, 'session, 1>,
        resource: &'static str,
        attempted: usize,
        limit: usize,
    ) -> ScanResult<()> {
        if attempted > limit {
            self.flush(work)?;
            return Err(self
                .ctx
                .resource_limit_exceeded(resource, to_u64(limit))
                .into());
        }
        Ok(())
    }

    fn set_frame_state(&mut self, state: FrameState) -> ScanResult<()> {
        self.active_frame_mut()?.state = state;
        Ok(())
    }

    fn active_frame_mut(&mut self) -> ScanResult<&mut JsonFrame> {
        let byte_offset = self.absolute_offset(self.cursor)?;
        let Some(frame) = self.frames.as_mut_slice().last_mut() else {
            return Err(self
                .ctx
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
        work: &mut ImportWorkScope<'_, 'session, 1>,
    ) -> ScanResult<usize> {
        match self.peek() {
            Some(byte) if byte >= 0x80 => self.valid_utf8_scalar_width(work),
            Some(_) => Ok(1),
            None => Ok(0),
        }
    }

    fn valid_utf8_scalar_width(
        &mut self,
        work: &mut ImportWorkScope<'_, 'session, 1>,
    ) -> ScanResult<usize> {
        let lead = self.bytes[self.cursor];
        let width = match lead {
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => 1,
        };
        let end = self.cursor.saturating_add(width).min(self.bytes.len());
        let encoded = &self.bytes[self.cursor..end];
        match std::str::from_utf8(encoded) {
            Ok(value) if value.chars().count() == 1 => Ok(width),
            Ok(_) => Err(self
                .ctx
                .format_failure(GltfFailure::invalid_json(
                    "json-syntax-map",
                    self.absolute_offset(self.cursor)?,
                    to_u64(encoded.len()),
                    "glTF JSON UTF-8 scalar probe was ambiguous",
                ))
                .into()),
            Err(source) => {
                let byte_length = source.error_len().unwrap_or(encoded.len()).max(1);
                self.flush(work)?;
                Err(self
                    .ctx
                    .format_failure(GltfFailure::invalid_utf8(
                        source,
                        self.absolute_offset(self.cursor)?,
                        to_u64(byte_length),
                    ))
                    .into())
            }
        }
    }

    fn syntax_error<T>(
        &mut self,
        work: &mut ImportWorkScope<'_, 'session, 1>,
        subject: &'static str,
        offset: usize,
        length: usize,
        reason: &'static str,
    ) -> ScanResult<T> {
        // UTF-8 conformance owns precedence over grammar errors anywhere in the admitted surface.
        self.scan_remaining_utf8(work)?;
        self.flush(work)?;
        Err(self
            .ctx
            .format_failure(GltfFailure::invalid_json(
                subject,
                self.absolute_offset(offset)?,
                to_u64(length),
                reason,
            ))
            .into())
    }

    fn syntax_error_current<T>(
        &mut self,
        work: &mut ImportWorkScope<'_, 'session, 1>,
        subject: &'static str,
        offset: usize,
        reason: &'static str,
    ) -> ScanResult<T> {
        let length = self.current_scalar_length(work)?;
        self.syntax_error(work, subject, offset, length, reason)
    }

    fn scan_remaining_utf8(
        &mut self,
        work: &mut ImportWorkScope<'_, 'session, 1>,
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
                Ok(value) if value.chars().count() == 1 => self.consume(work, width)?,
                Ok(_) => {
                    return Err(self
                        .ctx
                        .format_failure(GltfFailure::invalid_json(
                            "json-syntax-map",
                            self.absolute_offset(start)?,
                            to_u64(encoded.len()),
                            "glTF JSON UTF-8 scalar probe was ambiguous",
                        ))
                        .into());
                }
                Err(source) => {
                    let byte_length = source.error_len().unwrap_or(encoded.len()).max(1);
                    self.cursor = start;
                    self.flush(work)?;
                    return Err(self
                        .ctx
                        .format_failure(GltfFailure::invalid_utf8(
                            source,
                            self.absolute_offset(start)?,
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
        work: &mut ImportWorkScope<'_, 'session, 1>,
        amount: usize,
    ) -> ScanResult<()> {
        self.cursor = self
            .cursor
            .checked_add(amount)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| self.ctx.allocation_failure("gltf-json-scan-offset"))?;
        if self.cursor - self.accounted_cursor >= SCAN_CHUNK_BYTES {
            self.flush(work)?;
        }
        Ok(())
    }

    fn flush(&mut self, work: &mut ImportWorkScope<'_, 'session, 1>) -> ScanResult<()> {
        let completed = self.cursor - self.accounted_cursor;
        if completed != 0 {
            work.complete([to_u64(completed)])?;
            self.accounted_cursor = self.cursor;
        }
        Ok(work.poll()?)
    }

    fn absolute_offset(&self, relative: usize) -> ParseResult<u64> {
        self.raw_base
            .checked_add(
                u64::try_from(relative)
                    .map_err(|_| self.ctx.allocation_failure("gltf-json-byte-offset"))?,
            )
            .ok_or_else(|| self.ctx.allocation_failure("gltf-json-byte-offset"))
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
    let mut name = [0_u8; 24];
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
        _ => SourceMember::Other,
    }
}

/// These are concrete core property objects, not paths through arbitrary extension JSON.
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

fn validate_duplicate_keys(
    ctx: &ParseScope<'_>,
    bytes: &[u8],
    raw_base: u64,
    keys: &mut MeteredVec<'_, JsonKey>,
) -> ParseResult<()> {
    let sort_work = duplicate_sort_work(keys.len());
    if sort_work != 0 {
        ctx.with_work(
            ImportWorkClass::WorkUnits,
            sort_work,
            |work| -> ParseResult<()> {
                let mut comparisons = 0_u64;
                keys.try_sort_unstable_by(|left, right| -> Result<Ordering, ParseFailure> {
                    comparisons = comparisons.saturating_add(1);
                    if comparisons.is_multiple_of(DUPLICATE_POLL_COMPARISONS) {
                        work.poll()?;
                    }
                    Ok(left
                        .object_id
                        .cmp(&right.object_id)
                        .then_with(|| left.digest.cmp(&right.digest))
                        .then_with(|| left.raw_start.cmp(&right.raw_start)))
                })?;
                work.complete_all()
            },
        )?;
    }

    let keys = keys.as_slice();
    let mut group_start = 0usize;
    while group_start < keys.len() {
        ctx.poll()?;
        let mut group_end = group_start + 1;
        while group_end < keys.len()
            && keys[group_end].object_id == keys[group_start].object_id
            && keys[group_end].digest == keys[group_start].digest
        {
            group_end += 1;
        }

        for left in group_start..group_end {
            for right in left + 1..group_end {
                if decoded_keys_equal(ctx, bytes, &keys[left], &keys[right])? {
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

fn duplicate_sort_work(key_count: usize) -> u64 {
    if key_count <= 1 {
        return 0;
    }
    let count = to_u64(key_count);
    let levels = u64::from(usize::BITS - (key_count - 1).leading_zeros());
    count.saturating_mul(levels.saturating_mul(4).saturating_add(4))
}

fn decoded_keys_equal(
    ctx: &ParseScope<'_>,
    bytes: &[u8],
    left: &JsonKey,
    right: &JsonKey,
) -> ParseResult<bool> {
    let work_amount = left
        .content_end
        .checked_sub(left.content_start)
        .and_then(|left_len| {
            right
                .content_end
                .checked_sub(right.content_start)
                .and_then(|right_len| left_len.checked_add(right_len))
        })
        .map(to_u64)
        .filter(|amount| *amount != 0)
        .unwrap_or(1);
    ctx.with_work(
        ImportWorkClass::WorkUnits,
        work_amount,
        |work| -> ParseResult<bool> {
            let mut left = DecodedScalars::new(bytes, left.content_start, left.content_end);
            let mut right = DecodedScalars::new(bytes, right.content_start, right.content_end);
            let mut scalars = 0usize;
            let equal = loop {
                if scalars.is_multiple_of(SCAN_CHUNK_BYTES) {
                    work.poll()?;
                }
                match (left.next_scalar(), right.next_scalar()) {
                    (Ok(Some(left)), Ok(Some(right))) if left == right => {
                        scalars = scalars.saturating_add(1);
                    }
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
            work.complete_all()?;
            Ok(equal)
        },
    )
}

struct DecodedScalars<'bytes> {
    bytes: &'bytes [u8],
    cursor: usize,
    end: usize,
}

impl<'bytes> DecodedScalars<'bytes> {
    const fn new(bytes: &'bytes [u8], cursor: usize, end: usize) -> Self {
        Self { bytes, cursor, end }
    }

    fn next_scalar(&mut self) -> Result<Option<u32>, ()> {
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
        DetectionOutcome, DetectionResult, DetectionScope, FormatDescriptor, FormatImporter,
        FormatSelection, ImportConfigOverlay, ImportOperationControl, ImportRequest,
        ImportRequestLayer, OptionCompileLimits, Probe, ReadConfidence, ReadHint,
        RegistryBuildLimits, RegistryBuilder, SceneDraft, import_bytes,
    };
    use baozi_io::AssetPath;
    use std::error::Error;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering as AtomicOrdering},
    };

    type TestResult = Result<(), Box<dyn Error>>;
    type Inspect = fn(&ParseScope<'_>, &GltfSourceMap<'_>);

    struct InspectAdmission {
        inspect: Inspect,
        inspected: Arc<AtomicBool>,
    }

    impl FormatImporter for InspectAdmission {
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

        fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
            let map = GltfSourceMap::from_primary_for_test(scope)?;
            (self.inspect)(scope, &map);
            self.inspected.store(true, AtomicOrdering::SeqCst);
            // The test stops at admission, before either backend decode or scene construction.
            Err(scope.format_failure(GltfFailure::internal("source map inspection completed")))
        }
    }

    fn inspect_admission(bytes: &[u8], inspect: Inspect) -> TestResult {
        let inspected = Arc::new(AtomicBool::new(false));
        let mut registry = RegistryBuilder::new();
        registry.register(InspectAdmission {
            inspect,
            inspected: Arc::clone(&inspected),
        })?;
        let request = registry
            .freeze(RegistryBuildLimits::default())?
            .compile_request(
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
        let result = import_bytes(
            &request,
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

    fn assert_raw(scope: &ParseScope<'_>, range: SourceRange, expected: &str) {
        let raw = range.raw_range();
        let start = usize::try_from(raw.start).expect("fixture offset fits usize");
        let end = usize::try_from(raw.end).expect("fixture offset fits usize");
        assert_eq!(
            scope.primary_bytes().get(start..end),
            Some(expected.as_bytes())
        );
    }

    fn glb(json: &[u8]) -> Vec<u8> {
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
        fn inspect(scope: &ParseScope<'_>, map: &GltfSourceMap<'_>) {
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
                (material, SourcePayloadSite::TextureTransform),
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
