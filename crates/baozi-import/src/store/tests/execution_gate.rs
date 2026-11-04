//! Concrete lifetime entry gates over the production store execution capability.

use super::*;
use crate::authoring::{
    AnimationChannelAuthoring, AnimationClipAuthoring, AnimationExtrapolation,
    AnimationInterpolation, AnimationTargetAuthoring, AnimationTimeDomain, AuthoringRange,
};
use crate::dependency::DependencyOriginIssuer;
use baozi_operation::__retained_store_integration::activation::{
    StoreInvocationError, StoreInvocationParser, StoreInvocationServices,
};

fn with_execution<R>(
    owner: &mut ImportStoreSessionOwner<'_>,
    action: impl for<'loan, 'op> FnOnce(&mut ImportRootExecution<'loan, 'op>) -> R,
) -> R {
    owner
        .with_root_execution(|exec| {
            let result = action(exec);
            exec.poll()
                .expect("region success must pass the existing control checkpoint");
            Ok::<_, ImportStoreHostError>(result)
        })
        .expect("every actual parser channel must resolve before region exit")
}

struct SessionVec<'session, T> {
    values: AccountedVec<T>,
    issuer: &'session DependencyOriginIssuer,
}

impl<'session, T> SessionVec<'session, T> {
    fn new(issuer: &'session DependencyOriginIssuer) -> Self {
        Self {
            values: AccountedVec::new(),
            issuer,
        }
    }

    fn reserve_capacity(
        &mut self,
        issuer: &DependencyOriginIssuer,
        services: &mut StoreInvocationServices<
            '_,
            '_,
            ImportOperationOwner,
            ImportProgressPhase,
            ImportProgressUnit,
        >,
        phase: StorePhaseKey,
        dimension: StoreDimensionKey,
        required_capacity: usize,
    ) {
        assert!(
            std::ptr::eq(self.issuer, issuer),
            "storage growth must use the independent originating session identity"
        );
        self.values
            .try_reserve_capacity(required_capacity, "p4-execution-gate", |bytes| {
                let raw = services
                    .reserve_retained(phase, dimension, bytes, 1)
                    .expect("the same invocation must admit the real retained backing");
                Ok(ImportStoreReservation {
                    raw: Some(raw),
                    bytes,
                })
            })
            .expect("the existing allocator must admit and qualify the real backing capacity");
    }
}

#[test]
#[allow(
    clippy::result_large_err,
    reason = "the lifetime test forwards actual inline parser causes while preserving physical storage ownership"
)]
fn concrete_parser_channels_return_owned_storage_before_scene_submission() {
    let mut invocation_root = ImportStoreInvocationRoot::new();
    let (prepared, account) = run_scene_body(
        &mut invocation_root,
        ResourceLimits::default(),
        ImportOperationControl::default(),
        SceneParseOutcome::Accepted,
        |owner, account| {
            let issuer = DependencyOriginIssuer::new();
            let baseline = account.snapshot();
            let parent_dimensions = [owner.vocabulary.dimension(ImportDimension::Records)];
            let child_dimensions = [ImportDimension::Tokens, ImportDimension::DecodeWork]
                .map(|dimension| owner.vocabulary.dimension(dimension));
            let storage_phase = owner
                .vocabulary
                .phase(ImportLedgerPhase::FormatNormalization);
            let storage_dimension = owner.vocabulary.dimension(ImportDimension::ScratchBytes);

            let decoded = with_execution(owner, |exec| {
                let parent_group = exec
                    .raw
                    .bind_group(parent_dimensions)
                    .expect("the semantic parent layout must bind once");
                let child_group = exec
                    .raw
                    .bind_group(child_dimensions)
                    .expect("the repeated child layout must bind once");
                exec.raw
                    .parser_parent::<_, StoreInvocationError, 1>(
                        parent_group,
                        [2],
                        [None],
                        |parent| {
                            let mut decoded = SessionVec::new(&issuer);
                            let mut append = |child: &mut StoreInvocationParser<
                                '_,
                                '_,
                                '_,
                                ImportOperationOwner,
                                ImportProgressPhase,
                                ImportProgressUnit,
                                2,
                                true,
                            >,
                                              value| {
                                assert_eq!(child.remaining(), [1, 1]);
                                let previous_capacity = decoded.values.capacity();
                                let previous_len = decoded.values.len();
                                decoded.reserve_capacity(
                                    &issuer,
                                    &mut child.services(),
                                    storage_phase,
                                    storage_dimension,
                                    previous_capacity + 1,
                                );
                                assert!(decoded.values.capacity() > previous_capacity);
                                assert_eq!(decoded.values.len(), previous_len);
                                child
                                    .run_prefix([1, 1], 1, || {
                                        decoded.values.push_reserved_infallible(value);
                                        Ok::<_, StoreInvocationError>(())
                                    })
                                    .expect(
                                        "the callback must immediately record its actual physical prefix",
                                    );
                            };

                            for value in [7_u8, 11] {
                                parent
                                    .child::<_, StoreInvocationError, 2>(
                                        child_group,
                                        [1, 1],
                                        [None, None],
                                        |child| {
                                            append(child, value);
                                            assert_eq!(child.remaining(), [0, 0]);
                                            Ok(())
                                        },
                                    )
                                    .expect(
                                        "each child must close before the next child or its parent",
                                    );
                            }

                            parent
                                .checkpoint()
                                .expect("parent completion must follow a full checkpoint");
                            parent
                                .complete_parent()
                                .expect("the semantic parent must complete after its children");
                            Ok(decoded)
                        },
                    )
                    .expect("the parent must resolve on the sole work owner")
            });

            assert_eq!(&decoded.values[..], &[7, 11]);
            assert!(std::ptr::eq(decoded.issuer, &issuer));
            let retained = account.snapshot();
            assert_eq!(
                retained.live_retained_bytes(),
                baseline.live_retained_bytes() + decoded.values.capacity_bytes()
            );
            assert_eq!(
                retained.live_retained_allocations(),
                baseline.live_retained_allocations() + 1
            );

            submit_two_node(owner).expect(
                "the execution borrow must end before actual Scene construction submission",
            );
            drop(decoded);
        },
    );
    let terminal = prepared.unwrap().handoff();
    assert!(terminal.is_success());
    for dimension in [
        ImportDimension::Records,
        ImportDimension::Tokens,
        ImportDimension::DecodeWork,
    ] {
        let evidence = terminal
            .ledger_evidence()
            .dimension_by_semantic(import_dimension_semantic_id(dimension))
            .expect("the terminal ledger must retain the actual parser channel accounting");
        assert_eq!((evidence.attempted(), evidence.consumed()), (2, 2));
    }
    drop(terminal);
    let released = account.snapshot();
    assert_eq!(released.live_retained_bytes(), 0);
    assert_eq!(released.live_retained_allocations(), 0);
    assert!(!released.poisoned());
}

#[test]
fn staged_animation_descriptors_borrow_outer_payloads_before_complete_submission() {
    let mut invocation_root = ImportStoreInvocationRoot::new();
    let (prepared, account) = run_scene_body(
        &mut invocation_root,
        ResourceLimits::default(),
        ImportOperationControl::default(),
        SceneParseOutcome::Accepted,
        |owner, _account| {
            let issuer = DependencyOriginIssuer::new();
            let storage_phase = owner
                .vocabulary
                .phase(ImportLedgerPhase::FormatNormalization);
            let storage_dimension = owner.vocabulary.dimension(ImportDimension::ScratchBytes);
            let (times, values) = with_execution(owner, |exec| {
                let mut times = SessionVec::new(&issuer);
                times.reserve_capacity(
                    &issuer,
                    &mut exec.raw.services(),
                    storage_phase,
                    storage_dimension,
                    2,
                );
                times
                    .values
                    .try_extend_from_slice_reserved(&[0.0_f64, 1.0])
                    .expect("sample times must use admitted payload storage");
                let mut values = SessionVec::new(&issuer);
                values.reserve_capacity(
                    &issuer,
                    &mut exec.raw.services(),
                    storage_phase,
                    storage_dimension,
                    6,
                );
                values
                    .values
                    .try_extend_from_slice_reserved(&[0.0_f32, 0.0, 0.0, 1.0, 2.0, 3.0])
                    .expect("translation samples must use admitted payload storage");
                (times, values)
            });

            let (clips, channels) = with_execution(owner, |exec| {
                let mut channels = SessionVec::new(&issuer);
                channels.reserve_capacity(
                    &issuer,
                    &mut exec.raw.services(),
                    storage_phase,
                    storage_dimension,
                    1,
                );
                channels
                    .values
                    .push_reserved_infallible(AnimationChannelAuthoring::new(
                        0,
                        AnimationTargetAuthoring::NodeTranslation(NodeLocalId::new(0)),
                        AnimationInterpolation::Linear,
                        AnimationExtrapolation::Constant,
                        AnimationExtrapolation::Constant,
                        &times.values,
                        &values.values,
                        Presence::Absent,
                        Presence::Absent,
                    ));
                let mut clips = SessionVec::new(&issuer);
                clips.reserve_capacity(
                    &issuer,
                    &mut exec.raw.services(),
                    storage_phase,
                    storage_dimension,
                    1,
                );
                clips
                    .values
                    .push_reserved_infallible(AnimationClipAuthoring::new(
                        0,
                        1.0,
                        AnimationTimeDomain::Seconds,
                        AuthoringRange::new(0, 1),
                    ));
                (clips, channels)
            });

            // The batch borrows outer descriptor owners, which in turn borrow outer retained samples.
            let animations = AnimationAuthoringBatch::new(&clips.values, &channels.values);
            let roots = [NodeLocalId::new(0)];
            let nodes = [NodeAuthoringNode::new(
                Presence::Absent,
                AuthoredTransform::identity_trs(),
                Presence::Absent,
                Presence::Absent,
            )];
            let provenance = [ProvenanceLocalId::new(0)];
            let operation_kind = OperationKind::Generated;
            let provenance_nodes = [ProvenanceNodeAuthoring::operation(
                ProvenanceOperationAuthoring::new(
                    &operation_kind,
                    OptionsDigest::sha256([0; 32]),
                    &[],
                    &[],
                ),
            )];
            let owner_provenance = [
                ProfileOwner::Scene,
                ProfileOwner::SyntheticRoot,
                ProfileOwner::Node(NodeLocalId::new(0)),
                ProfileOwner::AnimationClip(0),
                ProfileOwner::AnimationChannel(0),
            ]
            .map(|owner| OwnerProvenanceAuthoring::new(owner, &provenance));
            let batch = SceneAuthoringBatch::new(
                SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &[], &[]),
                NodeAuthoringBatch::new(Presence::Present(&roots), &nodes),
                AppearanceAuthoringBatch::empty(),
                GeometryAuthoringBatch::empty(),
                DeformationAuthoringBatch::empty(),
                animations,
                ProfileAuthoringBatch::new(
                    LineageAuthoringBatch::new(&[], &[], &provenance_nodes, &owner_provenance),
                    MetadataAuthoringBatch::empty(),
                ),
            );
            let construction = SceneConstruction::try_new(batch, SceneValidationLimits::default())
                .expect("the staged buffers must form an actual complete Scene construction");
            owner
                .submit_scene(construction)
                .expect("complete borrowed input must submit after both execution regions end");
            drop((clips, channels));
            drop((times, values));
        },
    );
    let terminal = prepared.unwrap().handoff();
    let ImportStoreTerminalOutcome::Success { scene, .. } = &terminal else {
        panic!("the actual complete authoring batch must publish a canonical Scene")
    };
    let clip = scene
        .animation_clips()
        .next()
        .expect("the submitted animation must survive descriptor and payload destruction");
    let channel = clip
        .channel(0)
        .expect("the actual channel must be retained");
    assert_eq!(channel.key_times(), &[0.0, 1.0]);
    assert_eq!(channel.values(), &[0.0, 0.0, 0.0, 1.0, 2.0, 3.0]);
    drop(terminal);
    let released = account.snapshot();
    assert_eq!(released.live_retained_bytes(), 0);
    assert_eq!(released.live_retained_allocations(), 0);
    assert!(!released.poisoned());
}
