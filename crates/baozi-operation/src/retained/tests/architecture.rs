use super::*;

#[test]
fn retained_handles_do_not_gain_unattributed_clone() {
    trait AmbiguousIfClone<Marker> {
        fn assert_not_clone() {}
    }

    impl<T: ?Sized> AmbiguousIfClone<()> for T {}
    impl<T: Clone> AmbiguousIfClone<u8> for T {}

    let _ = <PhysicalStorageAccount as AmbiguousIfClone<_>>::assert_not_clone;
    let _ = <RetainedAllocation<u64> as AmbiguousIfClone<_>>::assert_not_clone;
}

#[test]
fn public_handles_are_send_and_sync_for_thread_safe_payloads() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<PhysicalStorageAccountFactory>();
    assert_send_sync::<PhysicalStorageAccount>();
    assert_send_sync::<RetainedAllocation<u64>>();
}
