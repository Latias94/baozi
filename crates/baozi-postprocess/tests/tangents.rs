mod common;

use baozi_core::{
    AttributeData, AttributeDomain, AttributeRef, AttributeSemanticRef, Axis, FormatId,
    FrontFaceWinding, Handedness, MorphDeltaSemantic, PrimitiveId, PrimitiveRef, ProvenanceKindRef,
    ProvenanceOperationKindRef, SceneLookupError, SceneProfileOwner, SceneSnapshot,
    SceneSnapshotLimits, TangentBasisRef, TangentConventionsRef, TangentEncodingRef, TargetSpace,
    ValidatedScene,
};
use baozi_import::authoring::{
    AnimationAuthoringBatch, AppearanceAuthoringBatch, AttributeSemantic, AuthoringRange,
    ContentDigest, CustomAttributeSemantic, DeformationAuthoringBatch, GeometryAttributeAuthoring,
    GeometryAuthoringBatch, GeometryLogicalMeshAuthoring, GeometryPrimitiveAuthoring,
    GeometryTopologyAuthoring, LineageAuthoringBatch, LogicalMorphAuthoring,
    MetadataAuthoringBatch, MorphDeltaAuthoring, MorphDeltaPayloadAuthoring,
    MorphTargetDefinitionAuthoring, MorphTargetIdentity, NodeAuthoringBatch,
    ObservedResourceAuthoring, OwnerProvenanceAuthoring, Presence, PrimitiveDeformationAuthoring,
    PrimitiveMorphTargetAuthoring, ProfileAuthoringBatch, ProfileOwner, ProvenanceLocalId,
    ProvenanceNodeAuthoring, ResourceIdentity, ResourceIdentityLocalId, ResourceSnapshotAuthoring,
    SceneAuthoringBatch, SceneSpace, SessionOpaqueReason, SourceIdentityAuthoring,
    SourceIdentityLocalId, SourceObjectKind, SourceObjectLocator, SpatialAuthoringBatch,
    StaticSceneBuilder, TangentBasisConvention, TangentConventions, TangentHandednessEncoding,
};
use baozi_import::{
    CapabilityStatus, CompiledOptions, DetectionOutcome, DetectionResult, DetectionScope, Format,
    FormatCapability, FormatDescriptor, FormatEncoding, FormatSelection, ImportConfigOverlay,
    ImportOperationControl, ImportReport, ImportRequest, ImportRequestLayer, NoFormatOptions,
    OptionCompileError, OptionCompileLimits, ParseContext, ParseResult, ReadConfidence, ReadHint,
    RegistryBuildLimits, RegistryBuildScope, RegistryBuilder,
};
use baozi_io::{AssetPath, MemoryAssetIo};
use baozi_postprocess::{
    ExistingNormals, ExistingTangents, NormalOptions, PostProcessCancellationToken,
    PostProcessControl, PostProcessControlCause, PostProcessOperation, PostProcessRunDisposition,
    PostProcessRunError, SpaceConversionOptions, TangentCause, TangentIssue, TangentOptions,
    convert_scene_space, generate_normals, generate_tangents,
};
use std::sync::Arc;

const TRIANGLE: &[u8] = b"v 0 0 0\nv 1 0 0\nv 0 1 0\n\
    vt 0 0\nvt 1 0\nvt 0 1\nvn 0 0 1\nf 1/1/1 2/2/1 3/3/1\n";
const MIRRORED_SEAM: &[u8] = b"v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\n\
    vt 0 0\nvt 1 0\nvt 1 1\nvt 0 0\nvt -1 1\nvt 0 1\n\
    vn 0 0 1\ns 7\nf 1/1/1 2/2/1 3/3/1\nf 1/4/1 3/5/1 4/6/1\n";

fn primitive(scene: &ValidatedScene) -> PrimitiveRef<'_> {
    scene.meshes().next().unwrap().primitives().next().unwrap()
}

fn corner_options() -> TangentOptions {
    TangentOptions::new(AttributeDomain::Corner, 0, AttributeDomain::Corner)
}

fn vertex_options() -> TangentOptions {
    TangentOptions::new(AttributeDomain::Vertex, 0, AttributeDomain::Vertex)
}

fn replacement(mut options: TangentOptions) -> TangentOptions {
    options.existing = ExistingTangents::Replace;
    options
}

fn snapshot(scene: &ValidatedScene) -> SceneSnapshot {
    let snapshot = scene
        .capture_snapshot(SceneSnapshotLimits::try_new(128 * 1024, 512, 1_000_000).unwrap())
        .unwrap();
    assert!(!snapshot.preview_bytes_truncated());
    assert!(!snapshot.preview_items_truncated());
    snapshot
}

fn generated_tangents(primitive: PrimitiveRef<'_>) -> &[[f32; 4]] {
    let tangent = primitive
        .attribute(AttributeSemanticRef::Tangent, AttributeDomain::Corner)
        .expect("generated Corner tangents");
    assert_eq!(
        tangent.tangent_conventions(),
        Some(TangentConventionsRef {
            basis: TangentBasisRef::TriangleUvDerivative,
            encoding: TangentEncodingRef::SignInW,
        })
    );
    assert_eq!(tangent.len(), primitive.corner_count() as usize);
    let AttributeData::DenseF32 { lanes: 4, values } = tangent.data() else {
        panic!("generated tangents must be dense F32x4")
    };
    let (values, remainder) = values.as_chunks::<4>();
    assert!(remainder.is_empty());
    values
}

fn assert_tangent(actual: &[f32; 4], expected: [f32; 4]) {
    for (actual, expected) in actual[..3].iter().zip(expected) {
        assert!((actual - expected).abs() < 2.0e-6, "{actual} != {expected}");
    }
    assert_eq!(actual[3], expected[3]);
    assert!((actual[..3].iter().map(|v| v * v).sum::<f32>() - 1.0).abs() < 2.0e-6);
}

#[derive(Debug, PartialEq)]
struct FieldState {
    semantic: String,
    domain: AttributeDomain,
    lanes: u8,
    storage: &'static str,
    bits: Vec<u64>,
}

fn field_state(attribute: AttributeRef<'_>) -> FieldState {
    let (storage, bits) = match attribute.data() {
        AttributeData::DenseF32 { values, .. } => (
            "F32",
            values
                .iter()
                .map(|value| u64::from(value.to_bits()))
                .collect(),
        ),
        AttributeData::DenseF64 { values, .. } => {
            ("F64", values.iter().map(|value| value.to_bits()).collect())
        }
        AttributeData::DenseU32 { values, .. } => (
            "U32",
            values.iter().map(|value| u64::from(*value)).collect(),
        ),
        _ => panic!("retention fixtures use dense floating fields and smoothing groups"),
    };
    FieldState {
        semantic: format!("{:?}", attribute.semantic()),
        domain: attribute.domain(),
        lanes: attribute.lanes(),
        storage,
        bits,
    }
}

fn non_tangent_fields(primitive: PrimitiveRef<'_>) -> Vec<FieldState> {
    primitive
        .attributes()
        .filter(|attribute| attribute.semantic() != AttributeSemanticRef::Tangent)
        .map(field_state)
        .collect()
}

fn assert_rejected(
    scene: ValidatedScene,
    target: PrimitiveId,
    options: TangentOptions,
    control: PostProcessControl,
    expected: impl FnOnce(&TangentCause) -> bool,
) -> ValidatedScene {
    let generation = scene.generation();
    let readable_target = primitive(&scene).id();
    let before = snapshot(&scene);
    let fields = non_tangent_fields(primitive(&scene));
    let failure = generate_tangents(scene, target, options, control)
        .expect_err("unsupported generation must return the immutable input");
    assert_eq!(failure.input().generation(), generation);
    assert!(failure.input().primitive(readable_target).is_ok());
    assert_eq!(non_tangent_fields(primitive(failure.input())), fields);
    let after = snapshot(failure.input());
    assert_eq!(after.digest(), before.digest());
    assert_eq!(after.preview(), before.preview());
    let PostProcessRunError::Tangents { failure: tangent } = failure.error() else {
        panic!("expected a typed tangent error: {:?}", failure.error())
    };
    assert!(
        expected(tangent.cause()),
        "unexpected cause: {:?}",
        tangent.cause()
    );
    let resources = tangent
        .resources()
        .expect("ordinary terminal rejection evidence");
    assert_eq!(resources.current_live_bytes(), 0);
    assert_eq!(resources.active_reservations(), 0);
    assert_eq!(resources.committed_canonical_bytes(), 0);
    let (scene, error) = failure.into_parts();
    drop(error);
    assert_eq!(snapshot(&scene).digest(), before.digest());
    scene
}

#[test]
fn ordinary_and_mirrored_uvs_publish_corner_tangents_and_detached_evidence() {
    for (uv, expected) in [
        ("1 0", [1.0, 0.0, 0.0, 1.0]),
        ("-1 0", [-1.0, 0.0, 0.0, -1.0]),
    ] {
        let source = String::from_utf8(TRIANGLE.to_vec())
            .unwrap()
            .replace("vt 1 0", &format!("vt {uv}"));
        let (scene, import_evidence) = common::import_obj(source.as_bytes()).into_parts();
        let input = primitive(&scene).id();
        let input_count = scene.attribute_count();
        let options = corner_options();
        assert_eq!(options.existing, ExistingTangents::Preserve);
        let (scene, report) =
            generate_tangents(scene, input, options, PostProcessControl::default())
                .unwrap()
                .into_parts();
        assert_eq!(report.disposition(), PostProcessRunDisposition::Changed);
        assert_eq!(
            report.operation(),
            PostProcessOperation::GenerateTangents {
                target: input,
                options
            }
        );
        assert_eq!(report.input_generation(), input.generation());
        assert_eq!(report.output_generation(), scene.generation());
        assert_ne!(scene.generation(), input.generation());
        assert!(report.matches_scene(&scene));
        assert!(!import_evidence.matches_scene(&scene));
        assert_eq!(scene.attribute_count(), input_count + 1);
        let evidence = report.tangent_generation().unwrap();
        assert_eq!(evidence.input_target(), input);
        assert!(!evidence.replaced_authored_tangents());
        let output = scene.primitive(evidence.output_target()).unwrap();
        for tangent in generated_tangents(output) {
            assert_tangent(tangent, expected);
        }
        assert_eq!(
            output
                .attribute(AttributeSemanticRef::Tangent, AttributeDomain::Corner)
                .unwrap()
                .metadata_owner()
                .descriptor(),
            SceneProfileOwner::Attribute(evidence.output_tangent_attribute_index())
        );
        assert_eq!(report.resources().current_live_bytes(), 0);
        assert_eq!(report.resources().active_reservations(), 0);
        assert!(report.resources().committed_canonical_bytes() > 0);
        let output = evidence.output_target();
        drop(scene);
        drop(import_evidence);
        assert_eq!(report.tangent_generation().unwrap().input_target(), input);
        assert_eq!(report.tangent_generation().unwrap().output_target(), output);
    }
}

#[test]
fn changed_tangent_generation_fits_a_one_mib_caller_stack() {
    let scene = common::import_obj(MIRRORED_SEAM).into_parts().0;
    let target = primitive(&scene).id();
    let success = std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(move || {
            generate_tangents(
                scene,
                target,
                corner_options(),
                PostProcessControl::default(),
            )
            .expect("changed tangent publication must fit a one MiB caller stack")
        })
        .unwrap()
        .join()
        .expect("the tangent operation thread must not panic");
    let (scene, report) = success.into_parts();
    assert_eq!(report.disposition(), PostProcessRunDisposition::Changed);
    assert_ne!(scene.generation(), target.generation());
    let evidence = report.tangent_generation().unwrap();
    assert_eq!(evidence.input_target(), target);
    let output = scene.primitive(evidence.output_target()).unwrap();
    assert_eq!(generated_tangents(output).len(), 6);
    assert_eq!(report.resources().current_live_bytes(), 0);
    assert_eq!(report.resources().active_reservations(), 0);
}

#[test]
fn mirrored_uv_seams_keep_shared_vertices_and_independent_corner_frames() {
    let scene = common::import_obj(MIRRORED_SEAM).into_parts().0;
    let target = primitive(&scene).id();
    assert_eq!(primitive(&scene).vertex_count(), 4);
    assert_eq!(
        primitive(&scene).indices(),
        Some([0, 1, 2, 0, 2, 3].as_slice())
    );
    let (scene, _) = generate_tangents(
        scene,
        target,
        corner_options(),
        PostProcessControl::default(),
    )
    .unwrap()
    .into_parts();
    let output = primitive(&scene);
    assert_eq!(output.vertex_count(), 4);
    assert_eq!(output.indices(), Some([0, 1, 2, 0, 2, 3].as_slice()));
    let tangents = generated_tangents(output);
    assert_eq!(tangents.len(), 6);
    for tangent in &tangents[..3] {
        assert_tangent(tangent, [1.0, 0.0, 0.0, 1.0]);
    }
    for tangent in &tangents[3..] {
        assert_tangent(tangent, [-1.0, 0.0, 0.0, -1.0]);
    }
}

#[test]
fn opposite_and_tilted_authored_normals_control_the_frame() {
    // The tilted normal (3/5, 0, 4/5) has the perpendicular tangent (4/5, 0, -3/5).
    for (normal, expected) in [
        ("0 0 -1", [1.0, 0.0, 0.0, -1.0]),
        ("0.6 0 0.8", [0.8, 0.0, -0.6, 1.0]),
        ("0 0 5", [1.0, 0.0, 0.0, 1.0]),
    ] {
        let source = String::from_utf8(TRIANGLE.to_vec())
            .unwrap()
            .replace("vn 0 0 1", &format!("vn {normal}"));
        let scene = common::import_obj(source.as_bytes()).into_parts().0;
        let target = primitive(&scene).id();
        let before = non_tangent_fields(primitive(&scene));
        let (scene, _) = generate_tangents(
            scene,
            target,
            corner_options(),
            PostProcessControl::default(),
        )
        .unwrap()
        .into_parts();
        assert_eq!(non_tangent_fields(primitive(&scene)), before);
        for tangent in generated_tangents(primitive(&scene)) {
            assert_tangent(tangent, expected);
        }
    }
}

#[test]
fn finite_extreme_position_and_uv_scales_keep_the_same_unit_frame() {
    for (position, uv) in [("1e30", "1e-30"), ("1e-30", "1e30")] {
        let source = format!(
            "v 0 0 0\nv {position} 0 0\nv 0 {position} 0\nvt 0 0\nvt {uv} 0\nvt 0 {uv}\nvn 0 0 1\nf 1/1/1 2/2/1 3/3/1\n"
        );
        let scene = common::import_obj(source.as_bytes()).into_parts().0;
        let target = primitive(&scene).id();
        let (scene, _) = generate_tangents(
            scene,
            target,
            corner_options(),
            PostProcessControl::default(),
        )
        .unwrap()
        .into_parts();
        for tangent in generated_tangents(primitive(&scene)) {
            assert_tangent(tangent, [1.0, 0.0, 0.0, 1.0]);
        }
    }
}

#[test]
fn generation_preserves_all_other_fields_and_an_unselected_primitive() {
    let mut source = MIRRORED_SEAM.to_vec();
    source.extend_from_slice(b"o untouched\ns off\nf 1/1/1 2/2/1 3/3/1\n");
    let scene = common::import_obj(&source).into_parts().0;
    assert_eq!(scene.primitive_count(), 2);
    let target = primitive(&scene).id();
    let before = non_tangent_fields(primitive(&scene));
    let untouched = scene
        .meshes()
        .flat_map(|mesh| mesh.primitives())
        .nth(1)
        .unwrap();
    let untouched_fields = non_tangent_fields(untouched);
    let untouched_indices = untouched.indices().map(<[u32]>::to_vec);
    let indices = primitive(&scene).indices().map(<[u32]>::to_vec);
    let space = scene.space();
    let counts = (
        scene.node_count(),
        scene.mesh_count(),
        scene.primitive_count(),
        scene.mesh_instance_count(),
    );
    let (scene, _) = generate_tangents(
        scene,
        target,
        corner_options(),
        PostProcessControl::default(),
    )
    .unwrap()
    .into_parts();
    assert_eq!(non_tangent_fields(primitive(&scene)), before);
    assert_eq!(primitive(&scene).indices(), indices.as_deref());
    assert_eq!(scene.space(), space);
    assert_eq!(
        (
            scene.node_count(),
            scene.mesh_count(),
            scene.primitive_count(),
            scene.mesh_instance_count()
        ),
        counts
    );
    let untouched = scene
        .meshes()
        .flat_map(|mesh| mesh.primitives())
        .nth(1)
        .unwrap();
    assert_eq!(non_tangent_fields(untouched), untouched_fields);
    assert_eq!(untouched.indices(), untouched_indices.as_deref());
    assert!(
        untouched
            .attributes()
            .all(|attribute| attribute.tangent_conventions().is_none())
    );
}

#[test]
fn generated_attribute_provenance_still_reaches_the_imported_obj_source() {
    let scene = common::import_obj(TRIANGLE).into_parts().0;
    let old = primitive(&scene).id();
    let (scene, _) = generate_tangents(scene, old, corner_options(), PostProcessControl::default())
        .unwrap()
        .into_parts();
    assert!(matches!(
        scene.primitive(old),
        Err(SceneLookupError::GenerationMismatch { .. })
    ));
    let tangent = primitive(&scene)
        .attribute(AttributeSemanticRef::Tangent, AttributeDomain::Corner)
        .unwrap();
    let attachments = scene.owner_provenance(tangent.metadata_owner()).unwrap();
    assert!(attachments.len() > 0);
    let mut pending: Vec<_> = attachments.collect();
    let mut visits = 0;
    let mut generated = false;
    let mut source = false;
    while let Some(node) = pending.pop() {
        visits += 1;
        assert!(visits <= 128, "fixture ancestry must remain bounded");
        match node.kind() {
            ProvenanceKindRef::Source(identity) => {
                assert_eq!(identity.generation(), scene.generation());
                if identity.resource().identity() == "postprocess.obj" {
                    source = true;
                }
            }
            ProvenanceKindRef::Operation(operation) => {
                generated |= operation.kind() == ProvenanceOperationKindRef::Generated;
                pending.extend(node.parents());
            }
        }
    }
    assert!(generated);
    assert!(source);
}

#[test]
fn stale_and_foreign_targets_reject_without_changing_a_readable_scene() {
    let scene = common::import_obj(TRIANGLE).into_parts().0;
    let stale = primitive(&scene).id();
    let (scene, _) = generate_tangents(
        scene,
        stale,
        corner_options(),
        PostProcessControl::default(),
    )
    .unwrap()
    .into_parts();
    let scene = assert_rejected(
        scene,
        stale,
        corner_options(),
        PostProcessControl::default(),
        |cause| {
            matches!(
                cause,
                TangentCause::Target {
                    source: SceneLookupError::GenerationMismatch { .. }
                }
            )
        },
    );
    let foreign = common::import_obj(TRIANGLE).into_parts().0;
    drop(assert_rejected(
        scene,
        primitive(&foreign).id(),
        corner_options(),
        PostProcessControl::default(),
        |cause| {
            matches!(
                cause,
                TangentCause::Target {
                    source: SceneLookupError::GenerationMismatch { .. }
                }
            )
        },
    ));
}

#[test]
fn wrong_uv_set_domains_and_missing_fields_have_typed_atomic_rejections() {
    for (options, expected_normal) in [
        (
            TangentOptions::new(AttributeDomain::Vertex, 0, AttributeDomain::Corner),
            true,
        ),
        (
            TangentOptions::new(AttributeDomain::Corner, 0, AttributeDomain::Vertex),
            false,
        ),
        (
            TangentOptions::new(AttributeDomain::Corner, 9, AttributeDomain::Corner),
            false,
        ),
    ] {
        let scene = common::import_obj(TRIANGLE).into_parts().0;
        let target = primitive(&scene).id();
        drop(assert_rejected(
            scene,
            target,
            options,
            PostProcessControl::default(),
            |cause| {
                if expected_normal {
                    matches!(cause, TangentCause::NormalField)
                } else {
                    matches!(cause, TangentCause::UvField)
                }
            },
        ));
    }
    for (source, expected_normal) in [
        (
            "v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nf 1/1 2/2 3/3\n",
            true,
        ),
        (
            "v 0 0 0\nv 1 0 0\nv 0 1 0\nvn 0 0 1\nf 1//1 2//1 3//1\n",
            false,
        ),
    ] {
        let scene = common::import_obj(source.as_bytes()).into_parts().0;
        let target = primitive(&scene).id();
        drop(assert_rejected(
            scene,
            target,
            corner_options(),
            PostProcessControl::default(),
            |cause| {
                if expected_normal {
                    matches!(cause, TangentCause::NormalField)
                } else {
                    matches!(cause, TangentCause::UvField)
                }
            },
        ));
    }
    for options in [
        TangentOptions::new(AttributeDomain::Face, 0, AttributeDomain::Corner),
        TangentOptions::new(AttributeDomain::Corner, 0, AttributeDomain::Face),
        TangentOptions::new(AttributeDomain::Instance, 0, AttributeDomain::Corner),
        TangentOptions::new(AttributeDomain::Corner, 0, AttributeDomain::Instance),
    ] {
        let scene = common::import_obj(TRIANGLE).into_parts().0;
        let target = primitive(&scene).id();
        drop(assert_rejected(
            scene,
            target,
            options,
            PostProcessControl::default(),
            |cause| matches!(cause, TangentCause::UnsupportedDomain),
        ));
    }
}

#[test]
fn polygons_and_degenerate_frames_reject_without_partial_publication() {
    let scene = common::canonical_quad_scene();
    let target = primitive(&scene).id();
    drop(assert_rejected(
        scene,
        target,
        corner_options(),
        PostProcessControl::default(),
        |cause| matches!(cause, TangentCause::UnsupportedTopology),
    ));
    let triangle = String::from_utf8(TRIANGLE.to_vec()).unwrap();
    let seam = String::from_utf8(MIRRORED_SEAM.to_vec()).unwrap();
    for source in [
        triangle.replace("v 0 1 0", "v 2 0 0"),
        triangle
            .replace("vt 1 0", "vt 0 0")
            .replace("vt 0 1", "vt 0 0"),
        triangle.replace("vn 0 0 1", "vn 0 0 0"),
        triangle.replace("vn 0 0 1", "vn 1 0 0"),
        triangle
            .replace("vn 0 0 1", "vn 0 0 1\nvn 0 0 -1")
            .replace("3/3/1", "3/3/2"),
        seam.replace("f 1/4/1 3/5/1 4/6/1", "f 1/1/1 3/1/1 4/1/1"),
    ] {
        let scene = common::import_obj(source.as_bytes()).into_parts().0;
        let target = primitive(&scene).id();
        drop(assert_rejected(
            scene,
            target,
            corner_options(),
            PostProcessControl::default(),
            |cause| {
                if let TangentCause::Issue { issue } = cause {
                    let _: &TangentIssue = issue;
                    true
                } else {
                    false
                }
            },
        ));
    }
}

#[test]
fn precancelled_generation_returns_the_input_and_releases_resources() {
    let scene = common::import_obj(TRIANGLE).into_parts().0;
    let target = primitive(&scene).id();
    let cancellation = PostProcessCancellationToken::new();
    cancellation.cancel();
    drop(assert_rejected(
        scene,
        target,
        corner_options(),
        PostProcessControl::default().with_cancellation(cancellation),
        |cause| {
            matches!(
                cause,
                TangentCause::Control {
                    cause: PostProcessControlCause::Cancelled { .. }
                }
            )
        },
    ));
}

#[test]
fn tight_selected_work_budget_rejects_indexed_generation_before_output_allocation() {
    let (scene, import_evidence) = import_fixture(Fixture::WorkLimit).into_parts();
    let input = primitive(&scene);
    let target = input.id();
    assert_eq!(input.vertex_count(), 3);
    assert_eq!(input.corner_count(), 909_093);
    assert_eq!(input.face_count(), 303_031);
    assert_eq!(input.attributes().len(), 3);
    assert_eq!(input.positions().unwrap().len(), 3);
    assert_eq!(input.normals(AttributeDomain::Vertex).unwrap().len(), 3);
    assert_eq!(
        input.texcoords(0, AttributeDomain::Vertex).unwrap().len(),
        3
    );
    assert_eq!(&input.indices().unwrap()[..3], &[0, 1, 2]);
    assert_eq!(&input.indices().unwrap()[909_090..], &[0, 1, 2]);
    let fields = non_tangent_fields(input);
    let generation = scene.generation();
    // The digest covers every index; only its diagnostic preview is deliberately bounded.
    let limits = SceneSnapshotLimits::try_new(4096, 16, 20_000_000).unwrap();
    let before = scene.capture_snapshot(limits).unwrap();
    let failure = baozi_postprocess::prepare_triangles(
        scene,
        &[target],
        baozi_postprocess::TrianglePreparationOptions {
            tangents: baozi_postprocess::TangentPolicy::Generate(vertex_options()),
            budgets: baozi_postprocess::PostProcessBudgets {
                work: 150,
                ..Default::default()
            },
            ..Default::default()
        },
        PostProcessControl::default(),
    )
    .expect_err("the explicit 150-unit allowance cannot complete indexed generation");
    assert_eq!(failure.input().generation(), generation);
    assert!(import_evidence.matches_scene(failure.input()));
    assert_eq!(non_tangent_fields(primitive(failure.input())), fields);
    assert_eq!(
        failure.input().primitive(target).unwrap().corner_count(),
        909_093
    );
    let after = failure.input().capture_snapshot(limits).unwrap();
    assert_eq!(after.digest(), before.digest());
    assert_eq!(after.preview(), before.preview());
    let PostProcessRunError::Preparation {
        failure: preparation,
    } = failure.error()
    else {
        panic!("expected typed selected work admission failure")
    };
    assert!(
        matches!(
            preparation.cause(),
            baozi_postprocess::PreparationCause::Runtime(
                baozi_operation::StoreRuntimeError::Ledger(_)
            )
        ),
        "expected a work admission failure, found {preparation:?}"
    );
    let resources = preparation.resources().unwrap();
    assert!(resources.consumed_work_items() <= 150);
    assert!(resources.attempted_work_items() >= resources.consumed_work_items());
    assert!(resources.peak_staging_bytes() < 909_093 * std::mem::size_of::<[f32; 4]>() as u64);
    assert_eq!(resources.current_live_bytes(), 0);
    assert_eq!(resources.active_reservations(), 0);
    assert_eq!(resources.committed_canonical_bytes(), 0);
    let (scene, error) = failure.into_parts();
    drop(error);
    assert_eq!(
        scene.capture_snapshot(limits).unwrap().digest(),
        before.digest()
    );
    assert_eq!(primitive(&scene).indices().unwrap().len(), 909_093);
}

#[test]
fn reflection_transports_generated_tangents_using_their_own_sign_encoding() {
    let scene = common::import_obj(MIRRORED_SEAM).into_parts().0;
    let target = primitive(&scene).id();
    let (scene, _) = generate_tangents(
        scene,
        target,
        corner_options(),
        PostProcessControl::default(),
    )
    .unwrap()
    .into_parts();
    let target = TargetSpace::try_new(
        Handedness::Left,
        Axis::PositiveY,
        Axis::NegativeZ,
        1.0,
        FrontFaceWinding::CounterClockwise,
    )
    .unwrap();
    let (scene, report) = convert_scene_space(
        scene,
        SpaceConversionOptions {
            target,
            source_assumptions: Some(TargetSpace::STANDARD),
        },
        PostProcessControl::default(),
    )
    .unwrap()
    .into_parts();
    assert!(
        report
            .space_conversion_effects()
            .unwrap()
            .reverses_triangle_winding()
    );
    let output = primitive(&scene);
    assert_eq!(output.indices(), Some([0, 2, 1, 0, 3, 2].as_slice()));
    let tangents = generated_tangents(output);
    for tangent in &tangents[..3] {
        assert_tangent(tangent, [-1.0, 0.0, 0.0, -1.0]);
    }
    for tangent in &tangents[3..] {
        assert_tangent(tangent, [1.0, 0.0, 0.0, 1.0]);
    }
}

const FORMAT: &str = "dev.baozi.test.tangent-fields";
const PATH: &str = "fixtures/fields.tangentfixture";
const POSITIONS: [f32; 9] = [0.0, -0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
const NORMALS: [f32; 9] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0];
const UVS: [f32; 6] = [0.0, -0.0, 1.0, 0.0, 0.0, 1.0];
const CORNER_UVS: [f32; 6] = [0.0, 1.0, 0.0, -0.0, 1.0, 0.0];
const PRECISE_VALUES: [f64; 3] = [16_777_217.0, -0.0, f64::from_bits(1)];
const AUTHORED: [f32; 12] = [
    0.0, 1.0, 0.0, -1.0, 0.0, 1.0, 0.0, -1.0, 0.0, 1.0, 0.0, -1.0,
];

#[derive(Clone, Copy)]
enum Fixture {
    Domains {
        normal: AttributeDomain,
        uv: AttributeDomain,
    },
    DualFields,
    SparseUv,
    Authored {
        multiple: bool,
    },
    WideNormal,
    WideTangent,
    Morph {
        semantic: Option<MorphDeltaSemantic>,
        authored: bool,
    },
    WorkLimit,
}

struct FieldImporter(Fixture);

impl Format for FieldImporter {
    type Options = NoFormatOptions;

    fn compile_defaults(
        &self,
        scope: &mut RegistryBuildScope<'_>,
    ) -> Result<CompiledOptions<Self::Options>, OptionCompileError> {
        NoFormatOptions::compile_defaults(scope)
    }

    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(
            FormatId::new(FORMAT).unwrap(),
            "Public tangent field fixture",
        )
        .encoding(FormatEncoding::Binary)
        .capabilities([
            (FormatCapability::Geometry, CapabilityStatus::Supported),
            (FormatCapability::MorphTargets, CapabilityStatus::Supported),
        ])
        .build()
        .unwrap()
    }

    fn detect(
        &self,
        _: &baozi_import::Probe<'_>,
        _: &ReadHint,
        _: &DetectionScope<'_>,
    ) -> DetectionResult<DetectionOutcome> {
        Ok(DetectionOutcome::Match(ReadConfidence::Certain))
    }

    fn parse(&self, _options: &Self::Options, context: &mut ParseContext<'_>) -> ParseResult<()> {
        let (normal, uv) = match self.0 {
            Fixture::Domains { normal, uv } => (normal, uv),
            _ => (AttributeDomain::Vertex, AttributeDomain::Vertex),
        };
        let uv_values = match uv {
            AttributeDomain::Vertex => UVS.as_slice(),
            AttributeDomain::Corner => CORNER_UVS.as_slice(),
            AttributeDomain::Face | AttributeDomain::Instance => {
                unreachable!("unsupported domains are options, never canonical fixture fields")
            }
        };
        let uv_attribute = if matches!(self.0, Fixture::SparseUv) {
            GeometryAttributeAuthoring::sparse_f32(
                AttributeSemantic::Texcoord(0),
                AttributeDomain::Vertex,
                2,
                3,
                &[1, 2],
                &[1.0, 0.0, 0.0, 1.0],
                &[0.0, -0.0],
            )
        } else {
            GeometryAttributeAuthoring::f32(
                AttributeSemantic::Texcoord(0),
                uv,
                2,
                uv_values,
                Presence::Absent,
            )
        };
        let mut attributes = vec![
            GeometryAttributeAuthoring::f32(
                AttributeSemantic::Position,
                AttributeDomain::Vertex,
                3,
                &POSITIONS,
                Presence::Absent,
            ),
            if matches!(self.0, Fixture::WideNormal) {
                GeometryAttributeAuthoring::f64(
                    AttributeSemantic::Normal,
                    normal,
                    3,
                    &[0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
                )
            } else {
                GeometryAttributeAuthoring::f32(
                    AttributeSemantic::Normal,
                    normal,
                    3,
                    &NORMALS,
                    Presence::Absent,
                )
            },
        ];
        if matches!(self.0, Fixture::DualFields) {
            attributes.push(GeometryAttributeAuthoring::f32(
                AttributeSemantic::Normal,
                AttributeDomain::Corner,
                3,
                &[0.6, 0.0, 0.8, 0.6, 0.0, 0.8, 0.6, 0.0, 0.8],
                Presence::Absent,
            ));
        }
        if matches!(
            self.0,
            Fixture::Authored { .. } | Fixture::Morph { authored: true, .. } | Fixture::WideTangent
        ) {
            // Leave this stream's declaration unknown to verify that Preserve never infers one.
            attributes.push(if matches!(self.0, Fixture::WideTangent) {
                GeometryAttributeAuthoring::f64(
                    AttributeSemantic::Tangent,
                    AttributeDomain::Vertex,
                    4,
                    &[
                        0.0, 1.0, 0.0, -1.0, 0.0, 1.0, 0.0, -1.0, 0.0, 1.0, 0.0, -1.0,
                    ],
                )
            } else {
                GeometryAttributeAuthoring::f32(
                    AttributeSemantic::Tangent,
                    AttributeDomain::Vertex,
                    4,
                    &AUTHORED,
                    Presence::Absent,
                )
            });
        }
        if matches!(self.0, Fixture::Authored { multiple: true }) {
            attributes.push(
                GeometryAttributeAuthoring::f32(
                    AttributeSemantic::Tangent,
                    AttributeDomain::Corner,
                    4,
                    &AUTHORED,
                    Presence::Absent,
                )
                .with_tangent_conventions(TangentConventions {
                    basis: TangentBasisConvention::MikkTspace,
                    encoding: TangentHandednessEncoding::SignInW,
                }),
            );
        }
        attributes.push(uv_attribute);
        if matches!(self.0, Fixture::DualFields) {
            attributes.push(GeometryAttributeAuthoring::f32(
                AttributeSemantic::Texcoord(0),
                AttributeDomain::Corner,
                2,
                &[0.0, 1.0, 0.0, 0.0, -1.0, 0.0],
                Presence::Absent,
            ));
        }
        if !matches!(self.0, Fixture::WorkLimit) {
            attributes.push(GeometryAttributeAuthoring::f64(
                AttributeSemantic::Custom(CustomAttributeSemantic::new("test:precision").unwrap()),
                AttributeDomain::Vertex,
                1,
                &PRECISE_VALUES,
            ));
        }
        let indices = if matches!(self.0, Fixture::WorkLimit) {
            [0, 1, 2].repeat(303_031)
        } else {
            vec![2, 0, 1]
        };
        let primitives = [GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::TriangleList,
            Presence::Present(&indices),
            baozi_core::MaterialAssignment::Unspecified,
            AuthoringRange::new(0, attributes.len() as u32),
        )];
        let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
        let Fixture::Morph { semantic, .. } = self.0 else {
            return context.submit(
                StaticSceneBuilder::new(Presence::Present(&[]), &[])
                    .with_scene_space(&SceneSpace::STANDARD_TARGET)
                    .with_geometry(&meshes, &primitives, &attributes, &[]),
            );
        };

        let deltas = semantic.map(|semantic| {
            MorphDeltaAuthoring::new(
                semantic,
                match semantic {
                    MorphDeltaSemantic::Position => MorphDeltaPayloadAuthoring::Dense {
                        values: &[0.0, 0.0, 0.25, 0.0, 0.0, 0.25, 0.0, 0.0, 0.25],
                    },
                    MorphDeltaSemantic::Normal => {
                        MorphDeltaPayloadAuthoring::Dense { values: &[0.0; 9] }
                    }
                    MorphDeltaSemantic::Tangent => MorphDeltaPayloadAuthoring::Sparse {
                        logical_count: 3,
                        indices: &[],
                        values: &[],
                    },
                },
            )
        });
        let targets = [PrimitiveMorphTargetAuthoring::new(deltas.as_slice())];
        let definitions = [MorphTargetDefinitionAuthoring::new(
            MorphTargetIdentity::new(0),
            Presence::Present("authored delta"),
        )];
        let logical_morphs = [LogicalMorphAuthoring::new(
            AuthoringRange::new(0, 1),
            &[0.0],
        )];
        let deformation = [PrimitiveDeformationAuthoring::new(
            Presence::Absent,
            &targets,
        )];
        let format = FormatId::new(FORMAT).unwrap();
        let resource = ResourceIdentity::new(PATH).unwrap();
        let resources = [ObservedResourceAuthoring::session_opaque(
            &resource,
            SessionOpaqueReason::CallerLabel,
        )];
        let locator = SourceObjectLocator::indexed(SourceObjectKind::Document, 0);
        let sources =
            [
                SourceIdentityAuthoring::new(ResourceIdentityLocalId::new(0), &format, &locator)
                    .with_content_digest(ContentDigest::sha256([
                        0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde,
                        0x5d, 0xae, 0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c,
                        0xb4, 0x10, 0xff, 0x61, 0xf2, 0x00, 0x15, 0xad,
                    ])),
            ];
        let provenance = [ProvenanceNodeAuthoring::source(SourceIdentityLocalId::new(
            0,
        ))];
        let attachment = [ProvenanceLocalId::new(0)];
        let mut owners = vec![
            ProfileOwner::Scene,
            ProfileOwner::SyntheticRoot,
            ProfileOwner::LogicalMesh(0),
            ProfileOwner::Primitive(0),
            ProfileOwner::MorphDefinition(0),
        ];
        owners.extend((0..attributes.len() as u32).map(ProfileOwner::Attribute));
        if semantic.is_some() {
            owners.push(ProfileOwner::MorphDelta(0));
        }
        let owners: Vec<_> = owners
            .into_iter()
            .map(|owner| OwnerProvenanceAuthoring::new(owner, &attachment))
            .collect();
        let snapshots = [ResourceSnapshotAuthoring::snapshotted(
            context.primary_bytes(),
        )];
        context.submit_complete(SceneAuthoringBatch::new(
            SpatialAuthoringBatch::new(&SceneSpace::STANDARD_TARGET, &[], &[]),
            NodeAuthoringBatch::new(Presence::Present(&[]), &[]),
            AppearanceAuthoringBatch::empty(),
            GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &[]),
            DeformationAuthoringBatch::new(&[], &definitions, &logical_morphs, &deformation, &[]),
            AnimationAuthoringBatch::empty(),
            ProfileAuthoringBatch::new(
                LineageAuthoringBatch::new(&resources, &sources, &provenance, &owners),
                MetadataAuthoringBatch::empty(),
            )
            .with_resource_snapshots(&snapshots),
        ))
    }
}

fn import_fixture(fixture: Fixture) -> ImportReport {
    let mut registry = RegistryBuilder::new();
    registry.register(FieldImporter(fixture)).unwrap();
    let registry = registry.freeze(RegistryBuildLimits::default()).unwrap();
    let request = ImportRequest::new().with_caller_layer(ImportRequestLayer::new(
        ImportConfigOverlay::default()
            .with_format_selection(FormatSelection::Forced(FormatId::new(FORMAT).unwrap())),
    ));
    let request = registry
        .plan(&request, OptionCompileLimits::default())
        .unwrap();
    let io = MemoryAssetIo::new();
    let path = AssetPath::new(PATH).unwrap();
    io.try_insert(path.clone(), Arc::<[u8]>::from(b"abc".as_slice()))
        .unwrap();
    request
        .read_asset(&io, path, ImportOperationControl::default())
        .expect("public authoring fixture must pass central validation")
}

#[test]
fn explicit_vertex_and_corner_field_reads_follow_indexed_geometry() {
    for normal in [AttributeDomain::Vertex, AttributeDomain::Corner] {
        for uv in [AttributeDomain::Vertex, AttributeDomain::Corner] {
            let scene = import_fixture(Fixture::Domains { normal, uv })
                .into_parts()
                .0;
            let target = primitive(&scene).id();
            let before = non_tangent_fields(primitive(&scene));
            let (scene, _) = generate_tangents(
                scene,
                target,
                TangentOptions::new(normal, 0, uv),
                PostProcessControl::default(),
            )
            .unwrap()
            .into_parts();
            assert_eq!(primitive(&scene).indices(), Some([2, 0, 1].as_slice()));
            assert_eq!(non_tangent_fields(primitive(&scene)), before);
            for tangent in generated_tangents(primitive(&scene)) {
                assert_tangent(tangent, [1.0, 0.0, 0.0, 1.0]);
            }
        }
    }
}

#[test]
fn selected_sparse_uv_defaults_and_explicit_values_match_dense_generation() {
    let dense = import_fixture(Fixture::Domains {
        normal: AttributeDomain::Vertex,
        uv: AttributeDomain::Vertex,
    })
    .into_parts()
    .0;
    let target = primitive(&dense).id();
    let (dense, _) = generate_tangents(
        dense,
        target,
        vertex_options(),
        PostProcessControl::default(),
    )
    .unwrap()
    .into_parts();
    let dense_tangents = generated_tangents(primitive(&dense));
    for tangent in dense_tangents {
        assert_tangent(tangent, [1.0, 0.0, 0.0, 1.0]);
    }

    let sparse = import_fixture(Fixture::SparseUv).into_parts().0;
    let target = primitive(&sparse).id();
    assert_eq!(primitive(&sparse).indices(), Some([2, 0, 1].as_slice()));
    let (sparse, _) = generate_tangents(
        sparse,
        target,
        vertex_options(),
        PostProcessControl::default(),
    )
    .unwrap()
    .into_parts();
    let output = primitive(&sparse);
    assert_eq!(generated_tangents(output), dense_tangents);
    let AttributeData::SparseF32 {
        lanes: 2,
        logical_count: 3,
        indices,
        values,
        default,
    } = output.texcoords(0, AttributeDomain::Vertex).unwrap().data()
    else {
        panic!("selected UVs must retain their Sparse representation")
    };
    assert_eq!(indices, &[1, 2]);
    assert_eq!(values, &[1.0, 0.0, 0.0, 1.0]);
    assert_eq!(default[0].to_bits(), 0.0_f32.to_bits());
    assert_eq!(default[1].to_bits(), (-0.0_f32).to_bits());
    assert_eq!(
        field_state(output.positions().unwrap()),
        field_state(primitive(&dense).positions().unwrap())
    );
    assert_eq!(
        field_state(output.normals(AttributeDomain::Vertex).unwrap()),
        field_state(primitive(&dense).normals(AttributeDomain::Vertex).unwrap())
    );
}

#[test]
fn simultaneous_fields_use_the_exact_requested_normal_and_uv_domains() {
    let mut options_digests = Vec::with_capacity(2);
    for (options, expected) in [
        (vertex_options(), [1.0, 0.0, 0.0, 1.0]),
        (corner_options(), [-0.8, 0.0, 0.6, -1.0]),
    ] {
        let scene = import_fixture(Fixture::DualFields).into_parts().0;
        let target = primitive(&scene).id();
        let before = non_tangent_fields(primitive(&scene));
        let (scene, _) = generate_tangents(scene, target, options, PostProcessControl::default())
            .unwrap()
            .into_parts();
        assert_eq!(non_tangent_fields(primitive(&scene)), before);
        for tangent in generated_tangents(primitive(&scene)) {
            assert_tangent(tangent, expected);
        }
        let tangent = primitive(&scene)
            .attribute(AttributeSemanticRef::Tangent, AttributeDomain::Corner)
            .unwrap();
        let (algorithm, digest) = scene
            .owner_provenance(tangent.metadata_owner())
            .unwrap()
            .find_map(|node| match node.kind() {
                ProvenanceKindRef::Operation(operation)
                    if operation.kind() == ProvenanceOperationKindRef::Generated =>
                {
                    Some(operation.options_digest())
                }
                _ => None,
            })
            .expect("the generated Tangent owner must retain its operation options");
        options_digests.push((algorithm, digest.to_vec()));
    }
    assert_eq!(options_digests[0].0, options_digests[1].0);
    assert_ne!(options_digests[0].1, options_digests[1].1);
}

#[test]
fn default_preserve_keeps_one_or_multiple_authored_streams_and_their_declarations() {
    for multiple in [false, true] {
        let (scene, import_evidence) = import_fixture(Fixture::Authored { multiple }).into_parts();
        let target = primitive(&scene).id();
        let before = snapshot(&scene);
        let generation = scene.generation();
        // Existing data wins before a missing requested field can trigger generation work.
        let options = TangentOptions::new(AttributeDomain::Corner, 9, AttributeDomain::Corner);
        let (scene, report) =
            generate_tangents(scene, target, options, PostProcessControl::default())
                .unwrap()
                .into_parts();
        assert_eq!(report.disposition(), PostProcessRunDisposition::Unchanged);
        assert!(report.tangent_generation().is_none());
        assert_eq!(scene.generation(), generation);
        assert!(import_evidence.matches_scene(&scene));
        assert_eq!(snapshot(&scene).digest(), before.digest());
        assert_eq!(snapshot(&scene).preview(), before.preview());
        assert_eq!(report.resources().current_live_bytes(), 0);
        assert_eq!(report.resources().committed_canonical_bytes(), 0);
        let tangent = primitive(&scene)
            .attribute(AttributeSemanticRef::Tangent, AttributeDomain::Vertex)
            .unwrap();
        assert_eq!(
            tangent.tangent_conventions(),
            Some(TangentConventionsRef {
                basis: TangentBasisRef::Unknown,
                encoding: TangentEncodingRef::Unknown
            })
        );
        let AttributeData::DenseF32 { values, lanes: 4 } = tangent.data() else {
            panic!("authored values")
        };
        assert_eq!(values, AUTHORED);
        if multiple {
            assert_eq!(
                primitive(&scene)
                    .attribute(AttributeSemanticRef::Tangent, AttributeDomain::Corner)
                    .unwrap()
                    .tangent_conventions(),
                Some(TangentConventionsRef {
                    basis: TangentBasisRef::MikkTspace,
                    encoding: TangentEncodingRef::SignInW
                })
            );
        }
    }
}

#[test]
fn explicit_replace_removes_the_single_authored_domain_and_publishes_corner_output() {
    let scene = import_fixture(Fixture::Authored { multiple: false })
        .into_parts()
        .0;
    let target = primitive(&scene).id();
    let attribute_count = scene.attribute_count();
    let before = non_tangent_fields(primitive(&scene));
    let (scene, report) = generate_tangents(
        scene,
        target,
        replacement(vertex_options()),
        PostProcessControl::default(),
    )
    .unwrap()
    .into_parts();
    assert_eq!(report.disposition(), PostProcessRunDisposition::Changed);
    assert!(
        report
            .tangent_generation()
            .unwrap()
            .replaced_authored_tangents()
    );
    assert_eq!(scene.attribute_count(), attribute_count);
    assert_eq!(non_tangent_fields(primitive(&scene)), before);
    assert!(
        primitive(&scene)
            .attribute(AttributeSemanticRef::Tangent, AttributeDomain::Vertex)
            .is_none()
    );
    for tangent in generated_tangents(primitive(&scene)) {
        assert_tangent(tangent, [1.0, 0.0, 0.0, 1.0]);
    }
}

#[test]
fn replacing_multiple_authored_tangent_fields_requires_an_explicit_selection_policy() {
    let scene = import_fixture(Fixture::Authored { multiple: true })
        .into_parts()
        .0;
    let target = primitive(&scene).id();
    drop(assert_rejected(
        scene,
        target,
        replacement(vertex_options()),
        PostProcessControl::default(),
        |cause| matches!(cause, TangentCause::MultipleTangentFields),
    ));
}

#[test]
fn dependent_normal_and_tangent_morph_deltas_reject_while_position_only_is_preserved() {
    for semantic in [
        MorphDeltaSemantic::Position,
        MorphDeltaSemantic::Normal,
        MorphDeltaSemantic::Tangent,
    ] {
        let scene = import_fixture(Fixture::Morph {
            semantic: Some(semantic),
            authored: semantic == MorphDeltaSemantic::Tangent,
        })
        .into_parts()
        .0;
        let target = primitive(&scene).id();
        assert_eq!(
            primitive(&scene)
                .morph_targets()
                .next()
                .unwrap()
                .delta_count(),
            1
        );
        let delta = primitive(&scene)
            .morph_targets()
            .next()
            .unwrap()
            .delta(semantic)
            .unwrap();
        match (semantic, delta.data()) {
            (MorphDeltaSemantic::Position, baozi_core::MorphDeltaData::Dense { values }) => {
                assert!(values.contains(&0.25));
            }
            (MorphDeltaSemantic::Normal, baozi_core::MorphDeltaData::Dense { values }) => {
                assert_eq!(values, &[0.0; 9]);
            }
            (
                MorphDeltaSemantic::Tangent,
                baozi_core::MorphDeltaData::Sparse {
                    logical_count,
                    indices,
                    values,
                },
            ) => {
                assert_eq!(logical_count, 3);
                assert!(indices.is_empty() && values.is_empty());
            }
            _ => panic!("the fixture must retain its authored morph representation"),
        }
        let scene = if semantic == MorphDeltaSemantic::Position {
            let fields = non_tangent_fields(primitive(&scene));
            let baozi_core::MorphDeltaData::Dense { values } = delta.data() else {
                panic!("dense Position delta")
            };
            let expected: Vec<_> = values.iter().map(|v| v.to_bits()).collect();
            let (scene, report) = generate_tangents(
                scene,
                target,
                replacement(vertex_options()),
                PostProcessControl::default(),
            )
            .unwrap()
            .into_parts();
            assert_eq!(report.disposition(), PostProcessRunDisposition::Changed);
            assert_eq!(non_tangent_fields(primitive(&scene)), fields);
            let baozi_core::MorphDeltaData::Dense { values } = primitive(&scene)
                .morph_targets()
                .next()
                .unwrap()
                .delta(semantic)
                .unwrap()
                .data()
            else {
                panic!("retained Position delta")
            };
            assert_eq!(
                values.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                expected
            );
            scene
        } else {
            assert_rejected(
                scene,
                target,
                replacement(vertex_options()),
                PostProcessControl::default(),
                |cause| matches!(cause, TangentCause::MorphDeltas),
            )
        };
        assert!(
            primitive(&scene)
                .morph_targets()
                .next()
                .unwrap()
                .delta(semantic)
                .is_some()
        );
    }
}

#[test]
fn preserve_returns_before_morph_rejection_but_replace_rejects_the_same_input() {
    let (scene, import_evidence) = import_fixture(Fixture::Morph {
        semantic: Some(MorphDeltaSemantic::Normal),
        authored: true,
    })
    .into_parts();
    let target = primitive(&scene).id();
    let before = snapshot(&scene);
    let (scene, report) = generate_tangents(
        scene,
        target,
        vertex_options(),
        PostProcessControl::default(),
    )
    .unwrap()
    .into_parts();
    assert_eq!(report.disposition(), PostProcessRunDisposition::Unchanged);
    assert!(report.tangent_generation().is_none());
    assert!(import_evidence.matches_scene(&scene));
    assert_eq!(snapshot(&scene).digest(), before.digest());
    drop(assert_rejected(
        scene,
        target,
        replacement(vertex_options()),
        PostProcessControl::default(),
        |cause| matches!(cause, TangentCause::MorphDeltas),
    ));
}

#[test]
fn empty_morph_targets_without_delta_streams_do_not_block_generation() {
    let scene = import_fixture(Fixture::Morph {
        semantic: None,
        authored: false,
    })
    .into_parts()
    .0;
    let target = primitive(&scene).id();
    let (scene, report) = generate_tangents(
        scene,
        target,
        vertex_options(),
        PostProcessControl::default(),
    )
    .unwrap()
    .into_parts();
    assert_eq!(report.disposition(), PostProcessRunDisposition::Changed);
    assert_eq!(
        primitive(&scene)
            .morph_targets()
            .next()
            .unwrap()
            .delta_count(),
        0
    );
    assert_eq!(
        scene.meshes().next().unwrap().default_morph_weights(),
        &[0.0]
    );
    for tangent in generated_tangents(primitive(&scene)) {
        assert_tangent(tangent, [1.0, 0.0, 0.0, 1.0]);
    }
}

#[test]
fn precision_contract_replacing_wide_base_fields_never_projects_to_f32() {
    for fixture in [Fixture::WideNormal, Fixture::WideTangent] {
        let scene = import_fixture(fixture).into_parts().0;
        let target = primitive(&scene).id();
        let generation = scene.generation();
        let before = snapshot(&scene);
        let normal = matches!(fixture, Fixture::WideNormal);
        let preserved = if normal {
            generate_normals(
                scene,
                target,
                NormalOptions::default(),
                baozi_postprocess::TangentPolicy::Preserve,
                PostProcessControl::default(),
            )
        } else {
            generate_tangents(
                scene,
                target,
                vertex_options(),
                PostProcessControl::default(),
            )
        }
        .unwrap();
        let (scene, report) = preserved.into_parts();
        assert_eq!(report.disposition(), PostProcessRunDisposition::Unchanged);
        assert_eq!(scene.generation(), generation);
        assert_eq!(snapshot(&scene).digest(), before.digest());
        let result = if normal {
            generate_normals(
                scene,
                target,
                NormalOptions {
                    existing: ExistingNormals::Replace,
                    ..NormalOptions::default()
                },
                baozi_postprocess::TangentPolicy::Preserve,
                PostProcessControl::default(),
            )
        } else {
            generate_tangents(
                scene,
                target,
                replacement(vertex_options()),
                PostProcessControl::default(),
            )
        };
        let output =
            result.expect("ordinary F32 dependencies replacing an F64 field must succeed at F64");
        let (scene, report) = output.into_parts();
        assert_eq!(report.disposition(), PostProcessRunDisposition::Changed);
        let semantic = if normal {
            AttributeSemanticRef::Normal
        } else {
            AttributeSemanticRef::Tangent
        };
        let field = primitive(&scene)
            .attribute(semantic, AttributeDomain::Corner)
            .unwrap();
        assert!(
            matches!(field.data(), AttributeData::DenseF64 { .. }),
            "a replacement must retain the widest dependency/old-field width"
        );
    }
}
