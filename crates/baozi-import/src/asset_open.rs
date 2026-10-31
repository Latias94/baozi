//! Work admission shared by primary and dependency acquisition.

#![allow(
    clippy::result_large_err,
    reason = "acquisition failures preserve bounded ledger and control evidence without a failure-path allocation"
)]

use std::panic::{AssertUnwindSafe, catch_unwind};

use baozi_io::{AssetIo, AssetOpenWorkError, AssetPath, OpenedAsset};

use crate::error::{ScopeAbortCause, TrustedExtensionBoundary};
use crate::format::discard_untrusted_panic_payload;
use crate::ledger::ImportDimension;
use crate::store::{ImportObservedWorkFailure, ImportStoreHost};

#[derive(Clone, Copy)]
pub(crate) enum AssetOpenContext {
    Primary,
    Dependency,
}

impl AssetOpenContext {
    const fn bound_panic_code(self) -> &'static str {
        match self {
            Self::Primary => "primary-open-work-panicked",
            Self::Dependency => "asset-open-work-panicked",
        }
    }

    const fn open_panic_code(self) -> &'static str {
        match self {
            Self::Primary => "primary-open-panicked",
            Self::Dependency => "asset-open-panicked",
        }
    }

    const fn bound_error_code(self, cause: AssetOpenWorkError) -> &'static str {
        match (self, cause) {
            (Self::Primary, AssetOpenWorkError::Unbounded) => "primary-open-work-unbounded",
            (Self::Primary, AssetOpenWorkError::Overflow) => "primary-open-work-overflow",
            (Self::Dependency, AssetOpenWorkError::Unbounded) => "asset-open-work-unbounded",
            (Self::Dependency, AssetOpenWorkError::Overflow) => "asset-open-work-overflow",
        }
    }
}

/// The caller owns identity checking and the separate attempted/successful-open counters.
/// Only a cache miss enters this function. The bound and open use the same borrowed backend/path;
/// no admission authority, lifetime token, or completion callback escapes into the backend.
pub(crate) fn open_asset(
    host: ImportStoreHost<'_>,
    io: &dyn AssetIo,
    path: &AssetPath,
    context: AssetOpenContext,
) -> Result<OpenedAsset, ScopeAbortCause> {
    let bound = catch_unwind(AssertUnwindSafe(|| io.open_work_bound(path)))
        .map_err(|payload| {
            discard_untrusted_panic_payload(payload);
            extension_failure(context.bound_panic_code())
        })?
        .map_err(|cause| extension_failure(context.bound_error_code(cause)))?
        .get();

    // Admission polls control again after the pure bound query, before any opaque backend IO.
    let opened = host
        .with_observed_work([(ImportDimension::WorkUnits, bound)], |work| {
            let opened = catch_unwind(AssertUnwindSafe(|| io.open(path))).map_err(|payload| {
                discard_untrusted_panic_payload(payload);
                extension_failure(context.open_panic_code())
            })?;
            // Returned errors complete an opaque call too. Panic leaves only attempted work.
            work.complete([bound])
                .map_err(|cause| cause.into_scope_abort_cause())?;
            Ok(opened)
        })
        .map_err(|cause| match cause {
            ImportObservedWorkFailure::Store(cause) => cause.into_scope_abort_cause(),
            ImportObservedWorkFailure::Action(cause) => cause,
        })?;
    opened.map_err(ScopeAbortCause::AssetOpen)
}

fn extension_failure(code: &'static str) -> ScopeAbortCause {
    ScopeAbortCause::TrustedExtension {
        boundary: TrustedExtensionBoundary::AssetIo,
        code,
    }
}
