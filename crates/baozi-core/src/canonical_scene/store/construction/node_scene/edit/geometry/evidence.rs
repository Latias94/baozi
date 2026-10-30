//! Rooted remap evidence. The borrowed recipe never owns or duplicates the origin buffers.

use baozi_operation::__retained_store_integration::activation::{
    CanonicalOutcomeEvidenceLane, OutcomeEvidenceDecision,
};
use baozi_operation::__retained_store_integration::authority::CanonicalInheritanceSource;
use baozi_operation::{
    OperationOwner, OutcomeEvidenceRecordRef, OutcomeEvidenceRecordRowWriter,
    OutcomeEvidenceSequenceRef, ProgressPhase, ProgressUnit, RecordBlock, RecordPlan, SequencePlan,
    StoreBuildError, StorePlanError,
};

use crate::canonical_scene::store::construction::node_scene::SceneStorage;
use crate::canonical_scene::store::construction::node_scene::materialization::NodeSceneEvidenceRecipe;
use crate::canonical_scene::store::schema::NodeSceneOutcomeEvidenceSchema;
use crate::canonical_scene::store::typed_storage::edit::GeometryTriangulationPlan;
use crate::provenance::{DigestAlgorithm, OptionsDigest};

use super::{
    SceneGeometryCornerOriginsTag, SceneGeometryEditEvidenceTag, SceneGeometryEditOutput,
    SceneGeometryEditPostflight, SceneGeometryFaceOffsetsTag, SceneTriangulationReplacement,
    schema,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct GeometryEditFacts {
    pub(super) words: [u32; 13],
    pub(super) digest: [u64; 4],
}

impl GeometryEditFacts {
    pub(super) fn new(
        input: &SceneStorage,
        replacement: SceneTriangulationReplacement<'_>,
        geometry: &GeometryTriangulationPlan<'_>,
    ) -> Self {
        let storage = input.read_geometry();
        let source = &storage.primitives[geometry.target_row()];
        let attribute_count = source
            .attributes
            .checked(storage.attributes.len())
            .map_or(0, |range| range.len());
        let options = replacement.options_digest();
        Self {
            words: [
                schema::OPERATION_TRIANGULATE,
                replacement.target().logical_mesh().index(),
                replacement.target().local(),
                geometry.target_row() as u32,
                source.vertex_count,
                source.corner_count,
                source.face_count,
                geometry.output_corner_count(),
                geometry.output_face_count(),
                attribute_count as u32,
                schema::PRESERVE_VERTEX_IDENTITY
                    | schema::PRESERVE_SOURCE_FIELDS_BY_ORIGIN
                    | schema::PRESERVE_SCALAR_BITS_AND_SPARSE_ENCODING,
                schema::INVALIDATE_PRIMITIVE_MESH_SCENE_BOUNDS,
                match options.algorithm() {
                    DigestAlgorithm::Sha256 => 0,
                    DigestAlgorithm::Blake3 => 1,
                },
            ],
            digest: digest_words(options),
        }
    }

    pub(super) fn write_prefix(
        self,
        row: &mut OutcomeEvidenceRecordRowWriter<'_>,
    ) -> Result<(), StoreBuildError> {
        row.write_scalar(schema::REVISION)?;
        row.write_scalar(schema::OUTCOME_CHANGED)?;
        row.write_scalar(schema::TRIANGULATION_SEMANTIC_VERSION)?;
        for word in self.words {
            row.write_scalar(word)?;
        }
        for word in self.digest {
            row.write_scalar(word)?;
        }
        Ok(())
    }
}

fn digest_words(options: OptionsDigest) -> [u64; 4] {
    let bytes = options.as_bytes();
    std::array::from_fn(|word| {
        let start = word * 8;
        u64::from_le_bytes(std::array::from_fn(|byte| bytes[start + byte]))
    })
}

/// Four-member changed evidence recipe, bound to the original borrowed origin relation.
#[derive(Clone, Copy)]
pub struct SceneGeometryEditEvidenceRecipe<'a> {
    pub(super) facts: GeometryEditFacts,
    pub(super) counts: NodeSceneEvidenceRecipe,
    pub(super) replacement: SceneTriangulationReplacement<'a>,
    root: RecordPlan,
}

impl<'a> SceneGeometryEditPostflight<'a> {
    /// Returns only core-derived counts and edit facts from the actual candidate output.
    pub fn candidate_evidence(
        &self,
        output: &SceneGeometryEditOutput,
    ) -> SceneGeometryEditEvidenceRecipe<'a> {
        SceneGeometryEditEvidenceRecipe {
            facts: output.facts,
            counts: NodeSceneEvidenceRecipe::from_counts(
                self.evidence.scene_counts().record_plan(),
                output.scene.inner().evidence_counts,
            ),
            replacement: self.replacement,
            root: self.evidence.root_record_plan(),
        }
    }
}

impl<'a> SceneGeometryEditEvidenceRecipe<'a> {
    pub const fn root_record_plan(&self) -> RecordPlan {
        self.root
    }

    pub const fn scene_counts(&self) -> &NodeSceneEvidenceRecipe {
        &self.counts
    }

    pub fn corner_origins_plan(&self) -> Result<SequencePlan<u32>, StorePlanError> {
        SequencePlan::new(self.facts.words[7] as usize)
    }

    pub fn face_offsets_plan(&self) -> Result<SequencePlan<u32>, StorePlanError> {
        SequencePlan::new(self.replacement.source_face_triangle_offsets().len())
    }

    pub const fn source_corner_triangles(&self) -> &'a [[u32; 3]] {
        self.replacement.source_corner_triangles()
    }

    pub const fn source_face_triangle_offsets(&self) -> &'a [u32] {
        self.replacement.source_face_triangle_offsets()
    }

    /// Retain the count child and both origin leaves before writing the root with these exact
    /// three references. Central validation follows all edges and compares every origin scalar.
    pub fn write_root_row(
        &self,
        row: &mut OutcomeEvidenceRecordRowWriter<'_>,
        counts: OutcomeEvidenceRecordRef<NodeSceneOutcomeEvidenceSchema>,
        corners: OutcomeEvidenceSequenceRef<u32, SceneGeometryCornerOriginsTag>,
        faces: OutcomeEvidenceSequenceRef<u32, SceneGeometryFaceOffsetsTag>,
    ) -> Result<(), StoreBuildError> {
        self.facts.write_prefix(row)?;
        row.write_record_ref(counts)?;
        row.write_sequence_ref(corners)?;
        row.write_sequence_ref(faces)
    }
}

impl SceneGeometryEditPostflight<'_> {
    /// Retains the exact changed Scene counts and borrowed origin relation under one rooted
    /// four-member outcome-evidence graph. Any member failure aborts the whole success graph.
    pub fn build_evidence<O, PP, U>(
        &self,
        lane: &mut CanonicalOutcomeEvidenceLane<'_, O, PP, U, CanonicalInheritanceSource>,
        output: &SceneGeometryEditOutput,
    ) -> OutcomeEvidenceDecision<RecordBlock, SceneGeometryEditEvidenceTag>
    where
        O: OperationOwner,
        PP: ProgressPhase,
        U: ProgressUnit,
    {
        let recipe = self.candidate_evidence(output);
        let counts = match lane.try_retain_records(
            recipe.scene_counts().record_plan(),
            crate::canonical_scene::store::schema::NodeSceneOutcomeEvidenceSchema,
            |writer| writer.write_row(|row| recipe.scene_counts().write_row(row)),
        ) {
            Ok(reference) => reference,
            Err(_) => return lane.abort(),
        };
        let corners = match recipe.corner_origins_plan() {
            Ok(plan) => {
                match lane.try_retain_sequence(plan, SceneGeometryCornerOriginsTag, |writer| {
                    for triangle in recipe.source_corner_triangles() {
                        for &corner in triangle {
                            writer.push(corner)?;
                        }
                    }
                    Ok(())
                }) {
                    Ok(reference) => reference,
                    Err(_) => return lane.abort(),
                }
            }
            Err(_) => return lane.abort(),
        };
        let faces = match recipe.face_offsets_plan() {
            Ok(plan) => {
                match lane.try_retain_sequence(plan, SceneGeometryFaceOffsetsTag, |writer| {
                    writer.extend_from_slice(recipe.source_face_triangle_offsets())
                }) {
                    Ok(reference) => reference,
                    Err(_) => return lane.abort(),
                }
            }
            Err(_) => return lane.abort(),
        };
        let root = match lane.try_retain_records(
            recipe.root_record_plan(),
            SceneGeometryEditEvidenceTag,
            |writer| writer.write_row(|row| recipe.write_root_row(row, counts, corners, faces)),
        ) {
            Ok(reference) => reference,
            Err(_) => return lane.abort(),
        };
        lane.complete(root)
    }
}
