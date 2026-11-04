
Plans retain external input and the outer reservation brand after the numeric loan ends:

```
use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit, StoreRuntimeError};
use baozi_operation::__retained_store_integration::activation::{
    CanonicalActivationSlots, CanonicalOutputSlot, RetainedStoreSessionOutcomeOwner,
};
fn plan<'input, 'inv, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
    owner: RetainedStoreSessionOutcomeOwner<'inv, O, P, U>,
    output: CanonicalOutputSlot<'inv, ()>,
    activation: CanonicalActivationSlots<'inv, ()>,
    input: &'input [u8],
) {
    let _ = owner.plan_and_activate_scoped::<(), (), (), ()>(output, activation, |mut planning| {
        let ((borrowed_input, reservation), _completion) = planning.with_semantic_work(|semantic| {
            semantic.run_unit(|| ())?;
            let reservation = semantic.reserve(4)?;
            Ok::<_, StoreRuntimeError>((input, reservation))
        }).unwrap();
        assert!(std::ptr::eq(borrowed_input, input));
        drop(reservation);
        planning.reject((), 1)
    });
}
```

An owned fact can leave the callback. The host issues completion after callback cleanup:

```
use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
use baozi_operation::__retained_store_integration::activation::{
    CanonicalActivationSlots, CanonicalOutputSlot, RetainedStoreSessionOutcomeOwner,
};
fn owned<'inv, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
    owner: RetainedStoreSessionOutcomeOwner<'inv, O, P, U>,
    output: CanonicalOutputSlot<'inv, ()>,
    activation: CanonicalActivationSlots<'inv, ()>,
) {
    let _ = owner.plan_and_activate_scoped::<(), (), (), ()>(output, activation, |mut planning| {
        let _result = planning.with_semantic_work::<_, ()>(|semantic| {
            semantic.run_unit(|| ()).unwrap();
            Ok(7_u64)
        });
        planning.reject((), 1)
    });
}
```

The corresponding mutable semantic loan cannot be returned:

```compile_fail
use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
use baozi_operation::__retained_store_integration::activation::{
    CanonicalActivationSlots, CanonicalOutputSlot, RetainedStoreSessionOutcomeOwner,
};
fn escape<'inv, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
    owner: RetainedStoreSessionOutcomeOwner<'inv, O, P, U>,
    output: CanonicalOutputSlot<'inv, ()>,
    activation: CanonicalActivationSlots<'inv, ()>,
) {
    let _ = owner.plan_and_activate_scoped::<(), (), (), ()>(output, activation, |mut planning| {
        let _result = planning.with_semantic_work::<_, ()>(|semantic| {
            semantic.run_unit(|| ()).unwrap();
            Ok(semantic)
        });
        planning.reject((), 1)
    });
}
```

The same callback cannot reenter its planning owner:

```compile_fail
use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
use baozi_operation::__retained_store_integration::activation::{
    CanonicalActivationSlots, CanonicalOutputSlot, RetainedStoreSessionOutcomeOwner,
};
fn reenter<'inv, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
    owner: RetainedStoreSessionOutcomeOwner<'inv, O, P, U>,
    output: CanonicalOutputSlot<'inv, ()>,
    activation: CanonicalActivationSlots<'inv, ()>,
) {
    let _ = owner.plan_and_activate_scoped::<(), (), (), ()>(output, activation, |mut planning| {
        let _result = planning.with_semantic_work::<_, ()>(|semantic| {
            if false {
                let _ = planning.with_semantic_work::<_, ()>(|inner| {
                    inner.run_unit(|| ()).unwrap();
                    Ok(())
                });
            }
            semantic.run_unit(|| ()).unwrap();
            Ok(7_u64)
        });
        planning.reject((), 1)
    });
}
```

Clients cannot finish the actual host or issue completion from inside its borrowed view:

```compile_fail,E0624
use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
use baozi_operation::__retained_store_integration::activation::{
    CanonicalActivationSlots, CanonicalOutputSlot, RetainedStoreSessionOutcomeOwner,
};
fn finish<'inv, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
    owner: RetainedStoreSessionOutcomeOwner<'inv, O, P, U>,
    output: CanonicalOutputSlot<'inv, ()>,
    activation: CanonicalActivationSlots<'inv, ()>,
) {
    let _ = owner.plan_and_activate_scoped::<(), (), (), ()>(output, activation, |mut planning| {
        let _result = planning.with_semantic_work::<_, ()>(|semantic| {
            if false { let _ = semantic.finish_exact(); }
            semantic.run_unit(|| ()).unwrap();
            Ok(7_u64)
        });
        planning.reject((), 1)
    });
}
```
