//! Ordinary vector/string growth shared by legacy and exclusive parser roles.
use super::*;
use crate::store::ImportOrdinaryStorage;

/// Short storage/poll borrow; returned containers retain only their session identity and token.
///
/// A session-owned container can leave a region and grow through the same session later:
/// ```
/// use baozi_import::{ParseContext, ParseFailure, ParseResult, MeteredVec, ImportStorageClass};
/// fn owned<'s>(c: &mut ParseContext<'s>) -> ParseResult<MeteredVec<'s, u32>> {
///     let mut values = c.with_parser_execution::<_, ParseFailure>(|root| {
///         let mut loan = root.storage()?;
///         let mut values = loan.metered_vec(ImportStorageClass::SceneConstruction, "owned-proof");
///         loan.reserve_vec(&mut values, 1)?;
///         Ok(values)
///     })?;
///     c.with_execution(|exec| values.try_push(exec, 7))?;
///     Ok(values)
/// }
/// ```
///
/// The temporary storage borrow cannot escape its region:
/// ```compile_fail
/// use baozi_import::{ParseContext, ParseFailure};
/// fn escape(c: &mut ParseContext<'_>) {
///     let _loan = c.with_parser_execution::<_, ParseFailure>(|root| root.storage());
/// }
/// ```
///
/// A live root loan excludes root reentry:
/// ```compile_fail
/// use baozi_import::{ParseRoot, ParseResult};
/// fn overlap(root: &mut ParseRoot<'_, '_, '_>) -> ParseResult<()> {
///     let mut loan = root.storage()?;
///     root.poll()?;
///     loan.poll()
/// }
/// ```
///
/// Parser loans support sequential reuse:
/// ```
/// use baozi_import::{ParseParser, ParseResult};
/// fn sequential(parser: &mut ParseParser<'_, '_, '_, '_, '_, 1, true>) -> ParseResult<()> {
///     { let mut loan = parser.storage()?; loan.poll()?; }
///     parser.poll()
/// }
/// ```
///
/// A live parser loan excludes parser reentry:
/// ```compile_fail
/// use baozi_import::{ParseParser, ParseResult};
/// fn overlap(parser: &mut ParseParser<'_, '_, '_, '_, '_, 1, true>) -> ParseResult<()> {
///     let mut loan = parser.storage()?;
///     parser.poll()?;
///     loan.poll()
/// }
/// ```
pub struct ParseStorage<'b, 'session: 'b> {
    raw: ImportOrdinaryStorage<'b>,
    identity: StorageIdentity<'session>,
    callback_active: &'b Cell<bool>,
}
impl<'b, 'session: 'b> ParseStorage<'b, 'session> {
    pub(super) fn new(
        raw: ImportOrdinaryStorage<'b>,
        identity: StorageIdentity<'session>,
        callback_active: &'b Cell<bool>,
    ) -> Self {
        Self {
            raw,
            identity,
            callback_active,
        }
    }
    fn preflight(&self) -> ParseResult<()> {
        if self.callback_active.get() {
            return Err(self.identity.abort(ScopeAbortCause::DescriptorContract {
                code: "parse-capability-reentered",
            }));
        }
        self.identity.preflight_active()
    }
    fn check_identity(&self, identity: StorageIdentity<'_>) -> ParseResult<()> {
        self.preflight()?;
        if !std::ptr::eq(self.identity.issuer, identity.issuer) {
            return Err(self.identity.abort(ScopeAbortCause::DescriptorContract {
                code: "parse-storage-foreign-session",
            }));
        }
        identity.preflight_active()
    }
    /// Performs the same actual poll; capacity-fit operations still reach it.
    pub fn poll(&mut self) -> ParseResult<()> {
        self.preflight()?;
        self.raw
            .poll()
            .map_err(|cause| self.identity.abort(cause.into_scope_abort_cause()))
    }
    fn reserve(
        &mut self,
        identity: StorageIdentity<'_>,
        class: ImportStorageClass,
        bytes: u64,
    ) -> ParseResult<Option<ImportStoreReservation>> {
        self.check_identity(identity)?;
        if bytes == 0 {
            return Ok(None);
        }
        let (phase, dimension) = class.accounting();
        self.raw
            .reserve(phase, dimension, bytes)
            .map(Some)
            .map_err(|cause| identity.abort(cause.into_scope_abort_cause()))
    }
    fn resize_exact(
        &mut self,
        identity: StorageIdentity<'_>,
        reservation: &mut Option<ImportStoreReservation>,
        bytes: u64,
    ) -> ParseResult<()> {
        self.check_identity(identity)?;
        match (reservation.as_mut(), bytes) {
            (None, 0) => Ok(()),
            (Some(_), 0) => Err(identity.abort(ScopeAbortCause::DescriptorContract {
                code: "metered-storage-unused-capacity-reservation",
            })),
            (Some(token), bytes) => self
                .raw
                .resize(token, bytes)
                .map_err(|cause| identity.abort(cause.into_scope_abort_cause())),
            (None, _) => Err(identity.abort(ScopeAbortCause::DescriptorContract {
                code: "metered-storage-capacity-without-reservation",
            })),
        }
    }
    /// Creates an empty session-owned vector without allocating or polling.
    pub fn metered_vec<T>(
        &mut self,
        class: ImportStorageClass,
        resource: &'static str,
    ) -> MeteredVec<'session, T> {
        MeteredVec {
            values: Vec::new(),
            capacity: None,
            identity: self.identity,
            class,
            resource,
        }
    }
    /// Creates an empty session-owned string without allocating or polling.
    pub fn metered_string(
        &mut self,
        class: ImportStorageClass,
        resource: &'static str,
    ) -> MeteredString<'session> {
        MeteredString {
            value: String::new(),
            capacity: None,
            identity: self.identity,
            class,
            resource,
        }
    }
    /// Admits and qualifies full replacement capacity while the source remains live.
    pub fn reserve_vec<T>(
        &mut self,
        values: &mut MeteredVec<'session, T>,
        additional: usize,
    ) -> ParseResult<()> {
        self.check_identity(values.identity)?;
        self.poll()?;
        let required = values
            .values
            .len()
            .checked_add(additional)
            .ok_or_else(|| values.identity.allocation_failure(values.resource))?;
        if required <= values.values.capacity() {
            return Ok(());
        }

        let (plan, replacement_plan) = metered_vec_replacement_plans::<T>(
            values.values.len(),
            values.values.capacity(),
            required,
        )
        .map_err(|cause| values.identity.capacity_error(values.resource, cause))?;
        let admitted = replacement_plan
            .envelope_delta_bytes()
            .map_err(|cause| values.identity.capacity_error(values.resource, cause))?;
        let mut token = self.reserve(values.identity, values.class, admitted)?;
        plan.verify_preflight(values.values.len(), values.values.capacity())
            .map_err(|cause| values.identity.capacity_error(values.resource, cause))?;
        replacement_plan
            .verify_preflight(0, 0)
            .map_err(|cause| values.identity.capacity_error(values.resource, cause))?;
        let mut replacement = Vec::new();
        if fail_next_metered_allocation() {
            return Err(values.identity.allocation_failure(values.resource));
        }
        replacement
            .try_reserve_exact(replacement_plan.allocator_additional())
            .map_err(|_| values.identity.allocation_failure(values.resource))?;
        let complete_bytes = replacement_plan
            .finish(replacement.len(), replacement.capacity())
            .map_err(|cause| values.identity.capacity_error(values.resource, cause))?;
        plan.finish(values.values.len(), replacement.capacity())
            .map_err(|cause| values.identity.capacity_error(values.resource, cause))?;
        self.resize_exact(values.identity, &mut token, complete_bytes)?;
        if fail_next_metered_qualification() {
            return Err(values.identity.qualification_failure(values.resource));
        }
        self.check_identity(values.identity)?;
        self.poll()?;

        assert!(
            replacement.capacity() >= values.values.len(),
            "a prepared metered vector must retain every live element"
        );
        replacement.append(&mut values.values);
        let retired_values = std::mem::replace(&mut values.values, replacement);
        let retired_token = std::mem::replace(&mut values.capacity, token);
        drop(retired_values);
        drop(retired_token);
        Ok(())
    }
    /// Grows through the same owner, then performs the real write.
    pub fn push_vec<T>(
        &mut self,
        values: &mut MeteredVec<'session, T>,
        value: T,
    ) -> ParseResult<()> {
        self.reserve_vec(values, 1)?;
        values.try_push_reserved(value)
    }
    /// Qualifies replacement capacity and copies old contents at the established frontiers.
    pub fn reserve_string(
        &mut self,
        value: &mut MeteredString<'session>,
        additional: usize,
    ) -> ParseResult<()> {
        self.check_identity(value.identity)?;
        self.poll()?;
        let required = value
            .value
            .len()
            .checked_add(additional)
            .ok_or_else(|| value.identity.allocation_failure(value.resource))?;
        if required <= value.value.capacity() {
            return Ok(());
        }

        let (plan, replacement_plan) = metered_replacement_plans_with_element_size(
            value.value.len(),
            value.value.capacity(),
            required,
            1,
        )
        .map_err(|cause| value.identity.capacity_error(value.resource, cause))?;
        let admitted = replacement_plan
            .envelope_delta_bytes()
            .map_err(|cause| value.identity.capacity_error(value.resource, cause))?;
        let mut token = self.reserve(value.identity, value.class, admitted)?;
        plan.verify_preflight(value.value.len(), value.value.capacity())
            .map_err(|cause| value.identity.capacity_error(value.resource, cause))?;
        replacement_plan
            .verify_preflight(0, 0)
            .map_err(|cause| value.identity.capacity_error(value.resource, cause))?;
        let mut replacement = String::new();
        replacement
            .try_reserve_exact(replacement_plan.allocator_additional())
            .map_err(|_| value.identity.allocation_failure(value.resource))?;
        let complete_bytes = replacement_plan
            .finish(replacement.len(), replacement.capacity())
            .map_err(|cause| value.identity.capacity_error(value.resource, cause))?;
        plan.finish(value.value.len(), replacement.capacity())
            .map_err(|cause| value.identity.capacity_error(value.resource, cause))?;
        self.resize_exact(value.identity, &mut token, complete_bytes)?;
        if fail_next_metered_qualification() {
            return Err(value.identity.qualification_failure(value.resource));
        }
        append_metered_string(self, &mut replacement, &value.value)?;
        self.check_identity(value.identity)?;
        self.poll()?;

        let retired_value = std::mem::replace(&mut value.value, replacement);
        let retired_token = std::mem::replace(&mut value.capacity, token);
        drop(retired_value);
        drop(retired_token);
        Ok(())
    }
    /// Copies actual text in bounded UTF-8 chunks; an error can retain a copied prefix.
    pub fn append_string(
        &mut self,
        value: &mut MeteredString<'session>,
        text: &str,
    ) -> ParseResult<()> {
        self.reserve_string(value, text.len())?;
        append_metered_string(self, &mut value.value, text)
    }
}

pub(super) fn append_metered_string(
    exec: &mut ParseStorage<'_, '_>,
    output: &mut String,
    value: &str,
) -> ParseResult<()> {
    let mut start = 0usize;
    while start < value.len() {
        exec.poll()?;
        let mut end = start
            .checked_add(METERED_COPY_POLL_INTERVAL)
            .map_or(value.len(), |end| end.min(value.len()));
        while end > start && !value.is_char_boundary(end) {
            end -= 1;
        }
        output.push_str(&value[start..end]);
        start = end;
    }
    Ok(())
}
