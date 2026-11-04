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
        TypedStorageBuildControl::run_unit(self, action)
    }
}

impl crate::canonical_scene::store::typed_storage::validation::TypedStorageValidationControl
    for Control
{
    type Error = StoreRuntimeError;

    fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<Option<R>, Self::Error> {
        TypedStorageBuildControl::run_unit(self, action).map(Some)
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

fn precision_source(
    account: &baozi_operation::PhysicalStorageAccount,
    wide: Option<crate::attribute::AttributeSemanticKind>,
    sparse: bool,
) -> crate::canonical_scene::store::construction::node_scene::NodeSceneSuccessParts {
    use crate::attribute::AttributeSemantic;
    use crate::canonical_scene::store::construction::node_scene::NodeSceneTestTransaction;
    use crate::canonical_scene::store::construction::node_scene::tests::{
        profile_fixture, single_trs_batch, successful_parts, test_limits,
    };
    use crate::canonical_scene::store::production::*;

    let positions = [0.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let normals = [0.0_f32, 0.0, 1.0].repeat(3);
    let tangents = [1.0_f32, 0.0, 0.0, 1.0].repeat(3);
    let uvs = [0.0_f32; 6];
    let positions64 = positions.map(f64::from);
    let normals64 = [0.0_f64, 0.0, 1.0].repeat(3);
    let tangents64 = [1.0_f64, 0.0, 0.0, 1.0].repeat(3);
    let uvs64 = [0.0_f64; 6];
    let mut attributes = Vec::new();
    for (semantic, lanes, values32, values64) in [
        (
            AttributeSemantic::Position,
            3,
            positions.as_slice(),
            positions64.as_slice(),
        ),
        (
            AttributeSemantic::Normal,
            3,
            normals.as_slice(),
            normals64.as_slice(),
        ),
        (
            AttributeSemantic::Tangent,
            4,
            tangents.as_slice(),
            tangents64.as_slice(),
        ),
        (
            AttributeSemantic::Texcoord(7),
            2,
            uvs.as_slice(),
            uvs64.as_slice(),
        ),
    ] {
        attributes.push(if wide == Some(semantic.kind()) {
            if sparse {
                GeometryAttributeAuthoring::sparse_f64(
                    semantic,
                    AttributeDomain::Vertex,
                    lanes,
                    3,
                    &[0, 1, 2],
                    values64,
                    &values64[..usize::from(lanes)],
                )
            } else {
                GeometryAttributeAuthoring::f64(semantic, AttributeDomain::Vertex, lanes, values64)
            }
        } else {
            GeometryAttributeAuthoring::f32(
                semantic,
                AttributeDomain::Vertex,
                lanes,
                values32,
                Presence::Absent,
            )
        });
    }
    // An unrelated wider UV set must not force the selected dependencies to widen.
    attributes.push(GeometryAttributeAuthoring::f64(
        AttributeSemantic::Texcoord(8),
        AttributeDomain::Vertex,
        2,
        &uvs64,
    ));
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::TriangleList,
        Presence::Absent,
        crate::MaterialAssignment::Unspecified,
        AuthoringRange::new(0, attributes.len() as u32),
    )];
    let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let geometry = GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &[]);
    let logical_morphs = [LogicalMorphAuthoring::new(AuthoringRange::new(0, 0), &[])];
    let deformations = [PrimitiveDeformationAuthoring::new(Presence::Absent, &[])];
    let deformation = DeformationAuthoringBatch::new(&[], &[], &logical_morphs, &deformations, &[]);
    let spatial = SpatialAuthoringBatch::new(&crate::SceneSpace::STANDARD_TARGET, &[], &[]);
    let appearance = AppearanceAuthoringBatch::empty();
    let animation = AnimationAuthoringBatch::empty();
    let profile = profile_fixture(
        spatial,
        single_trs_batch(),
        appearance,
        geometry,
        deformation,
        animation,
    );
    let input = SceneAuthoringBatch::new(
        spatial,
        single_trs_batch(),
        appearance,
        geometry,
        deformation,
        animation,
        profile.batch(),
    );
    let mut invocation_root_1 =
        crate::canonical_scene::store::construction::SceneCanonicalInvocationHost::new();
    let draft = NodeSceneTestTransaction::with_account(
        &mut invocation_root_1,
        account,
        test_limits(),
        input,
        SceneValidationLimits::default(),
    )
    .unwrap()
    .finish_draft()
    .unwrap();
    successful_parts(
        draft
            .publish_with_generation_source_for_test(|| {
                SceneGenerationId::reserve_from(&std::sync::atomic::AtomicU64::new(1))
            })
            .unwrap(),
    )
}

#[test]
fn actual_candidate_width_rejects_each_f64_dependency_without_changing_the_source() {
    use crate::attribute::AttributeSemanticKind as K;
    use crate::canonical_scene::store::construction::node_scene::tests::{
        test_account, test_factory,
    };

    let normals32 = [[0.0_f32, 0.0, 1.0]; 3];
    let normals64 = [[0.0_f64, 0.0, 1.0]; 3];
    let tangents32 = [[1.0_f32, 0.0, 0.0, 1.0]; 3];
    let tangents64 = [[1.0_f64, 0.0, 0.0, 1.0]; 3];
    let conventions = TangentConventions {
        basis: crate::TangentBasisConvention::TriangleUvDerivative,
        encoding: crate::TangentHandednessEncoding::SignInW,
    };
    for sparse in [false, true] {
        for (tangent, wide) in [
            (false, K::Position),
            (false, K::Normal),
            (true, K::Position),
            (true, K::Normal),
            (true, K::Texcoord),
            (true, K::Tangent),
        ] {
            let factory = test_factory();
            let account = test_account(&factory, 512);
            let source = precision_source(&account, Some(wide), sparse);
            let scene = ValidatedScene::from_node_scene(source.scene);
            let target = scene
                .meshes()
                .next()
                .unwrap()
                .primitives()
                .next()
                .unwrap()
                .id();
            let generation = scene.generation();
            let geometry = scene.as_node_scene().read_geometry();
            let bits: Vec<_> = geometry
                .f64_values
                .iter()
                .map(|value| value.to_bits())
                .collect();
            for is_f64 in [false, true] {
                let request = if tangent {
                    EditRequest::Tangents(SceneTangentReplacement::new(
                        target,
                        if is_f64 {
                            VectorValues::F64(&tangents64)
                        } else {
                            VectorValues::F32(&tangents32)
                        },
                        true,
                        AttributeDomain::Vertex,
                        7,
                        AttributeDomain::Vertex,
                        &conventions,
                        OptionsDigest::sha256([0; 32]),
                    ))
                } else {
                    EditRequest::Normals(SceneNormalReplacement::new(
                        target,
                        if is_f64 {
                            VectorValues::F64(&normals64)
                        } else {
                            VectorValues::F32(&normals32)
                        },
                        true,
                        true,
                        false,
                        OptionsDigest::sha256([0; 32]),
                    ))
                };
                let result = check_derivation_width(geometry, 0, request, &mut Control::default());
                if is_f64 {
                    assert!(
                        result.is_ok(),
                        "the F64 candidate retains {wide:?} precision"
                    );
                    let mut control = Control::default();
                    match request {
                        EditRequest::Normals(replacement) => {
                            let plan = GeometryNormalsPlan::try_new(
                                geometry,
                                0,
                                replacement.normals,
                                NormalWritePolicy::Replace,
                                TangentPolicy::Remove,
                                &mut control,
                            )
                            .unwrap();
                            let output = plan.materialize(&mut control).unwrap();
                            assert!(plan.validate_correspondence(&output, &mut control).unwrap());
                        }
                        EditRequest::Tangents(replacement) => {
                            let plan = GeometryTangentsPlan::try_new(
                                geometry,
                                0,
                                replacement.values,
                                replacement.had_tangent,
                                replacement.normal_domain,
                                replacement.uv_set,
                                replacement.uv_domain,
                                replacement.conventions,
                                &mut control,
                            )
                            .unwrap();
                            let output = plan.materialize(&mut control).unwrap();
                            assert!(plan.validate_correspondence(&output, &mut control).unwrap());
                        }
                        _ => unreachable!(),
                    }
                } else {
                    assert!(matches!(result,
                        Err(SceneEditRejection::PrecisionLoss { semantic }) if semantic == wide
                    ));
                }
                assert_eq!(scene.generation(), generation);
                assert_eq!(
                    scene.primitive(target).unwrap().topology(),
                    TopologyKind::TriangleList
                );
                assert_eq!(
                    geometry
                        .f64_values
                        .iter()
                        .map(|value| value.to_bits())
                        .collect::<Vec<_>>(),
                    bits
                );
            }
            drop(scene);
            drop(source.report);
            assert_eq!(account.snapshot().live_retained_bytes(), 0);
        }
    }
}

#[test]
fn unrelated_f64_uv_set_does_not_reject_f32_normal_or_tangent_candidates() {
    use crate::canonical_scene::store::construction::node_scene::tests::{
        test_account, test_factory,
    };

    let factory = test_factory();
    let account = test_account(&factory, 512);
    let source = precision_source(&account, None, false);
    let scene = ValidatedScene::from_node_scene(source.scene);
    let target = scene
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .next()
        .unwrap()
        .id();
    let normals = [[0.0_f32, 0.0, 1.0]; 3];
    let tangents = [[1.0_f32, 0.0, 0.0, 1.0]; 3];
    let conventions = TangentConventions {
        basis: crate::TangentBasisConvention::TriangleUvDerivative,
        encoding: crate::TangentHandednessEncoding::SignInW,
    };
    for request in [
        EditRequest::Normals(SceneNormalReplacement::new(
            target,
            VectorValues::F32(&normals),
            true,
            true,
            false,
            OptionsDigest::sha256([0; 32]),
        )),
        EditRequest::Tangents(SceneTangentReplacement::new(
            target,
            VectorValues::F32(&tangents),
            true,
            AttributeDomain::Vertex,
            7,
            AttributeDomain::Vertex,
            &conventions,
            OptionsDigest::sha256([0; 32]),
        )),
    ] {
        assert!(
            check_derivation_width(
                scene.as_node_scene().read_geometry(),
                0,
                request,
                &mut Control::default(),
            )
            .is_ok()
        );
    }
    drop(scene);
    drop(source.report);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}
