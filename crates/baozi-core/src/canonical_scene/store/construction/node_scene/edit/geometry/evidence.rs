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
    GeometryReplacement, SceneGeometryCornerOriginsTag, SceneGeometryEditEvidenceTag,
    SceneGeometryEditOutput, SceneGeometryEditPostflight, SceneGeometryFaceOffsetsTag,
    SceneNormalReplacement, SceneTriangulationReplacement, schema,
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

    pub(super) fn normals(
        input: &SceneStorage,
        replacement: SceneNormalReplacement<'_>,
        geometry: &crate::canonical_scene::store::typed_storage::edit::normals::GeometryNormalsPlan<
            '_,
        >,
        counts: crate::canonical_scene::store::construction::node_scene::SceneEvidenceCounts,
    ) -> Option<Self> {
        let source = &input.read_geometry().primitives[geometry.target_row()];
        let mapping = geometry.attribute_mapping();
        Some(Self {
            words: [
                schema::OPERATION_NORMALS,
                replacement.target.logical_mesh().index(),
                replacement.target.local(),
                geometry.target_row() as u32,
                input
                    .proof
                    .counts()
                    .profile_metadata_entry_count
                    .checked_sub(counts.profile_metadata_entry_count)?,
                source.corner_count,
                source.face_count,
                0,
                0,
                mapping.normal_attribute(),
                (u32::from(mapping.source_normal().is_some())
                    * schema::normal_field::REPLACED_NORMAL)
                    | (u32::from(
                        mapping
                            .source_tangent()
                            .is_some_and(|index| mapping.output_attribute(index).is_none()),
                    ) * schema::normal_field::REMOVED_TANGENT)
                    | (u32::from(replacement.smooth) * schema::normal_field::SMOOTH),
                geometry.output_attribute_count(),
                match replacement.options_digest.algorithm() {
                    DigestAlgorithm::Sha256 => 0,
                    DigestAlgorithm::Blake3 => 1,
                },
            ],
            digest: digest_words(replacement.options_digest),
        })
    }

    pub(super) fn write_prefix(
        self,
        row: &mut OutcomeEvidenceRecordRowWriter<'_>,
    ) -> Result<(), StoreBuildError> {
        row.write_scalar(schema::REVISION)?;
        row.write_scalar(schema::OUTCOME_CHANGED)?;
        row.write_scalar(schema::OPERATION_SEMANTIC_VERSION)?;
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

/// Concrete evidence recipe; only triangulation carries the borrowed origin relation.
#[derive(Clone, Copy)]
pub(super) struct SceneGeometryEditEvidenceRecipe<'a> {
    pub(super) facts: GeometryEditFacts,
    pub(super) counts: NodeSceneEvidenceRecipe,
    pub(super) replacement: GeometryReplacement<'a>,
    root: RecordPlan,
}

impl<'a> SceneGeometryEditPostflight<'a> {
    /// Returns only core-derived counts and edit facts from the actual candidate output.
    pub(super) fn candidate_evidence(
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
    const fn root_record_plan(&self) -> RecordPlan {
        self.root
    }

    pub(super) const fn scene_counts(&self) -> &NodeSceneEvidenceRecipe {
        &self.counts
    }

    fn corner_origins_plan(&self) -> Result<SequencePlan<u32>, StorePlanError> {
        SequencePlan::new(self.facts.words[7] as usize)
    }

    fn face_offsets_plan(&self) -> Result<SequencePlan<u32>, StorePlanError> {
        SequencePlan::new(self.replacement.face_offsets().len())
    }

    pub(super) const fn source_corner_triangles(&self) -> &'a [[u32; 3]] {
        self.replacement.triangles()
    }

    pub(super) const fn source_face_triangle_offsets(&self) -> &'a [u32] {
        self.replacement.face_offsets()
    }

    /// The origin pair is present only for triangulation. Central validation follows every edge.
    fn write_root_row(
        &self,
        row: &mut OutcomeEvidenceRecordRowWriter<'_>,
        counts: OutcomeEvidenceRecordRef<NodeSceneOutcomeEvidenceSchema>,
        origins: Option<(
            OutcomeEvidenceSequenceRef<u32, SceneGeometryCornerOriginsTag>,
            OutcomeEvidenceSequenceRef<u32, SceneGeometryFaceOffsetsTag>,
        )>,
    ) -> Result<(), StoreBuildError> {
        self.facts.write_prefix(row)?;
        row.write_record_ref(counts)?;
        match origins {
            Some((corners, faces)) => {
                row.write_sequence_ref(corners)?;
                row.write_sequence_ref(faces)
            }
            None => {
                row.write_scalar(0_u8)?;
                row.write_scalar(0_u8)
            }
        }
    }
}

impl SceneGeometryEditPostflight<'_> {
    /// Retains a two-member normal graph or four-member triangulation graph. Any member failure
    /// aborts the whole success graph.
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
        let corners = if matches!(self.replacement, GeometryReplacement::Normals(_)) {
            None
        } else {
            Some(match recipe.corner_origins_plan() {
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
            })
        };
        let faces = if matches!(self.replacement, GeometryReplacement::Normals(_)) {
            None
        } else {
            Some(match recipe.face_offsets_plan() {
                Ok(plan) => {
                    match lane.try_retain_sequence(plan, SceneGeometryFaceOffsetsTag, |writer| {
                        writer.extend_from_slice(recipe.source_face_triangle_offsets())
                    }) {
                        Ok(reference) => reference,
                        Err(_) => return lane.abort(),
                    }
                }
                Err(_) => return lane.abort(),
            })
        };
        let root = match lane.try_retain_records(
            recipe.root_record_plan(),
            SceneGeometryEditEvidenceTag,
            |writer| writer.write_row(|row| recipe.write_root_row(row, counts, corners.zip(faces))),
        ) {
            Ok(reference) => reference,
            Err(_) => return lane.abort(),
        };
        lane.complete(root)
    }
}
