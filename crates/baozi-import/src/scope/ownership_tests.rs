//! Real session ownership checks; no accounting or execution substitute is used.
//!
//! Work layouts are invariant in the root invocation. The paired owner/layout examples on
//! [`ParseRoot`] enforce rejection before any callback, including zero amounts and empty layouts.
//! The corresponding valid callbacks keep their layouts in their own roots:
//!
//! ```
//! use baozi_import::{ImportWorkClass, ParseContext, ParseFailure};
//! fn local(context: &mut ParseContext<'_, '_>, amount: u64) -> Result<(), ParseFailure> {
//!     context.with_execution(|root| {
//!         let tokens = root.bind_group([ImportWorkClass::Tokens])?;
//!         root.child(&tokens, [amount], |child| child.run([amount], || Ok(())))?;
//!         let empty = root.bind_group([])?;
//!         root.child(&empty, [], |child| child.run([], || Ok(())))
//!     })
//! }
//! ```
//!
//! ```compile_fail
//! use baozi_import::{ImportWorkClass, ParseContext, ParseFailure};
//! fn foreign(first: &mut ParseContext<'_, '_>, second: &mut ParseContext<'_, '_>, amount: u64)
//!     -> Result<(), ParseFailure>
//! {
//!     first.with_execution(|sender| {
//!         let tokens = sender.bind_group([ImportWorkClass::Tokens])?;
//!         second.with_execution(|receiver| {
//!             receiver.child(&tokens, [amount], |child| child.run([amount], || Ok(())))
//!         })
//!     })
//! }
//! ```
//!
//! ```compile_fail
//! use baozi_import::{ParseContext, ParseFailure};
//! fn foreign_empty(first: &mut ParseContext<'_, '_>, second: &mut ParseContext<'_, '_>)
//!     -> Result<(), ParseFailure>
//! {
//!     first.with_execution(|sender| {
//!         let empty = sender.bind_group([])?;
//!         second.with_execution(|receiver| {
//!             receiver.child(&empty, [], |child| child.run([], || Ok(())))
//!         })
//!     })
//! }
//! ```

use super::*;
use crate::SourceScopeDisposition;
use crate::descriptor::{
    TextBomPolicy, TextControlPolicy, TextHardCaps, TextLexicalMode, TextNewline, TextProfile,
    TextSourceEncoding,
};
use crate::ledger::ImportOperationControl;
use crate::store::{
    ImportRootExecution, ImportStoreBodyParts, ImportStoreBootstrapRequirements,
    ImportStoreHostError, ImportStoreInvocationRoot, try_import_store_bootstrap,
};
use crate::text_input::TextInputRecordCursor;
use baozi_core::__scene_integration::integration::SceneParseOutcome;
use baozi_operation::{
    PhysicalStorageAccount, PhysicalStorageAccountFactory, PhysicalStorageAccountFactoryLimits,
    PhysicalStorageAccountLimits,
};

fn start_store<'inv>(
    parts: ImportStoreBodyParts<'inv>,
) -> (ImportStoreSessionOwner<'inv>, PhysicalStorageAccount) {
    let limits = ResourceLimits::default();
    let factory = PhysicalStorageAccountFactory::try_new(
        PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1).unwrap(),
    )
    .unwrap();
    let account = factory
        .try_create_account(PhysicalStorageAccountLimits::new(u64::MAX, u64::MAX).unwrap())
        .unwrap();
    let requirements = ImportStoreBootstrapRequirements::for_node_scene(&limits).unwrap();
    let bootstrap = try_import_store_bootstrap(&limits, requirements).unwrap();
    let owner = ImportStoreSessionOwner::try_start_in_body(
        parts,
        ImportOperationControl::default(),
        &account,
        bootstrap,
    )
    .unwrap();
    (owner, account)
}

fn accounting(exec: &ImportRootExecution<'_, '_>) -> Vec<(u64, u64, u64)> {
    ImportDimension::ALL
        .into_iter()
        .map(|dimension| {
            let state = exec.dimension_state(dimension).unwrap();
            (state.attempted(), state.consumed(), state.live_bytes())
        })
        .collect()
}

fn reject_without_accounting_changes<'session>(
    context: &mut ParseContext<'session, '_>,
    exclusive: bool,
    before: &[(u64, u64, u64)],
    action: impl for<'op> FnOnce(&mut ParseRoot<'_, 'op, 'session>) -> ParseResult<()>,
) -> ParseResult<()> {
    let reject = |root: &mut ParseRoot<'_, '_, 'session>| {
        assert_eq!(accounting(&root.store).as_slice(), before);
        let result = action(root);
        assert!(result.is_err());
        assert_eq!(accounting(&root.store).as_slice(), before);
        result
    };
    if exclusive {
        context.with_parser_execution(reject)
    } else {
        context.with_execution(reject)
    }
}

fn text_profile() -> TextProfile {
    TextProfile::builder(TextHardCaps::new(1024, 1024, 1024, 1024, 1024).unwrap())
        .source_encodings([TextSourceEncoding::Utf8])
        .bom_policy(TextBomPolicy::Forbidden)
        .newlines([TextNewline::Lf])
        .lexical_mode(TextLexicalMode::Utf8Document)
        .decoding_modes([TextDecodingMode::RejectInvalid])
        .control_policy(TextControlPolicy::RejectAll)
        .build()
        .unwrap()
}

#[test]
fn paired_live_contexts_keep_zero_nonzero_and_empty_work_in_their_own_roots() {
    for amount in [Some(0), Some(2), None] {
        let mut sender_root = ImportStoreInvocationRoot::new();
        let mut receiver_root = ImportStoreInvocationRoot::new();
        let mut sender_account = None;
        let mut receiver_account = None;
        let sender_finish = sender_root
            .run_body(|sender_parts, sender_disposition| {
                let (mut sender_owner, started_sender_account) = start_store(sender_parts);
                let receiver_finish = receiver_root
                    .run_body(|receiver_parts, receiver_disposition| {
                        let (mut receiver_owner, started_receiver_account) =
                            start_store(receiver_parts);
                        testing::with_context(&mut sender_owner, |sender_context| {
                            sender_context
                                .with_execution(|sender| -> ParseResult<()> {
                                    let before = accounting(&sender.store);
                                    let physical_before = started_receiver_account.snapshot();
                                    let receiver_called = Cell::new(false);
                                    testing::with_context(
                                        &mut receiver_owner,
                                        |receiver_context| {
                                            assert_eq!(sender.source(), receiver_context.source());
                                            assert!(!std::ptr::eq(
                                                sender.error_view().session_issuer(),
                                                receiver_context.dependency_origin_issuer,
                                            ));
                                            receiver_context
                                                .with_execution(|receiver| -> ParseResult<()> {
                                                    assert_eq!(
                                                        receiver.work_counters(
                                                            ImportWorkClass::Tokens
                                                        )?,
                                                        (0, 0, 0),
                                                    );
                                                    if let Some(amount) = amount {
                                                        let group = receiver.bind_group([
                                                            ImportWorkClass::Tokens,
                                                        ])?;
                                                        receiver.child(
                                                            &group,
                                                            [amount],
                                                            |child| {
                                                                receiver_called.set(true);
                                                                child.run([amount], || {
                                                                    Ok::<_, ParseFailure>(())
                                                                })
                                                            },
                                                        )?;
                                                    } else {
                                                        let group = receiver.bind_group([])?;
                                                        receiver.child(&group, [], |child| {
                                                            receiver_called.set(true);
                                                            child.run([], || {
                                                                Ok::<_, ParseFailure>(())
                                                            })
                                                        })?;
                                                    }
                                                    let completed = amount.unwrap_or(0);
                                                    assert_eq!(
                                                        receiver.work_counters(
                                                            ImportWorkClass::Tokens
                                                        )?,
                                                        (completed, completed, 0),
                                                    );
                                                    Ok(())
                                                })
                                                .unwrap();
                                            assert!(receiver_context.terminal.borrow().is_none());
                                            assert!(
                                                receiver_context
                                                    .source_scope_record
                                                    .borrow()
                                                    .is_none()
                                            );
                                        },
                                    );
                                    assert!(receiver_called.get());
                                    assert_eq!(accounting(&sender.store), before);
                                    let physical_after = started_receiver_account.snapshot();
                                    assert_eq!(
                                        physical_after.live_retained_bytes(),
                                        physical_before.live_retained_bytes(),
                                    );
                                    assert_eq!(
                                        physical_after.live_retained_allocations(),
                                        physical_before.live_retained_allocations(),
                                    );
                                    let sender_called = Cell::new(false);
                                    if let Some(amount) = amount {
                                        let group = sender.bind_group([ImportWorkClass::Tokens])?;
                                        sender.child(&group, [amount], |child| {
                                            sender_called.set(true);
                                            child.run([amount], || Ok::<_, ParseFailure>(()))
                                        })?;
                                    } else {
                                        let group = sender.bind_group([])?;
                                        sender.child(&group, [], |child| {
                                            sender_called.set(true);
                                            child.run([], || Ok::<_, ParseFailure>(()))
                                        })?;
                                    }
                                    assert!(sender_called.get());
                                    let completed = amount.unwrap_or(0);
                                    assert_eq!(
                                        sender.work_counters(ImportWorkClass::Tokens)?,
                                        (completed, completed, 0),
                                    );
                                    sender.poll()
                                })
                                .unwrap();
                        });
                        receiver_owner
                            .request_finish(receiver_disposition, SceneParseOutcome::Rejected)
                            .unwrap();
                        receiver_account = Some(started_receiver_account);
                    })
                    .unwrap();
                let _receiver_finish = receiver_finish;
                sender_owner
                    .request_finish(sender_disposition, SceneParseOutcome::Rejected)
                    .unwrap();
                sender_account = Some(started_sender_account);
            })
            .unwrap();
        let _sender_finish = sender_finish;
        drop(sender_root);
        drop(receiver_root);
        for account in [sender_account.unwrap(), receiver_account.unwrap()] {
            let released = account.snapshot();
            assert_eq!(released.live_retained_bytes(), 0, "{amount:?}");
            assert_eq!(released.live_retained_allocations(), 0, "{amount:?}");
            assert!(!released.poisoned(), "{amount:?}");
        }
    }
}

#[test]
fn paired_live_contexts_reject_foreign_capabilities_before_zero_or_nonzero_actions() {
    #[derive(Clone, Copy, Debug)]
    enum Case {
        Vector { grow: bool },
        ExclusiveVector { grow: bool },
        PrepareVector { additional: usize },
        PrepareString { additional: usize },
        EmptyDecodedAppend,
        SourceObserveEmpty,
        SourceFinish,
        SourceRecord,
        Cursor { exhausted: bool },
    }

    for case in [
        Case::Vector { grow: false },
        Case::Vector { grow: true },
        Case::ExclusiveVector { grow: false },
        Case::ExclusiveVector { grow: true },
        Case::PrepareVector { additional: 0 },
        Case::PrepareVector { additional: 1 },
        Case::PrepareVector { additional: 8 },
        Case::PrepareString { additional: 0 },
        Case::PrepareString { additional: 1 },
        Case::PrepareString { additional: 8 },
        Case::EmptyDecodedAppend,
        Case::SourceObserveEmpty,
        Case::SourceFinish,
        Case::SourceRecord,
        Case::Cursor { exhausted: false },
        Case::Cursor { exhausted: true },
    ] {
        let mut sender_root = ImportStoreInvocationRoot::new();
        let mut receiver_root = ImportStoreInvocationRoot::new();
        let mut sender_account = None;
        let mut receiver_account = None;
        let sender_finish = sender_root
            .run_body(|sender_parts, sender_disposition| {
                let (mut sender_owner, started_sender_account) = start_store(sender_parts);
                let receiver_finish = receiver_root
                    .run_body(|receiver_parts, receiver_disposition| {
                        let (mut receiver_owner, started_receiver_account) =
                            start_store(receiver_parts);
                        testing::with_context(&mut sender_owner, |context| {
                            context
                .with_execution(|sender| -> ParseResult<()> {
                    let io = baozi_io::MemoryAssetIo::new();
                    let authority = io.authority_domain();
                    let source = sender.source().clone();
                    let issuer = DependencyOriginIssuer::new();
                    let limits = ResourceLimits::default();
                    let diagnostics = RefCell::new(DiagnosticSink::new(16, 4096, 512, 8192));
                    let graph = RefCell::new(ResourceGraph::new(limits.clone()));
                    let optional_evidence = RefCell::new(AccountedVec::new());
                    let loss_evidence = RefCell::new(AccountedVec::new());
                    let source_scope = RefCell::new(None);
                    let failure_storage = RefCell::new(FailureEvidenceStorage::default());
                    let terminal = RefCell::new(None);
                    let mut receiver = ParseContext::new(ParseContextParts {
                        entry: sender.views.entry.clone(),
                        primary_io: &io,
                        primary_authority: &authority,
                        primary_source: &source,
                        primary_bytes: b"fixture",
                        resolver: ResolverBinding::registered(
                            baozi_io::__semantic_identity::same_authority_resolver_v1(),
                        ),
                        dependency_origin_issuer: &issuer,
                        conformance_policy: ConformancePolicy::Strict,
                        loss_policy: LossPolicy::preserve_or_fail(),
                        limits: &limits,
                        store_owner: &mut receiver_owner,
                        diagnostics: &diagnostics,
                        resource_graph: &graph,
                        optional_dependency_evidence: &optional_evidence,
                        loss_evidence: &loss_evidence,
                        source_scope_record: &source_scope,
                        format_failure_storage: &failure_storage,
                        terminal: &terminal,
                    });
                    assert_eq!(sender.source(), receiver.source());
                    assert!(!std::ptr::eq(sender.error_view().session_issuer(), &issuer));
                    let receiver_before = receiver
                        .store_owner
                        .with_root_execution(|root| {
                            Ok::<_, ImportStoreHostError>(accounting(root))
                        })
                        .unwrap();
                    let physical_before = started_receiver_account.snapshot();
                    let callback_called = Cell::new(false);

                    let rejected = match case {
                        Case::Vector { grow } => {
                            let mut values = sender.services()?.metered_vec::<u8>(
                                ImportStorageClass::FormatNormalization,
                                "foreign-session-vector",
                            );
                            if grow {
                                values.try_reserve(&mut sender.services()?, 2)?;
                                values.try_push_reserved(7)?;
                            }
                            let before = accounting(&sender.store);
                            let source_exec = &mut *sender;
                            reject_without_accounting_changes(&mut receiver, false, &receiver_before, move |exec| {
                                let mut values = values;
                                let capacity = values.capacity();
                                let additional = if grow { capacity + 1 } else { 0 };
                                let rejected = values.try_reserve(&mut exec.services()?, additional);
                                assert!(rejected.is_err(), "{case:?}");
                                assert_eq!(values.capacity(), capacity);
                                assert_eq!(values.as_slice(), if grow { &[7][..] } else { &[] });
                                assert_eq!(accounting(&source_exec.store), before, "{case:?}");
                                values.identity.check(&source_exec.services()?)?;
                                if grow {
                                    values.try_push_reserved(9)?;
                                    assert_eq!(values.as_slice(), &[7, 9]);
                                }
                                rejected
                            })
                        }
                        Case::ExclusiveVector { .. } | Case::PrepareVector { .. } => {
                            let grow = !matches!(case, Case::ExclusiveVector { grow: false });
                            let mut values = sender.services()?.metered_vec::<u8>(
                                ImportStorageClass::FormatNormalization,
                                "foreign-exclusive-vector",
                            );
                            if grow {
                                values.try_reserve(&mut sender.services()?, 2)?;
                                values.try_push_reserved(7)?;
                            }
                            let before = accounting(&sender.store);
                            let source_exec = &mut *sender;
                            reject_without_accounting_changes(&mut receiver, true, &receiver_before, move |root| {
                                let mut values = values;
                                let capacity = values.capacity();
                                let pointer = values.as_slice().as_ptr();
                                let token_bytes = values.capacity.as_ref().map(ImportStoreReservation::bytes);
                                let additional = if grow { capacity + 1 } else { 0 };
                                let rejected = if let Case::PrepareVector { additional } = case {
                                    inject_next_metered_allocation_failure();
                                    let result = root.storage()?.prepare_vec_replacement(&values, additional).map(drop);
                                    assert!(FAIL_NEXT_METERED_ALLOCATION.with(|fault| fault.replace(false)));
                                    result
                                } else {
                                    root.storage()?.reserve_vec(&mut values, additional)
                                };
                                assert!(rejected.is_err());
                                assert_eq!(values.capacity(), capacity);
                                assert_eq!(values.as_slice().as_ptr(), pointer);
                                assert_eq!(values.capacity.as_ref().map(ImportStoreReservation::bytes), token_bytes);
                                assert_eq!(values.as_slice(), if grow { &[7][..] } else { &[] });
                                assert_eq!(accounting(&source_exec.store), before);
                                values.identity.check(&source_exec.services()?)?;
                                if grow {
                                    values.try_push_reserved(9)?;
                                    assert_eq!(values.as_slice(), &[7, 9]);
                                }
                                rejected
                            })
                        }
                        Case::PrepareString { additional } => {
                            let mut text = sender.services()?.metered_string(
                                ImportStorageClass::FormatNormalization, "foreign-prepared-string",
                            );
                            text.try_push_str(&mut sender.services()?, "a")?;
                            let before = accounting(&sender.store);
                            let source_exec = &mut *sender;
                            reject_without_accounting_changes(&mut receiver, true, &receiver_before, move |root| {
                                let mut text = text;
                                let old = (text.as_str().as_ptr(), text.capacity(), text.capacity.as_ref().unwrap().bytes());
                                FAIL_NEXT_METERED_QUALIFICATION.with(|fault| fault.set(true));
                                let rejected = root.storage()?.prepare_string_replacement(&text, additional).map(drop);
                                assert!(FAIL_NEXT_METERED_QUALIFICATION.with(|fault| fault.replace(false)));
                                assert!(rejected.is_err());
                                assert_eq!(text.as_str(), "a");
                                assert_eq!(old, (text.as_str().as_ptr(), text.capacity(), text.capacity.as_ref().unwrap().bytes()));
                                assert_eq!(accounting(&source_exec.store), before);
                                text.identity.check(&source_exec.services()?)?;
                                text.try_push_str_reserved("b")?;
                                assert_eq!(text.as_str(), "ab");
                                rejected
                            })
                        }
                        Case::EmptyDecodedAppend => {
                            let text = sender.services()?.metered_decoded_text(0, "foreign-decoded-text")?;
                            let before = accounting(&sender.store);
                            let source_exec = &mut *sender;
                            reject_without_accounting_changes(&mut receiver, false, &receiver_before, move |exec| {
                                let mut text = text;
                                let rejected = text.try_push_str(&mut exec.services()?, "");
                                assert!(rejected.is_err());
                                assert!(text.is_empty());
                                assert_eq!(accounting(&source_exec.store), before);
                                text.identity.check(&source_exec.services()?)?;
                                rejected
                            })
                        }
                        Case::SourceObserveEmpty | Case::SourceFinish | Case::SourceRecord => {
                            let before_builder = accounting(&sender.store);
                            let selection_outcome = SourceScopeSelectionOutcome::parentless_node_projection(
                                sender.source().clone(),
                            );
                            let builder = sender.services()?.source_scope_builder(
                                SourceScopeSelectionRequest::DocumentDefault,
                                selection_outcome,
                            )?;
                            let before = accounting(&sender.store);
                            match case {
                                Case::SourceObserveEmpty => {
                                    let source_exec = &*sender;
                                    reject_without_accounting_changes(&mut receiver, false, &receiver_before, move |exec| {
                                        let mut builder = builder;
                                        let rejected = builder.observe(
                                            &mut exec.services()?,
                                            SourceScopeEntityKind::Node,
                                            SourceScopeDisposition::Selected,
                                            "",
                                        );
                                        assert!(rejected.is_err());
                                        assert_eq!(accounting(&source_exec.store), before);
                                        rejected
                                    })
                                }
                                Case::SourceFinish => {
                                    let source_exec = &*sender;
                                    reject_without_accounting_changes(&mut receiver, false, &receiver_before, move |exec| {
                                        let rejected = builder.finish(&mut exec.services()?).map(|_| ());
                                        assert!(rejected.is_err());
                                        let after = accounting(&source_exec.store);
                                        for ((after, before), initial) in
                                            after.iter().zip(&before).zip(&before_builder)
                                        {
                                            assert_eq!((after.0, after.1), (before.0, before.1));
                                            assert_eq!(after.2, initial.2);
                                        }
                                        rejected
                                    })
                                }
                                Case::SourceRecord => {
                                    let record = builder.finish(&mut sender.services()?)?;
                                    let source_exec = &*sender;
                                    reject_without_accounting_changes(&mut receiver, false, &receiver_before, move |exec| {
                                        let rejected = exec.services()?.record_source_scope(record);
                                        assert!(rejected.is_err());
                                        let after = accounting(&source_exec.store);
                                        for ((after, before), initial) in
                                            after.iter().zip(&before).zip(&before_builder)
                                        {
                                            assert_eq!((after.0, after.1), (before.0, before.1));
                                            assert_eq!(after.2, initial.2);
                                        }
                                        rejected
                                    })
                                }
                                _ => unreachable!(),
                            }
                        }
                        Case::Cursor { exhausted } => {
                            let profile = text_profile();
                            let source = sender.services()?.diagnostic_source();
                            let mut options = TextInputOptions::new(
                                TextDecodingMode::RejectInvalid,
                                "foreign-record-cursor",
                            );
                            if exhausted {
                                options = options.with_raw_end(0);
                            }
                            let cursor = TextInputRecordCursor::new(
                                &mut sender.children()?,
                                Some(&profile),
                                source,
                                options,
                            )?
                            .unwrap();
                            let before = accounting(&sender.store);
                            let source_exec = &*sender;
                            let called = &callback_called;
                            reject_without_accounting_changes(&mut receiver, false, &receiver_before, move |exec| {
                                let mut cursor = cursor;
                                let offset = cursor.raw_offset();
                                let rejected = cursor
                                    .with_next_record(&mut exec.children()?, |_exec, _input, _recovery| {
                                        called.set(true);
                                        Ok(())
                                    })
                                    .map(|_| ());
                                assert!(rejected.is_err(), "{case:?}");
                                assert_eq!(cursor.raw_offset(), offset);
                                assert_eq!(accounting(&source_exec.store), before);
                                rejected
                            })
                        }
                    };
                    assert!(rejected.is_err(), "{case:?}");
                    assert!(!callback_called.get(), "{case:?}");
                    let expected = if matches!(case, Case::SourceRecord) {
                        "source-scope-record-owner-mismatch"
                    } else {
                        "parse-storage-foreign-session"
                    };
                    assert!(matches!(
                        terminal.borrow().as_ref(),
                        Some(ScopeAbortCause::DescriptorContract { code }) if *code == expected
                    ));
                    assert!(
                        receiver
                            .with_execution(|_exec| -> ParseResult<()> {
                                panic!("a rejected receiver must remain stopped")
                            })
                            .is_err()
                    );
                    assert!(
                        receiver
                            .store_owner
                            .with_root_execution(|_| Ok::<_, ImportStoreHostError>(()))
                            .is_err(),
                        "known-error state persists after the failed region: {case:?}"
                    );
                    let physical_after = started_receiver_account.snapshot();
                    assert_eq!(
                        physical_after.live_retained_bytes(),
                        physical_before.live_retained_bytes(),
                    );
                    assert_eq!(
                        physical_after.live_retained_allocations(),
                        physical_before.live_retained_allocations(),
                    );
                    assert!(source_scope.borrow().is_none());
                    sender.poll()?;
                    Ok(())
                })
                .unwrap();
                        });
                        receiver_owner
                            .request_finish(receiver_disposition, SceneParseOutcome::Rejected)
                            .unwrap();
                        receiver_account = Some(started_receiver_account);
                    })
                    .unwrap();
                let _receiver_finish = receiver_finish;
                sender_owner
                    .request_finish(sender_disposition, SceneParseOutcome::Rejected)
                    .unwrap();
                sender_account = Some(started_sender_account);
            })
            .unwrap();
        let _sender_finish = sender_finish;
        drop(sender_root);
        drop(receiver_root);
        for account in [sender_account.unwrap(), receiver_account.unwrap()] {
            let released = account.snapshot();
            assert_eq!(released.live_retained_bytes(), 0, "{case:?}");
            assert_eq!(released.live_retained_allocations(), 0, "{case:?}");
            assert!(!released.poisoned(), "{case:?}");
        }
    }
}

#[test]
fn source_builder_and_cursor_keep_their_owner_across_execution_regions() {
    let mut invocation_root = ImportStoreInvocationRoot::new();
    let mut account = None;
    let finish = invocation_root
        .run_body(|parts, disposition| {
            let (mut owner, started_account) = start_store(parts);
            testing::with_context(&mut owner, |context| {
                let (mut builder, mut cursor) = context
                    .with_execution(|exec| -> ParseResult<_> {
                        let selection_outcome =
                            SourceScopeSelectionOutcome::parentless_node_projection(
                                exec.source().clone(),
                            );
                        let builder = exec.services()?.source_scope_builder(
                            SourceScopeSelectionRequest::DocumentDefault,
                            selection_outcome,
                        )?;
                        let source = exec.services()?.diagnostic_source();
                        let cursor = TextInputRecordCursor::new(
                            &mut exec.children()?,
                            Some(&text_profile()),
                            source,
                            TextInputOptions::new(
                                TextDecodingMode::RejectInvalid,
                                "same-session-cursor",
                            ),
                        )?
                        .unwrap();
                        Ok((builder, cursor))
                    })
                    .unwrap();
                let record = context
                    .with_execution(|exec| -> ParseResult<_> {
                        builder.observe(
                            &mut exec.services()?,
                            SourceScopeEntityKind::Node,
                            SourceScopeDisposition::Selected,
                            "nodes/zero",
                        )?;
                        assert_eq!(
                            cursor.with_next_record(
                                &mut exec.children()?,
                                |_exec, input, _recovery| {
                                    assert_eq!(input.text(), "fixture");
                                    Ok(())
                                },
                            )?,
                            Ok(Some(())),
                        );
                        assert_eq!(cursor.raw_offset(), 7);
                        assert_eq!(
                            cursor.with_next_record(
                                &mut exec.children()?,
                                |_exec, _input, _recovery| -> ParseResult<()> {
                                    panic!("an exhausted correct-session cursor must not invoke its callback")
                                },
                            )?,
                            Ok(None),
                        );
                        builder.finish(&mut exec.services()?)
                    })
                    .unwrap();
                context
                    .with_execution(|exec| exec.services()?.record_source_scope(record))
                    .unwrap();
                let record = context
                    .source_scope_record
                    .borrow_mut()
                    .take()
                    .unwrap()
                    .into_terminal();
                assert_eq!(record.counts(SourceScopeEntityKind::Node).selected(), 1);
                assert_eq!(record.details().next().unwrap().identity(), "nodes/zero");
            });
            owner
                .request_finish(disposition, SceneParseOutcome::Rejected)
                .unwrap();
            account = Some(started_account);
        })
        .unwrap();
    let _finish = finish;
    drop(invocation_root);
    let released = account.unwrap().snapshot();
    assert_eq!(released.live_retained_bytes(), 0);
    assert_eq!(released.live_retained_allocations(), 0);
    assert!(!released.poisoned());
}
