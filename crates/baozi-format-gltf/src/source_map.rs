use baozi_import::{
    ImportStorageClass, ImportWorkClass, MeteredVec, ParseFailure, ParseResult, ParseScope,
};
use std::cmp::Ordering;
#[cfg(test)]
use std::ops::Range;

const SORT_POLL_COMPARISONS: u64 = 4 * 1024;

/// Format-owned identities refer to source array ordinals, including unselected entities.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum SourceEntityKind {
    Document,
    Scene,
    Node,
    Mesh,
    Primitive,
    Material,
    Texture,
    Image,
    Sampler,
    Skin,
    Buffer,
    BufferView,
    Accessor,
    Animation,
    AnimationChannel,
    AnimationSampler,
    Camera,
}

/// Nested ordinals are local to their mesh or animation, identified by `parent_index`.
/// Document uses index zero; top-level entities and Document have no parent index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SourceEntityId {
    pub(crate) kind: SourceEntityKind,
    pub(crate) parent_index: Option<usize>,
    pub(crate) source_index: usize,
}

/// A half-open range in the immutable primary snapshot, measured in raw bytes.
/// JSON whitespace and GLB chunk padding outside the value are excluded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SourceRange {
    pub(crate) byte_offset: u64,
    pub(crate) byte_length: u64,
}

impl SourceRange {
    pub(super) fn from_json_offsets(raw_base: u64, start: usize, end: usize) -> Option<Self> {
        let start = u64::try_from(start).ok()?;
        let end = u64::try_from(end).ok()?;
        let byte_offset = raw_base.checked_add(start)?;
        raw_base.checked_add(end)?;
        Some(Self {
            byte_offset,
            byte_length: end.checked_sub(start)?,
        })
    }

    #[cfg(test)]
    pub(crate) fn raw_range(self) -> Range<u64> {
        self.byte_offset..self.byte_offset + self.byte_length
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SourceEntity {
    pub(crate) id: SourceEntityId,
    pub(crate) range: SourceRange,
}

/// Core glTF objects nested inside an indexed entity retain that entity's source identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum SourcePayloadSite {
    Entity,
    Asset,
    MaterialPbrMetallicRoughness,
    BaseColorTexture,
    MetallicRoughnessTexture,
    NormalTexture,
    OcclusionTexture,
    EmissiveTexture,
    /// Direct payload members inside a recognized texture binding's KHR_texture_transform.
    TextureTransform,
    AccessorSparse,
    AccessorSparseIndices,
    AccessorSparseValues,
    AnimationChannelTarget,
    CameraOrthographic,
    CameraPerspective,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SourceExtensionKind {
    KhrTextureTransform,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SourcePayloadKind {
    Extras,
    /// The complete `extensions` member, including an empty or incorrectly typed value.
    Extensions,
    /// One direct member of an `extensions` object. Its raw key remains available below.
    Extension(SourceExtensionKind),
}

/// Raw payload syntax only; the caller decides extension support and semantic-loss policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SourcePayload {
    pub(crate) owner: SourceEntityId,
    pub(crate) site: SourcePayloadSite,
    pub(crate) kind: SourcePayloadKind,
    /// Includes the original quoted attribute name, preserving escapes.
    pub(crate) key_range: SourceRange,
    /// Includes the key, intervening colon/whitespace, and complete original JSON value.
    pub(crate) member_range: SourceRange,
    /// Includes delimiters or string quotes; excludes surrounding whitespace.
    pub(crate) value_range: SourceRange,
}

/// Admission-owned storage, independent of the decoder graph and its opaque allowance.
/// Moving this owner into NormalizedGltf keeps the complete allocation reservation alive.
/// Copying an individual range does not retain that reservation or the primary snapshot;
/// callers must keep the map and the session-owned immutable snapshot for their required phase.
pub(crate) struct GltfSourceMap<'scope> {
    entities: MeteredVec<'scope, SourceEntity>,
    payloads: MeteredVec<'scope, SourcePayload>,
}

impl<'scope> GltfSourceMap<'scope> {
    pub(super) fn new(ctx: &'scope ParseScope<'_>) -> Self {
        Self {
            entities: ctx.metered_vec(
                ImportStorageClass::FormatNormalization,
                "gltf-source-entities",
            ),
            payloads: ctx.metered_vec(
                ImportStorageClass::FormatNormalization,
                "gltf-source-payloads",
            ),
        }
    }

    /// Builds hand-authored normalized fixtures from the real primary snapshot without a backend.
    #[cfg(test)]
    pub(crate) fn from_primary_for_test(ctx: &'scope ParseScope<'_>) -> ParseResult<Self> {
        Ok(crate::admission::admit(ctx)?.into_source_map())
    }

    /// Only available after successful admission has completed and sorted the map.
    pub(crate) fn get(&self, id: SourceEntityId) -> Option<&SourceEntity> {
        let entities = self.entities.as_slice();
        let index = entities
            .binary_search_by_key(&id, |entity| entity.id)
            .ok()?;
        entities.get(index)
    }

    pub(crate) fn entities(&self) -> &[SourceEntity] {
        self.entities.as_slice()
    }

    /// Includes all declared owners, so unsupported unselected payloads remain observable.
    #[cfg(test)]
    pub(crate) fn payloads(&self) -> &[SourcePayload] {
        self.payloads.as_slice()
    }

    pub(crate) fn payloads_for(&self, owner: SourceEntityId) -> &[SourcePayload] {
        let payloads = self.payloads.as_slice();
        let start = payloads.partition_point(|payload| payload.owner < owner);
        let end = payloads.partition_point(|payload| payload.owner <= owner);
        &payloads[start..end]
    }

    pub(super) fn begin_entity(
        &mut self,
        id: SourceEntityId,
        start: SourceRange,
    ) -> ParseResult<usize> {
        let index = self.entities.len();
        self.entities.try_push(SourceEntity { id, range: start })?;
        Ok(index)
    }

    pub(super) fn entity_mut(&mut self, index: usize) -> Option<&mut SourceEntity> {
        self.entities.as_mut_slice().get_mut(index)
    }

    pub(super) fn begin_payload(&mut self, payload: SourcePayload) -> ParseResult<usize> {
        let index = self.payloads.len();
        self.payloads.try_push(payload)?;
        Ok(index)
    }

    pub(super) fn payload_mut(&mut self, index: usize) -> Option<&mut SourcePayload> {
        self.payloads.as_mut_slice().get_mut(index)
    }

    pub(super) fn finish(&mut self, ctx: &ParseScope<'_>) -> ParseResult<()> {
        sort_entries(ctx, &mut self.entities, |left, right| {
            left.id.cmp(&right.id)
        })?;
        sort_entries(ctx, &mut self.payloads, |left, right| {
            left.owner
                .cmp(&right.owner)
                .then_with(|| left.site.cmp(&right.site))
                .then_with(|| {
                    left.member_range
                        .byte_offset
                        .cmp(&right.member_range.byte_offset)
                })
        })
    }
}

fn sort_entries<T>(
    ctx: &ParseScope<'_>,
    entries: &mut MeteredVec<'_, T>,
    compare: impl Fn(&T, &T) -> Ordering,
) -> ParseResult<()> {
    let count = entries.len();
    if count <= 1 {
        return Ok(());
    }
    let levels = u64::from(usize::BITS - (count - 1).leading_zeros());
    let sort_work = u64::try_from(count)
        .unwrap_or(u64::MAX)
        .saturating_mul(levels.saturating_mul(4).saturating_add(4));
    ctx.with_work(ImportWorkClass::WorkUnits, sort_work, |work| {
        let mut comparisons = 0_u64;
        entries.try_sort_unstable_by(|left, right| -> Result<Ordering, ParseFailure> {
            comparisons = comparisons.saturating_add(1);
            if comparisons.is_multiple_of(SORT_POLL_COMPARISONS) {
                work.poll()?;
            }
            Ok(compare(left, right))
        })?;
        work.complete_all()
    })
}
