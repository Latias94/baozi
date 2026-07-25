use std::num::{NonZeroU32, NonZeroU64};
use std::sync::{Mutex, MutexGuard, PoisonError};

use baozi_operation::__u4c::MeasurementSession;
use baozi_operation::{
    BytesPlan, CanonicalSequenceRef, DimensionPolicy, FinalProgress, LedgerLimits,
    OperationControl, OperationOwner, OutcomeEvidenceSequenceRef, PhysicalStorageAccount,
    PhysicalStorageAccountFactory, PhysicalStorageAccountFactoryLimits,
    PhysicalStorageAccountLimits, PreparedStoreSuccessOutcome, ProgressPhase, ProgressUnit,
    PublishedCanonicalStore, PublishedOutcomeEvidenceStore, PublishedStoreSuccessParts, RecordPlan,
    RetainedStoreBatchLimits, RetainedStoreBlockTablePlan, RetainedStoreOperation,
    RetainedStorePublicationPlan, RetainedStoreSharePlan, RetainedStoreStartPlan,
    RetainedStoreWriterPlan, ScalarSequence, SequencePlan, StoreDimensionKey,
    StoreDimensionSemanticId, StoreFallbackEnvelope, StoreKeyDomain, StoreLedgerPlan,
    StorePhaseKey, StorePhaseSemanticId, StoreSemanticCatalogId, StoreSuccessValidation, Utf8Plan,
};
use syn::visit::Visit;

#[path = "../src/u4c_collector.rs"]
mod isolated_collector;

static COLLECTOR_TEST_LOCK: Mutex<()> = Mutex::new(());

fn module_items<'a>(items: &'a [syn::Item], name: &str) -> &'a [syn::Item] {
    let modules = items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Mod(module) if module.ident == name => Some(module),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        modules.len(),
        1,
        "expected exactly one checked module {name}, found {}",
        modules.len()
    );
    let module = modules[0];
    assert_not_conditionally_compiled(&module.attrs, &format!("module {name}"));
    module
        .content
        .as_ref()
        .map(|(_, items)| items.as_slice())
        .unwrap_or_else(|| panic!("{name} must remain an inline module"))
}

fn struct_item<'a>(items: &'a [syn::Item], name: &str) -> &'a syn::ItemStruct {
    let structs = items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Struct(item) if item.ident == name => Some(item),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        structs.len(),
        1,
        "expected exactly one checked struct {name}, found {}",
        structs.len()
    );
    let item = structs[0];
    assert_not_conditionally_compiled(&item.attrs, &format!("struct {name}"));
    item
}

fn type_last_ident(ty: &syn::Type) -> Option<String> {
    match ty {
        syn::Type::Path(path) => path
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string()),
        _ => None,
    }
}

fn is_path(ty: &syn::Type, expected: &str) -> bool {
    type_last_ident(ty).as_deref() == Some(expected)
}

fn is_reference_to_path(ty: &syn::Type, expected: &str) -> bool {
    match ty {
        syn::Type::Reference(reference) => is_path(&reference.elem, expected),
        _ => false,
    }
}

fn is_reference_to_slice_of_path(ty: &syn::Type, expected: &str) -> bool {
    match ty {
        syn::Type::Reference(reference) => match reference.elem.as_ref() {
            syn::Type::Slice(slice) => is_path(&slice.elem, expected),
            _ => false,
        },
        _ => false,
    }
}

fn impl_method<'a>(
    items: &'a [syn::Item],
    type_name: &str,
    method_name: &str,
) -> &'a syn::ImplItemFn {
    let mut methods = Vec::new();
    for item in items {
        let syn::Item::Impl(implementation) = item else {
            continue;
        };
        if type_last_ident(&implementation.self_ty).as_deref() != Some(type_name) {
            continue;
        }
        for item in &implementation.items {
            if let syn::ImplItem::Fn(method) = item
                && method.sig.ident == method_name
            {
                methods.push((implementation, method));
            }
        }
    }
    assert_eq!(
        methods.len(),
        1,
        "expected exactly one checked method {type_name}::{method_name}, found {}",
        methods.len()
    );
    let (implementation, method) = methods[0];
    assert_not_conditionally_compiled(
        &implementation.attrs,
        &format!("impl parent of {type_name}::{method_name}"),
    );
    assert_not_conditionally_compiled(&method.attrs, &format!("method {type_name}::{method_name}"));
    method
}

fn free_function<'a>(items: &'a [syn::Item], name: &str) -> &'a syn::ItemFn {
    let functions = items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Fn(function) if function.sig.ident == name => Some(function),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        functions.len(),
        1,
        "expected exactly one checked free function {name}, found {}",
        functions.len()
    );
    let function = functions[0];
    assert_not_conditionally_compiled(&function.attrs, &format!("function {name}"));
    function
}

fn assert_not_conditionally_compiled(attributes: &[syn::Attribute], subject: &str) {
    let conditional_attributes = attributes
        .iter()
        .filter(|attribute| {
            attribute.path().is_ident("cfg") || attribute.path().is_ident("cfg_attr")
        })
        .map(|attribute| path_name(attribute.path()))
        .collect::<Vec<_>>();
    assert!(
        conditional_attributes.is_empty(),
        "{subject} must not use conditional compilation: {conditional_attributes:?}"
    );
}

fn named_fields(item: &syn::ItemStruct) -> Vec<&syn::Field> {
    match &item.fields {
        syn::Fields::Named(fields) => fields.named.iter().collect(),
        _ => panic!("{} must keep named fields", item.ident),
    }
}

fn field_names(fields: &[&syn::Field]) -> Vec<String> {
    fields
        .iter()
        .map(|field| {
            field
                .ident
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| panic!("named field must have an identifier"))
        })
        .collect()
}

fn assert_private_visibility(visibility: &syn::Visibility, subject: &str) {
    assert!(
        matches!(visibility, syn::Visibility::Inherited),
        "{subject} must remain private to the capability module"
    );
}

fn assert_retained_visibility(visibility: &syn::Visibility, subject: &str) {
    let syn::Visibility::Restricted(restricted) = visibility else {
        panic!("{subject} must be visible only to retained");
    };
    assert_eq!(
        path_name(&restricted.path),
        "crate::retained",
        "{subject} must not widen beyond retained"
    );
}

#[derive(Default)]
struct LookupBodyShape {
    function_calls: Vec<String>,
    indirect_function_calls: usize,
    method_calls: Vec<String>,
    field_accesses: Vec<String>,
    macro_calls: Vec<String>,
    loops: usize,
    index_expressions: usize,
}

#[derive(Default)]
struct CapabilityBoundaryFinder {
    trait_implementations: usize,
    item_macros: usize,
    forbidden_attributes: Vec<String>,
}

impl<'ast> Visit<'ast> for CapabilityBoundaryFinder {
    fn visit_item_impl(&mut self, implementation: &'ast syn::ItemImpl) {
        if implementation.trait_.is_some() {
            self.trait_implementations += 1;
        }
        syn::visit::visit_item_impl(self, implementation);
    }

    fn visit_item_macro(&mut self, item: &'ast syn::ItemMacro) {
        self.item_macros += 1;
        syn::visit::visit_item_macro(self, item);
    }

    fn visit_attribute(&mut self, attribute: &'ast syn::Attribute) {
        if !attribute.path().is_ident("allow") && !attribute.path().is_ident("cfg") {
            self.forbidden_attributes.push(path_name(attribute.path()));
        }
        syn::visit::visit_attribute(self, attribute);
    }
}

impl<'ast> Visit<'ast> for LookupBodyShape {
    fn visit_expr_call(&mut self, expression: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = expression.func.as_ref() {
            self.function_calls.push(path_name(&path.path));
        } else {
            self.indirect_function_calls += 1;
        }
        syn::visit::visit_expr_call(self, expression);
    }

    fn visit_expr_method_call(&mut self, expression: &'ast syn::ExprMethodCall) {
        self.method_calls.push(expression.method.to_string());
        syn::visit::visit_expr_method_call(self, expression);
    }

    fn visit_expr_field(&mut self, expression: &'ast syn::ExprField) {
        if let syn::Member::Named(member) = &expression.member {
            self.field_accesses.push(member.to_string());
        }
        syn::visit::visit_expr_field(self, expression);
    }

    fn visit_expr_for_loop(&mut self, expression: &'ast syn::ExprForLoop) {
        self.loops += 1;
        syn::visit::visit_expr_for_loop(self, expression);
    }

    fn visit_expr_while(&mut self, expression: &'ast syn::ExprWhile) {
        self.loops += 1;
        syn::visit::visit_expr_while(self, expression);
    }

    fn visit_expr_loop(&mut self, expression: &'ast syn::ExprLoop) {
        self.loops += 1;
        syn::visit::visit_expr_loop(self, expression);
    }

    fn visit_expr_index(&mut self, expression: &'ast syn::ExprIndex) {
        self.index_expressions += 1;
        syn::visit::visit_expr_index(self, expression);
    }

    fn visit_macro(&mut self, invocation: &'ast syn::Macro) {
        self.macro_calls.push(path_name(&invocation.path));
        syn::visit::visit_macro(self, invocation);
    }
}

fn path_name(path: &syn::Path) -> String {
    path.segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}

fn sorted(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut values = values.into_iter().collect::<Vec<_>>();
    values.sort_unstable();
    values
}

fn assert_no_lookup_scan(observed: &LookupBodyShape) {
    assert_eq!(
        observed.loops, 0,
        "the checked published lookup call graph must not contain a loop"
    );
    assert_eq!(
        observed.index_expressions, 0,
        "the checked published lookup call graph must not use an index expression"
    );
    assert_eq!(
        observed.indirect_function_calls, 0,
        "the checked published lookup call graph must not use an indirect function call"
    );
    assert!(
        observed.macro_calls.is_empty(),
        "the checked published lookup call graph must not invoke or define a macro: {:?}",
        observed.macro_calls
    );
    for forbidden in [
        "all",
        "all_reachable",
        "any",
        "collect",
        "filter",
        "filter_map",
        "find",
        "flat_map",
        "fold",
        "for_each",
        "into_iter",
        "iter",
        "iter_mut",
        "map",
        "position",
        "reduce",
    ] {
        assert!(
            !observed
                .method_calls
                .iter()
                .any(|method| method == forbidden),
            "the checked published lookup call graph must not invoke {forbidden}"
        );
    }
}

fn assert_lookup_body_shape(
    block: &syn::Block,
    expected_functions: &[&str],
    expected_methods: &[&str],
    expected_fields: &[&str],
) {
    let mut observed = LookupBodyShape::default();
    observed.visit_block(block);
    assert_no_lookup_scan(&observed);
    assert_eq!(
        sorted(observed.function_calls),
        sorted(expected_functions.iter().map(|value| (*value).to_owned())),
        "the lookup body must call only the frozen function whitelist"
    );
    assert_eq!(
        sorted(observed.method_calls),
        sorted(expected_methods.iter().map(|value| (*value).to_owned())),
        "the lookup body must call only the frozen method whitelist"
    );
    assert_eq!(
        sorted(observed.field_accesses),
        sorted(expected_fields.iter().map(|value| (*value).to_owned())),
        "the lookup body must access only the frozen fields"
    );
}

fn assert_lookup_call_shape(
    block: &syn::Block,
    expected_functions: &[&str],
    expected_methods: &[&str],
) {
    let mut observed = LookupBodyShape::default();
    observed.visit_block(block);
    assert_no_lookup_scan(&observed);
    assert_eq!(
        sorted(observed.function_calls),
        sorted(expected_functions.iter().map(|value| (*value).to_owned())),
        "the lookup body must call only the frozen function whitelist"
    );
    assert_eq!(
        sorted(observed.method_calls),
        sorted(expected_methods.iter().map(|value| (*value).to_owned())),
        "the lookup body must call only the frozen method whitelist"
    );
}

fn assert_capability_module_has_no_implicit_execution(items: &[syn::Item]) {
    let mut finder = CapabilityBoundaryFinder::default();
    for item in items {
        finder.visit_item(item);
    }
    assert_eq!(
        finder.trait_implementations, 0,
        "the private capability module must not gain a trait-provided implicit execution path"
    );
    assert_eq!(
        finder.item_macros, 0,
        "the private capability module must not gain an item macro execution path"
    );
    assert!(
        finder.forbidden_attributes.is_empty(),
        "the private capability module must not gain an attribute macro execution path: {:?}",
        finder.forbidden_attributes
    );
}

fn collector_test_lock() -> MutexGuard<'static, ()> {
    COLLECTOR_TEST_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

struct TestOwner;

impl OperationOwner for TestOwner {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum TestProgress {
    Build,
}

impl ProgressPhase for TestProgress {
    fn order(self) -> u32 {
        0
    }

    fn stable_id(self) -> &'static str {
        "build"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum TestUnit {
    Items,
}

impl ProgressUnit for TestUnit {
    fn stable_id(self) -> &'static str {
        "items"
    }
}

#[derive(Clone, Copy)]
struct CanonicalSequenceTag;
#[derive(Clone, Copy)]
struct CanonicalBytesTag;
#[derive(Clone, Copy)]
struct CanonicalUtf8Tag;
#[derive(Clone, Copy)]
struct CanonicalRootTag;
#[derive(Clone, Copy)]
struct OutcomeEvidenceSequenceTag;
#[derive(Clone, Copy)]
struct OutcomeEvidenceBytesTag;
#[derive(Clone, Copy)]
struct OutcomeEvidenceUtf8Tag;
#[derive(Clone, Copy)]
struct OutcomeEvidenceRootTag;
#[derive(Clone, Copy)]
struct SharedCanonicalTag;
#[derive(Clone, Copy)]
struct SharedOutcomeEvidenceTag;

type TestOperation = RetainedStoreOperation<TestOwner, TestProgress, TestUnit>;
type SharedCanonical = PublishedCanonicalStore<ScalarSequence<u8>, SharedCanonicalTag>;
type SharedOutcomeEvidence =
    PublishedOutcomeEvidenceStore<ScalarSequence<u8>, SharedOutcomeEvidenceTag>;

#[derive(Clone, Copy)]
struct TestKeys {
    phase: StorePhaseKey,
    dimension: StoreDimensionKey,
}

struct PublishedSharedSource {
    canonical: SharedCanonical,
    canonical_root: CanonicalSequenceRef<u8, SharedCanonicalTag>,
    outcome_evidence: SharedOutcomeEvidence,
    outcome_evidence_root: OutcomeEvidenceSequenceRef<u8, SharedOutcomeEvidenceTag>,
}

const fn catalog() -> StoreSemanticCatalogId {
    StoreSemanticCatalogId::new(
        *b"baozi-u4c-test!!",
        NonZeroU32::new(1).expect("test catalog revision is nonzero"),
    )
}

const fn phase_semantic(local: u64) -> StorePhaseSemanticId {
    StorePhaseSemanticId::new(
        catalog(),
        NonZeroU64::new(local).expect("test phase semantic id is nonzero"),
    )
}

const fn dimension_semantic(local: u64) -> StoreDimensionSemanticId {
    StoreDimensionSemanticId::new(
        catalog(),
        NonZeroU64::new(local).expect("test dimension semantic id is nonzero"),
    )
}

fn keys() -> (StoreKeyDomain, TestKeys) {
    let domain = StoreKeyDomain::try_new(catalog()).expect("test store key domain");
    let keys = TestKeys {
        phase: domain
            .try_admit_phase(phase_semantic(1))
            .expect("test phase key"),
        dimension: domain
            .try_admit_dimension(dimension_semantic(1))
            .expect("test dimension key"),
    };
    (domain, keys)
}

fn writer_plan(keys: TestKeys, attributions: usize, commitments: usize) -> RetainedStoreWriterPlan {
    RetainedStoreWriterPlan::new(
        keys.phase,
        keys.dimension,
        keys.dimension,
        RetainedStoreBatchLimits::new(attributions, commitments).expect("test batch limits"),
    )
}

fn operation(account: &PhysicalStorageAccount) -> (TestOperation, TestKeys) {
    let (domain, keys) = keys();
    let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(u64::MAX);
    let limits = LedgerLimits::new(u64::MAX, [(keys.dimension, policy)]).expect("test limits");
    let ledger_plan = StoreLedgerPlan::try_new(domain, limits, &[keys.phase]).expect("test ledger");
    let start = RetainedStoreStartPlan::new(
        writer_plan(keys, 32, 8),
        RetainedStoreSharePlan::new(keys.dimension, keys.dimension),
        StoreFallbackEnvelope::new(91),
        keys.dimension,
        1,
        keys.dimension,
        1,
    );
    (
        TestOperation::try_start(OperationControl::default(), ledger_plan, account, start)
            .expect("test operation"),
        keys,
    )
}

fn publication_plan(
    keys: TestKeys,
    canonical_blocks: usize,
    outcome_evidence_blocks: usize,
) -> RetainedStorePublicationPlan {
    RetainedStorePublicationPlan::success(
        RetainedStoreBlockTablePlan::new(
            writer_plan(keys, canonical_blocks + 1, 2),
            canonical_blocks,
            canonical_blocks,
        )
        .expect("canonical table plan"),
        RetainedStoreBlockTablePlan::new(
            writer_plan(keys, outcome_evidence_blocks + 1, 2),
            outcome_evidence_blocks,
            outcome_evidence_blocks,
        )
        .expect("outcome-evidence table plan"),
    )
}

fn final_progress() -> FinalProgress<TestProgress, TestUnit> {
    FinalProgress::new(TestProgress::Build, TestUnit::Items, 1)
}

fn account() -> PhysicalStorageAccount {
    let factory = PhysicalStorageAccountFactory::try_new(
        PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1).expect("test factory limits"),
    )
    .expect("test factory");
    factory
        .try_create_account(
            PhysicalStorageAccountLimits::new(u64::MAX, 256).expect("test account limits"),
        )
        .expect("test account")
}

fn publish_shared_source(account: &PhysicalStorageAccount) -> PublishedSharedSource {
    let (operation, keys) = operation(account);
    let mut assembly = operation
        .try_publication_assembly(publication_plan(keys, 1, 1))
        .expect("source publication assembly");
    let canonical_root = assembly
        .try_retain_canonical_sequence(
            SequencePlan::<u8>::new(1).expect("canonical sequence plan"),
            SharedCanonicalTag,
            |writer| writer.push(41),
        )
        .expect("source canonical sequence");
    let outcome_evidence_root = assembly
        .try_retain_outcome_evidence_sequence(
            SequencePlan::<u8>::new(1).expect("outcome-evidence sequence plan"),
            SharedOutcomeEvidenceTag,
            |writer| writer.push(73),
        )
        .expect("source outcome-evidence sequence");
    let validated = match assembly.validate_success(
        canonical_root,
        outcome_evidence_root,
        |session, canonical_root, outcome_evidence_root| {
            session
                .canonical_sequence(canonical_root)
                .and_then(|view| view.get(0))
                == Some(41)
                && session
                    .outcome_evidence_sequence(outcome_evidence_root)
                    .and_then(|view| view.get(0))
                    == Some(73)
        },
    ) {
        StoreSuccessValidation::Valid(validated) => validated,
        _ => panic!("source must validate"),
    };
    let prepared = match validated
        .try_prepare(final_progress())
        .expect("source preparation")
    {
        PreparedStoreSuccessOutcome::Success(prepared) => prepared,
        _ => panic!("source must prepare success"),
    };
    let PublishedStoreSuccessParts {
        canonical,
        outcome_evidence,
        ledger_evidence,
    } = prepared.handoff();
    drop(ledger_evidence);
    PublishedSharedSource {
        canonical,
        canonical_root,
        outcome_evidence,
        outcome_evidence_root,
    }
}

#[test]
fn real_publication_and_view_paths_emit_the_fixed_taxonomy() {
    let _lock = collector_test_lock();
    let account = account();
    let (operation, keys) = operation(&account);
    let session = MeasurementSession::begin().expect("collector session");
    let mut assembly = operation
        .try_publication_assembly(publication_plan(keys, 4, 4))
        .expect("publication assembly");

    let canonical_sequence = assembly
        .try_retain_canonical_sequence(
            SequencePlan::<u8>::new(1).expect("canonical sequence plan"),
            CanonicalSequenceTag,
            |writer| writer.push(11),
        )
        .expect("canonical sequence");
    let canonical_bytes = assembly
        .try_retain_canonical_bytes(
            BytesPlan::new(3).expect("canonical bytes plan"),
            CanonicalBytesTag,
            |writer| writer.write_bytes(b"abc"),
        )
        .expect("canonical bytes");
    let canonical_utf8 = assembly
        .try_retain_canonical_utf8(
            Utf8Plan::new(3).expect("canonical UTF-8 plan"),
            CanonicalUtf8Tag,
            |writer| writer.write_str("cat"),
        )
        .expect("canonical UTF-8");
    let canonical_root = assembly
        .try_retain_canonical_records(
            RecordPlan::new(1, 4).expect("canonical record plan"),
            CanonicalRootTag,
            |writer| {
                writer.write_row(|row| {
                    row.write_sequence_ref(canonical_sequence)?;
                    row.write_bytes_ref(canonical_bytes)?;
                    row.write_utf8_ref(canonical_utf8)?;
                    row.write_scalar(5u8)
                })
            },
        )
        .expect("canonical record root");

    let outcome_evidence_sequence = assembly
        .try_retain_outcome_evidence_sequence(
            SequencePlan::<u8>::new(1).expect("outcome-evidence sequence plan"),
            OutcomeEvidenceSequenceTag,
            |writer| writer.push(17),
        )
        .expect("outcome-evidence sequence");
    let outcome_evidence_bytes = assembly
        .try_retain_outcome_evidence_bytes(
            BytesPlan::new(3).expect("outcome-evidence bytes plan"),
            OutcomeEvidenceBytesTag,
            |writer| writer.write_bytes(b"def"),
        )
        .expect("outcome-evidence bytes");
    let outcome_evidence_utf8 = assembly
        .try_retain_outcome_evidence_utf8(
            Utf8Plan::new(3).expect("outcome-evidence UTF-8 plan"),
            OutcomeEvidenceUtf8Tag,
            |writer| writer.write_str("dog"),
        )
        .expect("outcome-evidence UTF-8");
    let outcome_evidence_root = assembly
        .try_retain_outcome_evidence_records(
            RecordPlan::new(1, 4).expect("outcome-evidence record plan"),
            OutcomeEvidenceRootTag,
            |writer| {
                writer.write_row(|row| {
                    row.write_sequence_ref(outcome_evidence_sequence)?;
                    row.write_bytes_ref(outcome_evidence_bytes)?;
                    row.write_utf8_ref(outcome_evidence_utf8)?;
                    row.write_scalar(9u8)
                })
            },
        )
        .expect("outcome-evidence record root");

    let validated = match assembly.validate_success(
        canonical_root,
        outcome_evidence_root,
        |validation, canonical_root, outcome_evidence_root| {
            let Some(canonical_root) = validation.canonical_records(canonical_root) else {
                return false;
            };
            let Some(canonical_sequence) =
                canonical_root.sequence_ref::<u8, CanonicalSequenceTag>(0, 0)
            else {
                return false;
            };
            let Some(canonical_bytes) = canonical_root.bytes_ref::<CanonicalBytesTag>(0, 1) else {
                return false;
            };
            let Some(canonical_utf8) = canonical_root.utf8_ref::<CanonicalUtf8Tag>(0, 2) else {
                return false;
            };
            let Some(outcome_evidence_root) =
                validation.outcome_evidence_records(outcome_evidence_root)
            else {
                return false;
            };
            let Some(outcome_evidence_sequence) =
                outcome_evidence_root.sequence_ref::<u8, OutcomeEvidenceSequenceTag>(0, 0)
            else {
                return false;
            };
            let Some(outcome_evidence_bytes) =
                outcome_evidence_root.bytes_ref::<OutcomeEvidenceBytesTag>(0, 1)
            else {
                return false;
            };
            let Some(outcome_evidence_utf8) =
                outcome_evidence_root.utf8_ref::<OutcomeEvidenceUtf8Tag>(0, 2)
            else {
                return false;
            };

            validation
                .canonical_sequence(canonical_sequence)
                .and_then(|view| view.get(0))
                == Some(11)
                && validation.canonical_bytes(canonical_bytes) == Some(&b"abc"[..])
                && validation.canonical_utf8(canonical_utf8) == Some("cat")
                && validation
                    .outcome_evidence_sequence(outcome_evidence_sequence)
                    .and_then(|view| view.get(0))
                    == Some(17)
                && validation.outcome_evidence_bytes(outcome_evidence_bytes) == Some(&b"def"[..])
                && validation.outcome_evidence_utf8(outcome_evidence_utf8) == Some("dog")
        },
    ) {
        StoreSuccessValidation::Valid(validated) => validated,
        _ => panic!("publication must validate"),
    };
    let prepared = match validated
        .try_prepare(final_progress())
        .expect("publication preparation")
    {
        PreparedStoreSuccessOutcome::Success(prepared) => prepared,
        _ => panic!("publication must prepare success"),
    };
    let PublishedStoreSuccessParts {
        canonical,
        outcome_evidence,
        ledger_evidence,
    } = prepared.handoff();
    drop(ledger_evidence);

    let canonical_root = canonical.root();
    let canonical_sequence = canonical_root
        .sequence_ref::<u8, CanonicalSequenceTag>(0, 0)
        .expect("canonical sequence reference");
    let canonical_bytes = canonical_root
        .bytes_ref::<CanonicalBytesTag>(0, 1)
        .expect("canonical bytes reference");
    let canonical_utf8 = canonical_root
        .utf8_ref::<CanonicalUtf8Tag>(0, 2)
        .expect("canonical UTF-8 reference");
    assert_eq!(canonical_root.scalar::<u8>(0, 3), Some(5));
    assert_eq!(
        canonical
            .sequence(canonical_sequence)
            .and_then(|view| view.get(0)),
        Some(11)
    );
    assert_eq!(canonical.bytes(canonical_bytes), Some(&b"abc"[..]));
    assert_eq!(canonical.utf8(canonical_utf8), Some("cat"));

    let outcome_evidence_root = outcome_evidence.root();
    let outcome_evidence_sequence = outcome_evidence_root
        .sequence_ref::<u8, OutcomeEvidenceSequenceTag>(0, 0)
        .expect("outcome-evidence sequence reference");
    let outcome_evidence_bytes = outcome_evidence_root
        .bytes_ref::<OutcomeEvidenceBytesTag>(0, 1)
        .expect("outcome-evidence bytes reference");
    let outcome_evidence_utf8 = outcome_evidence_root
        .utf8_ref::<OutcomeEvidenceUtf8Tag>(0, 2)
        .expect("outcome-evidence UTF-8 reference");
    assert_eq!(outcome_evidence_root.scalar::<u8>(0, 3), Some(9));
    assert_eq!(
        outcome_evidence
            .sequence(outcome_evidence_sequence)
            .and_then(|view| view.get(0)),
        Some(17)
    );
    assert_eq!(
        outcome_evidence.bytes(outcome_evidence_bytes),
        Some(&b"def"[..])
    );
    assert_eq!(outcome_evidence.utf8(outcome_evidence_utf8), Some("dog"));

    let report = session.finish().expect("complete measurement");
    assert_eq!(report.semantic_cell_writes(), 16);
    assert_eq!(report.reference_writes(), 6);
    assert_eq!(report.producer_events(), 46);
    assert!(report.payload_copy_bytes() > 14);
    assert_eq!(report.payload_backing_allocations(), 8);
    assert_eq!(report.block_begins(), 8);
    assert_eq!(report.block_seals(), 8);
    assert_eq!(report.slot_publications(), 8);
    assert_eq!(report.new_block_begins(), 8);
    assert_eq!(report.new_block_seals(), 8);
    assert_eq!(report.new_slot_publications(), 8);
    assert_eq!(report.inherited_block_begins(), 0);
    assert_eq!(report.inherited_block_seals(), 0);
    assert_eq!(report.inherited_slot_publications(), 0);
    assert_eq!(report.element_decodes(), 0);
    assert_eq!(report.completed_lookups(), 8);
    assert_eq!(report.rejected_lookups(), 0);
    assert_eq!(report.lookup_events(), 64);
    assert_eq!(report.max_lookup_events(), 8);
    assert_eq!(report.member_probes(), 8);
    assert_eq!(report.max_member_probes_per_lookup(), 1);
}

#[test]
fn inherited_shares_emit_lifecycles_without_copying_or_hot_leaf_lookup() {
    let _lock = collector_test_lock();
    let account = account();
    let source = publish_shared_source(&account);
    let (operation, keys) = operation(&account);
    let session = MeasurementSession::begin().expect("collector session");
    let mut assembly = operation
        .try_publication_assembly(publication_plan(keys, 1, 1))
        .expect("shared publication assembly");
    let canonical_root = assembly
        .try_share_canonical(&source.canonical, source.canonical_root)
        .expect("shared canonical root");
    let outcome_evidence_root = assembly
        .try_share_outcome_evidence(&source.outcome_evidence, source.outcome_evidence_root)
        .expect("shared outcome-evidence root");
    let validated = match assembly.validate_success(
        canonical_root,
        outcome_evidence_root,
        |validation, canonical_root, outcome_evidence_root| {
            validation
                .canonical_sequence(canonical_root)
                .and_then(|view| view.get(0))
                == Some(41)
                && validation
                    .outcome_evidence_sequence(outcome_evidence_root)
                    .and_then(|view| view.get(0))
                    == Some(73)
        },
    ) {
        StoreSuccessValidation::Valid(validated) => validated,
        _ => panic!("shared publication must validate"),
    };
    let prepared = match validated
        .try_prepare(final_progress())
        .expect("shared preparation")
    {
        PreparedStoreSuccessOutcome::Success(prepared) => prepared,
        _ => panic!("shared publication must prepare success"),
    };
    let PublishedStoreSuccessParts {
        canonical,
        outcome_evidence,
        ledger_evidence,
    } = prepared.handoff();
    drop(ledger_evidence);

    let report = session.finish().expect("complete share measurement");
    assert_eq!(report.semantic_cell_writes(), 0);
    assert_eq!(report.reference_writes(), 0);
    assert_eq!(report.payload_copy_bytes(), 0);
    assert_eq!(report.payload_backing_allocations(), 0);
    assert_eq!(report.producer_events(), 6);
    assert_eq!(report.block_begins(), 2);
    assert_eq!(report.block_seals(), 2);
    assert_eq!(report.slot_publications(), 2);
    assert_eq!(report.new_block_begins(), 0);
    assert_eq!(report.new_block_seals(), 0);
    assert_eq!(report.new_slot_publications(), 0);
    assert_eq!(report.inherited_block_begins(), 2);
    assert_eq!(report.inherited_block_seals(), 2);
    assert_eq!(report.inherited_slot_publications(), 2);
    assert_eq!(report.element_decodes(), 0);
    assert_eq!(report.completed_lookups(), 0);
    assert_eq!(report.rejected_lookups(), 0);
    assert_eq!(report.lookup_events(), 0);
    assert_eq!(report.max_lookup_events(), 0);
    assert_eq!(report.member_probes(), 0);
    assert_eq!(report.max_member_probes_per_lookup(), 0);

    assert_eq!(canonical.root().get(0), Some(41));
    assert_eq!(outcome_evidence.root().get(0), Some(73));
}

#[test]
fn source_gates_keep_public_views_on_the_checked_lookup_seam() {
    let concrete_store = include_str!("../src/concrete_store.rs");
    let terminal_output = include_str!("../src/concrete_store/terminal_output.rs");
    let retained = include_str!("../src/retained/block_table.rs");
    let validation = include_str!("../src/concrete_store/validation.rs");

    assert_eq!(
        terminal_output
            .matches("project_sequence_view(self.raw.aggregate().lookup_member(slot.raw)?)")
            .count(),
        3,
        "all canonical, outcome-evidence, and rich-failure sequence views use the checked seam"
    );
    assert_eq!(
        terminal_output
            .matches("project_bytes_view(self.raw.aggregate().lookup_member(slot.raw)?)")
            .count(),
        3,
        "all binary views use the checked seam"
    );
    assert_eq!(
        terminal_output
            .matches("project_utf8_view(self.raw.aggregate().lookup_member(slot.raw)?)")
            .count(),
        3,
        "all UTF-8 views use the checked seam"
    );
    assert_eq!(
        terminal_output
            .matches("project_canonical_record_view(self.raw.aggregate().lookup_member(slot.raw)?)")
            .count(),
        1,
        "canonical record views use the checked seam"
    );
    assert_eq!(
        terminal_output
            .matches("project_outcome_evidence_record_view(self.raw.aggregate().lookup_member(slot.raw)?)")
            .count(),
        2,
        "outcome-evidence and rich-failure record views use the checked seam"
    );
    assert!(
        !terminal_output.contains(".get(slot.raw)")
            && !terminal_output.contains(".member(slot.raw)"),
        "published concrete-store views must not bypass checked lookup with a raw slot"
    );
    assert!(
        !validation.contains("record_lookup_"),
        "construction validation is not a published typed-view measurement path"
    );
    assert!(
        !concrete_store.contains("record_element_decode(")
            && !terminal_output.contains("record_element_decode("),
        "all selected public typed-view implementations stay zero-copy"
    );

    for stage in [
        "LookupStage::CheckTableBrand",
        "LookupStage::CheckSlotSet",
        "LookupStage::CheckRange",
        "LookupStage::LoadMember",
    ] {
        assert!(
            retained.contains(stage),
            "retained lookup must retain the {stage} taxonomy stage"
        );
    }
    for stage in [
        "LookupStage::CheckVariant",
        "LookupStage::CheckSchemaTag",
        "LookupStage::ProjectView",
        "LookupStage::ReturnView",
    ] {
        assert!(
            terminal_output.contains(stage),
            "terminal projection must retain the {stage} taxonomy stage"
        );
    }
    assert!(
        retained.contains("record_lookup_rejected") && terminal_output.contains("reject_lookup"),
        "both retained and terminal rejection paths must explicitly complete the taxonomy"
    );
}

#[test]
fn ast_gate_seals_the_published_lookup_call_graph() {
    let source = include_str!("../src/retained/block_table.rs");
    let file = syn::parse_file(source).expect("block_table.rs must remain valid Rust source");
    let capability_items = module_items(&file.items, "published_table_capability");
    assert_capability_module_has_no_implicit_execution(capability_items);

    let published_table = struct_item(capability_items, "PublishedRetainedTable");
    assert_private_visibility(&published_table.vis, "the raw published table owner");
    let published_table_fields = named_fields(published_table);
    assert_eq!(
        field_names(&published_table_fields),
        ["table"],
        "the raw table allocation must stay behind the private capability boundary"
    );
    assert_private_visibility(
        &published_table_fields[0].vis,
        "the raw published table allocation",
    );
    assert!(is_path(
        &published_table_fields[0].ty,
        "RetainedTableAllocation"
    ));

    let lookup_capability = struct_item(capability_items, "PublishedLookupCapability");
    assert_private_visibility(&lookup_capability.vis, "the indexed lookup capability");
    let lookup_fields = named_fields(lookup_capability);
    assert_eq!(
        field_names(&lookup_fields),
        ["table_membership", "members"],
        "lookup capability must carry only its table snapshot and member slice"
    );
    assert!(is_path(&lookup_fields[0].ty, "Option"));
    assert!(
        is_reference_to_slice_of_path(&lookup_fields[1].ty, "RetainedBlockMember"),
        "lookup capability must borrow an opaque member slice rather than raw table ownership"
    );

    let source_sharing_capability =
        struct_item(capability_items, "PublishedSourceSharingCapability");
    assert_private_visibility(
        &source_sharing_capability.vis,
        "the source-sharing capability",
    );
    let source_sharing_fields = named_fields(source_sharing_capability);
    assert_eq!(
        field_names(&source_sharing_fields),
        ["table"],
        "source sharing must be the only capability holding the raw table allocation"
    );
    assert!(
        is_reference_to_path(&source_sharing_fields[0].ty, "RetainedTableAllocation"),
        "source sharing must retain its separate raw-table borrow"
    );

    let probed_member = struct_item(capability_items, "ProbedPublishedMember");
    assert_private_visibility(&probed_member.vis, "the indexed lookup result");
    let probed_fields = named_fields(probed_member);
    assert_eq!(
        field_names(&probed_fields),
        ["block", "schema_tag"],
        "the indexed lookup result must not expose a member, allocation, or directory"
    );
    assert!(matches!(&probed_fields[0].ty, syn::Type::Reference(_)));
    assert!(is_path(&probed_fields[1].ty, "TypeId"));

    let aggregate = struct_item(capability_items, "PublishedRetainedAggregate");
    assert_retained_visibility(&aggregate.vis, "the published aggregate");
    let aggregate_fields = named_fields(aggregate);
    assert_eq!(
        field_names(&aggregate_fields),
        ["table", "set_id"],
        "the published aggregate must not expose a member directory or source capability"
    );
    for field in aggregate_fields {
        assert_private_visibility(&field.vis, "a published aggregate field");
    }

    let lookup_capability_factory = impl_method(
        capability_items,
        "PublishedRetainedTable",
        "lookup_capability",
    );
    assert_private_visibility(
        &lookup_capability_factory.vis,
        "the lookup capability factory",
    );
    assert_lookup_body_shape(
        &lookup_capability_factory.block,
        &[],
        &["as_ref", "as_slice", "membership"],
        &["members", "table", "table"],
    );

    let probe_member = impl_method(
        capability_items,
        "PublishedLookupCapability",
        "probe_member",
    );
    assert_private_visibility(&probe_member.vis, "the member probe");
    assert_lookup_body_shape(
        &probe_member.block,
        &["Some", "super::record_lookup_member_probe"],
        &["as_ref", "get"],
        &["allocation", "members", "tag"],
    );

    let table_membership = impl_method(
        capability_items,
        "PublishedLookupCapability",
        "table_membership",
    );
    assert_private_visibility(&table_membership.vis, "the lookup table-membership check");
    assert_lookup_body_shape(&table_membership.block, &[], &[], &["table_membership"]);

    let table_lookup = impl_method(capability_items, "PublishedRetainedTable", "lookup_member");
    assert_private_visibility(&table_lookup.vis, "the table-local typed lookup");
    assert_lookup_body_shape(
        &table_lookup.block,
        &["typed_lookup::lookup_member"],
        &["lookup_capability"],
        &[],
    );

    let source_capability_factory = impl_method(
        capability_items,
        "PublishedRetainedTable",
        "source_sharing_capability",
    );
    assert_private_visibility(
        &source_capability_factory.vis,
        "the source-sharing capability factory",
    );
    assert_lookup_body_shape(&source_capability_factory.block, &[], &[], &["table"]);

    let source_capability_belongs_to = impl_method(
        capability_items,
        "PublishedSourceSharingCapability",
        "belongs_to",
    );
    assert_private_visibility(
        &source_capability_belongs_to.vis,
        "the source-sharing account check",
    );
    assert_lookup_body_shape(
        &source_capability_belongs_to.block,
        &[],
        &["belongs_to"],
        &["table"],
    );

    let source_capability_allocation = impl_method(
        capability_items,
        "PublishedSourceSharingCapability",
        "source_allocation",
    );
    assert_private_visibility(
        &source_capability_allocation.vis,
        "the source-sharing indexed allocation lookup",
    );
    assert_lookup_body_shape(
        &source_capability_allocation.block,
        &["super::super::record_test_member_lookup"],
        &["and_then", "as_ref", "get", "then_some"],
        &[
            "allocation",
            "index",
            "members",
            "set_id",
            "table",
            "tag",
            "tag",
        ],
    );

    let aggregate_source_lookup = impl_method(
        capability_items,
        "PublishedRetainedAggregate",
        "source_allocation",
    );
    assert_private_visibility(
        &aggregate_source_lookup.vis,
        "the aggregate source-allocation gateway",
    );
    assert_lookup_body_shape(
        &aggregate_source_lookup.block,
        &[],
        &["source_allocation", "source_sharing_capability"],
        &["set_id", "table"],
    );

    let aggregate_belongs_to =
        impl_method(capability_items, "PublishedRetainedAggregate", "belongs_to");
    assert_private_visibility(&aggregate_belongs_to.vis, "the aggregate account check");
    assert_lookup_body_shape(
        &aggregate_belongs_to.block,
        &[],
        &["belongs_to", "source_sharing_capability"],
        &["table"],
    );

    let aggregate_lookup = impl_method(
        capability_items,
        "PublishedRetainedAggregate",
        "lookup_member",
    );
    assert_retained_visibility(&aggregate_lookup.vis, "the published aggregate lookup API");
    assert_lookup_body_shape(
        &aggregate_lookup.block,
        &[],
        &["lookup_member"],
        &["set_id", "table"],
    );

    let table_len = impl_method(capability_items, "PublishedRetainedTable", "len");
    assert_private_visibility(&table_len.vis, "the table member-count gateway");
    assert_lookup_body_shape(
        &table_len.block,
        &[],
        &["as_ref", "member_count"],
        &["table"],
    );

    let aggregate_len = impl_method(capability_items, "PublishedRetainedAggregate", "len");
    assert_retained_visibility(&aggregate_len.vis, "the published aggregate length API");
    assert_lookup_body_shape(&aggregate_len.block, &[], &["len"], &["table"]);

    let typed_lookup_items = module_items(capability_items, "typed_lookup");
    let typed_lookup = free_function(typed_lookup_items, "lookup_member");
    let first_parameter = typed_lookup
        .sig
        .inputs
        .first()
        .unwrap_or_else(|| panic!("typed lookup must accept a lookup capability"));
    let syn::FnArg::Typed(first_parameter) = first_parameter else {
        panic!("typed lookup must receive a named lookup capability");
    };
    assert!(
        is_path(&first_parameter.ty, "PublishedLookupCapability"),
        "typed lookup must not receive the raw table or source-sharing capability"
    );
    assert_lookup_body_shape(
        &typed_lookup.block,
        &[
            "Some",
            "Some",
            "TypeId::of",
            "super::super::record_lookup_rejected",
            "super::super::record_lookup_rejected",
            "super::super::record_lookup_rejected",
            "super::super::record_lookup_stage",
            "super::super::record_lookup_stage",
            "super::super::record_lookup_stage",
            "super::super::record_lookup_stage",
        ],
        &["probe_member", "table_membership"],
        &["block", "index", "schema_tag", "set_id", "tag", "tag"],
    );

    let source_share = free_function(capability_items, "share_retained_block");
    assert_lookup_call_shape(
        &source_share.block,
        &[
            "AssertUnwindSafe",
            "Err",
            "Err",
            "Err",
            "Err",
            "Err",
            "Err",
            "Err",
            "RetainedPublicationError::Writer",
            "RetainedPublicationError::Writer",
            "RetainedPublicationError::Writer",
            "RetainedPublicationError::Writer",
            "RetainedPublicationError::Writer",
            "RetainedPublicationError::Writer",
            "RetainedWriterError::AccountPoisoned",
            "RetainedWriterError::AccountPoisoned",
            "RetainedWriterError::AccountPoisoned",
            "TypeId::of",
            "catch_unwind",
            "discard_untrusted_panic_payload",
            "drop",
            "super::try_share_retained_block_after_admission",
            "u64::try_from",
        ],
        &[
            "admit_action",
            "and_then",
            "as_ref",
            "belongs_to",
            "belongs_to",
            "belongs_to",
            "capacity",
            "checked_add",
            "len",
            "len",
            "len",
            "map_err",
            "map_err",
            "map_err",
            "map_err",
            "ok",
            "ok_or",
            "ok_or",
            "record_poison",
            "record_poison",
            "record_poison",
            "source_allocation",
            "table_mut_during_construction",
            "try_begin_account_preflight",
        ],
    );

    let outer_source_share = free_function(&file.items, "try_share_retained_block");
    assert_lookup_body_shape(
        &outer_source_share.block,
        &["published_table_capability::share_retained_block"],
        &[],
        &[],
    );
}

#[test]
fn collector_state_machine_remains_fail_closed() {
    isolated_collector::run_state_machine_tests();
}
