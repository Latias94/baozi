# Behavior gates

This index maps contracts to executable checks. Test results establish behavior; source digests,
AST spellings, and repository-conformance receipts do not.

| Contract | Executable evidence |
| --- | --- |
| Runtime dependency policy | [`runtime_admission.rs`](../../crates/baozi-xtask/tests/runtime_admission.rs) checks normal/build reachability, target conditions, renamed/transitive dependencies, dev-only exclusions, cycles, and malformed metadata. [`cli.rs`](../../crates/baozi-xtask/tests/cli.rs) checks the real repository command and rejects retired commands and receipt options. |
| Host-owned admission | [`host_admission.rs`](../../crates/baozi/tests/host_admission.rs): `external_host_gate_is_the_only_concurrent_session_admission_authority` measures parser overlap at host capacities one and two and checks that all permits return. |
| Concurrent parser isolation | [`reentrancy_contract.rs`](../../crates/baozi-import/tests/reentrancy_contract.rs): `one_prepared_request_is_semantically_reentrant_for_distinct_primary_inputs` and `one_snapshot_is_semantically_reentrant_for_distinct_immutable_parser_options` require overlap and compare observations, reports, and ledgers with serial imports. |
| Terminal cancellation | [`cancellation_stage_contract.rs`](../../crates/baozi-import/tests/cancellation_stage_contract.rs): `final_progress_can_trigger_signal_cancellation_at_the_last_publication_checkpoint` rejects success after final progress; `import_validation_cancellation_preserves_draft_stage_and_releases_validation_storage` checks scratch release, preserved progress, and zero canonical commitment. |
| Healthy retained cleanup | [`accounting.rs`](../../crates/baozi-operation/src/retained/tests/accounting.rs): `healthy_single_and_shared_final_owners_release_during_unrelated_host_unwind`; [`publication.rs`](../../crates/baozi-operation/src/retained/tests/publication.rs): `healthy_prepared_and_published_owners_release_during_unrelated_host_unwind`. Both observe charges during destruction and zero retained/factory accounts afterward. Payload-destructor panic and poisoned-owner tests in the same modules retain their separate fail-closed expectations. |
| Physical limits and release order | [`accounting.rs`](../../crates/baozi-operation/src/retained/tests/accounting.rs): `retained_admission_accepts_exact_limits_and_rejects_excess`, `builder_local_storage_drops_before_its_physical_lease`, and `concurrent_exact_limit_admission_is_linearizable`; [`sharing.rs`](../../crates/baozi-operation/tests/concrete_store/sharing.rs): `same_account_share_is_zero_physical_growth_and_stale_refs_fail`. |
| Typed profile ownership and borrowed validation | [`typed_tests.rs`](../../crates/baozi-core/src/canonical_scene/store/validation/node_scene/profile/typed_tests.rs) checks lineage grammar, parent order, foreign segment nonces, all metadata shapes, exact action counts, and interruption. [`node_scene/tests.rs`](../../crates/baozi-core/src/canonical_scene/store/construction/node_scene/tests.rs): `central_validation_rejects_each_corrupted_profile_structure` includes malformed segment prefixes, owner membership/ranges/depth caches, and metadata descriptors. [`edit/tests.rs`](../../crates/baozi-core/src/canonical_scene/store/construction/node_scene/edit/tests.rs) checks chained and sibling lineage, unchanged shared owners, ordinary unwind, and failed edits. The manual [`storage_scale.rs`](../../crates/baozi-core/src/canonical_scene/store/construction/node_scene/tests/storage_scale.rs) pilot compares 1/8 MiB metadata payloads without a payload-sized temporary. |
| Normal generation and attribute-owner changes | [`normals_boundary.rs`](../../crates/baozi-postprocess/tests/normals_boundary.rs) checks actual Flat/Smooth values, UV seams, hard groups, explicit replacement, and original-input failure. [`normals_tests.rs`](../../crates/baozi-core/src/canonical_scene/store/construction/node_scene/edit/normals_tests.rs) checks real source-owner lineage unions, metadata compaction versus shared payloads, final owner indices, exact work, and final release. [`runtime_tests.rs`](../../crates/baozi-postprocess/src/normals/runtime_tests.rs) checks cancellation and caught control-callback panics during calculation, activation, validation, and publication, including zero outstanding allocations. |
| Normal determinant cancellation | [`normals/tests.rs`](../../crates/baozi-postprocess/src/normals/tests.rs): `widely_separated_coordinates_preserve_direction_under_cyclic_permutation` covers all winding permutations with coordinate scales down to the smallest F32 subnormal; `exact_collinearity_survives_widely_separated_coordinates` rejects exact degeneracy; `opposite_face_permutations_cancel_without_estimate_rounding_noise` checks identical cyclic estimate bits and exact Smooth cancellation. Existing extreme-area and opposing-area tests cover scaled normalization and compensated smoothing. |
| Central failure with borrowed scratch | [`activation.rs`](../../crates/baozi-operation/src/concrete_store/activation.rs): `evidence_allocation_failure_defers_freeze_until_borrowed_scratch_is_released` injects a real final-owner allocation failure with ordinary and retained-output reservations, checks that terminal evidence remains unfrozen during the borrow, then releases scratch and verifies zero terminal live bytes, reservations, and canonical commits. |
| Exact attribute-domain lookup | [`public_read.rs`](../../crates/baozi-core/src/canonical_scene/store/construction/node_scene/snapshot/tests/public_read.rs): `attribute_lookup_matches_semantic_and_domain_without_allocation` reads distinct Vertex/Corner Normal and UV fields from one real Scene with different cardinalities and values. [`normals_domains.rs`](../../crates/baozi-postprocess/tests/normals_domains.rs) verifies explicit operation capability errors for ambiguous multi-domain edits while preserving the original Scene. |
| Resource work independent of Scene record layout | [`hashing.rs`](../../crates/baozi-import/src/resource_graph/tests/hashing.rs) checks exact hash-byte increments and denies a second hash chunk while permitting retained-handle work. [`work_callback_contract.rs`](../../crates/baozi-import/tests/work_callback_contract.rs) compares actual parser-entry baselines with exact callback attempted/consumed prefixes. Ordinary handle work uses `WorkUnits`; Scene structural work uses its own record-count envelope. |
| Compiler-enforced ownership and thread safety | [`architecture.rs`](../../crates/baozi-operation/src/retained/tests/architecture.rs) rejects unattributed `Clone`; [`thread_safety.rs`](../../crates/baozi/tests/thread_safety.rs) requires public outcomes and frozen import surfaces to be `Send + Sync`. Compile-fail doctests in [`baozi-operation`](../../crates/baozi-operation/src/lib.rs) exercise linear scoped publication capabilities. |
| Ordinary caller stack | [`normal_stack.rs`](../../crates/baozi/tests/normal_stack.rs): `import_success_and_failure_fit_a_one_mib_stack` imports and inspects an OBJ triangle and exercises failure/cancellation on a one-MiB stack in a child process. Requires `format-obj`. |
| Native dependency partitions | [`native_fs.rs`](../../crates/baozi-xtask/tests/native_fs.rs): `canonical_target_filtered_fixture_passes`, `native_backend_packages_are_private_to_baozi_io`, `runtime_candidate_closure_must_not_reach_cap_fs_ext`, `resolved_facade_candidate_rejects_indirect_repository_activation`, `xtask_resolved_profiles_reject_indirect_and_default_repository_paths`, and the `portable_byte_graph_rejects_*` tests exercise metadata graphs. `normal_build_closure_includes_build_edges_and_excludes_dev_edges` keeps production and development reachability distinct. |
| Native package integrity and review basis | The same [`native_fs.rs`](../../crates/baozi-xtask/tests/native_fs.rs): `checksum_and_resolved_feature_drift_are_rejected`, `repository_closure_feature_checksum_and_baseline_drift_are_rejected`, `source_and_target_root_drift_are_rejected`, and `reviewed_unsafe_boundary_semantics_are_bound_to_the_baseline` bind package sources, checksums, resolved features, roots, and the reviewed third-party basis. `repository_native_fs_metadata_matches_reviewed_baselines` checks the real repository against the checked-in snapshots. |
| Native handle and root binding | [`fs.rs`](../../crates/baozi-io/src/fs.rs): `opens_and_inspects_the_same_regular_file_handle`, `rejects_non_directory_roots_and_non_regular_assets`, `retained_root_capability_cannot_be_redirected_by_textual_replacement`, and `opened_file_remains_bound_after_final_path_replacement` operate on actual files and retained capabilities. |
| Native link containment and platform objects | [`fs.rs`](../../crates/baozi-io/src/fs.rs): `rejects_symlink_escape` and `follows_in_root_symlink_without_reopening_by_host_path`; Windows: `windows_firewall_rejects_ads_device_and_namespace_forms_before_open` and `windows_junction_reparse_escape_is_rejected`; Unix: `unix_fifo_is_rejected_without_waiting_for_a_writer` and `unix_socket_and_device_are_rejected`. Fixture setup failures fail these tests. |
| Repository capture and bounded traversal | [`repository/tests.rs`](../../crates/baozi-io/src/repository/tests.rs): `repository_path_preserves_spelling_and_rejects_non_portable_forms`, `repository_limits_accept_exact_boundaries_and_reject_invalid_specs`, `reenumeration_scratch_is_bounded_by_original_entry_count`, and `rejects_duplicate_overlapping_and_case_alias_roots`. Unix additionally runs `deterministic_capture_records_files_and_empty_directories`, `repository_resource_limits_accept_exact_and_reject_one_over`, `rejects_hard_link_aliases`, `rejects_symlinks_in_roots_and_descendants`, and `unix_special_files_fail_closed_without_blocking`. |
| Repository replacement and verification | [`repository/tests.rs`](../../crates/baozi-io/src/repository/tests.rs): `endpoint_revalidation_rejects_child_and_root_drift`, `final_content_revalidation_rejects_same_identity_byte_drift`, and `final_directory_snapshot_rejects_same_identity_entry_drift` check observation comparisons. Unix filesystem fixtures additionally run `verify_detects_added_and_removed_paths`, `verify_detects_content_and_same_content_replacement`, `verify_detects_kind_changes`, and `retained_repository_capability_ignores_textual_root_replacement`. Windows runs `windows_rejects_case_alias_root_spelling` and `windows_repository_capture_fails_closed_before_traversal`; the latter requires `UnsupportedIdentityBackend` at `ValidateRoots`. |
| Facade IO routing | [`importer_boundary.rs`](../../crates/baozi/tests/importer_boundary.rs): `facade_opens_primary_exactly_once_and_reuses_the_bound_snapshot` checks an injected backend and open count; `foreign_prepared_request_is_rejected_before_native_path_resolution` checks zero native-path resolutions with `native-fs-candidate`. [`obj_facade.rs`](../../crates/baozi/tests/obj_facade.rs): `facade_reads_obj_from_path` requires `format-obj,native-fs-candidate`; `facade_reads_obj_with_memory_mtl_sidecar` requires `format-obj` and reads geometry/material data from memory. |

`runtime-admission-check` makes one `cargo metadata --locked --all-features --format-version 1`
query. It rejects the named runtime/scheduler packages reachable through normal or build dependencies
from production Baozi workspace packages. Dev-only edges and the `baozi-test-support`/`baozi-xtask`
roots are excluded. The policy includes all target conditions in Cargo's resolved graph; it does not
model individual build units or inspect Rust sources. Host admission and concurrency are checked by
the behavior tests above.

`native-fs-check` retains `cargo metadata --locked` queries for manifest declarations and
target-filtered runtime, repository, facade, xtask, and portable dependency profiles. It compares
resolved normal/build closures, package sources/checksums, feature sets, and the reviewed dependency
basis against the existing snapshots. FreeBSD remains a dependency-audit profile, not a runtime
candidate. Runtime candidates remain exactly `x86_64-unknown-linux-gnu`,
`x86_64-pc-windows-msvc`, and `aarch64-apple-darwin`. Repository capture is operational only on the
Linux and macOS candidates; Windows remains fail-closed. Release registration remains empty, also
checked by `native_fs_status_tests::candidate_targets_are_distinct_from_empty_release_registration`
in [`baozi-io`](../../crates/baozi-io/src/lib.rs).

The [`CI workflow`](../../.github/workflows/ci.yml) step
`Prove non-candidate native filesystem targets fail closed` requires installed targets and a failed
compiler invocation with the adapter's fail-closed diagnostic for `wasm32-unknown-unknown`,
`wasm32-wasip1`, `x86_64-apple-darwin`, `x86_64-unknown-freebsd`, and
`x86_64-unknown-linux-gnux32`. This checks actual compiler behavior, including the ABI boundary,
for those five targets. It is not evidence for every unlisted target. The `native-fs-candidate` job
checks the exact runner host and builds/tests facade, runtime IO, and repository IO on each of the
three candidate targets. Separate byte-only WASM/WASI compiler checks keep the portable profiles
buildable without the native feature.

The removed native source guards checked Rust path spelling, aliases, macros, cfg shapes, export
lists, test-body shapes, and normalized test-source hashes. Their adversarial source-mutation
fixtures tested those guards rather than running the adapter. The compiler jobs and filesystem
tests above now carry target availability and native behavior coverage; ordinary test refactors
need no digest updates. Neither metadata partitions nor these tests prove that arbitrary parser
code cannot call `std::fs`: the standard library is still available. Injected-IO and memory-sidecar
tests cover their exercised entry points, not all possible host-access paths or macro expansions.
Test selection, ignored tests, and fixture execution must be assessed from actual nextest results;
this index does not enforce a test inventory. Passing metadata, compiler, or candidate behavior
checks does not promote a target to release support.

CI runs the workspace behavior tests in default and all-feature configurations, tests xtask
separately, runs doctests, and invokes the dependency check:

```text
cargo nextest run --locked --workspace --exclude baozi-xtask
cargo nextest run --locked --workspace --all-features --exclude baozi-xtask
cargo nextest run --locked -p baozi-xtask
cargo nextest run --locked -p baozi-xtask --all-features
cargo test --locked --doc --workspace --all-features
cargo run --locked -p baozi-xtask -- runtime-admission-check
```

For the native cleanup, run the following serially on the Windows candidate after updating the
lockfile for pruned xtask dependencies. The real repository baseline test and CLI check must pass
against the reviewed snapshots; do not remove their comparisons to accept dependency drift.

```text
cargo +1.95.0 nextest run --locked -p baozi-xtask --test native_fs
cargo +1.95.0 nextest run --locked -p baozi-xtask --all-features
cargo +1.95.0 run --locked -p baozi-xtask -- native-fs-check
cargo +1.95.0 nextest run --locked --target x86_64-pc-windows-msvc -p baozi-io --no-default-features --features fs-candidate
cargo +1.95.0 nextest run --locked --target x86_64-pc-windows-msvc -p baozi-io --no-default-features --features repository-fs-candidate
cargo +1.95.0 nextest run --locked --target x86_64-pc-windows-msvc -p baozi --no-default-features --features all-formats,native-fs-candidate
```

The candidate CI job runs the corresponding commands on Linux and macOS. Negative-target compiler
checks belong to the installed-target CI job described above; native Windows results cannot replace
Unix special-file tests or cross-target compilation results.

Run these serially when sharing the local build directory. This index records coverage locations,
not an execution receipt or a claim that an unexecuted target passed.
