//! Actual retained ownership and lineage checks for appearance space conversion.

use super::*;
use crate::appearance::{
    AlphaMode, ColorInterpretation, MaterialFactors, MaterialSemantics, ShadingModel,
    SupplementalPbrFactors, TextureBindingSemantics, TextureBlendOperation, TextureChannelSwizzle,
    TextureRole, TextureStrength, TextureTransform,
};
use crate::canonical_scene::store::production::{
    ImageAuthoring, ImageLocalId, MaterialAuthoring, MaterialBindingAuthoring, SamplerAuthoring,
    SamplerLocalId, TextureAuthoring, TextureLocalId,
};
use crate::canonical_scene::store::typed_storage::appearance::validation::AppearanceMeteredCounts;
use crate::canonical_scene::store::typed_storage::edit::space::SceneSpacePlan;
use crate::space_conversion::{SpaceConversion, TargetSpace};
use crate::{SamplerState, Vec3};

fn source_appearance(account: &PhysicalStorageAccount, nonzero: bool) -> NodeSceneSuccessParts {
    let images = [ImageAuthoring::encoded(
        &[0xde, 0xad, 0xbe, 0xef],
        Presence::Absent,
    )];
    let samplers = [SamplerAuthoring::new(SamplerState::CANONICAL_DEFAULT)];
    let textures = [TextureAuthoring::new(
        ImageLocalId::new(0),
        SamplerLocalId::new(0),
    )];
    let bindings = [
        (TextureRole::Displacement, if nonzero { -2.0 } else { -0.0 }),
        (TextureRole::Normal, -0.0),
        (TextureRole::Displacement, 0.0),
    ]
    .map(|(role, strength)| {
        MaterialBindingAuthoring::new(
            TextureLocalId::new(0),
            TextureBindingSemantics::new(
                role,
                u16::MAX,
                if role == TextureRole::Normal {
                    TextureChannelSwizzle::RGB
                } else {
                    TextureChannelSwizzle::RED
                },
                ColorInterpretation::Data,
                TextureTransform::IDENTITY,
                TextureStrength::try_new(strength).unwrap(),
                TextureBlendOperation::Replace,
                None,
            ),
        )
    });
    let materials = std::array::from_fn::<_, 3, _>(|index| {
        let thickness = if index == 0 && nonzero { 4.0 } else { -0.0 };
        let factors = MaterialFactors::try_new_with_supplemental_pbr(
            Vec3::ZERO,
            None,
            None,
            None,
            SupplementalPbrFactors::try_new(0.0, 0.0, 0.0, thickness, Vec3::ZERO, 0.0).unwrap(),
        )
        .unwrap();
        MaterialAuthoring::new(
            AuthoringRange::new(index as u32, 1),
            MaterialSemantics::new(ShadingModel::Unlit, AlphaMode::Opaque, false, factors),
        )
        .with_name(["length", "normal", "zero"][index])
    });
    let appearance =
        AppearanceAuthoringBatch::new(&images, &samplers, &textures, &bindings, &materials);
    let owners = [
        ProfileOwner::Scene,
        ProfileOwner::SyntheticRoot,
        ProfileOwner::Node(NodeLocalId::new(0)),
        ProfileOwner::Image(0),
        ProfileOwner::Sampler(0),
        ProfileOwner::Texture(0),
        ProfileOwner::TextureBinding(0),
        ProfileOwner::TextureBinding(1),
        ProfileOwner::TextureBinding(2),
        ProfileOwner::Material(0),
        ProfileOwner::Material(1),
        ProfileOwner::Material(2),
    ];
    let resource = ResourceIdentity::new("fixtures/appearance/lengths").unwrap();
    let format = FormatId::new("org.baozi.fixture").unwrap();
    let locator = SourceObjectLocator::indexed(SourceObjectKind::Document, 0);
    let resources = [ObservedResourceAuthoring::session_opaque(
        &resource,
        SessionOpaqueReason::CallerLabel,
    )];
    let sources = [SourceIdentityAuthoring::new(
        ResourceIdentityLocalId::new(0),
        &format,
        &locator,
    )];
    // Distinct markers make own+Scene parent unions observable, even though the observed
    // document is shared. No test-only retained owner is constructed.
    let provenance = owners.map(|_| ProvenanceNodeAuthoring::source(SourceIdentityLocalId::new(0)));
    let markers: [_; 12] = std::array::from_fn(|index| [ProvenanceLocalId::new(index as u32)]);
    let owners: Vec<_> = owners
        .into_iter()
        .zip(&markers)
        .map(|(owner, marker)| OwnerProvenanceAuthoring::new(owner, marker))
        .collect();
    let input = SceneAuthoringBatch::new(
        SpatialAuthoringBatch::new(&SceneSpace::STANDARD_TARGET, &[], &[]),
        single_trs_batch(),
        appearance,
        GeometryAuthoringBatch::empty(),
        DeformationAuthoringBatch::empty(),
        AnimationAuthoringBatch::empty(),
        ProfileAuthoringBatch::new(
            LineageAuthoringBatch::new(&resources, &sources, &provenance, &owners),
            MetadataAuthoringBatch::empty(),
        ),
    );
    successful_parts(
        NodeSceneTestTransaction::with_account(
            account,
            test_limits(),
            input,
            SceneValidationLimits::default(),
        )
        .unwrap()
        .finish_draft()
        .unwrap()
        .publish_for_test()
        .unwrap(),
    )
}

fn double_lengths() -> SpaceConversion {
    let target = TargetSpace::try_new(
        crate::Handedness::Right,
        crate::Axis::PositiveY,
        crate::Axis::NegativeZ,
        0.5,
        crate::FrontFaceWinding::CounterClockwise,
    )
    .unwrap();
    SpaceConversion::try_new(&SceneSpace::STANDARD_TARGET, target, None).unwrap()
}

#[test]
fn changed_lengths_retain_only_rows_and_rebind_exact_owners_to_own_and_scene_parents() {
    let factory = test_factory();
    let account = test_account(&factory, 1_024);
    let source = source_appearance(&account, true);
    let space = SceneSpacePlan::try_new(
        &source.scene.tables,
        double_lengths(),
        &mut Control::default(),
    )
    .unwrap();
    assert!(space.appearance().changes_storage());
    assert_eq!(space.appearance().name_bytes(), 0);
    assert_eq!(space.appearance().work(), 9);
    assert!(
        !space
            .appearance()
            .validate_correspondence(&source.scene.tables.appearance, &mut Control::default())
            .unwrap()
    );
    let pending = BoundPendingNodeSceneEdit::try_space_conversion(
        &source.scene,
        space,
        OptionsDigest::sha256([51; 32]),
        &mut || Ok(()),
    )
    .unwrap();
    let plan = pending.counts.profile;
    assert_eq!(
        plan.selection_work, 0,
        "fixed-size rows use their existing owner action"
    );
    assert_eq!(
        plan.target_count.as_u32(),
        3,
        "Scene, one Material, one TextureBinding"
    );
    let output = retain_edit(pending);
    let tables = output.output();
    let before = &source.scene.tables.appearance;
    let after = &tables.appearance;
    assert!(!std::ptr::eq(before.rows.as_ref(), after.rows.as_ref()));
    assert!(std::ptr::eq(
        before.payloads.as_ref(),
        after.payloads.as_ref()
    ));
    assert_eq!(
        after.rows.as_ref().materials[0]
            .factors
            .supplemental_pbr()
            .thickness(),
        8.0
    );
    assert_eq!(after.rows.as_ref().bindings[0].strength.get(), -4.0);
    let counts =
        AppearanceMeteredCounts::for_storage(after.rows.as_ref(), after.payloads.as_ref()).unwrap();
    assert!(
        after
            .rows
            .as_ref()
            .validate(after.payloads.as_ref(), &mut Control::default(), counts)
            .unwrap()
    );
    for owner in [
        CanonicalProfileOwner::Material(0),
        CanonicalProfileOwner::TextureBinding(0),
    ] {
        let refs = attachments(&tables.profile, owner);
        let lineage = tables
            .profile
            .state
            .as_ref()
            .segments
            .last()
            .unwrap()
            .lineage
            .as_ref();
        assert_eq!(refs.len(), 1);
        let parents = lineage.provenance[refs[0].local() as usize]
            .parents
            .get(&lineage.parents)
            .unwrap();
        assert_eq!(parents.len(), 2);
        assert_eq!(
            parents,
            expected_parents(&source.scene, &[owner, CanonicalProfileOwner::Scene])
        );
    }
    for owner in [
        CanonicalProfileOwner::Material(1),
        CanonicalProfileOwner::Material(2),
        CanonicalProfileOwner::TextureBinding(1),
        CanonicalProfileOwner::TextureBinding(2),
        CanonicalProfileOwner::Image(0),
        CanonicalProfileOwner::Texture(0),
    ] {
        assert_eq!(
            attachments(&tables.profile, owner),
            attachments(&source.scene.tables.profile, owner)
        );
    }
    assert_valid_profile(&tables.profile, plan);
    let mut control = Control::default();
    assert!(space.validate_correspondence(tables, &mut control).unwrap());
    assert_eq!(control.0, space.correspondence_work());
    drop(source);
    assert_eq!(
        output.output().appearance.payloads.as_ref().image_bytes,
        [0xde, 0xad, 0xbe, 0xef]
    );
    drop(output);
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn unchanged_lengths_share_both_owners_and_equal_bytes_cannot_substitute_payload_identity() {
    let factory = test_factory();
    let account = test_account(&factory, 1_024);
    let source = source_appearance(&account, false);
    let sibling = source_appearance(&account, false);
    let space = SceneSpacePlan::try_new(
        &source.scene.tables,
        double_lengths(),
        &mut Control::default(),
    )
    .unwrap();
    let appearance = space.appearance();
    assert!(!appearance.changes_storage());
    let mut control = Control::default();
    assert!(
        appearance
            .validate_correspondence(&source.scene.tables.appearance, &mut control)
            .unwrap()
    );
    assert_eq!(control.0, appearance.correspondence_work());
    assert_eq!(
        source.scene.tables.appearance.payloads.as_ref().image_bytes,
        sibling
            .scene
            .tables
            .appearance
            .payloads
            .as_ref()
            .image_bytes
    );
    let mut control = Control::default();
    assert!(
        !appearance
            .validate_correspondence(&sibling.scene.tables.appearance, &mut control)
            .unwrap()
    );
    assert_eq!(
        control.0, 0,
        "a different actual payload owner fails in constant work"
    );
    let pending = BoundPendingNodeSceneEdit::try_space_conversion(
        &source.scene,
        space,
        OptionsDigest::sha256([52; 32]),
        &mut || Ok(()),
    )
    .unwrap();
    assert_eq!(pending.counts.profile.target_count.as_u32(), 1);
    let output = retain_edit(pending);
    assert!(std::ptr::eq(
        source.scene.tables.appearance.rows.as_ref(),
        output.output().appearance.rows.as_ref()
    ));
    assert!(std::ptr::eq(
        source.scene.tables.appearance.payloads.as_ref(),
        output.output().appearance.payloads.as_ref()
    ));
    drop(source);
    drop(sibling);
    drop(output);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn interrupted_appearance_retention_releases_new_rows_and_preserves_both_source_owners() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    struct Stop {
        armed: Arc<AtomicBool>,
        polls: Arc<AtomicUsize>,
        at: usize,
    }
    impl baozi_operation::CancellationSignal for Stop {
        fn is_cancelled(&self) -> bool {
            self.armed.load(Ordering::Relaxed)
                && self.polls.fetch_add(1, Ordering::Relaxed) >= self.at
        }
    }
    let factory = test_factory();
    let account = test_account(&factory, 1_024);
    let source = source_appearance(&account, true);
    let baseline = account.snapshot();
    let run = |at| {
        let space = SceneSpacePlan::try_new(
            &source.scene.tables,
            double_lengths(),
            &mut Control::default(),
        )
        .unwrap();
        let pending = BoundPendingNodeSceneEdit::try_space_conversion(
            &source.scene,
            space,
            OptionsDigest::sha256([53; 32]),
            &mut || Ok(()),
        )
        .unwrap();
        let polls = Arc::new(AtomicUsize::new(0));
        let armed = Arc::new(AtomicBool::new(false));
        let result = retain_controlled_edit(
            pending,
            OperationControl::default().with_cancellation(Stop {
                armed: armed.clone(),
                polls: polls.clone(),
                at,
            }),
            || armed.store(true, Ordering::Relaxed),
        );
        if at == usize::MAX {
            let output = result.expect("appearance row owner must fit its admitted recipe");
            assert!(!std::ptr::eq(
                output.output().appearance.rows.as_ref(),
                source.scene.tables.appearance.rows.as_ref()
            ));
            assert!(std::ptr::eq(
                output.output().appearance.payloads.as_ref(),
                source.scene.tables.appearance.payloads.as_ref()
            ));
            drop(output);
        } else {
            assert!(result.is_none(), "retention poll {at} must interrupt");
        }
        assert!(!account.snapshot().poisoned());
        assert_eq!(
            account.snapshot().live_retained_bytes(),
            baseline.live_retained_bytes()
        );
        assert_eq!(
            account.snapshot().live_retained_allocations(),
            baseline.live_retained_allocations()
        );
        assert_eq!(
            source.scene.tables.appearance.rows.as_ref().bindings[0]
                .strength
                .get(),
            -2.0
        );
        polls.load(Ordering::Relaxed)
    };
    let polls = run(usize::MAX);
    assert!(polls > 8, "expected separate retained-owner checkpoints");
    for at in 0..polls {
        run(at);
    }
    drop(source);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}
