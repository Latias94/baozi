//! Concrete import roles over the one retained operation execution owner.
use super::*;
use crate::ledger::ImportOperationOwner;
use crate::store::{ImportExecutionFailure, ImportStoreVocabulary, project_store_runtime_error};
use baozi_operation::__retained_store_integration::activation::{
    StoreExtensionResult, StoreFailureStorage, StoreInvocation, StoreInvocationError,
    StoreInvocationLayout, StoreInvocationParser, StoreOrdinaryStorage, StoreParserWorkError,
};

type Numeric<'op> =
    StoreInvocation<'op, ImportOperationOwner, ImportProgressPhase, ImportProgressUnit>;
type OrdinaryStorage<'b, 'op> =
    StoreOrdinaryStorage<'b, 'op, ImportOperationOwner, ImportProgressPhase, ImportProgressUnit>;
type FailureStorage<'b, 'op> =
    StoreFailureStorage<'b, 'op, ImportOperationOwner, ImportProgressPhase, ImportProgressUnit>;

#[derive(Clone, Copy)]
struct ParseViews<'borrow, 'session> {
    entry: &'borrow RegistryEntry,
    source: &'session AssetPath,
    bytes: &'session [u8],
    limits: &'session ResourceLimits,
    issuer: &'session DependencyOriginIssuer,
    terminal: &'session RefCell<Option<ScopeAbortCause>>,
    callback_active: &'borrow Cell<bool>,
    failure_storage: &'session RefCell<FailureEvidenceStorage>,
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
}
impl ParseErrorView<'_> {
    pub fn resource_limit_exceeded(&self, resource: &'static str, limit: u64) -> ParseFailure {
        latch_abort(
            self.terminal,
            ScopeAbortCause::ResourceLimit { resource, limit },
        )
    }
}

/// Layouts borrow the invariant numeric region; they carry no owning work permit.
#[derive(Clone, Copy)]
pub struct ParseWorkLayout<'op, const N: usize> {
    raw: StoreInvocationLayout<'op, N>,
    item_limits: [Option<(&'static str, u64)>; N],
}

/// The host owns the root; a decoder receives only closed physical callbacks.
///
/// ```
/// use baozi_import::{ParseContext, ParseFailure, ImportWorkClass};
/// fn owned(context: &mut ParseContext<'_>) -> Result<u32, ParseFailure> {
///     context.with_parser_execution(|root| {
///         let group = root.bind_group([ImportWorkClass::Tokens])?;
///         root.child(&group, [1], |child| child.run([1], || Ok(7)))
///     })
/// }
/// ```
///
/// ```compile_fail,E0501,E0500
/// use baozi_import::{ParseContext, ParseFailure, ImportWorkClass};
/// fn reenter(context: &mut ParseContext<'_>) -> Result<(), ParseFailure> {
///     context.with_parser_execution(|root| {
///         let group = root.bind_group([ImportWorkClass::Tokens])?;
///         root.child(&group, [1], |child| child.run([1], || root.poll()))
///     })
/// }
/// ```
///
/// ```compile_fail
/// use baozi_import::{ParseContext, ParseFailure, ImportWorkClass};
/// fn escape(context: &mut ParseContext<'_>) {
///     let _layout = context.with_parser_execution::<_, ParseFailure>(|root| root.bind_group([ImportWorkClass::Tokens]));
/// }
/// ```
///
/// Separate owners can return owned values; their bound layouts cannot be mixed:
///
/// ```
/// use baozi_import::{ParseContext, ParseFailure, ImportWorkClass};
/// fn separate(first: &mut ParseContext<'_>, second: &mut ParseContext<'_>) -> Result<(u32,u32), ParseFailure> {
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
/// fn mixed(first: &mut ParseContext<'_>, second: &mut ParseContext<'_>) -> Result<(), ParseFailure> {
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
/// fn escape_root(context: &mut ParseContext<'_>) {
///     let _root = context.with_parser_execution::<_, ParseFailure>(|root| Ok(root));
/// }
/// ```
///
/// ```
/// use baozi_import::{ParseContext, ParseFailure};
/// fn send_owned(context: &mut ParseContext<'_>) -> Result<u32, ParseFailure> {
///     let value = context.with_parser_execution::<_, ParseFailure>(|_| Ok(7_u32))?;
///     Ok(std::thread::scope(|threads| threads.spawn(move || value).join().unwrap()))
/// }
/// ```
///
/// ```compile_fail,E0277
/// use baozi_import::{ParseContext, ParseFailure};
/// fn send_context(context: &mut ParseContext<'_>) -> Result<(), ParseFailure> {
///     context.with_parser_execution(|root| {
///         std::thread::scope(|threads| { threads.spawn(move || root.poll()); });
///         Ok(())
///     })
/// }
/// ```
pub struct ParseRoot<'borrow, 'op, 'session> {
    numeric: &'borrow mut Numeric<'op>,
    vocabulary: &'borrow ImportStoreVocabulary,
    views: ParseViews<'borrow, 'session>,
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
    views: ParseViews<'borrow, 'session>,
}

impl<'session> ParseContext<'session> {
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
        };
        let result = self
            .store_owner
            .with_parser_execution(|numeric, vocabulary| {
                let mut root = ParseRoot {
                    numeric,
                    vocabulary,
                    views,
                };
                let result = action(&mut root);
                root.numeric.settle_returned_callback();
                // Import's normal region exit polls on Ok and eligible semantic Err. The operation
                // region does not manufacture a second final poll or a Requested(0) classification.
                if result.is_err() {
                    root.numeric.stop_known_error();
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
    /// Borrows ordinary storage/poll services without lending numeric work.
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
        self.numeric.stop_known_error();
        capture_format_failure(
            FormatFailureHost::exclusive(
                self.numeric.failure_storage(),
                self.vocabulary,
                self.views,
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
        self.numeric
            .prescribed_exit_checkpoint()
            .map_err(|cause| self.views.runtime(self.vocabulary, cause))
    }
    pub fn poll(&mut self) -> ParseResult<()> {
        self.views.preflight()?;
        self.numeric
            .checkpoint()
            .map_err(|cause| self.views.runtime(self.vocabulary, cause))
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
    pub fn work_counters(&self, class: ImportWorkClass) -> ParseResult<(u64, u64, u64)> {
        self.numeric
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
        self.views.preflight().map_err(E::from)?;
        let views = self.views;
        let vocabulary = self.vocabulary;
        let result = self.numeric.parser_child(
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
        let views = self.views;
        let vocabulary = self.vocabulary;
        let result = self.numeric.parser_parent(
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
    /// Performs one real caller-owned reader call. As in read_asset_chunk, this bills opaque
    /// work; the snapshot owner separately admits/reconciles observed payload byte capacity.
    pub fn read_chunk(
        &mut self,
        opened: &mut OpenedAsset,
        buffer: &mut [u8],
    ) -> ParseResult<Result<usize, baozi_io::AssetReadError>> {
        self.poll()?;
        let raw = self
            .numeric
            .bind_group([self.vocabulary.dimension(ImportDimension::WorkUnits)])
            .map_err(|cause| self.views.runtime(self.vocabulary, cause))?;
        match self
            .numeric
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
        }
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
    fn exit_poll(&mut self) -> ParseResult<()> {
        self.views.preflight()?;
        self.numeric
            .prescribed_exit_checkpoint()
            .map_err(|cause| self.views.runtime(self.vocabulary, cause))
    }
    pub fn poll(&mut self) -> ParseResult<()> {
        self.views.preflight()?;
        self.numeric
            .checkpoint()
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
impl<'op, 'session, const N: usize> ParseParser<'_, '_, '_, 'op, 'session, N, false> {
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
        self.views.preflight().map_err(E::from)?;
        let views = self.views;
        let vocabulary = self.vocabulary;
        let result = self.numeric.child(
            layout.raw,
            amounts,
            layout.item_limits.map(|x| x.map(|(_, v)| v)),
            |numeric| {
                body(&mut ParseParser {
                    numeric,
                    vocabulary,
                    views,
                })
                .map_err(ImportExecutionFailure::Owned)
            },
        );
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
                Err(E::from(views.abort(ScopeAbortCause::DescriptorContract {
                    code: "parse-work-callback-incomplete",
                })))
            }
            Err(ImportExecutionFailure::Runtime(cause)) => Err(E::from(views.item_error(
                vocabulary,
                layout.item_limits,
                cause,
            ))),
        }
    }
}

// Two concrete storage borrowers call the same retained reserve/resize owners. No numeric trait,
// alternate ledger or callback policy is introduced for this cold error materializer.
enum FormatStorage<'b, 'op> {
    Legacy(&'b mut ImportStoreExecution<'op>),
    Exclusive(FailureStorage<'b, 'op>, &'b ImportStoreVocabulary),
}
pub(super) struct FormatFailureHost<'b, 'op> {
    storage: FormatStorage<'b, 'op>,
    entry: &'b RegistryEntry,
    terminal: &'b RefCell<Option<ScopeAbortCause>>,
    callback_active: &'b Cell<bool>,
    format_failure_storage: &'b RefCell<FailureEvidenceStorage>,
}
impl<'b, 'op> FormatFailureHost<'b, 'op> {
    pub(super) fn legacy(
        store: &'b mut ImportStoreExecution<'op>,
        entry: &'b RegistryEntry,
        terminal: &'b RefCell<Option<ScopeAbortCause>>,
        callback_active: &'b Cell<bool>,
        format_failure_storage: &'b RefCell<FailureEvidenceStorage>,
    ) -> Self {
        Self {
            storage: FormatStorage::Legacy(store),
            entry,
            terminal,
            callback_active,
            format_failure_storage,
        }
    }
    fn exclusive<'session: 'b>(
        storage: FailureStorage<'b, 'op>,
        vocabulary: &'b ImportStoreVocabulary,
        views: ParseViews<'b, 'session>,
    ) -> Self {
        Self {
            storage: FormatStorage::Exclusive(storage, vocabulary),
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
        match &mut self.storage {
            FormatStorage::Legacy(store) => store
                .reserve_retained(phase, dimension, bytes, allocations)
                .map_err(|cause| cause.into_scope_abort_cause()),
            FormatStorage::Exclusive(store, vocabulary) => {
                ImportStoreReservation::reserve_failure_storage(
                    store,
                    vocabulary.phase(phase),
                    vocabulary.dimension(dimension),
                    bytes,
                    allocations,
                )
                .map_err(|cause| match cause {
                    StoreParserWorkError::Runtime(cause) => {
                        project_store_runtime_error(vocabulary, cause).into_scope_abort_cause()
                    }
                    StoreParserWorkError::ItemLimit { .. } => ScopeAbortCause::DescriptorContract {
                        code: "parse-work-item-limit-lane-invalid",
                    },
                })
            }
        }
    }
    fn resize_live(
        &mut self,
        reservation: &mut ImportLiveReservation<'_>,
        bytes: u64,
    ) -> ParseResult<()> {
        self.preflight_active()?;
        let result = match &mut self.storage {
            FormatStorage::Legacy(store) => store
                .resize(&mut reservation.reservation, bytes)
                .map_err(|cause| cause.into_scope_abort_cause()),
            FormatStorage::Exclusive(store, vocabulary) => reservation
                .reservation
                .resize_failure_storage(store, bytes)
                .map_err(|cause| match cause {
                    StoreParserWorkError::Runtime(cause) => {
                        project_store_runtime_error(vocabulary, cause).into_scope_abort_cause()
                    }
                    StoreParserWorkError::ItemLimit { .. } => ScopeAbortCause::DescriptorContract {
                        code: "parse-work-item-limit-lane-invalid",
                    },
                }),
        };
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
