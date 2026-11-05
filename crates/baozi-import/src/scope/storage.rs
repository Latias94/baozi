//! Ordinary vector/string growth shared by actual root, parent, child and service loans.
use super::*;
use crate::store::ImportOrdinaryStorage;

/// Short storage/poll borrow; returned containers retain only their session identity and token.
///
/// A session-owned container can leave a region and grow through the same session later:
/// ```
/// use baozi_import::{ParseContext, ParseFailure, ParseResult, MeteredVec, ImportStorageClass};
/// fn owned<'s>(c: &mut ParseContext<'s, '_>) -> ParseResult<MeteredVec<'s, u32>> {
///     let mut values = c.with_parser_execution::<_, ParseFailure>(|root| {
///         let mut loan = root.storage()?;
///         let mut values = loan.metered_vec(ImportStorageClass::SceneConstruction, "owned-proof");
///         loan.reserve_vec(&mut values, 1)?;
///         Ok(values)
///     })?;
///     c.with_execution(|root| values.try_push(&mut root.services()?, 7))?;
///     Ok(values)
/// }
/// ```
///
/// The inner operation lifetime does not extend a temporary root or vocabulary borrow:
/// ```
/// use baozi_import::{ParseRoot, ParseResult, ParseStorage};
/// fn short<'b, 'op: 'b, 's: 'b>(
///     root: &'b mut ParseRoot<'_, 'op, 's>,
/// ) -> ParseResult<ParseStorage<'b, 'op, 's>> {
///     root.storage()
/// }
/// ```
///
/// A root storage loan ends before reporting progress through the same control owner:
/// ```
/// use baozi_import::{ImportProgressPhase, ImportProgressUnit, ParseRoot, ParseResult};
/// fn sequential(exec: &mut ParseRoot<'_, '_, '_>) -> ParseResult<()> {
///     { let mut loan = exec.storage()?; loan.poll()?; }
///     exec.report_progress(ImportProgressPhase::Decode, ImportProgressUnit::Bytes, 1, None)
/// }
/// ```
///
/// A live root loan excludes that progress call until its last use:
/// ```compile_fail
/// use baozi_import::{ImportProgressPhase, ImportProgressUnit, ParseRoot, ParseResult};
/// fn overlap(exec: &mut ParseRoot<'_, '_, '_>) -> ParseResult<()> {
///     let mut loan = exec.storage()?;
///     exec.report_progress(ImportProgressPhase::Decode, ImportProgressUnit::Bytes, 1, None)?;
///     loan.poll()
/// }
/// ```
///
/// A loan cannot leave its execution region, although its owned container can:
/// ```compile_fail
/// use baozi_import::{ParseContext, ParseFailure};
/// fn escape(context: &mut ParseContext<'_, '_>) {
///     let _loan = context.with_execution::<_, ParseFailure>(|exec| exec.storage());
/// }
/// ```
///
/// The temporary storage borrow cannot escape its region:
/// ```compile_fail
/// use baozi_import::{ParseContext, ParseFailure};
/// fn escape(c: &mut ParseContext<'_, '_>) {
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
pub struct ParseStorage<'b, 'op: 'b, 'session: 'b> {
    raw: ImportOrdinaryStorage<'b, 'op>,
    identity: StorageIdentity<'session>,
    callback_active: &'b Cell<bool>,
}
impl<'b, 'op: 'b, 'session: 'b> ParseStorage<'b, 'op, 'session> {
    pub(super) fn new(
        raw: ImportOrdinaryStorage<'b, 'op>,
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
    /// Prepares covered, empty replacement capacity without copying or changing the source.
    pub(crate) fn prepare_vec_replacement<T>(
        &mut self,
        values: &MeteredVec<'session, T>,
        additional: usize,
    ) -> ParseResult<Option<MeteredVec<'session, T>>> {
        self.check_identity(values.identity)?;
        self.poll()?;
        let required = values
            .values
            .len()
            .checked_add(additional)
            .ok_or_else(|| values.identity.allocation_failure(values.resource))?;
        if required <= values.values.capacity() {
            return Ok(None);
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
        Ok(Some(MeteredVec {
            values: replacement,
            capacity: token,
            identity: values.identity,
            class: values.class,
            resource: values.resource,
        }))
    }

    /// Admits and qualifies full replacement capacity while the source remains live.
    pub fn reserve_vec<T>(
        &mut self,
        values: &mut MeteredVec<'session, T>,
        additional: usize,
    ) -> ParseResult<()> {
        let Some(mut replacement) = self.prepare_vec_replacement(values, additional)? else {
            return Ok(());
        };
        self.check_identity(values.identity)?;
        self.poll()?;

        assert!(
            replacement.values.capacity() >= values.values.len(),
            "a prepared metered vector must retain every live element"
        );
        replacement.values.append(&mut values.values);
        std::mem::swap(values, &mut replacement);
        drop(replacement.values);
        drop(replacement.capacity);
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

    /// Grows through the same reservation owner, then fills bounded physical prefixes.
    pub fn resize_vec<T: Copy>(
        &mut self,
        values: &mut MeteredVec<'session, T>,
        len: usize,
        value: T,
    ) -> ParseResult<()> {
        let additional = len.saturating_sub(values.values.len());
        self.reserve_vec(values, additional)?;
        while values.values.len() < len {
            self.check_identity(values.identity)?;
            self.poll()?;
            let chunk_end = values
                .values
                .len()
                .saturating_add(metered_copy_chunk_len::<T>())
                .min(len);
            values.values.resize(chunk_end, value);
        }
        values.values.truncate(len);
        Ok(())
    }
    /// Prepares a covered, empty String without copying or changing the source.
    pub(crate) fn prepare_string_replacement(
        &mut self,
        value: &MeteredString<'session>,
        additional: usize,
    ) -> ParseResult<Option<MeteredString<'session>>> {
        self.check_identity(value.identity)?;
        self.poll()?;
        let required = value
            .value
            .len()
            .checked_add(additional)
            .ok_or_else(|| value.identity.allocation_failure(value.resource))?;
        if required <= value.value.capacity() {
            return Ok(None);
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
        Ok(Some(MeteredString {
            value: replacement,
            capacity: token,
            identity: value.identity,
            class: value.class,
            resource: value.resource,
        }))
    }

    /// Qualifies replacement capacity and copies old contents at the established frontiers.
    pub fn reserve_string(
        &mut self,
        value: &mut MeteredString<'session>,
        additional: usize,
    ) -> ParseResult<()> {
        let Some(mut replacement) = self.prepare_string_replacement(value, additional)? else {
            return Ok(());
        };
        append_metered_string(self, &mut replacement.value, &value.value)?;
        self.check_identity(value.identity)?;
        self.poll()?;

        std::mem::swap(value, &mut replacement);
        drop(replacement.value);
        drop(replacement.capacity);
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

    /// Formats through the existing bounded append path, retaining its first failure.
    pub fn write_string_fmt(
        &mut self,
        value: &mut MeteredString<'session>,
        arguments: fmt::Arguments<'_>,
    ) -> ParseResult<()> {
        self.check_identity(value.identity)?;
        let mut writer = MeteredStringWriter {
            value,
            storage: self,
            failure: None,
        };
        let result = fmt::write(&mut writer, arguments);
        match (result, writer.failure) {
            (_, Some(failure)) => Err(failure),
            (Ok(()), None) => Ok(()),
            (Err(fmt::Error), None) => {
                Err(writer
                    .value
                    .identity
                    .abort(ScopeAbortCause::DescriptorContract {
                        code: "metered-string-format-failed",
                    }))
            }
        }
    }
}

struct MeteredStringWriter<'a, 'borrow, 'op, 'session> {
    value: &'a mut MeteredString<'session>,
    storage: &'a mut ParseStorage<'borrow, 'op, 'session>,
    failure: Option<ParseFailure>,
}

impl fmt::Write for MeteredStringWriter<'_, '_, '_, '_> {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        match self.storage.append_string(self.value, value) {
            Ok(()) => Ok(()),
            Err(failure) => {
                self.failure = Some(failure);
                Err(fmt::Error)
            }
        }
    }
}

pub(super) fn append_metered_string(
    exec: &mut ParseStorage<'_, '_, '_>,
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
