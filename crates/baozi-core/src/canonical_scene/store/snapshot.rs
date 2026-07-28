use baozi_operation::{CancellationToken, CapacityPlan, CapacityPlanError};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::entity::{AuthoredTransform, AuthoredTransformKind};

use super::construction::node_scene::ValidatedNodeScene;

const NODE_SCENE_SNAPSHOT_DOMAIN: &str = "baozi.canonical-node-scene.snapshot";
const NODE_SCENE_SNAPSHOT_SCHEMA_REVISION: u32 = 1;
const NODE_SCENE_SNAPSHOT_ALGORITHM_ID: &str = "sha256";
const NODE_SCENE_SNAPSHOT_POLL_INTERVAL: u64 = 64;
const MAX_PREVIEW_BYTES: usize = 1 << 20;
const MAX_COLLECTION_ITEMS: usize = 1 << 20;
const MAX_WORK: u64 = 100_000_000;
#[cfg(test)]
const TEST_TRANSCRIPT_MAX_BYTES: usize = 512;

const TAG_DOMAIN: u8 = 1;
const TAG_SCHEMA_REVISION: u8 = 2;
const TAG_ALGORITHM: u8 = 3;
const TAG_ROOTS: u8 = 4;
const TAG_NODES: u8 = 5;
const TAG_NODE: u8 = 6;
const TAG_CHILDREN: u8 = 7;
const TAG_ROOT_REFERENCE: u8 = 8;
const TAG_CHILD_REFERENCE: u8 = 9;
const TAG_TRANSFORM: u8 = 10;
const TAG_COLLECTION_TOTAL: u8 = 11;
const TAG_PRESENCE: u8 = 12;
const TAG_REFERENCE: u8 = 13;
const TAG_FLOAT_BITS: u8 = 14;

const PRESENCE_PRESENT: u8 = 1;
const TRANSFORM_MATRIX: u8 = 1;
const TRANSFORM_TRS: u8 = 2;

/// Bounds a private node-scene snapshot capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneSnapshotLimits {
    max_preview_bytes: usize,
    max_collection_items: usize,
    max_work: u64,
}

impl NodeSceneSnapshotLimits {
    /// Validates hard caps before traversal or allocation begins.
    ///
    /// Zero preview limits are valid and still produce a complete digest. A zero
    /// work limit is also valid, but capture will reject before encoding its first
    /// semantic value.
    pub(in crate::canonical_scene::store) fn try_new(
        max_preview_bytes: usize,
        max_collection_items: usize,
        max_work: u64,
    ) -> Result<Self, NodeSceneSnapshotError> {
        validate_usize_limit(
            NodeSceneSnapshotLimit::PreviewBytes,
            max_preview_bytes,
            MAX_PREVIEW_BYTES,
        )?;
        validate_usize_limit(
            NodeSceneSnapshotLimit::CollectionItems,
            max_collection_items,
            MAX_COLLECTION_ITEMS,
        )?;
        if max_work > MAX_WORK {
            return Err(NodeSceneSnapshotError::InvalidLimit {
                limit: NodeSceneSnapshotLimit::Work,
                value: max_work,
                maximum: MAX_WORK,
            });
        }
        Ok(Self {
            max_preview_bytes,
            max_collection_items,
            max_work,
        })
    }
}

/// Identifies the configured snapshot bound that rejected capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum NodeSceneSnapshotLimit {
    PreviewBytes,
    CollectionItems,
    Work,
}

/// Identifies the digest algorithm committed by the snapshot schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum NodeSceneSnapshotAlgorithm {
    Sha256,
}

/// Complete semantic totals independent from the bounded preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneSnapshotTotals {
    pub(in crate::canonical_scene::store) node_count: usize,
    pub(in crate::canonical_scene::store) root_count: usize,
    pub(in crate::canonical_scene::store) edge_count: usize,
    pub(in crate::canonical_scene::store) transcript_bytes: u64,
    pub(in crate::canonical_scene::store) work_units: u64,
}

/// Human-reviewable prefix produced from the same semantic events as the digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneSnapshotPreview {
    pub(in crate::canonical_scene::store) text: String,
    pub(in crate::canonical_scene::store) items_omitted_by_limit: u64,
    pub(in crate::canonical_scene::store) bytes_truncated: bool,
    pub(in crate::canonical_scene::store) items_truncated: bool,
}

/// Private, complete node-scene digest and a review-bounded semantic preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneSnapshot {
    pub(in crate::canonical_scene::store) domain: &'static str,
    pub(in crate::canonical_scene::store) schema_revision: u32,
    pub(in crate::canonical_scene::store) algorithm: NodeSceneSnapshotAlgorithm,
    pub(in crate::canonical_scene::store) digest: [u8; 32],
    pub(in crate::canonical_scene::store) preview: NodeSceneSnapshotPreview,
    pub(in crate::canonical_scene::store) totals: NodeSceneSnapshotTotals,
}

struct NodeSceneCaptureOutput {
    snapshot: NodeSceneSnapshot,
    #[cfg(test)]
    transcript: Option<TestCanonicalTranscript>,
}

#[cfg(test)]
struct TestCanonicalTranscript {
    bytes: Vec<u8>,
    complete: bool,
}

/// Capture failed before a complete snapshot could be returned.
#[derive(Debug, Error)]
pub(in crate::canonical_scene::store) enum NodeSceneSnapshotError {
    #[error("snapshot {limit:?} limit {value} exceeds hard maximum {maximum}")]
    InvalidLimit {
        limit: NodeSceneSnapshotLimit,
        value: u64,
        maximum: u64,
    },
    #[error("node-scene snapshot work {observed} exceeds limit {limit}")]
    WorkBudgetExceeded { observed: u64, limit: u64 },
    #[error("node-scene snapshot capture was cancelled")]
    Cancelled,
    #[error("node-scene canonical view is inconsistent: {reason}")]
    InconsistentView { reason: &'static str },
    #[error("node-scene snapshot transcript length overflowed")]
    TranscriptLengthOverflow,
    #[error("node-scene snapshot preview item total overflowed")]
    PreviewItemCountOverflow,
    #[error("node-scene snapshot preview capacity planning failed: {source}")]
    PreviewCapacity {
        #[source]
        source: CapacityPlanError,
    },
    #[error("node-scene snapshot preview allocation failed")]
    PreviewAllocation,
}

/// Captures a complete private canonical node-scene digest and bounded preview.
pub(in crate::canonical_scene::store) fn capture_node_scene(
    scene: &ValidatedNodeScene,
    limits: NodeSceneSnapshotLimits,
    cancellation: Option<&CancellationToken>,
) -> Result<NodeSceneSnapshot, NodeSceneSnapshotError> {
    capture_node_scene_with_encoder(scene, limits, cancellation, CanonicalDigestEncoder::new())
        .map(|output| output.snapshot)
}

#[cfg(test)]
fn capture_node_scene_with_test_transcript(
    scene: &ValidatedNodeScene,
    limits: NodeSceneSnapshotLimits,
    cancellation: Option<&CancellationToken>,
) -> Result<(NodeSceneSnapshot, TestCanonicalTranscript), NodeSceneSnapshotError> {
    let output = capture_node_scene_with_encoder(
        scene,
        limits,
        cancellation,
        CanonicalDigestEncoder::with_test_transcript(),
    )?;
    let transcript = output
        .transcript
        .expect("test transcript capture must return its bounded recorder");
    Ok((output.snapshot, transcript))
}

fn capture_node_scene_with_encoder(
    scene: &ValidatedNodeScene,
    limits: NodeSceneSnapshotLimits,
    cancellation: Option<&CancellationToken>,
    mut transcript: CanonicalDigestEncoder,
) -> Result<NodeSceneCaptureOutput, NodeSceneSnapshotError> {
    let mut work = SnapshotWorkMeter::new(limits.max_work, cancellation);
    work.poll()?;

    let node_count = scene
        .source_node_count()
        .ok_or(NodeSceneSnapshotError::InconsistentView {
            reason: "missing source node count",
        })?;
    let root_count = scene
        .source_root_count()
        .ok_or(NodeSceneSnapshotError::InconsistentView {
            reason: "missing source root count",
        })?;
    let expected_edges =
        node_count
            .checked_sub(root_count)
            .ok_or(NodeSceneSnapshotError::InconsistentView {
                reason: "root count exceeds node count",
            })?;

    if limits.max_preview_bytes != 0 {
        // Preview allocation is observable snapshot work and must be admitted
        // before capacity planning or allocator entry.
        work.observe()?;
    }
    let mut preview = PreviewSink::new(limits.max_preview_bytes, limits.max_collection_items)?;

    work.observe()?;
    transcript.tagged_bytes(TAG_DOMAIN, NODE_SCENE_SNAPSHOT_DOMAIN.as_bytes())?;
    preview.header(
        NODE_SCENE_SNAPSHOT_DOMAIN,
        NODE_SCENE_SNAPSHOT_SCHEMA_REVISION,
        NODE_SCENE_SNAPSHOT_ALGORITHM_ID,
    );
    work.observe()?;
    transcript.tagged_u32(TAG_SCHEMA_REVISION, NODE_SCENE_SNAPSHOT_SCHEMA_REVISION)?;
    work.observe()?;
    transcript.tagged_bytes(TAG_ALGORITHM, NODE_SCENE_SNAPSHOT_ALGORITHM_ID.as_bytes())?;

    work.observe()?;
    transcript.write_tag(TAG_ROOTS)?;
    transcript.tagged_u8(TAG_PRESENCE, PRESENCE_PRESENT)?;
    preview.begin_roots(root_count);
    for root_ordinal in 0..root_count {
        work.observe()?;
        let child =
            scene
                .child(None, root_ordinal)
                .ok_or(NodeSceneSnapshotError::InconsistentView {
                    reason: "root collection ended before declared total",
                })?;
        ensure_node_reference(child, node_count)?;
        transcript.write_tag(TAG_ROOT_REFERENCE)?;
        transcript.tagged_usize(TAG_REFERENCE, child)?;
        preview.reference(root_ordinal, child)?;
    }
    if scene.child(None, root_count).is_some() {
        return Err(NodeSceneSnapshotError::InconsistentView {
            reason: "root collection exceeds declared total",
        });
    }
    transcript.tagged_usize(TAG_COLLECTION_TOTAL, root_count)?;
    preview.end_collection();

    work.observe()?;
    transcript.write_tag(TAG_NODES)?;
    transcript.tagged_u8(TAG_PRESENCE, PRESENCE_PRESENT)?;
    preview.begin_nodes(node_count);
    let mut edge_count = 0_usize;
    for node_index in 0..node_count {
        work.observe()?;
        let transform = scene.authored_transform(node_index).ok_or(
            NodeSceneSnapshotError::InconsistentView {
                reason: "node has no authored transform",
            },
        )?;
        transcript.write_tag(TAG_NODE)?;
        transcript.tagged_usize(TAG_REFERENCE, node_index)?;
        let render_node = preview.begin_node(node_index)?;
        write_transform(
            &mut transcript,
            &mut preview,
            &mut work,
            transform,
            render_node,
        )?;

        work.observe()?;
        transcript.write_tag(TAG_CHILDREN)?;
        transcript.tagged_u8(TAG_PRESENCE, PRESENCE_PRESENT)?;
        preview.begin_children(render_node);
        let mut child_ordinal = 0_usize;
        while let Some(child) = scene.child(Some(node_index), child_ordinal) {
            if edge_count == expected_edges {
                return Err(NodeSceneSnapshotError::InconsistentView {
                    reason: "child collections exceed the forest edge total",
                });
            }
            work.observe()?;
            ensure_node_reference(child, node_count)?;
            transcript.write_tag(TAG_CHILD_REFERENCE)?;
            transcript.tagged_usize(TAG_REFERENCE, child)?;
            preview.child_reference(render_node, child_ordinal, child)?;
            child_ordinal =
                child_ordinal
                    .checked_add(1)
                    .ok_or(NodeSceneSnapshotError::InconsistentView {
                        reason: "child ordinal overflowed",
                    })?;
            edge_count =
                edge_count
                    .checked_add(1)
                    .ok_or(NodeSceneSnapshotError::InconsistentView {
                        reason: "edge total overflowed",
                    })?;
        }
        transcript.tagged_usize(TAG_COLLECTION_TOTAL, child_ordinal)?;
        preview.end_node(render_node, child_ordinal);
    }
    transcript.tagged_usize(TAG_COLLECTION_TOTAL, node_count)?;

    if edge_count != expected_edges {
        return Err(NodeSceneSnapshotError::InconsistentView {
            reason: "root and edge totals do not close over nodes",
        });
    }
    preview.summary(node_count, root_count, edge_count);
    work.finish()?;
    let work_units = work.observed();
    let encoded = transcript.finish();
    Ok(NodeSceneCaptureOutput {
        snapshot: NodeSceneSnapshot {
            domain: NODE_SCENE_SNAPSHOT_DOMAIN,
            schema_revision: NODE_SCENE_SNAPSHOT_SCHEMA_REVISION,
            algorithm: NodeSceneSnapshotAlgorithm::Sha256,
            digest: encoded.digest,
            preview: preview.finish(),
            totals: NodeSceneSnapshotTotals {
                node_count,
                root_count,
                edge_count,
                transcript_bytes: encoded.bytes,
                work_units,
            },
        },
        #[cfg(test)]
        transcript: encoded.transcript,
    })
}

fn validate_usize_limit(
    limit: NodeSceneSnapshotLimit,
    value: usize,
    maximum: usize,
) -> Result<(), NodeSceneSnapshotError> {
    if value > maximum {
        return Err(NodeSceneSnapshotError::InvalidLimit {
            limit,
            value: u64::try_from(value).unwrap_or(u64::MAX),
            maximum: u64::try_from(maximum).unwrap_or(u64::MAX),
        });
    }
    Ok(())
}

fn ensure_node_reference(
    reference: usize,
    node_count: usize,
) -> Result<(), NodeSceneSnapshotError> {
    if reference >= node_count {
        return Err(NodeSceneSnapshotError::InconsistentView {
            reason: "child reference is outside the node collection",
        });
    }
    Ok(())
}

fn write_transform(
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
    transform: AuthoredTransform,
    render_node: bool,
) -> Result<(), NodeSceneSnapshotError> {
    work.observe()?;
    transcript.write_tag(TAG_TRANSFORM)?;
    match transform.kind() {
        AuthoredTransformKind::Matrix => {
            transcript.write_u8(TRANSFORM_MATRIX)?;
            let matrix =
                transform
                    .authored_matrix()
                    .ok_or(NodeSceneSnapshotError::InconsistentView {
                        reason: "matrix transform lost its authored representation",
                    })?;
            preview.begin_transform(render_node, "matrix", 16);
            for (ordinal, value) in matrix.cols.into_iter().flatten().enumerate() {
                work.observe()?;
                let bits = value.to_bits();
                transcript.tagged_u32(TAG_FLOAT_BITS, bits)?;
                preview.transform_bits(render_node, ordinal, bits)?;
            }
            transcript.tagged_usize(TAG_COLLECTION_TOTAL, 16)?;
            preview.end_transform(render_node, 16);
        }
        AuthoredTransformKind::Trs => {
            transcript.write_u8(TRANSFORM_TRS)?;
            let trs = transform
                .authored_trs()
                .ok_or(NodeSceneSnapshotError::InconsistentView {
                    reason: "TRS transform lost its authored representation",
                })?;
            let translation = trs.translation();
            let rotation = trs.rotation();
            let scale = trs.scale();
            preview.begin_transform(render_node, "trs", 10);
            for (ordinal, value) in [
                translation.x,
                translation.y,
                translation.z,
                rotation.x,
                rotation.y,
                rotation.z,
                rotation.w,
                scale.x,
                scale.y,
                scale.z,
            ]
            .into_iter()
            .enumerate()
            {
                work.observe()?;
                let bits = value.to_bits();
                transcript.tagged_u32(TAG_FLOAT_BITS, bits)?;
                preview.transform_bits(render_node, ordinal, bits)?;
            }
            transcript.tagged_usize(TAG_COLLECTION_TOTAL, 10)?;
            preview.end_transform(render_node, 10);
        }
    }
    Ok(())
}

struct SnapshotWorkMeter<'a> {
    limit: u64,
    observed: u64,
    since_poll: u64,
    cancellation: Option<&'a CancellationToken>,
}

impl<'a> SnapshotWorkMeter<'a> {
    const fn new(limit: u64, cancellation: Option<&'a CancellationToken>) -> Self {
        Self {
            limit,
            observed: 0,
            since_poll: 0,
            cancellation,
        }
    }

    const fn observed(&self) -> u64 {
        self.observed
    }

    fn observe(&mut self) -> Result<(), NodeSceneSnapshotError> {
        self.observed =
            self.observed
                .checked_add(1)
                .ok_or(NodeSceneSnapshotError::WorkBudgetExceeded {
                    observed: u64::MAX,
                    limit: self.limit,
                })?;
        if self.observed > self.limit {
            return Err(NodeSceneSnapshotError::WorkBudgetExceeded {
                observed: self.observed,
                limit: self.limit,
            });
        }
        self.since_poll =
            self.since_poll
                .checked_add(1)
                .ok_or(NodeSceneSnapshotError::WorkBudgetExceeded {
                    observed: self.observed,
                    limit: self.limit,
                })?;
        if self.since_poll == NODE_SCENE_SNAPSHOT_POLL_INTERVAL {
            self.poll()?;
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<(), NodeSceneSnapshotError> {
        self.poll()
    }

    fn poll(&mut self) -> Result<(), NodeSceneSnapshotError> {
        if self
            .cancellation
            .is_some_and(CancellationToken::is_cancelled)
        {
            return Err(NodeSceneSnapshotError::Cancelled);
        }
        self.since_poll = 0;
        Ok(())
    }
}

struct CanonicalDigestEncoder {
    digest: Sha256,
    bytes: u64,
    #[cfg(test)]
    transcript: Option<TestTranscriptRecorder>,
}

struct CanonicalDigestOutput {
    digest: [u8; 32],
    bytes: u64,
    #[cfg(test)]
    transcript: Option<TestCanonicalTranscript>,
}

#[cfg(test)]
struct TestTranscriptRecorder {
    bytes: Vec<u8>,
    complete: bool,
}

#[cfg(test)]
impl TestTranscriptRecorder {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            complete: true,
        }
    }

    fn observe(&mut self, bytes: &[u8]) {
        let remaining = TEST_TRANSCRIPT_MAX_BYTES.saturating_sub(self.bytes.len());
        let captured = remaining.min(bytes.len());
        self.bytes.extend_from_slice(&bytes[..captured]);
        self.complete &= captured == bytes.len();
    }

    fn finish(self) -> TestCanonicalTranscript {
        TestCanonicalTranscript {
            bytes: self.bytes,
            complete: self.complete,
        }
    }
}

impl CanonicalDigestEncoder {
    fn new() -> Self {
        Self {
            digest: Sha256::new(),
            bytes: 0,
            #[cfg(test)]
            transcript: None,
        }
    }

    #[cfg(test)]
    fn with_test_transcript() -> Self {
        Self {
            digest: Sha256::new(),
            bytes: 0,
            transcript: Some(TestTranscriptRecorder::new()),
        }
    }

    fn finish(self) -> CanonicalDigestOutput {
        CanonicalDigestOutput {
            digest: self.digest.finalize().into(),
            bytes: self.bytes,
            #[cfg(test)]
            transcript: self.transcript.map(TestTranscriptRecorder::finish),
        }
    }

    fn tagged_u8(&mut self, tag: u8, value: u8) -> Result<(), NodeSceneSnapshotError> {
        self.write_tag(tag)?;
        self.write_u8(value)
    }

    fn tagged_u32(&mut self, tag: u8, value: u32) -> Result<(), NodeSceneSnapshotError> {
        self.write_tag(tag)?;
        self.write_bytes(&value.to_le_bytes())
    }

    fn tagged_usize(&mut self, tag: u8, value: usize) -> Result<(), NodeSceneSnapshotError> {
        let value =
            u64::try_from(value).map_err(|_| NodeSceneSnapshotError::TranscriptLengthOverflow)?;
        self.write_tag(tag)?;
        self.write_bytes(&value.to_le_bytes())
    }

    fn tagged_bytes(&mut self, tag: u8, value: &[u8]) -> Result<(), NodeSceneSnapshotError> {
        let length = u64::try_from(value.len())
            .map_err(|_| NodeSceneSnapshotError::TranscriptLengthOverflow)?;
        self.write_tag(tag)?;
        self.write_bytes(&length.to_le_bytes())?;
        self.write_bytes(value)
    }

    fn write_tag(&mut self, tag: u8) -> Result<(), NodeSceneSnapshotError> {
        self.write_u8(tag)
    }

    fn write_u8(&mut self, value: u8) -> Result<(), NodeSceneSnapshotError> {
        self.write_bytes(&[value])
    }

    fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), NodeSceneSnapshotError> {
        let length = u64::try_from(bytes.len())
            .map_err(|_| NodeSceneSnapshotError::TranscriptLengthOverflow)?;
        self.bytes = self
            .bytes
            .checked_add(length)
            .ok_or(NodeSceneSnapshotError::TranscriptLengthOverflow)?;
        self.digest.update(bytes);
        #[cfg(test)]
        if let Some(transcript) = &mut self.transcript {
            transcript.observe(bytes);
        }
        Ok(())
    }
}

struct PreviewSink {
    text: String,
    byte_limit: usize,
    item_limit: usize,
    items_omitted_by_limit: u64,
    bytes_truncated: bool,
    items_truncated: bool,
}

impl PreviewSink {
    fn new(byte_limit: usize, item_limit: usize) -> Result<Self, NodeSceneSnapshotError> {
        let text = if byte_limit == 0 {
            String::new()
        } else {
            // `byte_limit` is the exact visible-output cap. CapacityPlan separately
            // admits the allocator envelope, which remains below the hard preview
            // cap because that cap is a power of two.
            let plan = CapacityPlan::for_type::<u8>(0, 0, byte_limit)
                .map_err(|source| NodeSceneSnapshotError::PreviewCapacity { source })?;
            let mut text = String::new();
            text.try_reserve_exact(plan.allocator_additional())
                .map_err(|_| NodeSceneSnapshotError::PreviewAllocation)?;
            plan.finish(text.len(), text.capacity())
                .map_err(|source| NodeSceneSnapshotError::PreviewCapacity { source })?;
            text
        };
        Ok(Self {
            text,
            byte_limit,
            item_limit,
            items_omitted_by_limit: 0,
            bytes_truncated: false,
            items_truncated: false,
        })
    }

    fn finish(self) -> NodeSceneSnapshotPreview {
        NodeSceneSnapshotPreview {
            text: self.text,
            items_omitted_by_limit: self.items_omitted_by_limit,
            bytes_truncated: self.bytes_truncated,
            items_truncated: self.items_truncated,
        }
    }

    fn header(&mut self, domain: &str, schema_revision: u32, algorithm: &str) {
        self.push_str("snapshot(domain=");
        self.push_str(domain);
        self.push_str(",schema=");
        self.push_u64(u64::from(schema_revision));
        self.push_str(",algorithm=");
        self.push_str(algorithm);
        self.push_str(")\n");
    }

    fn begin_roots(&mut self, total: usize) {
        self.push_str("roots(present,total=");
        self.push_usize(total);
        self.push_str(")=[");
    }

    fn begin_nodes(&mut self, total: usize) {
        self.push_str("nodes(present,total=");
        self.push_usize(total);
        self.push_str(")\n");
    }

    fn begin_node(&mut self, index: usize) -> Result<bool, NodeSceneSnapshotError> {
        if index >= self.item_limit {
            self.omit_item()?;
            return Ok(false);
        }
        self.push_str("node[");
        self.push_usize(index);
        self.push_str("] transform=");
        Ok(true)
    }

    fn begin_transform(&mut self, render: bool, kind: &str, total: usize) {
        if !render {
            return;
        }
        self.push_str(kind);
        self.push_str("(total=");
        self.push_usize(total);
        self.push_str(")=[");
    }

    fn begin_children(&mut self, render: bool) {
        if render {
            self.push_str(" children(present)=[");
        }
    }

    fn reference(
        &mut self,
        ordinal: usize,
        reference: usize,
    ) -> Result<(), NodeSceneSnapshotError> {
        if ordinal >= self.item_limit {
            return self.omit_item();
        }
        self.delimit(ordinal);
        self.push_usize(reference);
        Ok(())
    }

    fn child_reference(
        &mut self,
        render_node: bool,
        ordinal: usize,
        reference: usize,
    ) -> Result<(), NodeSceneSnapshotError> {
        if !render_node {
            return Ok(());
        }
        self.reference(ordinal, reference)
    }

    fn transform_bits(
        &mut self,
        render_node: bool,
        ordinal: usize,
        bits: u32,
    ) -> Result<(), NodeSceneSnapshotError> {
        if !render_node {
            return Ok(());
        }
        if ordinal >= self.item_limit {
            return self.omit_item();
        }
        self.delimit(ordinal);
        self.push_hex_u32(bits);
        Ok(())
    }

    fn end_transform(&mut self, render: bool, _total: usize) {
        if render {
            self.push_str("]");
        }
    }

    fn end_collection(&mut self) {
        self.push_str("]\n");
    }

    fn end_node(&mut self, render: bool, child_total: usize) {
        if !render {
            return;
        }
        self.push_str("];total=");
        self.push_usize(child_total);
        self.push_str("\n");
    }

    fn summary(&mut self, nodes: usize, roots: usize, edges: usize) {
        self.push_str("summary(nodes=");
        self.push_usize(nodes);
        self.push_str(",roots=");
        self.push_usize(roots);
        self.push_str(",edges=");
        self.push_usize(edges);
        self.push_str(")\n");
    }

    fn omit_item(&mut self) -> Result<(), NodeSceneSnapshotError> {
        self.items_truncated = true;
        self.items_omitted_by_limit = self
            .items_omitted_by_limit
            .checked_add(1)
            .ok_or(NodeSceneSnapshotError::PreviewItemCountOverflow)?;
        Ok(())
    }

    fn delimit(&mut self, ordinal: usize) {
        if ordinal != 0 {
            self.push_str(",");
        }
    }

    fn push_usize(&mut self, value: usize) {
        self.push_u64(u64::try_from(value).unwrap_or(u64::MAX));
    }

    fn push_u64(&mut self, mut value: u64) {
        let mut digits = [0_u8; 20];
        let mut cursor = digits.len();
        loop {
            cursor -= 1;
            digits[cursor] = b'0' + u8::try_from(value % 10).unwrap_or(0);
            value /= 10;
            if value == 0 {
                break;
            }
        }
        let text = std::str::from_utf8(&digits[cursor..]).unwrap_or("");
        self.push_str(text);
    }

    fn push_hex_u32(&mut self, value: u32) {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut bytes = *b"0x00000000";
        for index in 0..8 {
            let shift = 28 - index * 4;
            bytes[index + 2] = HEX[((value >> shift) & 0x0f) as usize];
        }
        let text = std::str::from_utf8(&bytes).unwrap_or("");
        self.push_str(text);
    }

    fn push_str(&mut self, value: &str) {
        if self.bytes_truncated {
            return;
        }
        let remaining = self.byte_limit.saturating_sub(self.text.len());
        if value.len() > remaining {
            self.bytes_truncated = true;
            return;
        }
        self.text.push_str(value);
    }
}

#[cfg(test)]
mod tests;
