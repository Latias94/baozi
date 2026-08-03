use super::*;

pub(super) fn optional_records<'storage, Tag>(
    session: &mut StoreValidationSession<'_, 'storage>,
    record: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    count: u32,
    fields: usize,
) -> Option<(
    Option<CanonicalBlockRef<Tag>>,
    Option<CanonicalRecordView<'storage>>,
)>
where
    Tag: CanonicalSchema<StoreKind = baozi_operation::RecordBlock>,
{
    if count == 0 {
        require_absent(record, row, field)?;
        return Some((None, None));
    }
    let reference = record.record_ref::<Tag>(row, field)?;
    let view = session.canonical_records(reference)?;
    (view.rows() == usize::try_from(count).ok()? && view.fields() == fields)
        .then_some((Some(CanonicalBlockRef::from_store(reference)), Some(view)))
}

pub(super) fn required_records<'storage, Tag>(
    session: &mut StoreValidationSession<'_, 'storage>,
    record: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    count: u32,
    fields: usize,
) -> Option<(CanonicalBlockRef<Tag>, CanonicalRecordView<'storage>)>
where
    Tag: CanonicalSchema<StoreKind = baozi_operation::RecordBlock>,
{
    let (reference, view) = optional_records(session, record, row, field, count, fields)?;
    Some((reference?, view?))
}

pub(super) fn optional_sequence<'storage, T, Tag>(
    session: &mut StoreValidationSession<'_, 'storage>,
    record: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    count: u32,
) -> Option<OptionalCanonicalSequence<'storage, T, Tag>>
where
    T: StoredScalar,
    Tag: CanonicalSchema<StoreKind = baozi_operation::ScalarSequence<T>>,
{
    if count == 0 {
        require_absent(record, row, field)?;
        return Some((None, None));
    }
    let reference = record.sequence_ref::<T, Tag>(row, field)?;
    let view = session.canonical_sequence(reference)?;
    (view.len() == usize::try_from(count).ok()?)
        .then_some((Some(CanonicalBlockRef::from_store(reference)), Some(view)))
}

pub(super) fn optional_utf8<'storage, Tag>(
    session: &mut StoreValidationSession<'_, 'storage>,
    record: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    count: u32,
) -> Option<OptionalCanonicalValue<'storage, Tag, str>>
where
    Tag: CanonicalSchema<StoreKind = baozi_operation::Utf8Block>,
{
    if count == 0 {
        require_absent(record, row, field)?;
        return Some((None, None));
    }
    let reference = record.utf8_ref::<Tag>(row, field)?;
    let value = session.canonical_utf8(reference)?;
    (value.len() == usize::try_from(count).ok()?)
        .then_some((Some(CanonicalBlockRef::from_store(reference)), Some(value)))
}

pub(super) fn optional_bytes<'storage, Tag>(
    session: &mut StoreValidationSession<'_, 'storage>,
    record: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    count: u32,
) -> Option<OptionalCanonicalValue<'storage, Tag, [u8]>>
where
    Tag: CanonicalSchema<StoreKind = baozi_operation::ByteBlock>,
{
    if count == 0 {
        require_absent(record, row, field)?;
        return Some((None, None));
    }
    let reference = record.bytes_ref::<Tag>(row, field)?;
    let value = session.canonical_bytes(reference)?;
    (value.len() == usize::try_from(count).ok()?)
        .then_some((Some(CanonicalBlockRef::from_store(reference)), Some(value)))
}

pub(super) fn required_sequence_slice<'storage, T, Tag>(
    session: &mut StoreValidationSession<'_, 'storage>,
    record: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    logical_count: u32,
    stride: usize,
) -> Option<&'storage [T]>
where
    T: StoredScalar,
    Tag: CanonicalSchema<StoreKind = baozi_operation::ScalarSequence<T>>,
{
    let physical = logical_count.checked_mul(u32::try_from(stride).ok()?)?;
    let (_, view) = optional_sequence::<T, Tag>(session, record, row, field, physical)?;
    Some(view.map(|view| view.as_slice()).unwrap_or(&[]))
}

pub(super) fn required_utf8<'storage, Tag>(
    session: &mut StoreValidationSession<'_, 'storage>,
    record: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    count: u32,
) -> Option<&'storage str>
where
    Tag: CanonicalSchema<StoreKind = baozi_operation::Utf8Block>,
{
    let (_, value) = optional_utf8::<Tag>(session, record, row, field, count)?;
    Some(value.unwrap_or(""))
}

pub(super) fn required_bytes<'storage, Tag>(
    session: &mut StoreValidationSession<'_, 'storage>,
    record: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    count: u32,
) -> Option<&'storage [u8]>
where
    Tag: CanonicalSchema<StoreKind = baozi_operation::ByteBlock>,
{
    let (_, value) = optional_bytes::<Tag>(session, record, row, field, count)?;
    Some(value.unwrap_or(&[]))
}

pub(super) fn require_absent(
    record: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
) -> Option<()> {
    require_scalar(record.scalar::<u8>(row, field), PROFILE_ABSENT_REFERENCE)
}

pub(super) fn require_utf8_lane<Tag>(
    record: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    used: bool,
    expected: Option<CanonicalBlockRef<Tag>>,
) -> Option<()>
where
    Tag: CanonicalSchema<StoreKind = baozi_operation::Utf8Block>,
{
    if !used {
        return require_absent(record, row, field);
    }
    (CanonicalBlockRef::from_store(record.utf8_ref::<Tag>(row, field)?) == expected?).then_some(())
}

pub(super) fn require_bytes_lane<Tag>(
    record: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    used: bool,
    expected: Option<CanonicalBlockRef<Tag>>,
) -> Option<()>
where
    Tag: CanonicalSchema<StoreKind = baozi_operation::ByteBlock>,
{
    if !used {
        return require_absent(record, row, field);
    }
    (CanonicalBlockRef::from_store(record.bytes_ref::<Tag>(row, field)?) == expected?).then_some(())
}

pub(super) fn require_sequence_lane<T, Tag>(
    record: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    used: bool,
    expected: Option<CanonicalBlockRef<Tag>>,
) -> Option<()>
where
    T: StoredScalar,
    Tag: CanonicalSchema<StoreKind = baozi_operation::ScalarSequence<T>>,
{
    if !used {
        return require_absent(record, row, field);
    }
    (CanonicalBlockRef::from_store(record.sequence_ref::<T, Tag>(row, field)?) == expected?)
        .then_some(())
}

pub(super) fn scalar_row(values: &[u32], row: u32, stride: usize) -> Option<&[u32]> {
    let start = usize::try_from(row).ok()?.checked_mul(stride)?;
    values.get(start..start.checked_add(stride)?)
}

pub(super) fn observe_and_match_bytes(
    value: &[u8],
    expected: Option<&[u8]>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let chunk_size = usize::try_from(SCENE_VALIDATION_POLL_INTERVAL)
        .ok()
        .filter(|chunk_size| *chunk_size != 0)?;
    let mut matches = expected.is_none_or(|expected| expected.len() == value.len());
    for (offset, chunk) in value.chunks(chunk_size).enumerate() {
        observe_work(u64::try_from(chunk.len()).ok()?)?;
        if let Some(expected) = expected {
            let start = offset.checked_mul(chunk_size)?;
            let end = start.checked_add(chunk.len())?;
            matches &= expected.get(start..end) == Some(chunk);
        }
    }
    matches.then_some(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn consume_text<'storage>(
    text: &'storage str,
    cursor: &mut u32,
    offset: u32,
    count: u32,
    nonempty: bool,
    expected: Option<&str>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<&'storage str> {
    if offset != *cursor || nonempty && count == 0 {
        return None;
    }
    let start = usize::try_from(offset).ok()?;
    let end = start.checked_add(usize::try_from(count).ok()?)?;
    let value = text.get(start..end)?;
    observe_and_match_bytes(value.as_bytes(), expected.map(str::as_bytes), observe_work)?;
    *cursor = cursor.checked_add(count)?;
    Some(value)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn consume_optional_text<'storage>(
    text: &'storage str,
    cursor: &mut u32,
    presence: u32,
    offset: u32,
    count: u32,
    expected: Option<Option<&str>>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<Option<&'storage str>> {
    match presence {
        0 => {
            require_empty_text_range(*cursor, offset, count)?;
            if expected.is_some_and(|expected| expected.is_some()) {
                return None;
            }
            Some(None)
        }
        1 => {
            if expected.is_some_and(|expected| expected.is_none()) {
                return None;
            }
            consume_text(
                text,
                cursor,
                offset,
                count,
                true,
                expected.flatten(),
                observe_work,
            )
            .map(Some)
        }
        _ => None,
    }
}

pub(super) fn consume_bytes<'storage>(
    bytes: &'storage [u8],
    cursor: &mut u32,
    offset: u32,
    count: u32,
    expected: Option<&[u8]>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<&'storage [u8]> {
    if offset != *cursor {
        return None;
    }
    let start = usize::try_from(offset).ok()?;
    let end = start.checked_add(usize::try_from(count).ok()?)?;
    let value = bytes.get(start..end)?;
    observe_and_match_bytes(value, expected, observe_work)?;
    *cursor = cursor.checked_add(count)?;
    Some(value)
}

pub(super) fn consume_required_digest(
    bytes: &[u8],
    cursor: &mut u32,
    algorithm: u32,
    offset: u32,
    count: u32,
    expected: Option<(DigestAlgorithm, [u8; 32])>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let algorithm = decode_digest_algorithm(algorithm)?;
    if count != DIGEST_BYTES || expected.is_some_and(|expected| expected.0 != algorithm) {
        return None;
    }
    consume_bytes(
        bytes,
        cursor,
        offset,
        count,
        expected.as_ref().map(|expected| expected.1.as_slice()),
        observe_work,
    )?;
    Some(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn consume_optional_digest(
    bytes: &[u8],
    cursor: &mut u32,
    presence: u32,
    algorithm: u32,
    offset: u32,
    count: u32,
    expected: Option<Option<(DigestAlgorithm, [u8; 32])>>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    match presence {
        0 => {
            if algorithm != 0
                || offset != *cursor
                || count != 0
                || expected.is_some_and(|expected| expected.is_some())
            {
                return None;
            }
        }
        1 => {
            if expected.is_some_and(|expected| expected.is_none()) {
                return None;
            }
            consume_required_digest(
                bytes,
                cursor,
                algorithm,
                offset,
                count,
                expected.flatten(),
                observe_work,
            )?;
        }
        _ => return None,
    }
    Some(())
}

pub(super) fn consume_wide<'storage>(
    values: &'storage [u64],
    cursor: &mut u32,
    offset: u32,
    count: u32,
    required_count: u32,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<&'storage [u64]> {
    if count != required_count || offset != *cursor {
        return None;
    }
    observe_work(u64::from(count))?;
    let start = usize::try_from(offset).ok()?;
    let end = start.checked_add(usize::try_from(count).ok()?)?;
    let result = values.get(start..end)?;
    *cursor = cursor.checked_add(count)?;
    Some(result)
}

pub(super) fn consume_validated_scalar_range<'storage, T: StoredScalar>(
    values: &'storage [T],
    cursor: &mut u32,
    offset: u32,
    count: u32,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
    mut validate: impl FnMut(usize, &T) -> bool,
) -> Option<&'storage [T]> {
    if offset != *cursor {
        return None;
    }
    let start = usize::try_from(offset).ok()?;
    let end = start.checked_add(usize::try_from(count).ok()?)?;
    let result = values.get(start..end)?;
    let chunk_size = usize::try_from(SCENE_VALIDATION_POLL_INTERVAL)
        .ok()
        .filter(|chunk_size| *chunk_size != 0)?;
    let mut valid = true;
    for (chunk_ordinal, chunk) in result.chunks(chunk_size).enumerate() {
        observe_work(u64::try_from(chunk.len()).ok()?)?;
        let chunk_start = chunk_ordinal.checked_mul(chunk_size)?;
        for (index, value) in chunk.iter().enumerate() {
            valid &= validate(chunk_start.checked_add(index)?, value);
        }
    }
    if !valid {
        return None;
    }
    *cursor = cursor.checked_add(count)?;
    Some(result)
}
