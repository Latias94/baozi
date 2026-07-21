use super::account::{AdmissionCharge, AdmissionError, try_admit_locked};
use super::physical::{RetainedAdmissionPairPlan, StartupAccountOwners, StartupAdmissionGuard};
use super::{
    AccountLinearState, AccountPoisonEventWitness, AccountPoisonReason, AccountPoisoned,
    PhysicalStorageAccount, TerminalEvidenceKey, record_account_poison_event,
    terminal_evidence_catalog_is_qualified,
};
use crate::OperationOwner;
use crate::control::discard_untrusted_panic_payload;
use crate::ledger::{
    AttributionBatchCreateError, FallbackBatchBaseline, Ledger, LedgerCause,
    StartupAttributionGraph, StartupLedgerPlan,
};
use std::fmt;
use std::ops::{Deref, DerefMut};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::MutexGuard;

/// A single-use proof that operation startup linearized before account quarantine.
///
/// This capability deliberately cannot execute arbitrary work or release the admitted action. The
/// action remains part of the account's monotonic containment domain after the value is consumed.
#[must_use = "operation-start admission must reach the physical startup transition"]
pub(super) struct PendingOperationStartAdmission<'account> {
    account: &'account PhysicalStorageAccount,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OperationStartAdmissionError {
    AccountPoisoned(AccountPoisoned),
    AccountingOverflow,
}

/// Unforgeable capability consumed by the ledger graph's Ready-only runtime conversion.
pub(crate) struct ReadyStartupGraphSplitAuthority {
    private: (),
}

/// Unforgeable capability consumed by the physical Ready-only runtime conversion.
pub(super) struct ReadyStartupPhysicalSplitAuthority {
    private: (),
}

/// A fully started ledger that can initialize an operation coordinator without re-admission.
pub(crate) struct StartedLedger<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    ledger: Ledger<O, P, D>,
}

impl<O, P, D> StartedLedger<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    pub(crate) fn into_inner(self) -> Ledger<O, P, D> {
        self.ledger
    }
}

/// The exact pair of retained charges committed by one operation start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct StartupPhysicalManifest {
    fallback: AdmissionCharge,
    evidence: AdmissionCharge,
    aggregate: AdmissionCharge,
}

/// Pure, same-source proof for the two accounting planes of retained startup.
///
/// Construction checks the physical bytes against the qualified ledger attributions before an
/// account action, ledger, allocation, or account share can exist.
#[must_use = "the qualified startup plan must cross both accounting planes"]
pub(super) struct StartupCrossPlanePlan<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    ledger: StartupLedgerPlan<O, P, D>,
    physical: StartupPhysicalManifest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StartupCrossPlanePlanError {
    FallbackByteMismatch { qualified: u64, physical: u64 },
    EvidenceByteMismatch { qualified: u64, physical: u64 },
}

impl<O, P, D> StartupCrossPlanePlan<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    pub(super) fn try_new(
        ledger: StartupLedgerPlan<O, P, D>,
        physical: StartupPhysicalManifest,
    ) -> Result<Self, StartupCrossPlanePlanError> {
        let [fallback_bytes, evidence_bytes] = ledger.retained_bytes();
        if physical.fallback.bytes != fallback_bytes {
            return Err(StartupCrossPlanePlanError::FallbackByteMismatch {
                qualified: fallback_bytes,
                physical: physical.fallback.bytes,
            });
        }
        if physical.evidence.bytes != evidence_bytes {
            return Err(StartupCrossPlanePlanError::EvidenceByteMismatch {
                qualified: evidence_bytes,
                physical: physical.evidence.bytes,
            });
        }
        Ok(Self { ledger, physical })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StartupPhysicalManifestError {
    ByteOverflow,
    AllocationOverflow,
}

impl StartupPhysicalManifest {
    pub(super) fn try_new(
        fallback: AdmissionCharge,
        evidence: AdmissionCharge,
    ) -> Result<Self, StartupPhysicalManifestError> {
        let bytes = fallback
            .bytes
            .checked_add(evidence.bytes)
            .ok_or(StartupPhysicalManifestError::ByteOverflow)?;
        let allocations = fallback
            .allocations
            .checked_add(evidence.allocations)
            .ok_or(StartupPhysicalManifestError::AllocationOverflow)?;
        Ok(Self {
            fallback,
            evidence,
            aggregate: AdmissionCharge::new(bytes, allocations),
        })
    }
}

/// Four account owners whose share work must already be charged by the startup ledger graph.
///
/// Only the cross-plane transition can assemble this value, after ledger admission owns the
/// complete startup attribution graph.
pub(super) struct PreheldStartupAccountOwners {
    fallback_account: PhysicalStorageAccount,
    evidence_account: PhysicalStorageAccount,
    fallback_lease_owner: PhysicalStorageAccount,
    evidence_lease_owner: PhysicalStorageAccount,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StartupAccountOwnerRole {
    FallbackWitness,
    EvidenceWitness,
    FallbackLease,
    EvidenceLease,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StartupPhysicalNoMutation {
    ForeignStartAdmission,
    ForeignOwner(StartupAccountOwnerRole),
    RetainedOwnerOverflow,
    AccountingOverflow,
    ByteLimitExceeded { requested: u64, limit: u64 },
    AllocationLimitExceeded { requested: u64, limit: u64 },
}

pub(super) enum StartupPhysicalAdmissionFailure {
    NoMutation(StartupPhysicalNoMutation),
    Uncertain(UncertainStartupPhysicalAdmission),
}

/// The committed physical half of a complete cross-plane startup graph.
///
/// Dropping this value before the Ready split is an ownership fault. The four pre-held owners stay
/// intentionally unreachable so the committed physical charge cannot be released without moving
/// the ledger graph into its runtime baseline at the same boundary.
pub(super) struct ReadyStartupPhysicalAdmission<'account> {
    account: &'account PhysicalStorageAccount,
    manifest: StartupPhysicalManifest,
    owners: Option<PreheldStartupAccountOwners>,
}

/// Fail-closed containment for a physical transition whose commit point could not be observed.
pub(super) struct UncertainStartupPhysicalAdmission {
    manifest: StartupPhysicalManifest,
    owners: Option<PreheldStartupAccountOwners>,
    poison: AccountPoisonEventWitness,
}

/// The complete startup ownership graph after both accounting planes have committed.
///
/// Only the Ready consuming split can move its ledger, attribution baseline, and physical guard
/// into retained construction; neither accounting plane is independently extractable beforehand.
struct StartupCrossPlaneOwnership<'account, O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    ledger: Ledger<O, P, D>,
    graph: StartupAttributionGraph<O, P, D>,
    physical: ReadyStartupPhysicalAdmission<'account>,
}

/// Unique authority to pre-hold the four physical owners for one admitted ledger graph.
///
/// The graph is already structurally owned before the first refcount operation. No constructor for
/// `PreheldStartupAccountOwners` is reachable without consuming this authority.
struct StartupGraphPreholdAuthority<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    ledger: Ledger<O, P, D>,
    graph: StartupAttributionGraph<O, P, D>,
    manifest: StartupPhysicalManifest,
}

impl<O, P, D> StartupGraphPreholdAuthority<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn try_commit_physical<'account>(
        self,
        account: &'account PhysicalStorageAccount,
        admission: PendingOperationStartAdmission<'account>,
    ) -> Result<ReadyStartupCrossPlane<'account, O, P, D>, StartupCrossPlaneAdmissionFailure<O, P, D>>
    {
        let owners = PreheldStartupAccountOwners {
            fallback_account: PhysicalStorageAccount {
                inner: account.inner.share(),
            },
            evidence_account: PhysicalStorageAccount {
                inner: account.inner.share(),
            },
            fallback_lease_owner: PhysicalStorageAccount {
                inner: account.inner.share(),
            },
            evidence_lease_owner: PhysicalStorageAccount {
                inner: account.inner.share(),
            },
        };

        match account.try_admit_startup_physical(admission, self.manifest, owners) {
            Ok(physical) => Ok(ReadyStartupCrossPlane {
                ownership: Some(StartupCrossPlaneOwnership {
                    ledger: self.ledger,
                    graph: self.graph,
                    physical,
                }),
            }),
            Err(StartupPhysicalAdmissionFailure::NoMutation(error)) => {
                drop(self);
                Err(StartupCrossPlaneAdmissionFailure::PhysicalNoMutation(error))
            }
            Err(StartupPhysicalAdmissionFailure::Uncertain(physical)) => Err(
                StartupCrossPlaneAdmissionFailure::Uncertain(CrossPlaneAdmissionContainment {
                    ownership: Some(UncertainStartupCrossPlaneOwnership {
                        ledger: self.ledger,
                        graph: self.graph,
                        physical,
                    }),
                }),
            ),
        }
    }
}

impl<O, P, D> StartupCrossPlaneOwnership<'_, O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn contain(self) {
        let Self {
            ledger,
            graph,
            physical,
        } = self;
        std::mem::forget(ledger);
        std::mem::forget(graph);
        drop(physical);
    }
}

/// A startup pair that may cross the infallible runtime split exactly once.
#[must_use = "the ready startup pair must be split into runtime owners or contained"]
pub(super) struct ReadyStartupCrossPlane<'account, O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    ownership: Option<StartupCrossPlaneOwnership<'account, O, P, D>>,
}

impl<'account, O, P, D> ReadyStartupCrossPlane<'account, O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    /// Moves the complete Ready graph into the sole retained runtime representation.
    ///
    /// This transition performs no lock acquisition, allocation, refcount operation, or fallible
    /// work. Both accounting planes were admitted from the same qualified plan before this method
    /// can be called.
    #[allow(
        clippy::type_complexity,
        reason = "the split preserves each static runtime owner"
    )]
    pub(super) fn split(
        mut self,
    ) -> (
        StartedLedger<O, P, D>,
        FallbackBatchBaseline<O, P, D>,
        StartupAdmissionGuard<'account>,
    ) {
        let ownership = self
            .ownership
            .take()
            .expect("ready cross-plane startup owns its runtime graph");
        let StartupCrossPlaneOwnership {
            ledger,
            graph,
            physical,
        } = ownership;
        let baseline = graph.into_runtime_baseline(ReadyStartupGraphSplitAuthority { private: () });
        let physical =
            physical.into_runtime_guard(ReadyStartupPhysicalSplitAuthority { private: () });
        (StartedLedger { ledger }, baseline, physical)
    }
}

impl<O, P, D> Drop for ReadyStartupCrossPlane<'_, O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(ownership) = self.ownership.take() {
            ownership.contain();
        }
    }
}

struct UncertainStartupCrossPlaneOwnership<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    ledger: Ledger<O, P, D>,
    graph: StartupAttributionGraph<O, P, D>,
    physical: UncertainStartupPhysicalAdmission,
}

/// Fail-closed owner for a startup whose physical commit point could not be observed.
pub(super) struct CrossPlaneAdmissionContainment<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    ownership: Option<UncertainStartupCrossPlaneOwnership<O, P, D>>,
}

impl<O, P, D> CrossPlaneAdmissionContainment<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    pub(super) fn poisoned(&self) -> Option<AccountPoisoned> {
        self.ownership
            .as_ref()
            .map(|ownership| ownership.physical.poison.poisoned)
    }
}

impl<O, P, D> Drop for CrossPlaneAdmissionContainment<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(ownership) = self.ownership.take() {
            std::mem::forget(ownership.ledger);
            std::mem::forget(ownership.graph);
            std::mem::forget(ownership.physical);
        }
    }
}

#[allow(
    clippy::large_enum_variant,
    reason = "uncertain containment stays inline so the failure path never allocates"
)]
pub(super) enum StartupCrossPlaneAdmissionFailure<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    OperationStart(OperationStartAdmissionError),
    LedgerInitialization(LedgerCause<D>),
    TerminalEvidenceCatalogInvariant,
    LedgerAdmission(AttributionBatchCreateError<D>),
    PhysicalNoMutation(StartupPhysicalNoMutation),
    Uncertain(CrossPlaneAdmissionContainment<O, P, D>),
}

struct StartupPhysicalTransitionGuard<'account> {
    admission: PendingOperationStartAdmission<'account>,
    manifest: StartupPhysicalManifest,
    owners: PreheldStartupAccountOwners,
}

impl<'account> StartupPhysicalTransitionGuard<'account> {
    fn ready(self) -> ReadyStartupPhysicalAdmission<'account> {
        ReadyStartupPhysicalAdmission {
            account: self.admission.account,
            manifest: self.manifest,
            owners: Some(self.owners),
        }
    }

    fn uncertain(self, poison: AccountPoisonEventWitness) -> UncertainStartupPhysicalAdmission {
        UncertainStartupPhysicalAdmission {
            manifest: self.manifest,
            owners: Some(self.owners),
            poison,
        }
    }
}

impl<'account> ReadyStartupPhysicalAdmission<'account> {
    fn into_runtime_guard(
        mut self,
        authority: ReadyStartupPhysicalSplitAuthority,
    ) -> StartupAdmissionGuard<'account> {
        let owners = self
            .owners
            .take()
            .expect("ready physical startup owns all four preheld account handles");
        let PreheldStartupAccountOwners {
            fallback_account,
            evidence_account,
            fallback_lease_owner,
            evidence_lease_owner,
        } = owners;
        StartupAdmissionGuard::from_precommitted(
            self.account,
            RetainedAdmissionPairPlan {
                first: self.manifest.fallback,
                second: self.manifest.evidence,
                aggregate: self.manifest.aggregate,
            },
            StartupAccountOwners {
                fallback_account,
                evidence_account,
                fallback_lease_owner,
                evidence_lease_owner,
            },
            authority,
        )
    }
}

impl Drop for ReadyStartupPhysicalAdmission<'_> {
    fn drop(&mut self) {
        let Some(owners) = self.owners.as_ref() else {
            return;
        };
        let _ = owners
            .fallback_account
            .record_poison(AccountPoisonReason::OwnershipGraphUnverified);
        if let Some(owners) = self.owners.take() {
            std::mem::forget(owners);
        }
    }
}

impl Drop for UncertainStartupPhysicalAdmission {
    fn drop(&mut self) {
        if let Some(owners) = self.owners.take() {
            std::mem::forget(owners);
        }
    }
}

impl PhysicalStorageAccount {
    /// Admits one complete startup graph across the ledger and physical accounting planes.
    ///
    /// The operation-start gate always linearizes before ledger work. Once the graph exists, its
    /// unique ownership authorizes exactly four pre-held account owners without charging the old
    /// per-share path again.
    pub(super) fn try_prepare_startup_cross_plane<O, P, D>(
        &self,
        phases: &[P],
        plan: StartupCrossPlanePlan<O, P, D>,
    ) -> Result<ReadyStartupCrossPlane<'_, O, P, D>, StartupCrossPlaneAdmissionFailure<O, P, D>>
    where
        O: OperationOwner,
        P: TerminalEvidenceKey,
        D: TerminalEvidenceKey + fmt::Debug,
    {
        let admission = self
            .try_admit_operation_start()
            .map_err(StartupCrossPlaneAdmissionFailure::OperationStart)?;
        admission.try_prepare_startup_cross_plane(phases, plan)
    }

    pub(super) fn try_admit_operation_start(
        &self,
    ) -> Result<PendingOperationStartAdmission<'_>, OperationStartAdmissionError> {
        with_startup_account_lock(self, StartupLockKind::OperationGate, |state| {
            if let Some(error) = state.poisoned_error() {
                return Err(OperationStartAdmissionError::AccountPoisoned(error));
            }
            let admitted_account_actions = state
                .admitted_account_actions
                .checked_add(1)
                .ok_or(OperationStartAdmissionError::AccountingOverflow)?;
            state.admitted_account_actions = admitted_account_actions;
            Ok(PendingOperationStartAdmission { account: self })
        })
    }

    pub(super) fn try_admit_startup_physical<'account>(
        &'account self,
        admission: PendingOperationStartAdmission<'account>,
        manifest: StartupPhysicalManifest,
        owners: PreheldStartupAccountOwners,
    ) -> Result<ReadyStartupPhysicalAdmission<'account>, StartupPhysicalAdmissionFailure> {
        let guard = StartupPhysicalTransitionGuard {
            admission,
            manifest,
            owners,
        };
        let transition = catch_unwind(AssertUnwindSafe(|| {
            self.try_commit_startup_physical(&guard)
        }));
        match transition {
            Ok(Ok(())) => Ok(guard.ready()),
            Ok(Err(error)) => {
                drop(guard);
                Err(StartupPhysicalAdmissionFailure::NoMutation(error))
            }
            Err(payload) => {
                let poison = record_account_poison_event(
                    self,
                    AccountPoisonReason::OwnershipGraphUnverified,
                );
                discard_untrusted_panic_payload(payload);
                Err(StartupPhysicalAdmissionFailure::Uncertain(
                    guard.uncertain(poison),
                ))
            }
        }
    }

    fn try_commit_startup_physical(
        &self,
        guard: &StartupPhysicalTransitionGuard<'_>,
    ) -> Result<(), StartupPhysicalNoMutation> {
        if !self.is_same_account(guard.admission.account) {
            return Err(StartupPhysicalNoMutation::ForeignStartAdmission);
        }
        validate_startup_owner(
            self,
            &guard.owners.fallback_account,
            StartupAccountOwnerRole::FallbackWitness,
        )?;
        validate_startup_owner(
            self,
            &guard.owners.evidence_account,
            StartupAccountOwnerRole::EvidenceWitness,
        )?;
        validate_startup_owner(
            self,
            &guard.owners.fallback_lease_owner,
            StartupAccountOwnerRole::FallbackLease,
        )?;
        validate_startup_owner(
            self,
            &guard.owners.evidence_lease_owner,
            StartupAccountOwnerRole::EvidenceLease,
        )?;

        with_startup_account_lock(self, StartupLockKind::PhysicalTransition, |state| {
            let mut next = *state;
            next.admitted_retained_owners = next
                .admitted_retained_owners
                .checked_add(2)
                .ok_or(StartupPhysicalNoMutation::RetainedOwnerOverflow)?;
            try_admit_locked(
                &mut next.counters,
                self.limits().counter_limits(),
                guard.manifest.aggregate,
            )
            .map_err(map_startup_admission_error)?;

            inject_startup_transition_panic(StartupTransitionPanicPoint::BeforeCommit);
            *state = next;
            inject_startup_transition_panic(StartupTransitionPanicPoint::AfterCommit);
            Ok(())
        })
    }
}

impl<'account> PendingOperationStartAdmission<'account> {
    fn try_prepare_startup_cross_plane<O, P, D>(
        self,
        phases: &[P],
        plan: StartupCrossPlanePlan<O, P, D>,
    ) -> Result<ReadyStartupCrossPlane<'account, O, P, D>, StartupCrossPlaneAdmissionFailure<O, P, D>>
    where
        O: OperationOwner,
        P: TerminalEvidenceKey,
        D: TerminalEvidenceKey + fmt::Debug,
    {
        let account = self.account;
        let StartupCrossPlanePlan {
            ledger: qualified_ledger,
            physical: manifest,
        } = plan;
        let (limits, parts) = qualified_ledger.into_parts();
        let expected_dimensions = limits.dimension_count();
        let ledger = Ledger::try_new(limits, phases)
            .map_err(StartupCrossPlaneAdmissionFailure::LedgerInitialization)?;
        if !terminal_evidence_catalog_is_qualified(&ledger, expected_dimensions, phases.len()) {
            return Err(StartupCrossPlaneAdmissionFailure::TerminalEvidenceCatalogInvariant);
        }
        let graph = ledger
            .try_admit_startup_attribution_graph(parts)
            .map_err(StartupCrossPlaneAdmissionFailure::LedgerAdmission)?;
        StartupGraphPreholdAuthority {
            ledger,
            graph,
            manifest,
        }
        .try_commit_physical(account, self)
    }
}

fn validate_startup_owner(
    expected: &PhysicalStorageAccount,
    actual: &PhysicalStorageAccount,
    role: StartupAccountOwnerRole,
) -> Result<(), StartupPhysicalNoMutation> {
    if expected.is_same_account(actual) {
        Ok(())
    } else {
        Err(StartupPhysicalNoMutation::ForeignOwner(role))
    }
}

fn map_startup_admission_error(error: AdmissionError) -> StartupPhysicalNoMutation {
    match error {
        AdmissionError::AccountingOverflow => StartupPhysicalNoMutation::AccountingOverflow,
        AdmissionError::ByteLimitExceeded { requested, limit } => {
            StartupPhysicalNoMutation::ByteLimitExceeded { requested, limit }
        }
        AdmissionError::AllocationLimitExceeded { requested, limit } => {
            StartupPhysicalNoMutation::AllocationLimitExceeded { requested, limit }
        }
    }
}

#[derive(Clone, Copy)]
enum StartupLockKind {
    OperationGate,
    PhysicalTransition,
}

struct StartupAccountLock<'account> {
    state: MutexGuard<'account, AccountLinearState>,
    #[cfg(test)]
    probe: StartupLockDepthProbe,
}

impl Deref for StartupAccountLock<'_> {
    type Target = AccountLinearState;

    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl DerefMut for StartupAccountLock<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

fn with_startup_account_lock<T>(
    account: &PhysicalStorageAccount,
    _kind: StartupLockKind,
    action: impl FnOnce(&mut AccountLinearState) -> T,
) -> T {
    let state = super::account::lock_recovering_poison(&account.inner.value().linear);
    let mut lock = StartupAccountLock {
        state,
        #[cfg(test)]
        probe: StartupLockDepthProbe::enter(_kind),
    };
    action(&mut lock)
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct StartupLockProbeSnapshot {
    operation_gate_calls: usize,
    physical_transition_calls: usize,
    current_depth: usize,
    max_depth: usize,
}

#[cfg(test)]
thread_local! {
    static STARTUP_LOCK_PROBE: std::cell::Cell<StartupLockProbeSnapshot> =
        const { std::cell::Cell::new(StartupLockProbeSnapshot {
            operation_gate_calls: 0,
            physical_transition_calls: 0,
            current_depth: 0,
            max_depth: 0,
        }) };
}

#[cfg(test)]
struct StartupLockDepthProbe;

#[cfg(test)]
impl StartupLockDepthProbe {
    fn enter(kind: StartupLockKind) -> Self {
        STARTUP_LOCK_PROBE.with(|probe| {
            let mut snapshot = probe.get();
            match kind {
                StartupLockKind::OperationGate => snapshot.operation_gate_calls += 1,
                StartupLockKind::PhysicalTransition => snapshot.physical_transition_calls += 1,
            }
            snapshot.current_depth += 1;
            snapshot.max_depth = snapshot.max_depth.max(snapshot.current_depth);
            probe.set(snapshot);
        });
        Self
    }
}

#[cfg(test)]
impl Drop for StartupLockDepthProbe {
    fn drop(&mut self) {
        STARTUP_LOCK_PROBE.with(|probe| {
            let mut snapshot = probe.get();
            snapshot.current_depth -= 1;
            probe.set(snapshot);
        });
    }
}

#[cfg(test)]
fn reset_startup_lock_probe() {
    STARTUP_LOCK_PROBE.with(|probe| probe.set(StartupLockProbeSnapshot::default()));
}

#[cfg(test)]
fn startup_lock_probe_snapshot() -> StartupLockProbeSnapshot {
    STARTUP_LOCK_PROBE.with(std::cell::Cell::get)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StartupTransitionPanicPoint {
    BeforeCommit,
    AfterCommit,
}

#[cfg(test)]
thread_local! {
    static STARTUP_TRANSITION_PANIC: std::cell::Cell<Option<StartupTransitionPanicPoint>> =
        const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn inject_startup_transition_panic(point: StartupTransitionPanicPoint) {
    STARTUP_TRANSITION_PANIC.with(|injected| {
        if injected.get() == Some(point) {
            injected.set(None);
            std::panic::panic_any(point as u8);
        }
    });
}

#[cfg(not(test))]
fn inject_startup_transition_panic(_point: StartupTransitionPanicPoint) {}

#[cfg(test)]
fn set_startup_transition_panic(point: StartupTransitionPanicPoint) {
    STARTUP_TRANSITION_PANIC.with(|injected| injected.set(Some(point)));
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod cross_plane_tests;
