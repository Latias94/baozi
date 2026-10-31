//! Borrowed central profile validation. Only provenance depths require mutable scratch.

use crate::canonical_scene::store::production::ProfileOwnerLayout;
use crate::canonical_scene::store::typed_storage::validation::TypedStorageValidationControl;
use crate::extension::validate_media_type;
use crate::format_id::validate_format_id;
use crate::namespaced::validate_namespaced_name;
use crate::provenance::{
    DIGEST_LEN, MAX_OPERATION_EVIDENCE_SUMMARIES, validate_resource_identity,
    validate_source_display_name, validate_source_locator_value,
};

use super::*;

#[cfg(test)]
#[path = "../../validation/node_scene/profile/typed_tests.rs"]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct ProfileMeteredCounts {
    pub(in crate::canonical_scene::store) segment_count: u32,
    pub(in crate::canonical_scene::store) resource_count: u32,
    pub(in crate::canonical_scene::store) source_count: u32,
    pub(in crate::canonical_scene::store) provenance_count: u32,
    pub(in crate::canonical_scene::store) operation_count: u32,
    pub(in crate::canonical_scene::store) evidence_count: u32,
    pub(in crate::canonical_scene::store) parent_count: u32,
    pub(in crate::canonical_scene::store) wide_count: u32,
    pub(in crate::canonical_scene::store) lineage_text_bytes: u32,
    pub(in crate::canonical_scene::store) lineage_digest_bytes: u32,
    pub(in crate::canonical_scene::store) owner_count: u32,
    pub(in crate::canonical_scene::store) attachment_count: u32,
    pub(in crate::canonical_scene::store) metadata_entry_count: u32,
    pub(in crate::canonical_scene::store) metadata_text_bytes: u32,
    pub(in crate::canonical_scene::store) metadata_payload_bytes: u32,
    pub(in crate::canonical_scene::store) metadata_bool_count: u32,
    pub(in crate::canonical_scene::store) metadata_i64_count: u32,
    pub(in crate::canonical_scene::store) metadata_u64_count: u32,
    pub(in crate::canonical_scene::store) metadata_f64_count: u32,
    pub(in crate::canonical_scene::store) metadata_f32_count: u32,
}

impl ProfileMeteredCounts {
    #[cfg(test)]
    pub(in crate::canonical_scene::store) fn scratch_bytes(self) -> Option<u64> {
        u64::from(self.provenance_count)
            .checked_mul(u64::try_from(std::mem::size_of::<u32>()).ok()?)
    }

    /// The caller initializes its reserved depth slice before invoking this validator.
    /// This work is separate from `validation_work` and must be charged exactly once.
    pub(in crate::canonical_scene::store) fn scratch_initialization_work(self) -> Option<u64> {
        Some(u64::from(self.provenance_count))
    }

    /// Exact actions performed by `ProfileStorage::validate` on valid input.
    ///
    /// Three fixed actions guard the shape, finish metadata pools, and finish the profile.
    /// Each segment has start/end actions; each provenance node has header/depth actions;
    /// each owner has header/depth-cache actions. Other rows, edges, text/payload bytes,
    /// and metadata scalars each take one action.
    /// Inline wide values and digests are counted within their owning row actions.
    pub(in crate::canonical_scene::store) fn validation_work(self) -> Option<u64> {
        [
            3,
            u64::from(self.segment_count).checked_mul(2)?,
            u64::from(self.resource_count),
            u64::from(self.source_count),
            u64::from(self.provenance_count).checked_mul(2)?,
            u64::from(self.operation_count),
            u64::from(self.evidence_count),
            u64::from(self.parent_count),
            u64::from(self.owner_count).checked_mul(2)?,
            u64::from(self.attachment_count),
            u64::from(self.metadata_entry_count),
            u64::from(self.lineage_text_bytes),
            u64::from(self.metadata_text_bytes),
            u64::from(self.metadata_payload_bytes),
            u64::from(self.metadata_bool_count),
            u64::from(self.metadata_i64_count),
            u64::from(self.metadata_u64_count),
            u64::from(self.metadata_f64_count),
            u64::from(self.metadata_f32_count),
        ]
        .into_iter()
        .try_fold(0_u64, u64::checked_add)
    }

    fn lineage(self) -> Option<LineageCounts> {
        Some(LineageCounts {
            resources: usize::try_from(self.resource_count).ok()?,
            sources: usize::try_from(self.source_count).ok()?,
            provenance: usize::try_from(self.provenance_count).ok()?,
            operations: usize::try_from(self.operation_count).ok()?,
            evidence: usize::try_from(self.evidence_count).ok()?,
            parents: usize::try_from(self.parent_count).ok()?,
            wide: usize::try_from(self.wide_count).ok()?,
            text: usize::try_from(self.lineage_text_bytes).ok()?,
            digests: usize::try_from(self.lineage_digest_bytes).ok()?,
        })
    }
}

pub(in crate::canonical_scene::store) struct ProfileMeteredScratch<'scratch> {
    pub(in crate::canonical_scene::store) provenance_depths: &'scratch mut [u32],
}

impl<'scratch> ProfileMeteredScratch<'scratch> {
    pub(in crate::canonical_scene::store) const fn new(
        provenance_depths: &'scratch mut [u32],
    ) -> Self {
        Self { provenance_depths }
    }

    pub(in crate::canonical_scene::store) fn required_provenance_depths(
        provenance_count: u32,
    ) -> Option<usize> {
        usize::try_from(provenance_count).ok()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct TypedProfileValidation {
    pub(in crate::canonical_scene::store) maximum_provenance_depth: u32,
}

impl ProfileStorage {
    pub(in crate::canonical_scene::store) fn validate<C: TypedStorageValidationControl>(
        &self,
        control: &mut C,
        counts: ProfileMeteredCounts,
        owners: ProfileOwnerLayout,
        maximum_depth: u32,
        scratch: &mut ProfileMeteredScratch<'_>,
    ) -> Result<Option<TypedProfileValidation>, C::Error> {
        let state = self.state.as_ref();
        let metadata = self.metadata.as_ref();
        let Some(expected) = run(control, || {
            if counts.segment_count == 0
                || counts.owner_count < 2
                || owners.owner_count() != u64::from(counts.owner_count)
                || !count_matches(state.segments.len(), counts.segment_count)
                || !count_matches(state.owners.len(), counts.owner_count)
                || !count_matches(state.attachments.len(), counts.attachment_count)
                || !count_matches(scratch.provenance_depths.len(), counts.provenance_count)
                || !count_matches(state.metadata_entries.len(), counts.metadata_entry_count)
                || !count_matches(metadata.text.len(), counts.metadata_text_bytes)
                || !count_matches(metadata.bytes.len(), counts.metadata_payload_bytes)
                || !count_matches(metadata.bools.len(), counts.metadata_bool_count)
                || !count_matches(metadata.i64s.len(), counts.metadata_i64_count)
                || !count_matches(metadata.u64s.len(), counts.metadata_u64_count)
                || !count_matches(metadata.f64s.len(), counts.metadata_f64_count)
                || !count_matches(metadata.f32s.len(), counts.metadata_f32_count)
            {
                return None;
            }
            let expected = counts.lineage()?;
            (state.lineage_counts == expected).then_some(expected)
        })?
        else {
            return Ok(None);
        };

        let mut totals = LineageCounts::default();
        let mut previous_nonce = None;
        let mut maximum_provenance_depth = 0;
        for (ordinal, segment) in state.segments.iter().enumerate() {
            let Some(next) = run(control, || {
                let lineage = segment.lineage.as_ref();
                if segment.prefixes != totals
                    || previous_nonce.is_some_and(|previous| lineage.nonce <= previous)
                {
                    return None;
                }
                segment.domain(ordinal)?;
                let next = totals.checked_add(lineage.counts())?;
                lineage_fits(next, expected).then_some(next)
            })?
            else {
                return Ok(None);
            };
            let Some(depth) = validate_lineage(
                control,
                segment.lineage.as_ref(),
                &state.segments,
                ordinal,
                totals,
                maximum_depth,
                scratch.provenance_depths,
            )?
            else {
                return Ok(None);
            };
            maximum_provenance_depth = maximum_provenance_depth.max(depth);
            previous_nonce = Some(segment.lineage.as_ref().nonce);
            totals = next;
        }
        let Some(attachment_end) =
            validate_owners(control, state, owners, scratch.provenance_depths)?
        else {
            return Ok(None);
        };
        if !validate_metadata(control, &state.metadata_entries, metadata, owners)? {
            return Ok(None);
        }
        run(control, || {
            (totals == expected && attachment_end == state.attachments.len()).then_some(
                TypedProfileValidation {
                    maximum_provenance_depth,
                },
            )
        })
    }
}

fn count_matches(actual: usize, expected: u32) -> bool {
    usize::try_from(expected).ok() == Some(actual)
}

fn lineage_fits(actual: LineageCounts, expected: LineageCounts) -> bool {
    actual.resources <= expected.resources
        && actual.sources <= expected.sources
        && actual.provenance <= expected.provenance
        && actual.operations <= expected.operations
        && actual.evidence <= expected.evidence
        && actual.parents <= expected.parents
        && actual.wide <= expected.wide
        && actual.text <= expected.text
        && actual.digests <= expected.digests
}

fn run<C: TypedStorageValidationControl, R>(
    control: &mut C,
    action: impl FnOnce() -> Option<R>,
) -> Result<Option<R>, C::Error> {
    Ok(control.run_unit(action)?.flatten())
}

/// All values in a byte pool are valid; visiting them still observes central work/control.
fn visit_bytes<C: TypedStorageValidationControl>(
    control: &mut C,
    bytes: &[u8],
) -> Result<bool, C::Error> {
    for byte in bytes {
        if control.run_unit(|| *byte)?.is_none() {
            return Ok(false);
        }
    }
    Ok(true)
}

#[derive(Default)]
struct LineageCursors {
    text: usize,
    wide: usize,
    digests: usize,
    operations: usize,
    evidence: usize,
    parents: usize,
}

#[allow(clippy::too_many_arguments)]
fn validate_lineage<C: TypedStorageValidationControl>(
    control: &mut C,
    lineage: &LineageSegment,
    segments: &[StoredProfileSegment],
    ordinal: usize,
    prefixes: LineageCounts,
    maximum_depth: u32,
    depths: &mut [u32],
) -> Result<Option<u32>, C::Error> {
    if !visit_bytes(control, lineage.text.as_bytes())? {
        return Ok(None);
    }
    let mut cursors = LineageCursors::default();
    for resource in &lineage.resources {
        if run(control, || {
            let identity = consume_text(resource.identity, &lineage.text, &mut cursors.text)?;
            validate_resource_identity(identity).ok()?;
            if let StoredResourceStability::Stable { owner, schema, .. } = resource.stability {
                let owner = consume_text(owner, &lineage.text, &mut cursors.text)?;
                validate_namespaced_name(owner).ok()?;
                let schema = consume_text(schema, &lineage.text, &mut cursors.text)?;
                validate_namespaced_name(schema).ok()?;
            }
            Some(())
        })?
        .is_none()
        {
            return Ok(None);
        }
    }
    for source in &lineage.sources {
        if run(control, || {
            if usize::try_from(source.resource).ok()?
                >= prefixes.resources.checked_add(lineage.resources.len())?
            {
                return None;
            }
            let format = consume_text(source.format, &lineage.text, &mut cursors.text)?;
            validate_format_id(format).ok()?;
            match source.locator {
                StoredSourceLocator::Indexed { .. } => {
                    cursors.wide = cursors.wide.checked_add(1)?;
                }
                StoredSourceLocator::Namespaced { schema, value, .. } => {
                    let schema = consume_text(schema, &lineage.text, &mut cursors.text)?;
                    validate_namespaced_name(schema).ok()?;
                    let value = consume_text(value, &lineage.text, &mut cursors.text)?;
                    validate_source_locator_value(value).ok()?;
                }
            }
            if let Some(display_name) = source.display_name {
                let display_name = consume_text(display_name, &lineage.text, &mut cursors.text)?;
                validate_source_display_name(display_name).ok()?;
            }
            if source.content_digest.is_some() {
                cursors.digests = cursors.digests.checked_add(DIGEST_LEN)?;
            }
            Some(())
        })?
        .is_none()
        {
            return Ok(None);
        }
    }

    let mut maximum_provenance_depth = 0;
    for (local, node) in lineage.provenance.iter().enumerate() {
        let Some((global, parents, operation)) = run(control, || {
            let global = prefixes.provenance.checked_add(local)?;
            let parents = consume_range(node.parents, &lineage.parents, &mut cursors.parents)?;
            let operation = match node.kind {
                StoredProvenanceKind::Source(source) => {
                    if !parents.is_empty()
                        || usize::try_from(source).ok()?
                            >= prefixes.sources.checked_add(lineage.sources.len())?
                    {
                        return None;
                    }
                    None
                }
                StoredProvenanceKind::Operation(index) => {
                    if usize::try_from(index).ok()? != cursors.operations {
                        return None;
                    }
                    Some(lineage.operations.get(cursors.operations)?)
                }
            };
            Some((global, parents, operation))
        })?
        else {
            return Ok(None);
        };
        let mut previous_parent = None;
        let mut maximum_parent_depth = 0_u32;
        for parent in parents {
            let Some(parent_depth) = run(control, || {
                let parent = resolve_parent(segments, lineage, ordinal, prefixes, *parent)?;
                if parent >= global || previous_parent.is_some_and(|previous| parent <= previous) {
                    return None;
                }
                let depth = *depths.get(parent)?;
                if depth == 0 {
                    return None;
                }
                previous_parent = Some(parent);
                Some(depth)
            })?
            else {
                return Ok(None);
            };
            maximum_parent_depth = maximum_parent_depth.max(parent_depth);
        }
        if let Some(operation) = operation
            && !validate_operation(control, operation, lineage, &mut cursors)?
        {
            return Ok(None);
        }
        if run(control, || {
            let depth = maximum_parent_depth.checked_add(1)?;
            if node.depth != depth || depth > maximum_depth {
                return None;
            }
            *depths.get_mut(global)? = depth;
            maximum_provenance_depth = maximum_provenance_depth.max(depth);
            Some(())
        })?
        .is_none()
        {
            return Ok(None);
        }
    }
    run(control, || {
        (cursors.operations == lineage.operations.len()
            && cursors.evidence == lineage.evidence.len()
            && cursors.parents == lineage.parents.len()
            && cursors.text == lineage.text.len()
            && cursors.wide == lineage.wide_value_count
            && cursors.digests == lineage.digest_byte_count)
            .then_some(maximum_provenance_depth)
    })
}

fn resolve_parent(
    segments: &[StoredProfileSegment],
    lineage: &LineageSegment,
    ordinal: usize,
    prefixes: LineageCounts,
    reference: ProfileProvenanceRef,
) -> Option<usize> {
    let parent_ordinal = usize::try_from(reference.segment()).ok()?;
    if parent_ordinal > ordinal {
        return None;
    }
    let domain = if parent_ordinal == ordinal {
        ProfileSegmentAddressDomain::new(
            u32::try_from(ordinal).ok()?,
            lineage.nonce,
            u32::try_from(prefixes.provenance).ok()?,
            u32::try_from(lineage.provenance.len()).ok()?,
        )
    } else {
        segments.get(parent_ordinal)?.domain(parent_ordinal)?
    };
    usize::try_from(reference.resolve(domain)?).ok()
}

fn validate_operation<C: TypedStorageValidationControl>(
    control: &mut C,
    operation: &StoredOperation,
    lineage: &LineageSegment,
    cursors: &mut LineageCursors,
) -> Result<bool, C::Error> {
    let Some(evidence) = run(control, || {
        if operation.evidence.len() > MAX_OPERATION_EVIDENCE_SUMMARIES {
            return None;
        }
        if let StoredOperationKind::Namespaced(name) = operation.kind {
            let name = consume_text(name, &lineage.text, &mut cursors.text)?;
            validate_namespaced_name(name).ok()?;
        }
        cursors.digests = cursors
            .digests
            .checked_add(operation.options.as_bytes().len())?;
        cursors.operations = cursors.operations.checked_add(1)?;
        consume_range(operation.evidence, &lineage.evidence, &mut cursors.evidence)
    })?
    else {
        return Ok(false);
    };
    let mut previous_code = None;
    for row in evidence {
        if run(control, || {
            let code = consume_text(row.code, &lineage.text, &mut cursors.text)?;
            validate_namespaced_name(code).ok()?;
            if previous_code.is_some_and(|previous: &str| previous >= code) {
                return None;
            }
            previous_code = Some(code);
            cursors.wide = cursors.wide.checked_add(1)?;
            if row.digest.is_some() {
                cursors.digests = cursors.digests.checked_add(DIGEST_LEN)?;
            }
            Some(())
        })?
        .is_none()
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn validate_owners<C: TypedStorageValidationControl>(
    control: &mut C,
    state: &ProfileState,
    owners: ProfileOwnerLayout,
    depths: &[u32],
) -> Result<Option<usize>, C::Error> {
    let mut attachment_cursor = 0;
    for (ordinal, row) in state.owners.iter().enumerate() {
        let Some(attachments) = run(control, || {
            if owners.owner_at(u64::try_from(ordinal).ok()?) != Some(row.owner)
                || row.attachments.is_empty()
            {
                return None;
            }
            consume_range(row.attachments, &state.attachments, &mut attachment_cursor)
        })?
        else {
            return Ok(None);
        };
        let mut previous = None;
        let mut maximum_depth = 0_u32;
        for reference in attachments {
            if run(control, || {
                let ordinal = usize::try_from(reference.segment()).ok()?;
                let segment = state.segments.get(ordinal)?;
                let global = usize::try_from(reference.resolve(segment.domain(ordinal)?)?).ok()?;
                if global >= state.lineage_counts.provenance
                    || previous.is_some_and(|previous| global <= previous)
                {
                    return None;
                }
                let depth = *depths.get(global)?;
                if depth == 0 {
                    return None;
                }
                maximum_depth = maximum_depth.max(depth);
                previous = Some(global);
                Some(())
            })?
            .is_none()
            {
                return Ok(None);
            }
        }
        if control.run_unit(|| row.maximum_depth == maximum_depth)? != Some(true) {
            return Ok(None);
        }
    }
    Ok(Some(attachment_cursor))
}

#[derive(Default)]
struct MetadataCursors {
    text: usize,
    bytes: usize,
    bools: usize,
    i64s: usize,
    u64s: usize,
    f64s: usize,
    f32s: usize,
}

fn validate_metadata<C: TypedStorageValidationControl>(
    control: &mut C,
    entries: &[StoredMetadataEntry],
    pools: &MetadataPayloads,
    owners: ProfileOwnerLayout,
) -> Result<bool, C::Error> {
    if !visit_bytes(control, pools.text.as_bytes())?
        || !visit_bytes(control, &pools.bytes)?
        || !visit_scalars(control, &pools.bools, |value| *value <= 1)?
        || !visit_scalars(control, &pools.i64s, |_| true)?
        || !visit_scalars(control, &pools.u64s, |_| true)?
        || !visit_scalars(control, &pools.f64s, |value| value.is_finite())?
        || !visit_scalars(control, &pools.f32s, |value| value.is_finite())?
    {
        return Ok(false);
    }
    let mut cursors = MetadataCursors::default();
    let mut previous_owner = None;
    let mut previous_key = None;
    for row in entries {
        if run(control, || {
            if !owners.contains(row.owner) {
                return None;
            }
            let key = consume_text(row.key, &pools.text, &mut cursors.text)?;
            validate_namespaced_name(key).ok()?;
            if previous_owner.is_some_and(|previous| row.owner < previous)
                || previous_owner == Some(row.owner)
                    && previous_key.is_some_and(|previous: &str| previous >= key)
            {
                return None;
            }
            consume_metadata_value(row.value, pools, &mut cursors)?;
            previous_owner = Some(row.owner);
            previous_key = Some(key);
            Some(())
        })?
        .is_none()
        {
            return Ok(false);
        }
    }
    Ok(control.run_unit(|| {
        cursors.text == pools.text.len()
            && cursors.bytes == pools.bytes.len()
            && cursors.bools == pools.bools.len()
            && cursors.i64s == pools.i64s.len()
            && cursors.u64s == pools.u64s.len()
            && cursors.f64s == pools.f64s.len()
            && cursors.f32s == pools.f32s.len()
    })? == Some(true))
}

fn consume_metadata_value(
    value: StoredMetadataValue,
    pools: &MetadataPayloads,
    cursors: &mut MetadataCursors,
) -> Option<()> {
    use StoredMetadataValue as Value;
    match value {
        Value::Bool(range) => {
            consume_shaped_range(range, &pools.bools, &mut cursors.bools, 1, false)?
        }
        Value::I64(range) => consume_shaped_range(range, &pools.i64s, &mut cursors.i64s, 1, false)?,
        Value::U64(range) => consume_shaped_range(range, &pools.u64s, &mut cursors.u64s, 1, false)?,
        Value::F64(range) => consume_shaped_range(range, &pools.f64s, &mut cursors.f64s, 1, false)?,
        Value::String(range) => {
            consume_text(range, &pools.text, &mut cursors.text)?;
        }
        Value::Bytes {
            media_type,
            payload,
        } => {
            let media_type = consume_text(media_type, &pools.text, &mut cursors.text)?;
            validate_media_type(media_type).ok()?;
            consume_range(payload, &pools.bytes, &mut cursors.bytes)?;
        }
        Value::Vec2(range) => {
            consume_shaped_range(range, &pools.f32s, &mut cursors.f32s, 2, false)?
        }
        Value::Vec3(range) => {
            consume_shaped_range(range, &pools.f32s, &mut cursors.f32s, 3, false)?
        }
        Value::Vec4(range) | Value::Color(range) => {
            consume_shaped_range(range, &pools.f32s, &mut cursors.f32s, 4, false)?
        }
        Value::Mat4(range) => {
            consume_shaped_range(range, &pools.f32s, &mut cursors.f32s, 16, false)?
        }
        Value::BoolArray(range) => {
            consume_shaped_range(range, &pools.bools, &mut cursors.bools, 1, true)?
        }
        Value::I64Array(range) => {
            consume_shaped_range(range, &pools.i64s, &mut cursors.i64s, 1, true)?
        }
        Value::U64Array(range) => {
            consume_shaped_range(range, &pools.u64s, &mut cursors.u64s, 1, true)?
        }
        Value::F64Array(range) => {
            consume_shaped_range(range, &pools.f64s, &mut cursors.f64s, 1, true)?
        }
        Value::Vec2Array(range) => {
            consume_shaped_range(range, &pools.f32s, &mut cursors.f32s, 2, true)?
        }
        Value::Vec3Array(range) => {
            consume_shaped_range(range, &pools.f32s, &mut cursors.f32s, 3, true)?
        }
        Value::Vec4Array(range) | Value::ColorArray(range) => {
            consume_shaped_range(range, &pools.f32s, &mut cursors.f32s, 4, true)?
        }
        Value::Mat4Array(range) => {
            consume_shaped_range(range, &pools.f32s, &mut cursors.f32s, 16, true)?
        }
    }
    Some(())
}

fn consume_shaped_range<T>(
    range: TableRange<T>,
    pool: &[T],
    cursor: &mut usize,
    width: usize,
    array: bool,
) -> Option<()> {
    let valid_count = if array {
        range.len().is_multiple_of(width)
    } else {
        range.len() == width
    };
    if !valid_count {
        return None;
    }
    consume_range(range, pool, cursor)?;
    Some(())
}

fn consume_text<'storage>(
    range: TextRange,
    text: &'storage str,
    cursor: &mut usize,
) -> Option<&'storage str> {
    if range.0.start() != *cursor {
        return None;
    }
    let value = range.get_str(text)?;
    *cursor = cursor.checked_add(range.0.len())?;
    Some(value)
}

fn consume_range<'storage, T>(
    range: TableRange<T>,
    pool: &'storage [T],
    cursor: &mut usize,
) -> Option<&'storage [T]> {
    if range.start() != *cursor {
        return None;
    }
    let values = range.get(pool)?;
    *cursor = cursor.checked_add(values.len())?;
    Some(values)
}

fn visit_scalars<C: TypedStorageValidationControl, T>(
    control: &mut C,
    values: &[T],
    valid: impl Fn(&T) -> bool,
) -> Result<bool, C::Error> {
    for value in values {
        if control.run_unit(|| valid(value))? != Some(true) {
            return Ok(false);
        }
    }
    Ok(true)
}
