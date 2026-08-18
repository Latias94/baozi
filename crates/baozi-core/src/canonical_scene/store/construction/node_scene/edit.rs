use baozi_operation::{CanonicalRecordView, RecordBlock, StoreRuntimeError};

use crate::canonical_scene::store::schema::{CanonicalBlockRef, CanonicalSchema};

use super::ValidatedNodeScene;

mod execution;
mod graph;
mod leaf_plan;
mod plan;
mod publication;
mod report;
mod source;

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(in crate::canonical_scene::store) use publication::NodeSceneEditTerminal;
#[cfg(test)]
pub(in crate::canonical_scene::store) use source::{
    NodeTransformEditStart, try_begin_node_transform_edit,
};

fn required_record_view<'scene, Schema>(
    input: &'scene ValidatedNodeScene,
    source: CanonicalBlockRef<Schema>,
) -> Result<CanonicalRecordView<'scene>, StoreRuntimeError>
where
    Schema: CanonicalSchema<StoreKind = RecordBlock>,
{
    input
        .canonical
        .records(source.into_store())
        .ok_or(StoreRuntimeError::InvalidSourceSlot)
}
