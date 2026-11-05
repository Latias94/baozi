//! Concrete import roles over the one retained operation execution owner.
use super::*;
use crate::ledger::ImportOperationOwner;
use crate::store::{ImportExecutionFailure, ImportStoreVocabulary, project_store_runtime_error};
use baozi_operation::__retained_store_integration::activation::{
    StoreChildAdmission, StoreExtensionResult, StoreFailureStorage, StoreInvocationError,
    StoreInvocationLayout, StoreInvocationParser, StoreInvocationServices, StoreOrdinaryStorage,
    StoreParserWorkError, StoreScanReturn,
};

type OrdinaryStorage<'b, 'op> =
    StoreOrdinaryStorage<'b, 'op, ImportOperationOwner, ImportProgressPhase, ImportProgressUnit>;
type FailureStorage<'b, 'op> =
    StoreFailureStorage<'b, 'op, ImportOperationOwner, ImportProgressPhase, ImportProgressUnit>;

/// A scan either exhausts its admission or returns a classified boundary before its end.
///
/// `Boundary` releases the unused suffix without consuming it. It does not clear a prior
/// failure, complete unfinished physical work, or permit a nested child on the scan role.
pub enum ParseScanReturn<T> {
    Exhausted(T),
    Boundary(T),
}

/// Immutable metadata owned once by the root and borrowed by its short execution roles.
pub(super) struct ParseViews<'borrow, 'session> {
    pub(super) entry: &'borrow RegistryEntry,
    pub(super) source: &'session AssetPath,
    pub(super) bytes: &'session [u8],
    pub(super) limits: &'session ResourceLimits,
    pub(super) issuer: &'session DependencyOriginIssuer,
    pub(super) terminal: &'session RefCell<Option<ScopeAbortCause>>,
    pub(super) callback_active: &'borrow Cell<bool>,
    pub(super) failure_storage: &'session RefCell<FailureEvidenceStorage>,
    pub(super) primary_io: &'session dyn AssetIo,
    pub(super) primary_authority: &'session AuthorityDomain,
    pub(super) conformance_policy: ConformancePolicy,
    pub(super) loss_policy: LossPolicy,
    pub(super) diagnostics: &'session RefCell<DiagnosticSink>,
    pub(super) loss_evidence: &'session RefCell<AccountedVec<ImportLossEvidence>>,
    pub(super) source_scope_record: &'session RefCell<Option<LiveSourceScopeRecord>>,
}
impl<'session> ParseViews<'_, 'session> {
    fn identity(&self) -> StorageIdentity<'session> {
        StorageIdentity {
            issuer: self.issuer,
            terminal: self.terminal,
        }
    }
    fn abort(&self, cause: ScopeAbortCause) -> ParseFailure {
        latch_abort(self.terminal, cause)
    }
    fn preflight(&self) -> ParseResult<()> {
        if self.callback_active.get() {
            return Err(self.abort(ScopeAbortCause::DescriptorContract {
                code: "parse-capability-reentered",
            }));
        }
        preflight_terminal(self.terminal)
    }
    fn runtime(
        &self,
        vocabulary: &ImportStoreVocabulary,
        cause: StoreInvocationError,
    ) -> ParseFailure {
        match cause {
            StoreParserWorkError::Runtime(cause) => {
                self.abort(project_store_runtime_error(vocabulary, cause).into_scope_abort_cause())
            }
            StoreParserWorkError::ItemLimit { .. } => {
                self.abort(ScopeAbortCause::DescriptorContract {
                    code: "parse-work-item-limit-lane-invalid",
                })
            }
        }
    }
    fn item_error<const N: usize>(
        &self,
        vocabulary: &ImportStoreVocabulary,
        limits: [Option<(&'static str, u64)>; N],
        cause: StoreInvocationError,
    ) -> ParseFailure {
        match cause {
            StoreParserWorkError::ItemLimit {
                index,
                value,
                limit,
            } => {
                debug_assert!(value > limit);
                if let Some((resource, _)) = limits.get(index).copied().flatten() {
                    return self.abort(ScopeAbortCause::ResourceLimit { resource, limit });
                }
                self.abort(ScopeAbortCause::DescriptorContract {
                    code: "parse-work-item-limit-lane-invalid",
                })
            }
            cause => self.runtime(vocabulary, cause),
        }
    }
}

/// Cold first-cause latching; this view owns no work/storage/control capability.
#[derive(Clone, Copy)]
pub struct ParseErrorView<'session> {
    terminal: &'session RefCell<Option<ScopeAbortCause>>,
    limits: &'session ResourceLimits,
    issuer: &'session DependencyOriginIssuer,
}

/// A physical piece retains an existing parse failure or returns format evidence for
/// the host to materialize after its ordinary storage loan ends.
pub enum ParsePieceError<P> {
    Parse(ParseFailure),
    Format(P),
}

impl<P> From<ParseFailure> for ParsePieceError<P> {
    fn from(cause: ParseFailure) -> Self {
        Self::Parse(cause)
    }
}
impl<'session> ParseErrorView<'session> {
    pub fn allocation_failure(&self, resource: &'static str) -> ParseFailure {
        if let Err(cause) = preflight_terminal(self.terminal) {
            return cause;
        }
        latch_abort(self.terminal, ScopeAbortCause::Allocation { resource })
    }

    pub fn resource_limit_exceeded(&self, resource: &'static str, limit: u64) -> ParseFailure {
        latch_abort(
            self.terminal,
            ScopeAbortCause::ResourceLimit { resource, limit },
        )
    }

    pub(crate) fn limits(&self) -> &ResourceLimits {
        self.limits
    }

    pub(crate) fn session_issuer(&self) -> &'session DependencyOriginIssuer {
        self.issuer
    }

    pub(crate) fn enforce_resource_limit(
        &self,
        resource: &'static str,
        value: u64,
        limit: u64,
    ) -> ParseResult<()> {
        preflight_terminal(self.terminal)?;
        if value > limit {
            Err(self.resource_limit_exceeded(resource, limit))
        } else {
            Ok(())
        }
    }

    pub(crate) fn descriptor_contract_failure(&self, code: &'static str) -> ParseFailure {
        if let Err(failure) = preflight_terminal(self.terminal) {
            return failure;
        }
        latch_abort(self.terminal, ScopeAbortCause::DescriptorContract { code })
    }

    pub(crate) fn check_session(&self, issuer: &DependencyOriginIssuer) -> ParseResult<()> {
        preflight_terminal(self.terminal)?;
        if std::ptr::eq(self.issuer, issuer) {
            Ok(())
        } else {
            Err(self.descriptor_contract_failure("parse-storage-foreign-session"))
        }
    }
}

/// Layouts borrow the invariant numeric region; they carry no owning work permit.
/// Zero amounts and empty layouts still belong to their issuing invocation:
///
/// ```
/// use baozi_import::{ImportWorkClass, ParseContext, ParseFailure, ParseResult};
/// fn local(context: &mut ParseContext<'_, '_>, amount: u64) -> ParseResult<()> {
///     context.with_execution(|root| {
///         let tokens = root.bind_group([ImportWorkClass::Tokens])?;
///         root.child(&tokens, [amount], |child| child.run([amount], || Ok::<_, ParseFailure>(())))?;
///         let empty = root.bind_group([])?;
///         root.child(&empty, [], |child| child.run([], || Ok::<_, ParseFailure>(())))
///     })
/// }
/// ```
///
/// Even an empty layout cannot be substituted into another root:
///
/// ```compile_fail
/// use baozi_import::{ParseContext, ParseFailure, ParseResult};
/// fn foreign_empty(first: &mut ParseContext<'_, '_>, second: &mut ParseContext<'_, '_>)
///     -> ParseResult<()>
/// {
///     first.with_execution(|sender| {
///         let empty = sender.bind_group([])?;
///         second.with_execution(|receiver| {
///             receiver.child(&empty, [], |child| child.run([], || Ok::<_, ParseFailure>(())))
///         })
///     })
/// }
/// ```
#[derive(Clone, Copy)]
pub struct ParseWorkLayout<'op, const N: usize> {
    raw: StoreInvocationLayout<'op, N>,
    item_limits: [Option<(&'static str, u64)>; N],
}

/// The host owns the root; a decoder receives only closed physical callbacks.
///
/// ```
/// use baozi_import::{ParseContext, ParseFailure, ImportWorkClass};
/// fn owned(context: &mut ParseContext<'_, '_>) -> Result<u32, ParseFailure> {
///     context.with_parser_execution(|root| {
///         let group = root.bind_group([ImportWorkClass::Tokens])?;
///         root.child(&group, [1], |child| child.run([1], || Ok(7)))
///     })
/// }
/// ```
///
/// ```compile_fail,E0501,E0500
/// use baozi_import::{ParseContext, ParseFailure, ImportWorkClass};
/// fn reenter(context: &mut ParseContext<'_, '_>) -> Result<(), ParseFailure> {
///     context.with_parser_execution(|root| {
///         let group = root.bind_group([ImportWorkClass::Tokens])?;
///         root.child(&group, [1], |child| child.run([1], || root.poll()))
///     })
/// }
/// ```
///
/// ```compile_fail
/// use baozi_import::{ParseContext, ParseFailure, ImportWorkClass};
/// fn escape(context: &mut ParseContext<'_, '_>) {
///     let _layout = context.with_parser_execution::<_, ParseFailure>(|root| root.bind_group([ImportWorkClass::Tokens]));
/// }
/// ```
///
/// Separate owners can return owned values; their bound layouts cannot be mixed:
///
/// ```
/// use baozi_import::{ParseContext, ParseFailure, ImportWorkClass};
/// fn separate(first: &mut ParseContext<'_, '_>, second: &mut ParseContext<'_, '_>) -> Result<(u32,u32), ParseFailure> {
///     let left = first.with_parser_execution::<_, ParseFailure>(|root| {
///         let group = root.bind_group([ImportWorkClass::Tokens])?;
///         root.child(&group, [1], |child| child.run([1], || Ok(7)))
///     })?;
///     let right = second.with_parser_execution::<_, ParseFailure>(|root| {
///         let group = root.bind_group([ImportWorkClass::Tokens])?;
///         root.child(&group, [1], |child| child.run([1], || Ok(9)))
///     })?;
///     Ok((left,right))
/// }
/// ```
///
/// ```compile_fail
/// use baozi_import::{ParseContext, ParseFailure, ImportWorkClass};
/// fn mixed(first: &mut ParseContext<'_, '_>, second: &mut ParseContext<'_, '_>) -> Result<(), ParseFailure> {
///     first.with_parser_execution::<_, ParseFailure>(|outer| {
///         let group = outer.bind_group([ImportWorkClass::Tokens])?;
///         second.with_parser_execution::<_, ParseFailure>(|inner| {
///             inner.child(&group, [1], |child| child.run([1], || Ok(())))
///         })
///     })
/// }
/// ```
///
/// ```compile_fail
/// use baozi_import::{ParseContext, ParseFailure};
/// fn escape_root(context: &mut ParseContext<'_, '_>) {
///     let _root = context.with_parser_execution::<_, ParseFailure>(|root| Ok(root));
/// }
/// ```
///
/// ```
/// use baozi_import::{ParseContext, ParseFailure};
/// fn send_owned(context: &mut ParseContext<'_, '_>) -> Result<u32, ParseFailure> {
///     let value = context.with_parser_execution::<_, ParseFailure>(|_| Ok(7_u32))?;
///     Ok(std::thread::scope(|threads| threads.spawn(move || value).join().unwrap()))
/// }
/// ```
///
/// ```compile_fail,E0277
/// use baozi_import::{ParseContext, ParseFailure};
/// fn send_context(context: &mut ParseContext<'_, '_>) -> Result<(), ParseFailure> {
///     context.with_parser_execution(|root| {
///         std::thread::scope(|threads| { threads.spawn(move || root.poll()); });
///         Ok(())
///     })
/// }
/// ```
pub struct ParseRoot<'borrow, 'op, 'session> {
    pub(super) store: crate::store::ImportRootExecution<'borrow, 'op>,
    vocabulary: &'borrow ImportStoreVocabulary,
    pub(super) views: ParseViews<'borrow, 'session>,
    pub(super) resolver: ResolverBinding<'session>,
    pub(super) resource_graph: &'session RefCell<ResourceGraph>,
    pub(super) optional_dependency_evidence:
        &'session RefCell<AccountedVec<OptionalDependencyEvidence>>,
    pub(super) dependency_failure: &'borrow RefCell<Option<DependencyResolutionContext>>,
    pub(super) dependency_failure_context: &'borrow RefCell<Option<DependencyFailureContext>>,
}

/// A short sequential child-admission loan from a root or parent.
pub struct ParseChildren<'borrow, 'op, 'session> {
    numeric: StoreChildAdmission<
        'borrow,
        'op,
        ImportOperationOwner,
        ImportProgressPhase,
        ImportProgressUnit,
    >,
    vocabulary: &'borrow ImportStoreVocabulary,
    pub(super) views: &'borrow ParseViews<'borrow, 'session>,
}

/// A sequential parent/child borrow. It cannot recover the root or escape its host.
pub struct ParseParser<'borrow, 'frame, 'ctx, 'op, 'session, const N: usize, const CHILD: bool> {
    numeric: &'borrow mut StoreInvocationParser<
        'frame,
        'ctx,
        'op,
        ImportOperationOwner,
        ImportProgressPhase,
        ImportProgressUnit,
        N,
        CHILD,
    >,
    vocabulary: &'borrow ImportStoreVocabulary,
    views: &'borrow ParseViews<'borrow, 'session>,
}

impl<'borrow, 'op, 'session> ParseServices<'borrow, 'op, 'session> {
    pub fn error_view(&self) -> ParseErrorView<'session> {
        ParseErrorView {
            terminal: self.views.terminal,
            limits: self.views.limits,
            issuer: self.views.issuer,
        }
    }

    fn bind(
        numeric: StoreInvocationServices<
            'borrow,
            'op,
            ImportOperationOwner,
            ImportProgressPhase,
            ImportProgressUnit,
        >,
        vocabulary: &'borrow ImportStoreVocabulary,
        views: &'borrow ParseViews<'borrow, 'session>,
    ) -> Self {
        Self {
            store: crate::store::ImportStoreServices::new(numeric, vocabulary),
            views,
        }
    }
}

impl<'op, 'session> ParseChildren<'_, 'op, 'session> {
    pub fn error_view(&self) -> ParseErrorView<'session> {
        ParseErrorView {
            terminal: self.views.terminal,
            limits: self.views.limits,
            issuer: self.views.issuer,
        }
    }

    pub(crate) fn diagnostic_source_is_primary(&self, source: &DiagnosticSource<'_>) -> bool {
        source.authority() == self.views.primary_authority
            && source.path() == self.views.source
            && std::ptr::eq(source.bytes(), self.views.bytes)
    }

    pub fn services(&mut self) -> ParseResult<ParseServices<'_, 'op, 'session>> {
        self.views.preflight()?;
        Ok(ParseServices::bind(
            self.numeric.services(),
            self.vocabulary,
            self.views,
        ))
    }

    pub fn bind_group<const N: usize>(
        &mut self,
        classes: [ImportWorkClass; N],
    ) -> ParseResult<ParseWorkLayout<'op, N>> {
        self.views.preflight()?;
        let lanes = classes.map(|class| class.accounting(self.views.limits));
        let raw = self
            .numeric
            .bind_group(lanes.map(|(dimension, _)| self.vocabulary.dimension(dimension)))
            .map_err(|cause| self.views.runtime(self.vocabulary, cause))?;
        Ok(ParseWorkLayout {
            raw,
            item_limits: lanes.map(|(_, limit)| limit),
        })
    }

    fn exit_poll(&mut self) -> ParseResult<()> {
        self.views.preflight()?;
        self.numeric
            .prescribed_exit_checkpoint()
            .map_err(|cause| self.views.runtime(self.vocabulary, cause))
    }

    /// Polls a completed result without reopening failed work. Control errors take priority.
    pub fn poll_result<T, E: From<ParseFailure>>(&mut self, result: Result<T, E>) -> Result<T, E> {
        if result.is_ok() {
            self.services().map_err(E::from)?.poll().map_err(E::from)?;
        } else {
            self.exit_poll().map_err(E::from)?;
        }
        result
    }

    #[allow(
        clippy::result_large_err,
        reason = "the child host preserves original owned callback errors and exact inline admission evidence"
    )]
    pub fn child<T, E: From<ParseFailure>, const N: usize>(
        &mut self,
        layout: &ParseWorkLayout<'op, N>,
        amounts: [u64; N],
        body: impl FnOnce(&mut ParseParser<'_, '_, '_, 'op, 'session, N, true>) -> Result<T, E>,
    ) -> Result<T, E> {
        self.views.preflight().map_err(E::from)?;
        let views = self.views;
        let vocabulary = self.vocabulary;
        let result = self.numeric.child(
            layout.raw,
            amounts,
            layout.item_limits.map(|x| x.map(|(_, limit)| limit)),
            |numeric| {
                body(&mut ParseParser {
                    numeric,
                    vocabulary,
                    views,
                })
                .map_err(ImportExecutionFailure::Owned)
            },
        );
        self.finish_parser(result, layout.item_limits)
    }

    /// Closes a scan after its explicit normal boundary or exhausted result.
    #[allow(
        clippy::result_large_err,
        reason = "trusted scan close preserves owned callback errors without failure-path allocation"
    )]
    pub(crate) fn scan_child<T, E: From<ParseFailure>, const N: usize>(
        &mut self,
        layout: &ParseWorkLayout<'op, N>,
        amounts: [u64; N],
        body: impl FnOnce(
            &mut ParseParser<'_, '_, '_, 'op, 'session, N, true>,
        ) -> Result<StoreScanReturn<T>, E>,
    ) -> Result<T, E> {
        self.views.preflight().map_err(E::from)?;
        let views = self.views;
        let vocabulary = self.vocabulary;
        let result = self.numeric.scan_child(
            layout.raw,
            amounts,
            layout.item_limits.map(|x| x.map(|(_, limit)| limit)),
            |numeric| {
                body(&mut ParseParser {
                    numeric,
                    vocabulary,
                    views,
                })
                .map_err(ImportExecutionFailure::Owned)
            },
        );
        self.finish_parser(result, layout.item_limits)
    }

    fn finish_parser<T, E: From<ParseFailure>, const N: usize>(
        &mut self,
        result: Result<T, ImportExecutionFailure<E>>,
        limits: [Option<(&'static str, u64)>; N],
    ) -> Result<T, E> {
        let exit = if matches!(
            &result,
            Ok(_)
                | Err(ImportExecutionFailure::Owned(_))
                | Err(ImportExecutionFailure::Runtime(
                    StoreParserWorkError::Runtime(
                        baozi_operation::StoreRuntimeError::ObservedWorkIncomplete
                    )
                ))
        ) {
            self.exit_poll()
        } else {
            Ok(())
        };
        match result {
            Ok(value) => exit.map(|()| value).map_err(E::from),
            Err(ImportExecutionFailure::Owned(cause)) => Err(cause),
            Err(ImportExecutionFailure::Runtime(StoreParserWorkError::Runtime(
                baozi_operation::StoreRuntimeError::ObservedWorkIncomplete,
            ))) => {
                exit.map_err(E::from)?;
                Err(E::from(self.views.abort(
                    ScopeAbortCause::DescriptorContract {
                        code: "parse-work-callback-incomplete",
                    },
                )))
            }
            Err(ImportExecutionFailure::Runtime(cause)) => Err(E::from(self.views.item_error(
                self.vocabulary,
                limits,
                cause,
            ))),
        }
    }
}

impl<'session, 'inv: 'session> ParseContext<'session, 'inv> {
    /// Replacement entry: the real registry and the actual session outcome owner close it.
    #[allow(
        clippy::result_large_err,
        reason = "the region preserves owned callback errors and exact inline numeric causes until actual import exit classification"
    )]
    pub fn with_parser_execution<R, E: From<ParseFailure>>(
        &mut self,
        action: impl for<'op> FnOnce(&mut ParseRoot<'_, 'op, 'session>) -> Result<R, E>,
    ) -> Result<R, E> {
        preflight_terminal(self.terminal).map_err(E::from)?;
        self.check_execution_running().map_err(E::from)?;
        let views = ParseViews {
            entry: &self.entry,
            source: self.primary_source,
            bytes: self.primary_bytes,
            limits: self.limits,
            issuer: self.dependency_origin_issuer,
            terminal: self.terminal,
            callback_active: &self.callback_active,
            failure_storage: self.format_failure_storage,
            primary_io: self.primary_io,
            primary_authority: self.primary_authority,
            conformance_policy: self.conformance_policy,
            loss_policy: self.loss_policy,
            diagnostics: self.diagnostics,
            loss_evidence: self.loss_evidence,
            source_scope_record: self.source_scope_record,
        };
        let result = self
            .store_owner
            .with_parser_execution(|numeric, vocabulary| {
                let mut root = ParseRoot {
                    store: crate::store::ImportRootExecution::new(numeric, vocabulary),
                    vocabulary,
                    views,
                    resolver: self.resolver,
                    resource_graph: self.resource_graph,
                    optional_dependency_evidence: self.optional_dependency_evidence,
                    dependency_failure: &self.dependency_failure,
                    dependency_failure_context: &self.dependency_failure_context,
                };
                let result = action(&mut root);
                root.store.raw.settle_returned_callback();
                // Import's normal region exit polls on Ok and eligible semantic Err. The operation
                // region does not manufacture a second final poll or a Requested(0) classification.
                if result.is_err() {
                    root.store.raw.stop_known_error();
                }
                let exit = root.exit_poll();
                match result {
                    Ok(value) => exit
                        .map(|()| value)
                        .map_err(|cause| ImportExecutionFailure::Owned(E::from(cause))),
                    Err(cause) => Err(ImportExecutionFailure::Owned(cause)),
                }
            });
        match result {
            Ok(value) => Ok(value),
            Err(ImportExecutionFailure::Owned(cause)) => Err(cause),
            Err(ImportExecutionFailure::Runtime(cause)) => {
                // All start/close runtime diagnostics use the real vocabulary after the borrow.
                Err(E::from(
                    self.store_owner
                        .parser_runtime_failure(cause, self.terminal),
                ))
            }
        }
    }
}

impl<'op, 'session> ParseRoot<'_, 'op, 'session> {
    pub fn error_view(&self) -> ParseErrorView<'session> {
        ParseErrorView {
            terminal: self.views.terminal,
            limits: self.views.limits,
            issuer: self.views.issuer,
        }
    }

    /// Borrows this root's numeric services and immutable metadata together.
    /// Session data can outlive the short service loan:
    ///
    /// ```
    /// use baozi_import::{ParseContext, ParseFailure};
    /// fn primary<'session, 'inv: 'session>(
    ///     context: &mut ParseContext<'session, 'inv>,
    /// ) -> Result<&'session [u8], ParseFailure> {
    ///     context.with_parser_execution(|root| Ok(root.services()?.primary_bytes()))
    /// }
    /// ```
    ///
    /// The service capability itself cannot escape the root:
    ///
    /// ```compile_fail
    /// use baozi_import::{ParseContext, ParseFailure};
    /// fn escape(context: &mut ParseContext<'_, '_>) {
    ///     let _services = context.with_parser_execution::<_, ParseFailure>(|root| root.services());
    /// }
    /// ```
    pub fn services(&mut self) -> ParseResult<ParseServices<'_, 'op, 'session>> {
        self.views.preflight()?;
        Ok(ParseServices::bind(
            self.store.raw.services(),
            self.vocabulary,
            &self.views,
        ))
    }

    pub fn children(&mut self) -> ParseResult<ParseChildren<'_, 'op, 'session>> {
        self.views.preflight()?;
        Ok(ParseChildren {
            numeric: self.store.raw.child_admission(),
            vocabulary: self.vocabulary,
            views: &self.views,
        })
    }
    /// Borrows ordinary storage/poll services without lending numeric work.
    pub fn storage(&mut self) -> ParseResult<ParseStorage<'_, 'op, 'session>> {
        self.views.preflight()?;
        Ok(ParseStorage::new(
            crate::store::ImportOrdinaryStorage::new(
                self.store.raw.ordinary_storage(),
                self.vocabulary,
            ),
            self.views.identity(),
            self.views.callback_active,
        ))
    }
    /// Preserves cold preflight and the actual allocation failure cause.
    pub fn allocation_failure(&self, resource: &'static str) -> ParseFailure {
        if let Err(cause) = self.views.preflight() {
            return cause;
        }
        self.views.abort(ScopeAbortCause::Allocation { resource })
    }
    /// Materializes the real accounted failure without a dummy work action.
    pub fn format_failure<P: FormatFailureEvidenceProducer>(
        &mut self,
        producer: P,
    ) -> ParseFailure {
        if let Err(cause) = self.views.preflight() {
            return cause;
        }
        capture_format_failure(
            FormatFailureHost::exclusive(
                self.store.raw.failure_storage(),
                self.vocabulary,
                &self.views,
            ),
            producer,
        )
    }

    pub fn source(&self) -> &'session AssetPath {
        self.views.source
    }
    pub fn primary_bytes(&self) -> &'session [u8] {
        self.views.bytes
    }
    pub fn resource_limit_exceeded(&self, resource: &'static str, limit: u64) -> ParseFailure {
        self.views
            .abort(ScopeAbortCause::ResourceLimit { resource, limit })
    }
    fn exit_poll(&mut self) -> ParseResult<()> {
        self.views.preflight()?;
        self.store
            .raw
            .prescribed_exit_checkpoint()
            .map_err(|cause| self.views.runtime(self.vocabulary, cause))
    }
    pub fn poll(&mut self) -> ParseResult<()> {
        self.views.preflight()?;
        self.store
            .raw
            .checkpoint()
            .map_err(|cause| self.views.runtime(self.vocabulary, cause))
    }
    /// Polls a completed result without reopening failed work. Control errors take priority.
    pub fn poll_result<T, E: From<ParseFailure>>(&mut self, result: Result<T, E>) -> Result<T, E> {
        if result.is_ok() {
            self.poll().map_err(E::from)?;
        } else {
            self.exit_poll().map_err(E::from)?;
        }
        result
    }
    /// Reports progress through the same invocation that owns this numeric region.
    pub fn report_progress(
        &mut self,
        phase: ImportProgressPhase,
        unit: ImportProgressUnit,
        completed: u64,
        total: Option<u64>,
    ) -> ParseResult<()> {
        self.views.preflight()?;
        self.store
            .raw
            .report_progress(phase, unit, completed, total)
            .map_err(|cause| self.views.runtime(self.vocabulary, cause))
    }
    pub fn bind_group<const N: usize>(
        &mut self,
        classes: [ImportWorkClass; N],
    ) -> ParseResult<ParseWorkLayout<'op, N>> {
        self.views.preflight()?;
        let lanes = classes.map(|class| class.accounting(self.views.limits));
        let raw = self
            .store
            .raw
            .bind_group(lanes.map(|(dimension, _)| self.vocabulary.dimension(dimension)))
            .map_err(|cause| self.views.runtime(self.vocabulary, cause))?;
        Ok(ParseWorkLayout {
            raw,
            item_limits: lanes.map(|(_, limit)| limit),
        })
    }
    pub fn work_counters(&self, class: ImportWorkClass) -> ParseResult<(u64, u64, u64)> {
        self.store
            .raw
            .work_counters(
                self.vocabulary
                    .dimension(class.accounting(self.views.limits).0),
            )
            .map_err(|cause| self.views.runtime(self.vocabulary, cause))
    }
    #[allow(
        clippy::result_large_err,
        reason = "the closed child host retains original E and inline admission/close evidence without failure-path allocation"
    )]
    pub fn child<T, E: From<ParseFailure>, const N: usize>(
        &mut self,
        layout: &ParseWorkLayout<'op, N>,
        amounts: [u64; N],
        body: impl FnOnce(&mut ParseParser<'_, '_, '_, 'op, 'session, N, true>) -> Result<T, E>,
    ) -> Result<T, E> {
        self.children()
            .map_err(E::from)?
            .child(layout, amounts, body)
    }
    /// Scans until exhaustion or a classified boundary through the same closed child host.
    ///
    /// Return an actual error as `Err`. A normal boundary can carry a value that the caller
    /// must validate after the physical scan loan ends, such as a syntax-budget handback.
    ///
    /// ```
    /// use baozi_import::{ImportWorkClass, ParseContext, ParseFailure, ParseResult, ParseScanReturn};
    /// fn first_record(context: &mut ParseContext<'_, '_>) -> ParseResult<usize> {
    ///     context.with_parser_execution(|root| {
    ///         let bytes = root.bind_group([ImportWorkClass::DecodeWork])?;
    ///         root.scan(&bytes, [6], |scan| {
    ///             scan.run([2], || Ok::<_, ParseFailure>(()))?;
    ///             Ok(ParseScanReturn::Boundary(2))
    ///         })
    ///     })
    /// }
    /// ```
    #[allow(
        clippy::result_large_err,
        reason = "scan close preserves the original owned error and exact inline admission evidence"
    )]
    pub fn scan<T, E: From<ParseFailure>, const N: usize>(
        &mut self,
        layout: &ParseWorkLayout<'op, N>,
        amounts: [u64; N],
        body: impl FnOnce(
            &mut ParseParser<'_, '_, '_, 'op, 'session, N, true>,
        ) -> Result<ParseScanReturn<T>, E>,
    ) -> Result<T, E> {
        self.children()
            .map_err(E::from)?
            .scan_child(layout, amounts, |scan| {
                body(scan).map(|result| match result {
                    ParseScanReturn::Exhausted(value) => StoreScanReturn::Exhausted(value),
                    ParseScanReturn::Boundary(value) => StoreScanReturn::Boundary(value),
                })
            })
    }
    #[allow(
        clippy::result_large_err,
        reason = "parent completion retains original E and inline numeric evidence through the prescribed exit poll"
    )]
    pub fn parent<T, E: From<ParseFailure>, const N: usize>(
        &mut self,
        layout: &ParseWorkLayout<'op, N>,
        amounts: [u64; N],
        body: impl FnOnce(&mut ParseParser<'_, '_, '_, 'op, 'session, N, false>) -> Result<T, E>,
    ) -> Result<T, E> {
        self.views.preflight().map_err(E::from)?;
        let views = &self.views;
        let vocabulary = self.vocabulary;
        let result = self.store.raw.parser_parent(
            layout.raw,
            amounts,
            layout.item_limits.map(|x| x.map(|(_, limit)| limit)),
            |numeric| {
                let mut parent = ParseParser {
                    numeric,
                    vocabulary,
                    views,
                };
                let value = body(&mut parent).map_err(ImportExecutionFailure::Owned)?;
                parent
                    .poll()
                    .map_err(|cause| ImportExecutionFailure::Owned(E::from(cause)))?;
                parent.numeric.complete_parent()?;
                Ok(value)
            },
        );
        self.finish_parser(result, layout.item_limits)
    }
    fn finish_parser<T, E: From<ParseFailure>, const N: usize>(
        &mut self,
        result: Result<T, ImportExecutionFailure<E>>,
        limits: [Option<(&'static str, u64)>; N],
    ) -> Result<T, E> {
        ParseChildren {
            numeric: self.store.raw.child_admission(),
            vocabulary: self.vocabulary,
            views: &self.views,
        }
        .finish_parser(result, limits)
    }
    /// Performs one real caller-owned reader call. As in read_asset_chunk, this bills opaque
    /// work; the snapshot owner separately admits/reconciles observed payload byte capacity.
    pub fn read_chunk(
        &mut self,
        opened: &mut OpenedAsset,
        buffer: &mut [u8],
    ) -> ParseResult<Result<usize, baozi_io::AssetReadError>> {
        self.poll()?;
        let raw = self
            .store
            .raw
            .bind_group([self.vocabulary.dimension(ImportDimension::WorkUnits)])
            .map_err(|cause| self.views.runtime(self.vocabulary, cause))?;
        match self
            .store
            .raw
            .run_extension(raw, [1], 1, || opened.read_chunk(buffer))
            .map_err(|cause| self.views.runtime(self.vocabulary, cause))?
        {
            StoreExtensionResult::Returned(value) => Ok(value),
            StoreExtensionResult::Panicked => {
                Err(self.views.abort(ScopeAbortCause::TrustedExtension {
                    boundary: TrustedExtensionBoundary::AssetIo,
                    code: "asset-read-panicked",
                }))
            }
        }
    }
}

impl<'op, 'session, const N: usize, const CHILD: bool>
    ParseParser<'_, '_, '_, 'op, 'session, N, CHILD>
{
    pub fn remaining(&self) -> [u64; N] {
        self.numeric.remaining()
    }

    /// Session bytes remain available after parent and child service loans end:
    ///
    /// ```
    /// use baozi_import::{ImportWorkClass, ParseContext, ParseFailure};
    /// fn source<'session, 'inv: 'session>(
    ///     context: &mut ParseContext<'session, 'inv>,
    /// ) -> Result<&'session [u8], ParseFailure> {
    ///     context.with_parser_execution(|root| {
    ///         let work = root.bind_group([ImportWorkClass::WorkUnits])?;
    ///         root.parent(&work, [0], |parent| {
    ///             let bytes = parent.services()?.primary_bytes();
    ///             parent.poll()?;
    ///             parent.child(&work, [0], |child| {
    ///                 let child_bytes = child.services()?.primary_bytes();
    ///                 child.poll()?;
    ///                 Ok::<_, ParseFailure>(child_bytes)
    ///             })?;
    ///             Ok(bytes)
    ///         })
    ///     })
    /// }
    /// ```
    ///
    /// A live service loan excludes reentry of either role:
    ///
    /// ```compile_fail,E0499
    /// use baozi_import::{ImportWorkClass, ParseContext, ParseFailure};
    /// fn parent_reentry(context: &mut ParseContext<'_, '_>) -> Result<(), ParseFailure> {
    ///     context.with_parser_execution(|root| {
    ///         let work = root.bind_group([ImportWorkClass::WorkUnits])?;
    ///         root.parent(&work, [0], |parent| {
    ///             let mut services = parent.services()?;
    ///             parent.poll()?;
    ///             services.poll()
    ///         })
    ///     })
    /// }
    /// ```
    ///
    /// ```compile_fail,E0499
    /// use baozi_import::{ImportWorkClass, ParseContext, ParseFailure};
    /// fn child_reentry(context: &mut ParseContext<'_, '_>) -> Result<(), ParseFailure> {
    ///     context.with_parser_execution(|root| {
    ///         let work = root.bind_group([ImportWorkClass::WorkUnits])?;
    ///         root.child(&work, [0], |child| {
    ///             let mut services = child.services()?;
    ///             child.poll()?;
    ///             services.poll()
    ///         })
    ///     })
    /// }
    /// ```
    pub fn services(&mut self) -> ParseResult<ParseServices<'_, 'op, 'session>> {
        self.views.preflight()?;
        Ok(ParseServices::bind(
            self.numeric.services(),
            self.vocabulary,
            self.views,
        ))
    }
    /// The live storage loan prevents reentry of this parser role.
    pub fn storage(&mut self) -> ParseResult<ParseStorage<'_, 'op, 'session>> {
        self.views.preflight()?;
        Ok(ParseStorage::new(
            crate::store::ImportOrdinaryStorage::new(
                self.numeric.ordinary_storage(),
                self.vocabulary,
            ),
            self.views.identity(),
            self.views.callback_active,
        ))
    }
    /// Admits one real growing push; an unused value stays outside the denial path.
    pub fn push_growing<T>(
        &mut self,
        amounts: [u64; N],
        values: &mut MeteredVec<'session, T>,
        value: T,
    ) -> ParseResult<()> {
        let mut unused = Some(value);
        self.views.preflight()?;
        let views = self.views;
        let vocabulary = self.vocabulary;
        let mut slot = Some(|mut raw: OrdinaryStorage<'_, 'op>| {
            let mut storage = ParseStorage::new(
                crate::store::ImportOrdinaryStorage::new(raw.reborrow(), vocabulary),
                views.identity(),
                views.callback_active,
            );
            storage.reserve_vec(values, 1)?;
            values.try_push_reserved(unused.take().expect("a compound push moves its value once"))
        });
        self.numeric.run_prefix_with_storage(
            amounts,
            1,
            &mut slot,
            |cause| views.runtime(vocabulary, cause),
            |_, cause| cause,
        )
    }
    /// Appends actual text under the same parser guard and shared growth owner.
    pub fn append_string(
        &mut self,
        amounts: [u64; N],
        value: &mut MeteredString<'session>,
        text: &str,
    ) -> ParseResult<()> {
        self.views.preflight()?;
        let views = self.views;
        let vocabulary = self.vocabulary;
        let mut slot = Some(|mut raw: OrdinaryStorage<'_, 'op>| {
            let mut storage = ParseStorage::new(
                crate::store::ImportOrdinaryStorage::new(raw.reborrow(), vocabulary),
                views.identity(),
                views.callback_active,
            );
            storage.append_string(value, text)
        });
        self.numeric.run_prefix_with_storage(
            amounts,
            1,
            &mut slot,
            |cause| views.runtime(vocabulary, cause),
            |_, cause| cause,
        )
    }
    pub fn error_view(&self) -> ParseErrorView<'session> {
        ParseErrorView {
            terminal: self.views.terminal,
            limits: self.views.limits,
            issuer: self.views.issuer,
        }
    }
    /// Runs one physical parser piece and commits the literal amounts returned by that piece.
    /// The host checks those amounts against the existing admission before accepting completion.
    pub fn run_returned<T, E: From<ParseFailure>>(
        &mut self,
        action: impl FnOnce() -> Result<(T, [u64; N]), E>,
    ) -> Result<T, E> {
        self.views.preflight().map_err(E::from)?;
        let views = self.views;
        let vocabulary = self.vocabulary;
        let mut slot = Some(|_storage: OrdinaryStorage<'_, 'op>| action());
        self.numeric.run_returned_prefix_with_storage(
            1,
            &mut slot,
            |cause| E::from(views.runtime(vocabulary, cause)),
            |_, cause| cause,
        )
    }

    /// Returns physical completion from the same piece that reads or grows ordinary storage.
    /// Format evidence is captured by the guarded host before its parser frame closes.
    pub fn run_returned_with_storage<T, P: FormatFailureEvidenceProducer>(
        &mut self,
        action: impl for<'piece> FnOnce(
            ParseStorage<'piece, 'op, 'session>,
        ) -> Result<(T, [u64; N]), ParsePieceError<P>>,
    ) -> ParseResult<T> {
        self.views.preflight()?;
        let views = self.views;
        let vocabulary = self.vocabulary;
        let mut slot = Some(|raw: OrdinaryStorage<'_, 'op>| {
            action(ParseStorage::new(
                crate::store::ImportOrdinaryStorage::new(raw, vocabulary),
                views.identity(),
                views.callback_active,
            ))
        });
        self.numeric.run_returned_prefix_with_storage(
            1,
            &mut slot,
            |cause| views.runtime(vocabulary, cause),
            |numeric, cause| match cause {
                ParsePieceError::Parse(cause) => cause,
                ParsePieceError::Format(producer) => capture_format_failure(
                    FormatFailureHost::exclusive(numeric.failure_storage(), vocabulary, views),
                    producer,
                ),
            },
        )
    }
    pub fn run<T, E: From<ParseFailure>>(
        &mut self,
        amounts: [u64; N],
        action: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, E> {
        self.views.preflight().map_err(E::from)?;
        let views = self.views;
        let vocabulary = self.vocabulary;
        let mut slot = Some(|_storage: OrdinaryStorage<'_, 'op>| action());
        self.numeric.run_prefix_with_storage(
            amounts,
            1,
            &mut slot,
            |cause| E::from(views.runtime(vocabulary, cause)),
            |_, cause| cause,
        )
    }
    pub fn poll(&mut self) -> ParseResult<()> {
        self.views.preflight()?;
        self.numeric
            .checkpoint()
            .map_err(|cause| self.views.runtime(self.vocabulary, cause))
    }
    /// Polls a completed result without reopening failed work. Control errors take priority.
    pub fn poll_result<T, E: From<ParseFailure>>(&mut self, result: Result<T, E>) -> Result<T, E> {
        self.views.preflight().map_err(E::from)?;
        let checkpoint = if result.is_ok() {
            self.numeric.checkpoint()
        } else {
            self.numeric.prescribed_exit_checkpoint()
        };
        checkpoint.map_err(|cause| E::from(self.views.runtime(self.vocabulary, cause)))?;
        result
    }
    /// Reports without closing or releasing the active parser's physical work prefix.
    pub fn report_progress(
        &mut self,
        phase: ImportProgressPhase,
        unit: ImportProgressUnit,
        completed: u64,
        total: Option<u64>,
    ) -> ParseResult<()> {
        self.views.preflight()?;
        self.numeric
            .report_progress(phase, unit, completed, total)
            .map_err(|cause| self.views.runtime(self.vocabulary, cause))
    }
    pub fn work_counters(&self, class: ImportWorkClass) -> ParseResult<(u64, u64, u64)> {
        self.numeric
            .work_counters(
                self.vocabulary
                    .dimension(class.accounting(self.views.limits).0),
            )
            .map_err(|cause| self.views.runtime(self.vocabulary, cause))
    }
    /// A bounded producer returns from the decoder; host evidence construction precedes frame close.
    pub fn run_format<T, P: FormatFailureEvidenceProducer>(
        &mut self,
        amounts: [u64; N],
        action: impl FnOnce() -> Result<T, P>,
    ) -> ParseResult<T> {
        self.views.preflight()?;
        let views = self.views;
        let vocabulary = self.vocabulary;
        let mut slot = Some(|_storage: OrdinaryStorage<'_, 'op>| action());
        self.numeric.run_prefix_with_storage(
            amounts,
            1,
            &mut slot,
            |cause| views.runtime(vocabulary, cause),
            |numeric, producer| {
                capture_format_failure(
                    FormatFailureHost::exclusive(numeric.failure_storage(), vocabulary, views),
                    producer,
                )
            },
        )
    }
}
impl<const CHILD: bool> ParseParser<'_, '_, '_, '_, '_, 1, CHILD> {
    /// Admits another singleton chunk after the previous chunk has been completed.
    ///
    /// Replenishment preserves the same parser channel and invocation. A per-item limit or
    /// an unfinished suffix rejects the request before it can admit additional work.
    ///
    /// ```
    /// use baozi_import::{ImportWorkClass, ParseContext, ParseFailure, ParseResult};
    /// fn locate(context: &mut ParseContext<'_, '_>, needle: u32) -> ParseResult<Option<usize>> {
    ///     context.with_parser_execution(|root| {
    ///         let comparisons = root.bind_group([ImportWorkClass::WorkUnits])?;
    ///         root.child(&comparisons, [0], |child| {
    ///             for (index, value) in [2_u32, 5, 9, 14].into_iter().enumerate() {
    ///                 child.replenish(1)?;
    ///                 if child.run([1], || Ok::<_, ParseFailure>(value == needle))? {
    ///                     return Ok(Some(index));
    ///                 }
    ///             }
    ///             Ok(None)
    ///         })
    ///     })
    /// }
    /// ```
    ///
    /// A multi-lane item cannot be split by replenishing only one of its dimensions:
    ///
    /// ```compile_fail,E0599
    /// use baozi_import::{ImportWorkClass, ParseContext, ParseResult};
    /// fn split_item(context: &mut ParseContext<'_, '_>) -> ParseResult<()> {
    ///     context.with_parser_execution(|root| {
    ///         let item = root.bind_group([ImportWorkClass::Tokens, ImportWorkClass::WorkUnits])?;
    ///         root.child(&item, [0, 0], |child| child.replenish(1))
    ///     })
    /// }
    /// ```
    pub fn replenish(&mut self, amount: u64) -> ParseResult<()> {
        self.views.preflight()?;
        self.numeric
            .replenish(amount)
            .map_err(|cause| self.views.runtime(self.vocabulary, cause))
    }
}

impl<'op, 'session, const N: usize> ParseParser<'_, '_, '_, 'op, 'session, N, false> {
    /// A parent admits sequential children on the same numeric owner:
    ///
    /// ```
    /// use baozi_import::{ImportWorkClass, ParseContext, ParseFailure, ParseResult};
    /// fn sequential(context: &mut ParseContext<'_, '_>) -> ParseResult<()> {
    ///     context.with_parser_execution(|root| {
    ///         let records = root.bind_group([ImportWorkClass::Records])?;
    ///         let tokens = root.bind_group([ImportWorkClass::Tokens])?;
    ///         root.parent(&records, [1], |parent| {
    ///             parent.child(&tokens, [1], |child| child.run([1], || Ok::<_, ParseFailure>(())))?;
    ///             parent.child(&tokens, [1], |child| child.run([1], || Ok::<_, ParseFailure>(())))
    ///         })
    ///     })
    /// }
    /// ```
    ///
    /// A current child cannot admit another child:
    ///
    /// ```compile_fail,E0599
    /// use baozi_import::{ImportWorkClass, ParseContext, ParseFailure, ParseResult};
    /// fn nested(context: &mut ParseContext<'_, '_>) -> ParseResult<()> {
    ///     context.with_parser_execution(|root| {
    ///         let tokens = root.bind_group([ImportWorkClass::Tokens])?;
    ///         root.child(&tokens, [1], |child| {
    ///             child.child(&tokens, [1], |nested| nested.run([1], || Ok::<_, ParseFailure>(())))
    ///         })
    ///     })
    /// }
    /// ```
    pub fn children(&mut self) -> ParseResult<ParseChildren<'_, 'op, 'session>> {
        self.views.preflight()?;
        Ok(ParseChildren {
            numeric: self.numeric.child_admission(),
            vocabulary: self.vocabulary,
            views: self.views,
        })
    }
    #[allow(
        clippy::result_large_err,
        reason = "the closed child host retains original E and inline admission/close evidence without failure-path allocation"
    )]
    pub fn child<T, E: From<ParseFailure>, const M: usize>(
        &mut self,
        layout: &ParseWorkLayout<'op, M>,
        amounts: [u64; M],
        body: impl FnOnce(&mut ParseParser<'_, '_, '_, 'op, 'session, M, true>) -> Result<T, E>,
    ) -> Result<T, E> {
        self.children()
            .map_err(E::from)?
            .child(layout, amounts, body)
    }
}

pub(super) struct FormatFailureHost<'b, 'op> {
    storage: FailureStorage<'b, 'op>,
    vocabulary: &'b ImportStoreVocabulary,
    entry: &'b RegistryEntry,
    terminal: &'b RefCell<Option<ScopeAbortCause>>,
    callback_active: &'b Cell<bool>,
    format_failure_storage: &'b RefCell<FailureEvidenceStorage>,
}
impl<'b, 'op> FormatFailureHost<'b, 'op> {
    pub(super) fn services(
        (storage, vocabulary): (FailureStorage<'b, 'op>, &'b ImportStoreVocabulary),
        entry: &'b RegistryEntry,
        terminal: &'b RefCell<Option<ScopeAbortCause>>,
        callback_active: &'b Cell<bool>,
        format_failure_storage: &'b RefCell<FailureEvidenceStorage>,
    ) -> Self {
        Self {
            storage,
            vocabulary,
            entry,
            terminal,
            callback_active,
            format_failure_storage,
        }
    }
    fn exclusive<'session: 'b>(
        storage: FailureStorage<'b, 'op>,
        vocabulary: &'b ImportStoreVocabulary,
        views: &'b ParseViews<'b, 'session>,
    ) -> Self {
        Self {
            storage,
            vocabulary,
            entry: views.entry,
            terminal: views.terminal,
            callback_active: views.callback_active,
            format_failure_storage: views.failure_storage,
        }
    }
    fn abort(&self, cause: ScopeAbortCause) -> ParseFailure {
        latch_abort(self.terminal, cause)
    }
    fn reentrant_capability_failure(&self) -> ParseFailure {
        self.abort(ScopeAbortCause::DescriptorContract {
            code: "parse-capability-reentered",
        })
    }
    fn preflight_active(&self) -> ParseResult<()> {
        if self.callback_active.get() {
            return Err(self.reentrant_capability_failure());
        }
        preflight_terminal(self.terminal)
    }
    fn enter_parse_callback(&self) -> ParseResult<ParseCallbackGuard<'b>> {
        if self.callback_active.replace(true) {
            return Err(self.reentrant_capability_failure());
        }
        Ok(ParseCallbackGuard {
            active: self.callback_active,
        })
    }
    fn capacity_plan_error(
        &self,
        resource: &'static str,
        cause: CapacityPlanError,
    ) -> ParseFailure {
        if let Err(failure) = self.preflight_active() {
            return failure;
        }
        match cause {
            CapacityPlanError::CapacityOverflow | CapacityPlanError::AllocationTooLarge { .. } => {
                self.abort(ScopeAbortCause::ResourceLimit {
                    resource,
                    limit: u64::MAX,
                })
            }
            CapacityPlanError::InvalidState { .. } | CapacityPlanError::StalePlan { .. } => self
                .abort(ScopeAbortCause::DescriptorContract {
                    code: "capacity-plan-contract-failed",
                }),
            cause @ CapacityPlanError::CapacityEnvelopeViolated { .. } => {
                self.abort(ScopeAbortCause::CapacityEnvelope { resource, cause })
            }
            _ => self.abort(ScopeAbortCause::DescriptorContract {
                code: "capacity-plan-error-unknown",
            }),
        }
    }
    fn reserve_retained_storage_cause(
        &mut self,
        class: ImportStorageClass,
        bytes: u64,
        allocations: u64,
    ) -> Result<ImportStoreReservation, ScopeAbortCause> {
        let (phase, dimension) = class.accounting();
        ImportStoreReservation::reserve_failure_storage(
            &mut self.storage,
            self.vocabulary.phase(phase),
            self.vocabulary.dimension(dimension),
            bytes,
            allocations,
        )
        .map_err(|cause| match cause {
            StoreParserWorkError::Runtime(cause) => {
                project_store_runtime_error(self.vocabulary, cause).into_scope_abort_cause()
            }
            StoreParserWorkError::ItemLimit { .. } => ScopeAbortCause::DescriptorContract {
                code: "parse-work-item-limit-lane-invalid",
            },
        })
    }
    fn resize_live(
        &mut self,
        reservation: &mut ImportLiveReservation<'_>,
        bytes: u64,
    ) -> ParseResult<()> {
        self.preflight_active()?;
        let result = reservation
            .reservation
            .resize_failure_storage(&self.storage, bytes)
            .map_err(|cause| match cause {
                StoreParserWorkError::Runtime(cause) => {
                    project_store_runtime_error(self.vocabulary, cause).into_scope_abort_cause()
                }
                StoreParserWorkError::ItemLimit { .. } => ScopeAbortCause::DescriptorContract {
                    code: "parse-work-item-limit-lane-invalid",
                },
            });
        result.map_err(|cause| self.abort(cause))?;
        Ok(())
    }
}

pub(super) fn capture_format_failure<P: FormatFailureEvidenceProducer>(
    mut host: FormatFailureHost<'_, '_>,
    producer: P,
) -> ParseFailure {
    let format_failure_storage = host.format_failure_storage;

    let mut producer = Some(producer);
    let captured = (|| -> ParseResult<(FormatFailureEnvelope, FormatFailureEvidence)> {
        host.preflight_active()?;
        let failure_storage_is_active = match format_failure_storage.try_borrow() {
            Ok(storage) => storage.is_active(),
            Err(_) => return Err(host.reentrant_capability_failure()),
        };
        if failure_storage_is_active {
            return Err(host.abort(ScopeAbortCause::DescriptorContract {
                code: "format-failure-storage-already-active",
            }));
        }
        let capacity_plan = BoundedFormatFailureDetailSink::capacity_plan()
            .map_err(|cause| host.capacity_plan_error("format-failure-evidence-storage", cause))?;
        let envelope_bytes = capacity_plan
            .envelope_bytes()
            .map_err(|cause| host.capacity_plan_error("format-failure-evidence-storage", cause))?;
        let mut storage = ImportLiveReservation {
            reservation: host
                .reserve_retained_storage_cause(
                    ImportStorageClass::FormatNormalization,
                    envelope_bytes,
                    3,
                )
                .map_err(|cause| host.abort(cause))?,
            scope: PhantomData,
        };
        let allocation_error = |cause| match cause {
            FormatFailureDetailCause::CapacityPlan(cause) => {
                host.capacity_plan_error("format-failure-evidence-storage", cause)
            }
            cause => host.abort(ScopeAbortCause::FormatFailureDetail(cause)),
        };
        let (envelope, owner_storage_bytes) =
            FormatFailureEnvelope::try_allocate(host.entry.descriptor_handle(), capacity_plan)
                .map_err(allocation_error)?;
        let (mut evidence, actual_storage_bytes) =
            BoundedFormatFailureDetailSink::try_allocate(capacity_plan)
                .map_err(allocation_error)?;
        let actual_storage_bytes = actual_storage_bytes
            .checked_add(owner_storage_bytes)
            .ok_or_else(|| {
                host.capacity_plan_error(
                    "format-failure-evidence-storage",
                    CapacityPlanError::CapacityOverflow,
                )
            })?;
        host.resize_live(&mut storage, actual_storage_bytes)?;
        {
            let mut failure_storage = match format_failure_storage.try_borrow_mut() {
                Ok(storage) => storage,
                Err(_) => return Err(host.reentrant_capability_failure()),
            };
            if failure_storage.is_active() {
                return Err(host.abort(ScopeAbortCause::DescriptorContract {
                    code: "format-failure-storage-already-active",
                }));
            }
            failure_storage.reservation = Some(storage.into_reservation());
            failure_storage.format_captured = true;
        }

        let Some(producer) = producer.as_ref() else {
            return Err(host.abort(ScopeAbortCause::DescriptorContract {
                code: "format-failure-producer-was-lost",
            }));
        };
        let code_result = {
            let _callback = host.enter_parse_callback()?;
            catch_unwind(AssertUnwindSafe(|| producer.code()))
        };
        let code = match code_result {
            Ok(code) => code,
            Err(payload) => {
                discard_untrusted_panic_payload(payload);
                return Err(host.abort(ScopeAbortCause::TrustedExtension {
                    boundary: TrustedExtensionBoundary::FormatFailureDetail,
                    code: "format-failure-code-panicked",
                }));
            }
        };
        host.preflight_active()?;
        if !valid_diagnostic_code(code) {
            return Err(host.abort(ScopeAbortCause::DescriptorContract {
                code: "invalid-format-failure-code",
            }));
        }
        evidence
            .store_code(code)
            .map_err(|cause| host.abort(ScopeAbortCause::FormatFailureDetail(cause)))?;
        let details_result = {
            let _callback = host.enter_parse_callback()?;
            catch_unwind(AssertUnwindSafe(|| {
                producer.visit_details(&mut evidence);
            }))
        };
        if let Err(payload) = details_result {
            discard_untrusted_panic_payload(payload);
            return Err(host.abort(ScopeAbortCause::TrustedExtension {
                boundary: TrustedExtensionBoundary::FormatFailureDetail,
                code: "format-failure-detail-panicked",
            }));
        }
        host.preflight_active()?;
        evidence
            .finish()
            .map(|evidence| (envelope, evidence))
            .map_err(|cause| host.abort(ScopeAbortCause::FormatFailureDetail(cause)))
    })();

    let producer_drop = match host.enter_parse_callback() {
        Ok(_callback) => catch_unwind(AssertUnwindSafe(|| drop(producer.take()))),
        Err(failure) => {
            if let Err(payload) = catch_unwind(AssertUnwindSafe(|| drop(producer.take()))) {
                discard_untrusted_panic_payload(payload);
            }
            return failure;
        }
    };
    if let Err(payload) = producer_drop {
        discard_untrusted_panic_payload(payload);
        return host.abort(ScopeAbortCause::TrustedExtension {
            boundary: TrustedExtensionBoundary::FormatFailureDetail,
            code: "format-failure-producer-drop-panicked",
        });
    }

    match captured {
        Ok((envelope, evidence)) => match host.preflight_active() {
            Ok(()) => {
                let storage = format_failure_storage.borrow_mut().reservation.take();
                if storage.is_none() {
                    return host.abort(ScopeAbortCause::DescriptorContract {
                        code: "format-failure-without-storage-reservation",
                    });
                }
                ParseFailure::format(envelope.seal(evidence, storage))
            }
            Err(failure) => failure,
        },
        Err(failure) => failure,
    }
}
