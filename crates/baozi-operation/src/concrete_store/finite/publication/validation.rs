use super::*;

/// Every success-validation branch retains the original finite publication fields.
#[must_use = "the finite validation result retains final publication authority"]
pub enum FiniteStoreSuccessValidation<'inv, O, PP, U, CK, CT, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    Valid(ValidatedFiniteStoreSuccess<'inv, O, PP, U, CK, CT, EK, ET>),
    Rejected(FiniteStoreFailureAssembly<'inv, O, PP, U, EK, ET>),
    FallbackOnly(FiniteStoreFallbackOnly<'inv, O, PP, U>),
}

/// Controlled validation preserves both the writable owner and original verified work error.
#[must_use = "the controlled finite result retains final publication authority"]
pub enum FiniteStoreControlledSuccessValidation<
    'inv,
    O,
    PP,
    U,
    CK,
    CT,
    EK,
    ET,
    L = FreshStoreLineage,
> where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    InvalidWorkDimension {
        assembly: FiniteRetainedStorePublicationAssembly<'inv, O, PP, U, L>,
        cause: StoreRuntimeError,
    },
    Valid(ValidatedFiniteStoreSuccess<'inv, O, PP, U, CK, CT, EK, ET>),
    Rejected(FiniteStoreFailureAssembly<'inv, O, PP, U, EK, ET>),
    Interrupted {
        failure: FiniteStoreFailureAssembly<'inv, O, PP, U, EK, ET>,
        cause: StoreWorkError,
    },
    FallbackOnly(FiniteStoreFallbackOnly<'inv, O, PP, U>),
}

/// Exact evidence validation retains finite authority even when its graph is rejected.
#[must_use = "the finite failure result must be prepared or deliberately dropped"]
pub enum FiniteStoreFailureValidation<'inv, O, PP, U, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    Valid(ValidatedFiniteStoreFailure<'inv, O, PP, U, EK, ET>),
    Rejected(FiniteStoreFailureAssembly<'inv, O, PP, U, EK, ET>),
    FallbackOnly(FiniteStoreFallbackOnly<'inv, O, PP, U>),
}

impl<'inv, O, PP, U, L> FinitePublicationTransition<'inv, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(super) fn bind_success_resolution<CK, CT, EK, ET>(
        self,
        resolution: Result<ordinary::StoreSuccessResolution, ordinary::StoreClosureDecision>,
        canonical_root: CanonicalStoreRef<CK, CT>,
        evidence_root: OutcomeEvidenceStoreRef<EK, ET>,
    ) -> FiniteStoreControlledSuccessValidation<'inv, O, PP, U, CK, CT, EK, ET, L> {
        let (fields, state, error) = self.finish();
        let resolution = match resolution {
            Ok(resolution) => resolution,
            Err(_decision) => {
                return FiniteStoreControlledSuccessValidation::FallbackOnly(
                    FiniteStoreFallbackOnly {
                        fields,
                        cause: PreparationFailureCode::domain(CONTAINMENT_FALLBACK_CODE),
                        provenance: ordinary::StoreSuccessFallbackProvenance::EarlyAssemblyLatch,
                        error,
                        original_freeze_control: None,
                    },
                );
            }
        };
        match resolution {
            ordinary::StoreSuccessResolution::Valid(proof) => {
                if error.is_some() {
                    return FiniteStoreControlledSuccessValidation::FallbackOnly(
                        FiniteStoreFallbackOnly {
                            fields,
                            cause: PreparationFailureCode::domain(CONTAINMENT_FALLBACK_CODE),
                            provenance:
                                ordinary::StoreSuccessFallbackProvenance::EarlyAssemblyLatch,
                            error,
                            original_freeze_control: None,
                        },
                    );
                }
                FiniteStoreControlledSuccessValidation::Valid(ValidatedFiniteStoreSuccess {
                    fields,
                    proof,
                    canonical_root,
                    evidence_root,
                })
            }
            ordinary::StoreSuccessResolution::Rejected {
                failure_cause,
                forced_cause,
            } => FiniteStoreControlledSuccessValidation::Rejected(FiniteStoreFailureAssembly {
                fields,
                state,
                failure_cause,
                forced_cause,
                evidence_root,
                error,
                original_freeze_control: None,
            }),
            ordinary::StoreSuccessResolution::Interrupted {
                failure_cause,
                cause,
            } => FiniteStoreControlledSuccessValidation::Interrupted {
                failure: FiniteStoreFailureAssembly {
                    fields,
                    state,
                    failure_cause,
                    forced_cause: Some(failure_cause),
                    evidence_root,
                    error,
                    original_freeze_control: None,
                },
                cause,
            },
            ordinary::StoreSuccessResolution::FallbackOnly { cause, provenance } => {
                FiniteStoreControlledSuccessValidation::FallbackOnly(FiniteStoreFallbackOnly {
                    fields,
                    cause,
                    provenance,
                    error,
                    original_freeze_control: None,
                })
            }
        }
    }
}
