use super::*;

fn poll_interval() -> usize {
    usize::try_from(SCENE_VALIDATION_POLL_INTERVAL).expect("poll interval must fit usize")
}

#[test]
fn byte_comparison_observes_each_bounded_chunk() {
    let len = poll_interval()
        .checked_mul(2)
        .and_then(|len| len.checked_add(1))
        .expect("test payload length");
    let value = vec![7_u8; len];
    let expected = value.clone();
    let mut observations = Vec::new();

    let result = observe_and_match_bytes(&value, Some(&expected), &mut |amount| {
        observations.push(amount);
        (observations.len() < 2).then_some(())
    });

    assert!(result.is_none());
    assert_eq!(
        observations,
        vec![
            u64::try_from(poll_interval()).expect("poll interval work"),
            u64::try_from(poll_interval()).expect("poll interval work"),
        ]
    );
}

#[test]
fn scalar_validation_polls_before_scanning_each_bounded_chunk() {
    let len = poll_interval()
        .checked_mul(2)
        .and_then(|len| len.checked_add(1))
        .expect("test scalar length");
    let values = vec![11_i64; len];
    let mut cursor = 0;
    let mut observations = Vec::new();
    let mut validated = 0;

    let result = consume_validated_scalar_range(
        &values,
        &mut cursor,
        0,
        u32::try_from(values.len()).expect("test scalar count"),
        &mut |amount| {
            observations.push(amount);
            (observations.len() < 2).then_some(())
        },
        |_index, value| {
            validated += 1;
            *value == 11
        },
    );

    assert!(result.is_none());
    assert_eq!(cursor, 0);
    assert_eq!(validated, poll_interval());
    assert_eq!(
        observations,
        vec![
            u64::try_from(poll_interval()).expect("poll interval work"),
            u64::try_from(poll_interval()).expect("poll interval work"),
        ]
    );
}
