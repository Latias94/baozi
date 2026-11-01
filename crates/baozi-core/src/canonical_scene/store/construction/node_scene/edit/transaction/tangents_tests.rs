use super::*;
use crate::canonical_scene::store::typed_storage::TableRange;
use crate::canonical_scene::store::typed_storage::deformation::{
    DeformationStorage, StoredMorphDelta, StoredMorphDeltaPayload, StoredMorphTarget,
    StoredMorphTargetIndex, StoredPrimitiveDeformation,
};
use crate::deform::MorphDeltaSemantic;

#[derive(Default)]
struct Control {
    completed: u64,
    reject: bool,
}

impl TypedStorageBuildControl for Control {
    type Error = StoreRuntimeError;

    fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, Self::Error> {
        if self.reject {
            return Err(StoreRuntimeError::ConstructionInvariantUnverified);
        }
        self.completed += 1;
        Ok(action())
    }

    fn run_string_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, Self::Error> {
        self.run_unit(action)
    }
}

fn source() -> DeformationStorage {
    DeformationStorage {
        skins: Vec::new(),
        logical_morphs: Vec::new(),
        morph_definitions: Vec::new(),
        primitives: vec![
            StoredPrimitiveDeformation {
                influences: None,
                morph_targets: TableRange::new(0, 0),
            },
            StoredPrimitiveDeformation {
                influences: None,
                morph_targets: TableRange::new(0, 1),
            },
        ],
        morph_targets: vec![StoredMorphTarget {
            primitive: 1,
            target_ordinal: 0,
            deltas: TableRange::new(0, 1),
        }],
        morph_deltas: vec![StoredMorphDelta {
            target: StoredMorphTargetIndex(0),
            semantic: MorphDeltaSemantic::Position,
            payload: StoredMorphDeltaPayload::Sparse {
                logical_count: 3,
                indices: TableRange::new(0, 0),
                values: TableRange::new(0, 0),
            },
        }],
        mesh_instances: Vec::new(),
        joints: Vec::new(),
        inverse_bind_matrices: Vec::new(),
        names: Vec::new(),
        influence_vertex_offsets: Vec::new(),
        influence_joints: Vec::new(),
        influence_weights: Vec::new(),
        influence_source_orders: Vec::new(),
        morph_weights: Vec::new(),
        morph_values: Vec::new(),
        morph_sparse_indices: Vec::new(),
    }
}

#[test]
fn tangent_preflight_rejects_every_geometric_morph_delta_only_on_the_selected_target() {
    let mut source = source();
    for semantic in [
        MorphDeltaSemantic::Position,
        MorphDeltaSemantic::Normal,
        MorphDeltaSemantic::Tangent,
    ] {
        source.morph_deltas[0].semantic = semantic;
        assert!(check_morph_dependencies(&source, 0, |_| true, &mut Control::default()).is_ok());
        assert!(matches!(
            check_morph_dependencies(&source, 1, |_| true, &mut Control::default()),
            Err(SceneEditRejection::MorphDependency { semantic: actual }) if actual == semantic
        ));
        assert_eq!(source.morph_deltas.len(), 1);
        assert!(source.morph_values.is_empty());
    }
    source.morph_targets[0].deltas = TableRange::new(0, 0);
    let mut control = Control::default();
    assert!(check_morph_dependencies(&source, 1, |_| true, &mut control).is_ok());
    assert_eq!(control.completed, 2);
    let mut control = Control {
        reject: true,
        ..Control::default()
    };
    assert!(matches!(
        check_morph_dependencies(&source, 1, |_| true, &mut control),
        Err(SceneEditRejection::Runtime(_))
    ));
    assert_eq!(control.completed, 0);
}
