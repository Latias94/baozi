//! Central-session adapter for validation of the actual retained typed profile.

use baozi_operation::__retained_store_integration::activation::CanonicalValidationSession;
use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit, StoreWorkError};

use crate::canonical_scene::store::production::ProfileOwnerLayout;
use crate::canonical_scene::store::typed_storage::profile::ProfileStorage;

pub(in crate::canonical_scene::store) use crate::canonical_scene::store::typed_storage::profile::validation::{
    ProfileMeteredCounts, ProfileMeteredScratch, TypedProfileValidation,
};

#[cfg(test)]
mod tests;

pub(super) fn validate_profile_graph_metered<O, PP, U>(
    session: &mut CanonicalValidationSession<'_, '_, '_, '_, O, PP, U>,
    profile: &ProfileStorage,
    counts: ProfileMeteredCounts,
    owners: ProfileOwnerLayout,
    maximum_depth: u32,
    scratch: &mut ProfileMeteredScratch<'_>,
) -> Result<Option<TypedProfileValidation>, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    profile.validate(session, counts, owners, maximum_depth, scratch)
}
