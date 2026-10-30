use baozi_core::{NumericConversionError, SceneSnapshotLimits, TargetSpace, ValidatedScene};
use baozi_import::authoring::{
    AlphaMode, AuthoredTransform, AuthoringRange, Axis, Color, ColorInterpretation,
    ExtensionMediaType, FormatId, FrontFaceWinding, Handedness, ImageAuthoring, ImageLocalId,
    LegacyMaterialFactors, MaterialAuthoring, MaterialBindingAuthoring, MaterialFactors,
    NodeAuthoringNode, NodeLocalId, PbrMetallicRoughnessFactors, PbrSpecularGlossinessFactors,
    Presence, SamplerAuthoring, SamplerLocalId, SamplerState, SceneSpace, ShadingModel,
    StaticSceneBuilder, SupplementalPbrFactors, TextureAuthoring, TextureBlendOperation,
    TextureChannelSwizzle, TextureLocalId, TextureRole, TextureSourceKey, TextureStrength,
    TextureTransform, Vec2, Vec3,
};
use baozi_import::{
    CapabilityStatus, DetectionOutcome, DetectionResult, DetectionScope, FormatCapability,
    FormatDescriptor, FormatEncoding, FormatImporter, FormatSelection, ImportConfigOverlay,
    ImportOperationControl, ImportReport, ImportRequest, ImportRequestLayer, OptionCompileLimits,
    ParseResult, ParseScope, ReadConfidence, ReadHint, RegistryBuildLimits, RegistryBuilder,
    SceneDraft, import_bytes,
};
use baozi_io::AssetPath;
use baozi_postprocess::{
    PostProcessControl, PostProcessResourceEvidence, PostProcessRunDisposition,
    PostProcessRunError, SpaceConversionCause, SpaceConversionIssue, SpaceConversionOptions,
    convert_scene_space,
};

const FORMAT: &str = "dev.baozi.test.space-appearance";
const PATH: &str = "fixtures/materials.spacefixture";
const ROLES: [TextureRole; 5] = [
    TextureRole::Normal,
    TextureRole::Displacement,
    TextureRole::Thickness,
    TextureRole::Displacement,
    TextureRole::Displacement,
];
const UV_SETS: [u16; 5] = [u16::MAX, 7, 0, 23, 1];
const KEYS: [Option<&str>; 5] = [
    Some("test:normal"),
    Some("test:positive"),
    None,
    Some("test:negative"),
    Some("test:zero"),
];

#[derive(Debug, Clone, Copy)]
struct Lengths {
    thickness: [f32; 3],
    displacement: [f32; 3],
}

impl Lengths {
    const SOURCE: Self = Self {
        thickness: [1.25, 0.25, -0.0],
        displacement: [1.5, -0.75, -0.0],
    };

    const DOUBLED: Self = Self {
        thickness: [2.5, 0.5, -0.0],
        displacement: [3.0, -1.5, -0.0],
    };
}

fn factors(material: usize, thickness: f32) -> MaterialFactors {
    let (metallic, specular, legacy) = match material {
        0 => (
            Some(
                PbrMetallicRoughnessFactors::try_new(
                    Color::linear_rgba(-0.0, 0.125, 0.5, 1.0),
                    -0.0,
                    0.75,
                )
                .unwrap(),
            ),
            Some(
                PbrSpecularGlossinessFactors::try_new(
                    Color::linear_rgba(1.0, -0.0, 0.5, 0.25),
                    Vec3::new(-0.0, 1.0, 0.125),
                    f32::from_bits(0x3e80_0001),
                )
                .unwrap(),
            ),
            Some(
                LegacyMaterialFactors::try_new(
                    Vec3::new(-0.0, 1.0, 2.0),
                    Vec3::new(0.25, -0.0, 3.0),
                    Vec3::new(4.0, 0.5, -0.0),
                    Vec3::new(0.0, -0.0, 7.0),
                    -0.0,
                    -0.0,
                )
                .unwrap(),
            ),
        ),
        1 => (None, None, None),
        2 => (
            Some(
                PbrMetallicRoughnessFactors::try_new(
                    Color::linear_rgba(0.0, 0.0, 0.0, 0.0),
                    0.0,
                    0.0,
                )
                .unwrap(),
            ),
            None,
            None,
        ),
        _ => unreachable!(),
    };
    MaterialFactors::try_new_with_supplemental_pbr(
        Vec3::new(-0.0, 2.0, f32::from_bits(1)),
        metallic,
        specular,
        legacy,
        SupplementalPbrFactors::try_new(
            -0.0,
            0.125,
            0.25,
            thickness,
            Vec3::new(-0.0, 1.5, f32::from_bits(1)),
            0.5,
        )
        .unwrap(),
    )
    .unwrap()
}

fn transform() -> TextureTransform {
    TextureTransform::try_new(
        Vec2::new(-0.0, f32::from_bits(1)),
        f32::from_bits(0xbe00_0001),
        Vec2::new(1.0, -2.0),
    )
    .unwrap()
}

fn strengths(lengths: Lengths) -> [f32; 5] {
    [
        f32::from_bits(0xbe80_0001),
        lengths.displacement[0],
        1.0,
        lengths.displacement[1],
        lengths.displacement[2],
    ]
}

struct AppearanceImporter {
    lengths: Lengths,
    image_len: usize,
}

impl FormatImporter for AppearanceImporter {
    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(FormatId::new(FORMAT).unwrap(), "Appearance space fixture")
            .encoding(FormatEncoding::Binary)
            .capabilities([
                (FormatCapability::Materials, CapabilityStatus::Supported),
                (FormatCapability::Textures, CapabilityStatus::Supported),
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

    fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
        let roots = [NodeLocalId::new(0)];
        let nodes = [NodeAuthoringNode::new(
            Presence::Absent,
            AuthoredTransform::identity_trs(),
            Presence::Absent,
            Presence::Absent,
        )
        .with_name("materials")];
        // The encoded contents are opaque to canonical storage and to space conversion.
        let encoded = vec![0xa5; self.image_len];
        let media = ExtensionMediaType::new("image/png").unwrap();
        let images = [ImageAuthoring::encoded(&encoded, Presence::Present(&media))];
        let samplers = [SamplerAuthoring::new(SamplerState::CANONICAL_DEFAULT)];
        let textures = [TextureAuthoring::new(
            ImageLocalId::new(0),
            SamplerLocalId::new(0),
        )];
        let keys = KEYS.map(|key| key.map(|key| TextureSourceKey::try_new(key).unwrap()));
        let strengths = strengths(self.lengths);
        let bindings = std::array::from_fn::<_, 5, _>(|index| {
            MaterialBindingAuthoring::try_from_parts(
                TextureLocalId::new(0),
                ROLES[index],
                UV_SETS[index],
                if index == 0 {
                    TextureChannelSwizzle::RGB
                } else {
                    TextureChannelSwizzle::RED
                },
                ColorInterpretation::Data,
                transform(),
                TextureStrength::try_new(strengths[index]).unwrap(),
                TextureBlendOperation::Replace,
                keys[index].as_ref(),
            )
            .unwrap()
        });
        let materials = [
            MaterialAuthoring::try_from_parts(
                AuthoringRange::new(0, 3),
                ShadingModel::PbrMetallicRoughness,
                AlphaMode::mask(-0.0).unwrap(),
                true,
                factors(0, self.lengths.thickness[0]),
            )
            .unwrap()
            .with_name("layered"),
            MaterialAuthoring::try_from_parts(
                AuthoringRange::new(3, 1),
                ShadingModel::Unspecified,
                AlphaMode::Blend,
                false,
                factors(1, self.lengths.thickness[1]),
            )
            .unwrap()
            .with_name(""),
            MaterialAuthoring::try_from_parts(
                AuthoringRange::new(4, 1),
                ShadingModel::PbrMetallicRoughness,
                AlphaMode::Opaque,
                false,
                factors(2, self.lengths.thickness[2]),
            )
            .unwrap(),
        ];
        scope.submit_static_scene(
            StaticSceneBuilder::new(Presence::Present(&roots), &nodes)
                .with_appearance(&images, &samplers, &textures, &bindings, &materials)
                .with_scene_space(&SceneSpace::STANDARD_TARGET),
        )
    }
}

fn import_fixture(lengths: Lengths, image_len: usize) -> ImportReport {
    let mut registry = RegistryBuilder::new();
    registry
        .register(AppearanceImporter { lengths, image_len })
        .unwrap();
    let registry = registry.freeze(RegistryBuildLimits::default()).unwrap();
    let request = ImportRequest::new().with_caller_layer(ImportRequestLayer::new(
        ImportConfigOverlay::default()
            .with_format_selection(FormatSelection::Forced(FormatId::new(FORMAT).unwrap())),
    ));
    let request = registry
        .compile_request(&request, OptionCompileLimits::default())
        .unwrap();
    import_bytes(
        &request,
        AssetPath::new(PATH).unwrap(),
        b"abc",
        ImportOperationControl::default(),
    )
    .expect("public appearance fixture must pass central import validation")
}

fn options(units: f64) -> SpaceConversionOptions {
    SpaceConversionOptions {
        target: TargetSpace::try_new(
            Handedness::Left,
            Axis::PositiveZ,
            Axis::PositiveX,
            units,
            FrontFaceWinding::CounterClockwise,
        )
        .unwrap(),
        source_assumptions: None,
    }
}

fn assert_bits(actual: &[f32], expected: &[f32]) {
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.iter().zip(expected) {
        assert_eq!(actual.to_bits(), expected.to_bits());
    }
}

fn assert_factors(actual: MaterialFactors, expected: MaterialFactors) {
    // Derived equality checks every optional group's presence; the explicit comparisons below
    // additionally distinguish authored negative zeros in every concrete numeric group.
    assert_eq!(actual, expected);
    let a = actual.emissive();
    let e = expected.emissive();
    assert_bits(&[a.x, a.y, a.z], &[e.x, e.y, e.z]);
    if let (Some(a), Some(e)) = (
        actual.pbr_metallic_roughness(),
        expected.pbr_metallic_roughness(),
    ) {
        let ac = a.base_color();
        let ec = e.base_color();
        assert_bits(
            &[ac.r, ac.g, ac.b, ac.a, a.metallic(), a.roughness()],
            &[ec.r, ec.g, ec.b, ec.a, e.metallic(), e.roughness()],
        );
    }
    if let (Some(a), Some(e)) = (
        actual.pbr_specular_glossiness(),
        expected.pbr_specular_glossiness(),
    ) {
        let ac = a.diffuse();
        let ec = e.diffuse();
        let aspec = a.specular();
        let espec = e.specular();
        assert_bits(
            &[
                ac.r,
                ac.g,
                ac.b,
                ac.a,
                aspec.x,
                aspec.y,
                aspec.z,
                a.glossiness(),
            ],
            &[
                ec.r,
                ec.g,
                ec.b,
                ec.a,
                espec.x,
                espec.y,
                espec.z,
                e.glossiness(),
            ],
        );
    }
    if let (Some(a), Some(e)) = (actual.legacy(), expected.legacy()) {
        for (a, e) in [
            (a.ambient(), e.ambient()),
            (a.diffuse(), e.diffuse()),
            (a.specular(), e.specular()),
            (a.emissive(), e.emissive()),
        ] {
            assert_bits(&[a.x, a.y, a.z], &[e.x, e.y, e.z]);
        }
        assert_bits(&[a.shininess(), a.opacity()], &[e.shininess(), e.opacity()]);
    }
    let a = actual.supplemental_pbr();
    let e = expected.supplemental_pbr();
    let asheen = a.sheen_color();
    let esheen = e.sheen_color();
    assert_bits(
        &[
            a.clearcoat(),
            a.clearcoat_roughness(),
            a.transmission(),
            a.thickness(),
            asheen.x,
            asheen.y,
            asheen.z,
            a.sheen_roughness(),
        ],
        &[
            e.clearcoat(),
            e.clearcoat_roughness(),
            e.transmission(),
            e.thickness(),
            esheen.x,
            esheen.y,
            esheen.z,
            e.sheen_roughness(),
        ],
    );
}

fn assert_appearance(scene: &ValidatedScene, expected: Lengths, image_len: usize) {
    assert_eq!((scene.node_count(), scene.root_count()), (1, 1));
    assert_eq!(scene.material_count(), 3);
    assert_eq!(scene.material_binding_count(), 5);
    assert_eq!(
        (
            scene.image_count(),
            scene.sampler_count(),
            scene.texture_count()
        ),
        (1, 1, 1)
    );
    let image = scene.images().next().unwrap();
    assert_eq!(image.media_type(), Some("image/png"));
    assert_eq!(image.encoded_bytes().len(), image_len);
    assert_eq!(image.encoded_bytes().first(), Some(&0xa5));
    assert_eq!(image.encoded_bytes().last(), Some(&0xa5));
    let sampler = scene.samplers().next().unwrap();
    assert_eq!(sampler.state(), SamplerState::CANONICAL_DEFAULT);
    let texture = scene.textures().next().unwrap();
    assert_eq!(texture.image().id(), image.id());
    assert_eq!(texture.sampler().id(), sampler.id());
    let expected_strengths = strengths(expected);
    let mut binding_index = 0;
    for (index, material) in scene.materials().enumerate() {
        assert_eq!(material.name(), [Some("layered"), Some(""), None][index]);
        assert_eq!(material.binding_count(), [3, 1, 1][index]);
        assert_eq!(material.double_sided(), index == 0);
        assert_eq!(
            material.shading_model(),
            [
                ShadingModel::PbrMetallicRoughness,
                ShadingModel::Unspecified,
                ShadingModel::PbrMetallicRoughness,
            ][index]
        );
        match (index, material.alpha_mode()) {
            (0, AlphaMode::Mask(cutoff)) => {
                assert_eq!(cutoff.get().to_bits(), (-0.0_f32).to_bits())
            }
            (1, AlphaMode::Blend) | (2, AlphaMode::Opaque) => {}
            _ => panic!("material alpha mode must retain its authored variant"),
        }
        assert_factors(
            material.factors(),
            factors(index, expected.thickness[index]),
        );
        for binding in material.bindings() {
            assert_eq!(binding.role(), ROLES[binding_index]);
            assert_eq!(binding.uv_set(), UV_SETS[binding_index]);
            assert_eq!(binding.source_key(), KEYS[binding_index]);
            assert_eq!(binding.texture().id(), texture.id());
            assert_eq!(binding.color_interpretation(), ColorInterpretation::Data);
            assert_eq!(binding.blend(), TextureBlendOperation::Replace);
            assert_eq!(
                binding.channel_mapping(),
                if binding_index == 0 {
                    TextureChannelSwizzle::RGB
                } else {
                    TextureChannelSwizzle::RED
                }
            );
            assert_eq!(
                binding.strength().get().to_bits(),
                expected_strengths[binding_index].to_bits()
            );
            let actual = binding.transform();
            let expected = transform();
            assert_bits(
                &[
                    actual.offset().x,
                    actual.offset().y,
                    actual.rotation_radians(),
                    actual.scale().x,
                    actual.scale().y,
                ],
                &[
                    expected.offset().x,
                    expected.offset().y,
                    expected.rotation_radians(),
                    expected.scale().x,
                    expected.scale().y,
                ],
            );
            binding_index += 1;
        }
    }
    assert_eq!(binding_index, 5);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PayloadAddresses {
    image: *const u8,
    media: *const u8,
    names: [Option<*const u8>; 3],
    keys: [Option<*const u8>; 5],
}

fn payload_addresses(scene: &ValidatedScene) -> PayloadAddresses {
    let image = scene.images().next().unwrap();
    let mut names = [None; 3];
    let mut keys = [None; 5];
    let mut binding_index = 0;
    for (index, material) in scene.materials().enumerate() {
        names[index] = material.name().map(str::as_ptr);
        for binding in material.bindings() {
            keys[binding_index] = binding.source_key().map(str::as_ptr);
            binding_index += 1;
        }
    }
    assert_eq!(binding_index, 5);
    PayloadAddresses {
        image: image.encoded_bytes().as_ptr(),
        media: image.media_type().unwrap().as_ptr(),
        names,
        keys,
    }
}

fn assert_finished(resources: &PostProcessResourceEvidence) {
    assert_eq!(resources.current_live_bytes(), 0);
    assert_eq!(resources.active_reservations(), 0);
    assert!(resources.committed_canonical_bytes() > 0);
}

#[test]
fn reflection_scales_thickness_and_signed_displacement_preserving_nonlength_truth() {
    let (input, evidence) = import_fixture(Lengths::SOURCE, 64).into_parts();
    assert_appearance(&input, Lengths::SOURCE, 64);
    let generation = input.generation();
    let addresses = payload_addresses(&input);
    let (scene, report) = convert_scene_space(input, options(0.5), PostProcessControl::default())
        .unwrap()
        .into_parts();
    assert_appearance(&scene, Lengths::DOUBLED, 64);
    assert_eq!(payload_addresses(&scene), addresses);
    assert_ne!(scene.generation(), generation);
    assert_eq!(report.disposition(), PostProcessRunDisposition::Changed);
    assert!(report.matches_scene(&scene));
    assert!(!evidence.matches_scene(&scene));
    assert_finished(report.resources());
    let effects = report.space_conversion_effects().unwrap();
    assert_eq!(effects.source(), TargetSpace::STANDARD);
    assert_eq!(effects.target(), options(0.5).target);
    assert!(effects.reverses_triangle_winding());
}

#[test]
fn signed_zeros_and_representable_subnormal_lengths_keep_exact_bits() {
    let source = Lengths {
        thickness: [-0.0, 0.0, f32::from_bits(2)],
        displacement: [-0.0, 0.0, -f32::from_bits(4)],
    };
    let expected = Lengths {
        thickness: [-0.0, 0.0, f32::from_bits(1)],
        displacement: [-0.0, 0.0, -f32::from_bits(2)],
    };
    let (input, evidence) = import_fixture(source, 64).into_parts();
    let addresses = payload_addresses(&input);
    let (scene, report) = convert_scene_space(input, options(2.0), PostProcessControl::default())
        .unwrap()
        .into_parts();
    assert_appearance(&scene, expected, 64);
    assert_eq!(payload_addresses(&scene), addresses);
    assert_finished(report.resources());
    let (restored, inverse) = convert_scene_space(
        scene,
        SpaceConversionOptions::default(),
        PostProcessControl::default(),
    )
    .unwrap()
    .into_parts();
    assert_appearance(&restored, source, 64);
    assert_eq!(payload_addresses(&restored), addresses);
    assert_finished(inverse.resources());
    assert!(!evidence.matches_scene(&restored));
}

#[test]
fn overflow_and_underflow_return_typed_material_or_binding_errors_and_the_original_scene() {
    for (displacement, value, units, numeric) in [
        (
            false,
            f32::MAX,
            0.5,
            NumericConversionError::NonFiniteOutput,
        ),
        (
            false,
            f32::from_bits(1),
            2.0,
            NumericConversionError::UnderflowToZero,
        ),
        (
            true,
            -f32::MAX,
            0.5,
            NumericConversionError::NonFiniteOutput,
        ),
        (
            true,
            -f32::from_bits(1),
            2.0,
            NumericConversionError::UnderflowToZero,
        ),
    ] {
        let mut lengths = Lengths::SOURCE;
        let expected = if displacement {
            lengths.displacement[1] = value;
            SpaceConversionIssue::NumericDisplacement {
                binding: 3,
                source: numeric,
            }
        } else {
            lengths.thickness[1] = value;
            SpaceConversionIssue::NumericMaterial {
                material: 1,
                source: numeric,
            }
        };
        let (input, evidence) = import_fixture(lengths, 64).into_parts();
        let generation = input.generation();
        let addresses = payload_addresses(&input);
        let limits = SceneSnapshotLimits::try_new(32 * 1024, 128, 1_000_000).unwrap();
        let snapshot = input.capture_snapshot(limits).unwrap();
        let failure =
            convert_scene_space(input, options(units), PostProcessControl::default()).unwrap_err();
        assert_eq!(failure.input().generation(), generation);
        assert!(evidence.matches_scene(failure.input()));
        assert_appearance(failure.input(), lengths, 64);
        assert_eq!(payload_addresses(failure.input()), addresses);
        assert_eq!(failure.input().capture_snapshot(limits).unwrap(), snapshot);
        let PostProcessRunError::SpaceConversion { failure } = failure.error() else {
            panic!("typed appearance space-conversion failure expected");
        };
        assert!(matches!(
            failure.cause(), SpaceConversionCause::Issue { issue } if *issue == expected
        ));
        let resources = failure.resources().unwrap();
        assert_eq!(resources.committed_canonical_bytes(), 0);
        assert_eq!(resources.current_live_bytes(), 0);
        assert_eq!(resources.active_reservations(), 0);
    }
}

#[test]
fn one_and_eight_mib_images_share_payloads_without_growing_conversion_allocations_or_work() {
    // Warm the runtime outside the measurement; imports are also outside each conversion probe.
    let input = import_fixture(Lengths::SOURCE, 64).into_parts().0;
    drop(convert_scene_space(input, options(0.5), PostProcessControl::default()).unwrap());
    let mut baseline = None;
    for image_len in [1024 * 1024, 8 * 1024 * 1024] {
        let (input, evidence) = import_fixture(Lengths::SOURCE, image_len).into_parts();
        let addresses = payload_addresses(&input);
        let mut outcome = None;
        let allocations = allocation_counter::measure(|| {
            outcome = Some(
                convert_scene_space(input, options(0.5), PostProcessControl::default()).unwrap(),
            );
        });
        let outcome = outcome.unwrap();
        assert_appearance(outcome.scene(), Lengths::DOUBLED, image_len);
        assert_eq!(payload_addresses(outcome.scene()), addresses);
        assert!(!evidence.matches_scene(outcome.scene()));
        let resources = outcome.report().resources();
        assert_finished(resources);
        assert!(allocations.count_total > 0);
        assert!(allocations.bytes_max > 0);
        assert!(resources.consumed_work_items() > 0);
        let measured = (
            allocations.count_total,
            allocations.bytes_total,
            allocations.bytes_max,
            resources.peak_live_bytes(),
            resources.peak_staging_bytes(),
            resources.committed_canonical_bytes(),
            resources.attempted_work_items(),
            resources.consumed_work_items(),
        );
        if let Some(baseline) = baseline {
            assert_eq!(
                measured, baseline,
                "conversion must not copy or walk encoded image bytes"
            );
        } else {
            baseline = Some(measured);
        }
    }
}

#[test]
fn repeated_inverse_conversions_restore_values_and_release_all_scene_and_report_allocations() {
    let run = || {
        let (mut scene, evidence) = import_fixture(Lengths::SOURCE, 64).into_parts();
        let addresses = payload_addresses(&scene);
        for _ in 0..3 {
            let (converted, report) =
                convert_scene_space(scene, options(0.5), PostProcessControl::default())
                    .unwrap()
                    .into_parts();
            assert_appearance(&converted, Lengths::DOUBLED, 64);
            assert_eq!(payload_addresses(&converted), addresses);
            assert_finished(report.resources());
            let (restored, inverse) = convert_scene_space(
                converted,
                SpaceConversionOptions::default(),
                PostProcessControl::default(),
            )
            .unwrap()
            .into_parts();
            assert_appearance(&restored, Lengths::SOURCE, 64);
            assert_eq!(payload_addresses(&restored), addresses);
            assert_finished(inverse.resources());
            assert!(!evidence.matches_scene(&restored));
            scene = restored;
        }
        let generation = scene.generation();
        let (scene, report) = convert_scene_space(
            scene,
            SpaceConversionOptions::default(),
            PostProcessControl::default(),
        )
        .unwrap()
        .into_parts();
        assert_eq!(scene.generation(), generation);
        assert_eq!(report.disposition(), PostProcessRunDisposition::Unchanged);
        assert_eq!(report.resources().committed_canonical_bytes(), 0);
        assert_eq!(report.resources().current_live_bytes(), 0);
        assert_eq!(report.resources().active_reservations(), 0);
        assert_appearance(&scene, Lengths::SOURCE, 64);
        assert_eq!(payload_addresses(&scene), addresses);
    };
    run();
    let allocations = allocation_counter::measure(run);
    assert!(allocations.count_total > 0);
    assert_eq!(allocations.count_current, 0);
    assert_eq!(allocations.bytes_current, 0);
}

#[test]
fn ordinary_host_unwind_releases_converted_rows_shared_payloads_and_detached_reports() {
    const PAYLOAD: &str = "host unwind while owning converted appearance and detached evidence";
    let run = || {
        let unwound = std::panic::catch_unwind(|| {
            let (input, evidence) = import_fixture(Lengths::SOURCE, 64).into_parts();
            let (scene, report) =
                convert_scene_space(input, options(0.5), PostProcessControl::default())
                    .unwrap()
                    .into_parts();
            assert_appearance(&scene, Lengths::DOUBLED, 64);
            assert_finished(report.resources());
            std::hint::black_box((&scene, &report, &evidence));
            // Bypass the global hook so the probe measures the owners and caught host payload.
            std::panic::resume_unwind(Box::new(PAYLOAD));
        });
        let payload = unwound.expect_err("the host must unwind through its published owners");
        assert_eq!(payload.downcast_ref::<&str>(), Some(&PAYLOAD));
        drop(payload);
    };
    run();
    let allocations = allocation_counter::measure(run);
    assert!(allocations.count_total > 0);
    assert!(allocations.bytes_max > 0);
    assert_eq!(allocations.count_current, 0);
    assert_eq!(allocations.bytes_current, 0);
}
