use baozi_operation::{PhysicalStorageAccount, StoreLedgerEvidence};

use crate::canonical_scene::store::schema::{
    NODE_SCENE_MATRIX_SCALAR_COUNT, NODE_SCENE_TRANSFORM_MATRIX, NODE_SCENE_TRANSFORM_TRS,
    NODE_SCENE_TRS_SCALAR_COUNT, node_scene_child_reference_field, node_scene_directory_field,
    node_scene_node_field,
};
use crate::entity::AuthoredTransform;
use crate::identity::SceneGenerationId;
use crate::math::{Mat4, Quat, Vec3};

use super::{NodeSceneRichFailure, NodeSceneSuccessReport, ValidatedNodeScene};

impl ValidatedNodeScene {
    pub(in crate::canonical_scene::store) const fn generation(&self) -> SceneGenerationId {
        self.generation
    }

    pub(in crate::canonical_scene::store) fn account(&self) -> &PhysicalStorageAccount {
        self.canonical.account()
    }

    pub(in crate::canonical_scene::store) fn source_node_count(&self) -> Option<usize> {
        let directory = self.canonical.records(self.refs.directory.into_store())?;
        let canonical_count = directory.scalar::<u32>(0, node_scene_directory_field::NODE_COUNT)?;
        usize::try_from(canonical_count).ok()?.checked_sub(1)
    }

    pub(in crate::canonical_scene::store) fn source_root_count(&self) -> Option<usize> {
        let directory = self.canonical.records(self.refs.directory.into_store())?;
        usize::try_from(directory.scalar::<u32>(0, node_scene_directory_field::SOURCE_ROOT_COUNT)?)
            .ok()
    }

    pub(in crate::canonical_scene::store) fn canonical_block_count(&self) -> usize {
        self.canonical.len()
    }

    /// Returns one ordered child source index in canonical authored order.
    ///
    /// `parent_source_index == None` selects the synthetic root. A present parent
    /// index addresses one authored node and excludes the synthetic root.
    pub(in crate::canonical_scene::store) fn child(
        &self,
        parent_source_index: Option<usize>,
        child_ordinal: usize,
    ) -> Option<usize> {
        let parent_row = match parent_source_index {
            None => 0,
            Some(source_index) => source_index.checked_add(1)?,
        };
        let nodes = self.canonical.records(self.refs.nodes.into_store())?;
        if parent_row >= nodes.rows() {
            return None;
        }
        let reference_offset = usize::try_from(
            nodes.scalar::<u32>(parent_row, node_scene_node_field::CHILD_REFERENCE_OFFSET)?,
        )
        .ok()?;
        let child_count = usize::try_from(
            nodes.scalar::<u32>(parent_row, node_scene_node_field::CHILD_REFERENCE_COUNT)?,
        )
        .ok()?;
        if child_ordinal >= child_count {
            return None;
        }
        let reference_row = reference_offset.checked_add(child_ordinal)?;
        let references = self.canonical.records(self.refs.references?.into_store())?;
        usize::try_from(
            references.scalar::<u32>(reference_row, node_scene_child_reference_field::CHILD)?,
        )
        .ok()?
        .checked_sub(1)
    }

    /// Derives the parent source index from canonical ordered-child ranges.
    ///
    /// The outer option rejects an out-of-range source index. The inner option is
    /// `None` for a source root and excludes the synthetic canonical root.
    pub(in crate::canonical_scene::store) fn parent(
        &self,
        source_index: usize,
    ) -> Option<Option<usize>> {
        let row = source_index.checked_add(1)?;
        let nodes = self.canonical.records(self.refs.nodes.into_store())?;
        if row >= nodes.rows() {
            return None;
        }
        let references = self.canonical.records(self.refs.references?.into_store())?;
        if references.scalar::<u32>(source_index, node_scene_child_reference_field::CHILD)?
            != u32::try_from(row).ok()?
        {
            return None;
        }
        let mut lower = 0_usize;
        let mut upper = nodes.rows();
        while lower < upper {
            let parent_row = lower.checked_add((upper - lower) / 2)?;
            let reference_offset = usize::try_from(
                nodes.scalar::<u32>(parent_row, node_scene_node_field::CHILD_REFERENCE_OFFSET)?,
            )
            .ok()?;
            let child_count = usize::try_from(
                nodes.scalar::<u32>(parent_row, node_scene_node_field::CHILD_REFERENCE_COUNT)?,
            )
            .ok()?;
            if reference_offset.checked_add(child_count)? <= source_index {
                lower = parent_row.checked_add(1)?;
            } else {
                upper = parent_row;
            }
        }

        let parent_row = lower;
        let reference_offset = usize::try_from(
            nodes.scalar::<u32>(parent_row, node_scene_node_field::CHILD_REFERENCE_OFFSET)?,
        )
        .ok()?;
        let child_count = usize::try_from(
            nodes.scalar::<u32>(parent_row, node_scene_node_field::CHILD_REFERENCE_COUNT)?,
        )
        .ok()?;
        let reference_end = reference_offset.checked_add(child_count)?;
        (reference_offset..reference_end)
            .contains(&source_index)
            .then_some(if parent_row == 0 {
                None
            } else {
                Some(parent_row - 1)
            })
    }

    pub(in crate::canonical_scene::store) fn authored_transform(
        &self,
        source_index: usize,
    ) -> Option<AuthoredTransform> {
        let row = source_index.checked_add(1)?;
        let nodes = self.canonical.records(self.refs.nodes.into_store())?;
        let kind = nodes.scalar::<u8>(row, node_scene_node_field::TRANSFORM_KIND)?;
        let offset =
            usize::try_from(nodes.scalar::<u32>(row, node_scene_node_field::TRANSFORM_OFFSET)?)
                .ok()?;
        let length =
            usize::try_from(nodes.scalar::<u32>(row, node_scene_node_field::TRANSFORM_LENGTH)?)
                .ok()?;
        let transforms = self
            .canonical
            .sequence(self.refs.transforms?.into_store())?;
        let values = transforms
            .as_slice()
            .get(offset..offset.checked_add(length)?)?;
        match kind {
            NODE_SCENE_TRANSFORM_MATRIX => {
                let values = <&[f32; NODE_SCENE_MATRIX_SCALAR_COUNT]>::try_from(values).ok()?;
                AuthoredTransform::try_matrix(Mat4 {
                    cols: [
                        [values[0], values[1], values[2], values[3]],
                        [values[4], values[5], values[6], values[7]],
                        [values[8], values[9], values[10], values[11]],
                        [values[12], values[13], values[14], values[15]],
                    ],
                })
                .ok()
            }
            NODE_SCENE_TRANSFORM_TRS => {
                let values = <&[f32; NODE_SCENE_TRS_SCALAR_COUNT]>::try_from(values).ok()?;
                AuthoredTransform::try_trs(
                    Vec3::new(values[0], values[1], values[2]),
                    Quat::from_xyzw(values[3], values[4], values[5], values[6]),
                    Vec3::new(values[7], values[8], values[9]),
                )
                .ok()
            }
            _ => None,
        }
    }
}

impl NodeSceneSuccessReport {
    pub(in crate::canonical_scene::store) fn outcome_evidence_block_count(&self) -> usize {
        self.outcome_evidence.len()
    }

    pub(in crate::canonical_scene::store) const fn ledger_evidence(&self) -> &StoreLedgerEvidence {
        &self.ledger_evidence
    }
}

impl NodeSceneRichFailure {
    pub(in crate::canonical_scene::store) fn account(&self) -> &PhysicalStorageAccount {
        self.outcome_evidence.account()
    }

    pub(in crate::canonical_scene::store) const fn ledger_evidence(&self) -> &StoreLedgerEvidence {
        &self.ledger_evidence
    }

    pub(in crate::canonical_scene::store) fn outcome_evidence_block_count(&self) -> usize {
        self.outcome_evidence.outcome_evidence().len()
    }
}
