use baozi_operation::{
    OperationControl, PhysicalStorageAccount, PhysicalStorageAccountFactory,
    PhysicalStorageAccountLimits,
};
use thiserror::Error;

use crate::SceneValidationLimits;
use crate::identity::{ReservedSceneGeneration, SceneGenerationError, SceneGenerationId};

use super::{
    PendingSceneDraftOwner, PreparedSceneConstruction, SceneDraft, SceneDraftBuild,
    ValidatedSceneConstruction,
};
use crate::canonical_scene::store::construction::node_scene::NodeSceneConstructionRejection;
use crate::canonical_scene::store::construction::{
    SceneConstructionError, SceneConstructionLimits, SceneConstructionOperation,
    SceneConstructionPhase, SceneConstructionStartError, SceneConstructionUnit,
};
use crate::canonical_scene::store::production::SceneAuthoringBatch;

/// Opaque, single-use continuation for one canonical Scene construction.
///
/// The continuation owns the private physical account before authoring starts, then owns either the
/// exact retained draft state or an already prepared failure. A parser can receive only the
/// descriptor returned by [`Self::finish_draft`]; it cannot extract the retained owner, physical
/// account, operation control, or ledger state.
#[must_use = "a Scene construction must be prepared or deliberately dropped"]
pub(crate) struct SceneConstruction<'account> {
    state: SceneConstructionState<'account>,
}

#[allow(
    clippy::large_enum_variant,
    reason = "retained owners stay inline so transition and failure containment do not allocate"
)]
enum SceneConstructionState<'account> {
    Ready(SceneConstructionReady<'account>),
    Transitioning,
    Draft(PendingSceneDraftOwner),
    Failure(PreparedSceneConstruction),
}

struct SceneConstructionReady<'account> {
    account: SceneConstructionAccount<'account>,
    limits: SceneConstructionLimits,
    validation_limits: SceneValidationLimits,
    control: OperationControl<SceneConstructionPhase, SceneConstructionUnit>,
}

enum SceneConstructionAccount<'account> {
    Private(PhysicalStorageAccount),
    Explicit(&'account PhysicalStorageAccount),
}

impl SceneConstructionAccount<'_> {
    const fn as_account(&self) -> &PhysicalStorageAccount {
        match self {
            Self::Private(account) => account,
            Self::Explicit(account) => account,
        }
    }
}

/// Failure observed while a parser asks the continuation to materialize its one draft.
#[derive(Debug, Error)]
pub(crate) enum SceneConstructionFinishError {
    #[error("the canonical Scene construction already left its ready state")]
    AlreadyFinished,
    #[error("the canonical Scene construction was left fail-closed by an interrupted transition")]
    TransitionFailedClosed,
    #[error("canonical Scene construction startup failed")]
    Start(#[source] SceneConstructionStartError),
    #[error("canonical Scene authoring validation rejected the producer input")]
    AuthoringRejected(#[source] SceneAuthoringRejection),
    #[error("canonical Scene materialization prepared a terminal failure")]
    MaterializationFailed,
    #[error("canonical Scene materialization failed closed")]
    Construction(#[source] SceneConstructionError),
}

/// Stable domain classification plus the exact core-owned authoring rejection chain.
#[derive(Debug)]
pub(crate) struct SceneAuthoringRejection {
    domain: SceneAuthoringDomain,
    source: NodeSceneConstructionRejection,
}

impl SceneAuthoringRejection {
    fn from_node_scene(source: NodeSceneConstructionRejection) -> Self {
        let domain = match &source {
            NodeSceneConstructionRejection::SpatialAuthoring(_) => SceneAuthoringDomain::Spatial,
            NodeSceneConstructionRejection::NodeAuthoring(_) => SceneAuthoringDomain::Node,
            NodeSceneConstructionRejection::AppearanceAuthoring(_) => {
                SceneAuthoringDomain::Appearance
            }
            NodeSceneConstructionRejection::GeometryAuthoring(_) => SceneAuthoringDomain::Geometry,
            NodeSceneConstructionRejection::DeformationAuthoring(_) => {
                SceneAuthoringDomain::Deformation
            }
            NodeSceneConstructionRejection::AnimationAuthoring(_) => {
                SceneAuthoringDomain::Animation
            }
            NodeSceneConstructionRejection::ProfileOwnerCatalog(_)
            | NodeSceneConstructionRejection::ProfileAuthoring(_) => SceneAuthoringDomain::Profile,
            NodeSceneConstructionRejection::Aggregate(_) => SceneAuthoringDomain::Aggregate,
            NodeSceneConstructionRejection::Planning(_) => SceneAuthoringDomain::Planning,
        };
        Self { domain, source }
    }

    pub(crate) const fn domain(&self) -> SceneAuthoringDomain {
        self.domain
    }
}

impl std::fmt::Display for SceneAuthoringRejection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.source.fmt(formatter)
    }
}

impl std::error::Error for SceneAuthoringRejection {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SceneAuthoringDomain {
    Spatial,
    Node,
    Appearance,
    Geometry,
    Deformation,
    Animation,
    Profile,
    Aggregate,
    Planning,
}

/// Invalid pairing between a completed continuation and the parser result presented to it.
#[derive(Debug, Error)]
pub(crate) enum SceneConstructionPrepareError {
    #[error("the parser attempted to prepare a Scene construction before finishing its draft")]
    DraftNotFinished,
    #[error("the canonical Scene construction was left fail-closed by an interrupted transition")]
    TransitionFailedClosed,
    #[error("canonical Scene terminal preparation failed closed")]
    Construction(#[source] SceneConstructionError),
}

impl SceneConstruction<'static> {
    /// Creates one private per-import physical account without exposing that account to the caller.
    pub(in crate::canonical_scene::store) fn try_new(
        factory: &PhysicalStorageAccountFactory,
        account_limits: PhysicalStorageAccountLimits,
        limits: SceneConstructionLimits,
        validation_limits: SceneValidationLimits,
        control: OperationControl<SceneConstructionPhase, SceneConstructionUnit>,
    ) -> Result<Self, SceneConstructionStartError> {
        let account = factory.try_create_account(account_limits)?;
        Ok(Self::from_account(
            SceneConstructionAccount::Private(account),
            limits,
            validation_limits,
            control,
        ))
    }
}

impl<'account> SceneConstruction<'account> {
    /// Uses one host-owned account as the aggregate retention domain without exposing it downstream.
    pub(in crate::canonical_scene::store) fn with_account(
        account: &'account PhysicalStorageAccount,
        limits: SceneConstructionLimits,
        validation_limits: SceneValidationLimits,
        control: OperationControl<SceneConstructionPhase, SceneConstructionUnit>,
    ) -> Self {
        Self::from_account(
            SceneConstructionAccount::Explicit(account),
            limits,
            validation_limits,
            control,
        )
    }

    fn from_account(
        account: SceneConstructionAccount<'account>,
        limits: SceneConstructionLimits,
        validation_limits: SceneValidationLimits,
        control: OperationControl<SceneConstructionPhase, SceneConstructionUnit>,
    ) -> Self {
        Self {
            state: SceneConstructionState::Ready(SceneConstructionReady {
                account,
                limits,
                validation_limits,
                control,
            }),
        }
    }

    /// Materializes exactly one producer batch and returns only its non-owning descriptor.
    pub(in crate::canonical_scene::store) fn finish_draft(
        &mut self,
        input: SceneAuthoringBatch<'_>,
    ) -> Result<SceneDraft, SceneConstructionFinishError> {
        let ready = self.take_ready()?;
        let SceneConstructionReady {
            account,
            limits,
            validation_limits,
            control,
        } = ready;
        let operation = SceneConstructionOperation::with_account_and_control_for_scene_input(
            account.as_account(),
            limits,
            control,
            input,
            validation_limits,
        )
        .map_err(SceneConstructionFinishError::Start)?;
        let validated = match operation.try_validate() {
            Ok(validated) => ValidatedSceneConstruction::from_node_scene(validated),
            Err(rejected) => {
                let (prepared, rejection) = rejected
                    .prepare_rich_failure()
                    .map_err(SceneConstructionFinishError::Construction)?;
                self.state = SceneConstructionState::Failure(PreparedSceneConstruction(prepared));
                return Err(SceneConstructionFinishError::AuthoringRejected(
                    SceneAuthoringRejection::from_node_scene(rejection),
                ));
            }
        };
        match validated
            .finish_draft()
            .map_err(SceneConstructionFinishError::Construction)?
        {
            SceneDraftBuild::Draft { draft, owner } => {
                self.state = SceneConstructionState::Draft(owner);
                Ok(draft)
            }
            SceneDraftBuild::Failure(failure) => {
                self.state = SceneConstructionState::Failure(failure.into());
                Err(SceneConstructionFinishError::MaterializationFailed)
            }
        }
    }

    /// Consumes the continuation with the parser's exact result.
    ///
    /// A retained draft accepts only its exact descriptor; a missing or foreign descriptor becomes
    /// an ordinary rich failure before generation reservation. An already prepared core failure is
    /// authoritative regardless of what an untrusted parser returns afterward.
    pub(in crate::canonical_scene::store) fn prepare(
        self,
        draft: Option<SceneDraft>,
    ) -> Result<PreparedSceneConstruction, SceneConstructionPrepareError> {
        self.prepare_with_generation_source(draft, SceneGenerationId::reserve)
    }

    fn prepare_with_generation_source(
        self,
        draft: Option<SceneDraft>,
        reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
    ) -> Result<PreparedSceneConstruction, SceneConstructionPrepareError> {
        match (self.state, draft) {
            (SceneConstructionState::Draft(owner), Some(draft)) => owner
                .prepare_with_generation_source(draft, reserve_generation)
                .map_err(SceneConstructionPrepareError::Construction),
            (SceneConstructionState::Failure(prepared), _) => Ok(prepared),
            (SceneConstructionState::Draft(owner), None) => owner
                .prepare_missing_draft()
                .map_err(SceneConstructionPrepareError::Construction),
            (SceneConstructionState::Ready(_), _) => {
                Err(SceneConstructionPrepareError::DraftNotFinished)
            }
            (SceneConstructionState::Transitioning, _) => {
                Err(SceneConstructionPrepareError::TransitionFailedClosed)
            }
        }
    }

    fn take_ready(
        &mut self,
    ) -> Result<SceneConstructionReady<'account>, SceneConstructionFinishError> {
        let state = std::mem::replace(&mut self.state, SceneConstructionState::Transitioning);
        match state {
            SceneConstructionState::Ready(ready) => Ok(ready),
            SceneConstructionState::Transitioning => {
                Err(SceneConstructionFinishError::TransitionFailedClosed)
            }
            state @ (SceneConstructionState::Draft(_) | SceneConstructionState::Failure(_)) => {
                self.state = state;
                Err(SceneConstructionFinishError::AlreadyFinished)
            }
        }
    }

    #[cfg(test)]
    fn panic_after_transition_for_test(&mut self) {
        let _ready = self.take_ready().expect("test construction starts ready");
        panic!("test panic after Scene construction state extraction");
    }
}

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use baozi_operation::{
        OperationControl, PhysicalStorageAccountFactoryLimits, PhysicalStorageAccountLimits,
        StorePlanError,
    };

    use crate::FormatId;
    use crate::canonical_scene::store::production::{
        AnimationAuthoringBatch, AppearanceAuthoringBatch, DeformationAuthoringBatch,
        GeometryAuthoringBatch, LineageAuthoringBatch, MetadataAuthoringBatch, NodeAuthoringBatch,
        NodeAuthoringNode, NodeLocalId, ObservedResourceAuthoring, OwnerProvenanceAuthoring,
        Presence, ProfileAuthoringBatch, ProfileOwner, ProvenanceLocalId, ProvenanceNodeAuthoring,
        ResourceIdentityLocalId, SceneAuthoringBatch, SessionOpaqueReason, SourceIdentityAuthoring,
        SourceIdentityLocalId, SpatialAuthoringBatch,
    };
    use crate::entity::AuthoredTransform;
    use crate::provenance::{ResourceIdentity, SourceObjectKind, SourceObjectLocator};
    use crate::scene_space::SceneSpace;

    use super::*;
    use crate::canonical_scene::store::api::SceneConstructionTerminal;

    const ROOTS: [NodeLocalId; 1] = [NodeLocalId::new(0)];
    const NODES: [NodeAuthoringNode; 1] = [NodeAuthoringNode::new(
        Presence::Absent,
        AuthoredTransform::identity_trs(),
        Presence::Absent,
        Presence::Absent,
    )];

    #[test]
    fn planning_rejection_has_a_distinct_authoring_domain() {
        let rejection = SceneAuthoringRejection::from_node_scene(
            NodeSceneConstructionRejection::Planning(StorePlanError::InvalidBlockTable),
        );

        assert_eq!(rejection.domain(), SceneAuthoringDomain::Planning);
    }

    struct ProfileFixtureVocabulary {
        resource: ResourceIdentity,
        format: FormatId,
        locator: SourceObjectLocator,
    }

    struct ProfileFixture {
        resources: [ObservedResourceAuthoring<'static>; 1],
        sources: [SourceIdentityAuthoring<'static>; 1],
        provenance: [ProvenanceNodeAuthoring<'static>; 1],
        owners: [OwnerProvenanceAuthoring<'static>; 3],
    }

    impl ProfileFixture {
        fn batch(&'static self) -> ProfileAuthoringBatch<'static> {
            ProfileAuthoringBatch::new(
                LineageAuthoringBatch::new(
                    &self.resources,
                    &self.sources,
                    &self.provenance,
                    &self.owners,
                ),
                MetadataAuthoringBatch::empty(),
            )
        }
    }

    fn profile_fixture() -> &'static ProfileFixture {
        static VOCABULARY: OnceLock<ProfileFixtureVocabulary> = OnceLock::new();
        static FIXTURE: OnceLock<ProfileFixture> = OnceLock::new();
        let vocabulary = VOCABULARY.get_or_init(|| ProfileFixtureVocabulary {
            resource: ResourceIdentity::new("fixtures/continuation/input")
                .expect("test profile resource identity"),
            format: FormatId::new("org.baozi.fixture").expect("test profile format"),
            locator: SourceObjectLocator::indexed(SourceObjectKind::Document, 0),
        });
        FIXTURE.get_or_init(|| {
            static SOURCE_PROVENANCE: [ProvenanceLocalId; 1] = [ProvenanceLocalId::new(0)];
            ProfileFixture {
                resources: [ObservedResourceAuthoring::session_opaque(
                    &vocabulary.resource,
                    SessionOpaqueReason::CallerLabel,
                )],
                sources: [SourceIdentityAuthoring::new(
                    ResourceIdentityLocalId::new(0),
                    &vocabulary.format,
                    &vocabulary.locator,
                )],
                provenance: [ProvenanceNodeAuthoring::source(SourceIdentityLocalId::new(
                    0,
                ))],
                owners: [
                    OwnerProvenanceAuthoring::new(ProfileOwner::Scene, &SOURCE_PROVENANCE),
                    OwnerProvenanceAuthoring::new(ProfileOwner::SyntheticRoot, &SOURCE_PROVENANCE),
                    OwnerProvenanceAuthoring::new(
                        ProfileOwner::Node(NodeLocalId::new(0)),
                        &SOURCE_PROVENANCE,
                    ),
                ],
            }
        })
    }

    fn factory(max_live_accounts: u64) -> PhysicalStorageAccountFactory {
        PhysicalStorageAccountFactory::try_new(
            PhysicalStorageAccountFactoryLimits::new(u64::MAX, max_live_accounts)
                .expect("Scene continuation test factory limits"),
        )
        .expect("Scene continuation test factory")
    }

    fn limits() -> SceneConstructionLimits {
        SceneConstructionLimits {
            operation_live_bytes: u64::MAX,
            dimension_live_bytes: u64::MAX,
            attempted_work: u64::MAX,
            consumed_work: u64::MAX,
            committed_output: u64::MAX,
            published_evidence: u64::MAX,
        }
    }

    fn scene_input() -> SceneAuthoringBatch<'static> {
        SceneAuthoringBatch::new(
            SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &[], &[]),
            NodeAuthoringBatch::new(Presence::Present(&ROOTS), &NODES),
            AppearanceAuthoringBatch::empty(),
            GeometryAuthoringBatch::empty(),
            DeformationAuthoringBatch::empty(),
            AnimationAuthoringBatch::empty(),
            profile_fixture().batch(),
        )
    }

    fn construction(
        factory: &PhysicalStorageAccountFactory,
        validation_limits: SceneValidationLimits,
    ) -> SceneConstruction<'static> {
        SceneConstruction::try_new(
            factory,
            PhysicalStorageAccountLimits::new(u64::MAX, 256)
                .expect("Scene continuation test account limits"),
            limits(),
            validation_limits,
            OperationControl::default(),
        )
        .expect("Scene continuation starts")
    }

    #[test]
    fn explicit_account_uses_the_same_continuation_lifecycle() {
        let factory = factory(1);
        let account = factory
            .try_create_account(
                PhysicalStorageAccountLimits::new(u64::MAX, 256)
                    .expect("explicit Scene continuation account limits"),
            )
            .expect("explicit Scene continuation account");
        let mut construction = SceneConstruction::with_account(
            &account,
            limits(),
            SceneValidationLimits::default(),
            OperationControl::default(),
        );

        let draft = construction
            .finish_draft(scene_input())
            .expect("explicit-account Scene draft");
        let terminal = construction
            .prepare(Some(draft))
            .expect("explicit-account Scene prepares")
            .handoff();
        let SceneConstructionTerminal::Success(parts) = &terminal else {
            panic!("explicit-account Scene must succeed");
        };
        assert!(parts.scene.account().is_same_account(&account));

        drop(terminal);
        assert_eq!(factory.snapshot().live_accounts(), 1);
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn startup_rejection_releases_the_private_account_and_seals_the_continuation() {
        let factory = factory(1);
        let mut construction =
            construction(&factory, SceneValidationLimits::default().with_max_nodes(0));

        assert!(matches!(
            construction.finish_draft(scene_input()),
            Err(SceneConstructionFinishError::Start(
                SceneConstructionStartError::InputEnvelope(_)
            ))
        ));
        assert_eq!(factory.snapshot().live_accounts(), 0);
        assert!(matches!(
            construction.finish_draft(scene_input()),
            Err(SceneConstructionFinishError::TransitionFailedClosed)
        ));
    }

    #[test]
    fn draft_descriptor_never_owns_retained_scene_storage() {
        let factory = factory(1);
        let mut construction = construction(&factory, SceneValidationLimits::default());

        let draft = construction
            .finish_draft(scene_input())
            .expect("single-node Scene draft");
        assert_eq!(factory.snapshot().live_accounts(), 1);

        drop(draft);
        assert_eq!(
            factory.snapshot().live_accounts(),
            1,
            "dropping the descriptor must not release the hidden retained owner"
        );

        drop(construction);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn continuation_finishes_once_without_losing_the_pending_owner() {
        let factory = factory(1);
        let mut construction = construction(&factory, SceneValidationLimits::default());
        let draft = construction
            .finish_draft(scene_input())
            .expect("single-node Scene draft");

        assert!(matches!(
            construction.finish_draft(scene_input()),
            Err(SceneConstructionFinishError::AlreadyFinished)
        ));
        assert_eq!(factory.snapshot().live_accounts(), 1);

        let terminal = construction
            .prepare(Some(draft))
            .expect("exact Scene draft prepares")
            .handoff();
        assert!(matches!(&terminal, SceneConstructionTerminal::Success(_)));
        drop(terminal);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn foreign_draft_rejects_before_generation_reservation() {
        let factory = factory(2);
        let mut first = construction(&factory, SceneValidationLimits::default());
        let mut second = construction(&factory, SceneValidationLimits::default());
        let first_draft = first
            .finish_draft(scene_input())
            .expect("first Scene draft");
        let second_draft = second
            .finish_draft(scene_input())
            .expect("second Scene draft");
        let reservations = AtomicUsize::new(0);

        let terminal = first
            .prepare_with_generation_source(Some(second_draft), || {
                reservations.fetch_add(1, Ordering::AcqRel);
                SceneGenerationId::reserve()
            })
            .expect("foreign draft becomes a prepared rich failure")
            .handoff();
        assert!(matches!(
            &terminal,
            SceneConstructionTerminal::RichFailure(_)
        ));
        assert_eq!(reservations.load(Ordering::Acquire), 0);

        drop(terminal);
        drop(first_draft);
        drop(second);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn missing_draft_preserves_a_rich_terminal_without_reserving_generation() {
        let factory = factory(1);
        let mut construction = construction(&factory, SceneValidationLimits::default());
        let draft = construction
            .finish_draft(scene_input())
            .expect("single-node Scene draft");
        let reservations = AtomicUsize::new(0);

        drop(draft);
        let terminal = construction
            .prepare_with_generation_source(None, || {
                reservations.fetch_add(1, Ordering::AcqRel);
                SceneGenerationId::reserve()
            })
            .expect("missing descriptor becomes a prepared rich failure")
            .handoff();
        assert!(matches!(
            &terminal,
            SceneConstructionTerminal::RichFailure(_)
        ));
        assert_eq!(reservations.load(Ordering::Acquire), 0);
        assert_eq!(factory.snapshot().live_accounts(), 1);

        drop(terminal);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn authoring_rejection_remains_owned_until_failure_handoff() {
        let factory = factory(1);
        let validation_limits = SceneValidationLimits::default().with_max_canonical_bytes(167);
        let mut construction = construction(&factory, validation_limits);

        let rejection = match construction.finish_draft(scene_input()) {
            Err(SceneConstructionFinishError::AuthoringRejected(rejection)) => rejection,
            _ => panic!("canonical-byte limit must reject node authoring"),
        };
        assert_eq!(rejection.domain(), SceneAuthoringDomain::Node);
        assert!(rejection.to_string().contains("node-domain"));
        assert_eq!(factory.snapshot().live_accounts(), 1);

        let terminal = construction
            .prepare(None)
            .expect("authoring rejection owns a prepared failure")
            .handoff();
        assert!(matches!(
            &terminal,
            SceneConstructionTerminal::RichFailure(_)
        ));
        drop(terminal);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn latched_failure_wins_over_an_unexpected_parser_draft() {
        let factory = factory(2);
        let mut failed = construction(
            &factory,
            SceneValidationLimits::default().with_max_canonical_bytes(167),
        );
        let mut foreign = construction(&factory, SceneValidationLimits::default());

        assert!(matches!(
            failed.finish_draft(scene_input()),
            Err(SceneConstructionFinishError::AuthoringRejected(_))
        ));
        let foreign_draft = foreign
            .finish_draft(scene_input())
            .expect("foreign construction draft");

        let terminal = failed
            .prepare(Some(foreign_draft))
            .expect("the first latched failure remains authoritative")
            .handoff();
        assert!(matches!(
            &terminal,
            SceneConstructionTerminal::RichFailure(_)
        ));
        assert_eq!(factory.snapshot().live_accounts(), 2);

        drop(terminal);
        drop(foreign);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn caught_transition_panic_leaves_no_reusable_authority() {
        let factory = factory(1);
        let mut construction = construction(&factory, SceneValidationLimits::default());

        let panic = catch_unwind(AssertUnwindSafe(|| {
            construction.panic_after_transition_for_test();
        }));
        assert!(panic.is_err());
        assert_eq!(factory.snapshot().live_accounts(), 0);
        assert!(matches!(
            construction.finish_draft(scene_input()),
            Err(SceneConstructionFinishError::TransitionFailedClosed)
        ));
        assert!(matches!(
            construction.prepare(None),
            Err(SceneConstructionPrepareError::TransitionFailedClosed)
        ));
    }
}
