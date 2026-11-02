//! Real session ownership checks; no accounting or execution substitute is used.

use super::*;
use crate::SourceScopeDisposition;
use crate::descriptor::{
    TextBomPolicy, TextControlPolicy, TextHardCaps, TextLexicalMode, TextNewline, TextProfile,
    TextSourceEncoding,
};
use crate::ledger::ImportOperationControl;
use crate::store::{ImportStoreBootstrapRequirements, try_import_store_bootstrap};
use crate::text_input::TextInputRecordCursor;
use baozi_operation::{
    PhysicalStorageAccount, PhysicalStorageAccountFactory, PhysicalStorageAccountFactoryLimits,
    PhysicalStorageAccountLimits,
};

fn start_store() -> (ImportStoreSessionOwner, PhysicalStorageAccount) {
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
    let owner =
        ImportStoreSessionOwner::try_start(ImportOperationControl::default(), &account, bootstrap)
            .unwrap();
    (owner, account)
}

fn accounting(exec: &ImportStoreExecution<'_>) -> Vec<(u64, u64, u64)> {
    ImportDimension::ALL
        .into_iter()
        .map(|dimension| {
            let state = exec.dimension_state(dimension).unwrap();
            (state.attempted(), state.consumed(), state.live_bytes())
        })
        .collect()
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
fn paired_live_contexts_reject_foreign_capabilities_before_zero_or_nonzero_actions() {
    #[derive(Clone, Copy, Debug)]
    enum Case {
        Group(u64),
        EmptyGroup,
        Vector { grow: bool },
        EmptyDecodedAppend,
        SourceObserveEmpty,
        SourceFinish,
        SourceRecord,
        Cursor { exhausted: bool },
    }

    for case in [
        Case::Group(0),
        Case::Group(2),
        Case::EmptyGroup,
        Case::Vector { grow: false },
        Case::Vector { grow: true },
        Case::EmptyDecodedAppend,
        Case::SourceObserveEmpty,
        Case::SourceFinish,
        Case::SourceRecord,
        Case::Cursor { exhausted: false },
        Case::Cursor { exhausted: true },
    ] {
        let (mut sender_owner, sender_account) = start_store();
        let (mut receiver_owner, receiver_account) = start_store();
        testing::with_context(&mut sender_owner, |context| {
            context
                .with_execution(|sender| -> ParseResult<()> {
                    // The receiving session is shorter than the live sender region. Its own
                    // services and issuer let real bound groups be borrowed without escaping.
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
                        entry: sender.entry.clone(),
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
                    assert!(!std::ptr::eq(sender.session_issuer(), &issuer));
                    let receiver_before = accounting(&receiver.store_owner.execution().unwrap());
                    let physical_before = receiver_account.snapshot();
                    let callback_called = Cell::new(false);

                    let rejected = match case {
                        Case::Group(amount) => {
                            let group = sender.bind_work_group([ImportWorkClass::Tokens])?;
                            let before = accounting(&sender.store);
                            let rejected = receiver.with_execution(|exec| {
                                exec.with_work(&group, [amount], |_exec, work| {
                                    callback_called.set(true);
                                    work.complete_all()
                                })
                            });
                            assert_eq!(accounting(&sender.store), before, "{case:?}");
                            sender
                                .with_work(&group, [amount], |_exec, work| work.complete_all())?;
                            rejected
                        }
                        Case::EmptyGroup => {
                            let group = sender.bind_work_group([])?;
                            let before = accounting(&sender.store);
                            let rejected = receiver.with_execution(|exec| {
                                exec.with_work(&group, [], |_exec, work| {
                                    callback_called.set(true);
                                    work.complete_all()
                                })
                            });
                            assert_eq!(accounting(&sender.store), before, "{case:?}");
                            sender.with_work(&group, [], |_exec, work| work.complete_all())?;
                            rejected
                        }
                        Case::Vector { grow } => {
                            let mut values = sender.metered_vec::<u8>(
                                ImportStorageClass::FormatNormalization,
                                "foreign-session-vector",
                            );
                            if grow {
                                values.try_reserve(sender, 2)?;
                                values.try_push_reserved(7)?;
                            }
                            let before = accounting(&sender.store);
                            let source_exec = &*sender;
                            receiver.with_execution(move |exec| {
                                let mut values = values;
                                let capacity = values.capacity();
                                let additional = if grow { capacity + 1 } else { 0 };
                                let rejected = values.try_reserve(exec, additional);
                                assert!(rejected.is_err(), "{case:?}");
                                assert_eq!(values.capacity(), capacity);
                                assert_eq!(values.as_slice(), if grow { &[7][..] } else { &[] });
                                assert_eq!(accounting(&source_exec.store), before, "{case:?}");
                                values.identity.check(source_exec)?;
                                if grow {
                                    values.try_push_reserved(9)?;
                                    assert_eq!(values.as_slice(), &[7, 9]);
                                }
                                rejected
                            })
                        }
                        Case::EmptyDecodedAppend => {
                            let text = sender.metered_decoded_text(0, "foreign-decoded-text")?;
                            let before = accounting(&sender.store);
                            let source_exec = &*sender;
                            receiver.with_execution(move |exec| {
                                let mut text = text;
                                let rejected = text.try_push_str(exec, "");
                                assert!(rejected.is_err());
                                assert!(text.is_empty());
                                assert_eq!(accounting(&source_exec.store), before);
                                text.identity.check(source_exec)?;
                                rejected
                            })
                        }
                        Case::SourceObserveEmpty | Case::SourceFinish | Case::SourceRecord => {
                            let before_builder = accounting(&sender.store);
                            let builder = sender.source_scope_builder(
                                SourceScopeSelectionRequest::DocumentDefault,
                                SourceScopeSelectionOutcome::parentless_node_projection(
                                    sender.source().clone(),
                                ),
                            )?;
                            let before = accounting(&sender.store);
                            match case {
                                Case::SourceObserveEmpty => {
                                    let source_exec = &*sender;
                                    receiver.with_execution(move |exec| {
                                        let mut builder = builder;
                                        let rejected = builder.observe(
                                            exec,
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
                                    receiver.with_execution(move |exec| {
                                        let rejected = builder.finish(exec).map(|_| ());
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
                                    let record = builder.finish(sender)?;
                                    let source_exec = &*sender;
                                    receiver.with_execution(move |exec| {
                                        let rejected = exec.record_source_scope(record);
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
                            let source = sender.diagnostic_source();
                            let mut options = TextInputOptions::new(
                                TextDecodingMode::RejectInvalid,
                                "foreign-record-cursor",
                            );
                            if exhausted {
                                options = options.with_raw_end(0);
                            }
                            let cursor = TextInputRecordCursor::new(
                                sender,
                                Some(&profile),
                                source,
                                options,
                            )?
                            .unwrap();
                            let before = accounting(&sender.store);
                            let source_exec = &*sender;
                            let called = &callback_called;
                            receiver.with_execution(move |exec| {
                                let mut cursor = cursor;
                                let offset = cursor.raw_offset();
                                let rejected = cursor
                                    .with_next_record(exec, |_exec, _input, _recovery| {
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
                    assert_eq!(
                        accounting(&receiver.store_owner.execution().unwrap()),
                        receiver_before,
                        "{case:?}",
                    );
                    let physical_after = receiver_account.snapshot();
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
        drop(sender_owner);
        drop(receiver_owner);
        for account in [sender_account, receiver_account] {
            let released = account.snapshot();
            assert_eq!(released.live_retained_bytes(), 0, "{case:?}");
            assert_eq!(released.live_retained_allocations(), 0, "{case:?}");
            assert!(!released.poisoned(), "{case:?}");
        }
    }
}

#[test]
fn source_builder_and_cursor_keep_their_owner_across_execution_regions() {
    let (mut owner, account) = start_store();
    testing::with_context(&mut owner, |context| {
        let (mut builder, mut cursor) = context
            .with_execution(|exec| -> ParseResult<_> {
                let builder = exec.source_scope_builder(
                    SourceScopeSelectionRequest::DocumentDefault,
                    SourceScopeSelectionOutcome::parentless_node_projection(exec.source().clone()),
                )?;
                let source = exec.diagnostic_source();
                let cursor = TextInputRecordCursor::new(
                    exec,
                    Some(&text_profile()),
                    source,
                    TextInputOptions::new(TextDecodingMode::RejectInvalid, "same-session-cursor"),
                )?
                .unwrap();
                Ok((builder, cursor))
            })
            .unwrap();
        let record =
            context
                .with_execution(|exec| -> ParseResult<_> {
                    builder.observe(
                        exec,
                        SourceScopeEntityKind::Node,
                        SourceScopeDisposition::Selected,
                        "nodes/zero",
                    )?;
                    assert_eq!(
                        cursor.with_next_record(exec, |_exec, input, _recovery| {
                            assert_eq!(input.text(), "fixture");
                            Ok(())
                        })?,
                        Ok(Some(())),
                    );
                    assert_eq!(cursor.raw_offset(), 7);
                    assert_eq!(
                    cursor.with_next_record(exec, |_exec, _input, _recovery| -> ParseResult<()> {
                        panic!("an exhausted correct-session cursor must not invoke its callback")
                    })?,
                    Ok(None),
                );
                    builder.finish(exec)
                })
                .unwrap();
        context
            .with_execution(|exec| exec.record_source_scope(record))
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
    drop(owner);
    let released = account.snapshot();
    assert_eq!(released.live_retained_bytes(), 0);
    assert_eq!(released.live_retained_allocations(), 0);
    assert!(!released.poisoned());
}
