use super::*;

pub(super) fn require_table_layout(
    table: CanonicalRecordView<'_>,
    rows: usize,
    fields: usize,
    reason: &'static str,
) -> NodeSceneSemanticResult<()> {
    if table.rows() != rows || table.fields() != fields {
        return Err(invariant(reason));
    }
    Ok(())
}

pub(super) fn require_record_reference<Tag>(
    table: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    expected: CanonicalBlockRef<Tag>,
    unreadable_reason: &'static str,
    mismatch_reason: &'static str,
) -> NodeSceneSemanticResult<()>
where
    Tag: CanonicalSchema<StoreKind = RecordBlock>,
{
    let reference = table
        .record_ref::<Tag>(row, field)
        .map(CanonicalBlockRef::from_store)
        .ok_or_else(|| invariant(unreadable_reason))?;
    if reference != expected {
        return Err(invariant(mismatch_reason));
    }
    Ok(())
}

pub(super) fn require_sequence_reference<T, Tag>(
    table: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    expected: CanonicalBlockRef<Tag>,
    unreadable_reason: &'static str,
    mismatch_reason: &'static str,
) -> NodeSceneSemanticResult<()>
where
    T: StoredScalar,
    Tag: CanonicalSchema<StoreKind = ScalarSequence<T>>,
{
    let reference = table
        .sequence_ref::<T, Tag>(row, field)
        .map(CanonicalBlockRef::from_store)
        .ok_or_else(|| invariant(unreadable_reason))?;
    if reference != expected {
        return Err(invariant(mismatch_reason));
    }
    Ok(())
}

pub(super) fn require_optional_sequence_reference<T, Tag>(
    table: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    present: bool,
    expected: Option<CanonicalBlockRef<Tag>>,
    reason: &'static str,
) -> NodeSceneSemanticResult<()>
where
    T: StoredScalar,
    Tag: CanonicalSchema<StoreKind = ScalarSequence<T>>,
{
    if !present {
        return require_absent(table, row, field, reason);
    }
    let expected = expected.ok_or_else(|| invariant(reason))?;
    require_sequence_reference::<T, Tag>(table, row, field, expected, reason, reason)
}

pub(super) fn require_optional_utf8_reference<Tag>(
    table: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    present: bool,
    expected: Option<CanonicalBlockRef<Tag>>,
    reason: &'static str,
) -> NodeSceneSemanticResult<()>
where
    Tag: CanonicalSchema<StoreKind = Utf8Block>,
{
    if !present {
        return require_absent(table, row, field, reason);
    }
    let expected = expected.ok_or_else(|| invariant(reason))?;
    let reference = table
        .utf8_ref::<Tag>(row, field)
        .map(CanonicalBlockRef::from_store)
        .ok_or_else(|| invariant(reason))?;
    if reference != expected {
        return Err(invariant(reason));
    }
    Ok(())
}

pub(super) fn require_optional_bytes_reference<Tag>(
    table: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    present: bool,
    expected: Option<CanonicalBlockRef<Tag>>,
    reason: &'static str,
) -> NodeSceneSemanticResult<()>
where
    Tag: CanonicalSchema<StoreKind = baozi_operation::ByteBlock>,
{
    if !present {
        return require_absent(table, row, field, reason);
    }
    let expected = expected.ok_or_else(|| invariant(reason))?;
    let reference = table
        .bytes_ref::<Tag>(row, field)
        .map(CanonicalBlockRef::from_store)
        .ok_or_else(|| invariant(reason))?;
    if reference != expected {
        return Err(invariant(reason));
    }
    Ok(())
}

pub(super) fn require_absent(
    table: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    reason: &'static str,
) -> NodeSceneSemanticResult<()> {
    if table.scalar::<u8>(row, field) != Some(PROFILE_ABSENT_REFERENCE) {
        return Err(invariant(reason));
    }
    Ok(())
}

pub(super) fn consume_utf8<'scene>(
    text: Option<&'scene str>,
    cursor: &mut usize,
    offset: usize,
    count: usize,
    nonempty: bool,
    reason: &'static str,
) -> NodeSceneSemanticResult<&'scene str> {
    if offset != *cursor || nonempty && count == 0 {
        return Err(invariant(reason));
    }
    let value = slice_utf8(text, offset, count, reason)?;
    *cursor = checked_add(offset, count)?;
    Ok(value)
}

pub(super) fn consume_optional_utf8<'scene>(
    text: &'scene str,
    cursor: &mut usize,
    presence: u32,
    offset: u32,
    count: u32,
    reason: &'static str,
) -> NodeSceneSemanticResult<Option<&'scene str>> {
    match presence {
        0 => {
            require_empty_range(*cursor, offset, count, reason)?;
            Ok(None)
        }
        1 => consume_utf8(
            Some(text),
            cursor,
            u32_to_usize(offset)?,
            u32_to_usize(count)?,
            true,
            reason,
        )
        .map(Some),
        _ => Err(invariant(reason)),
    }
}

pub(super) fn consume_required_digest<'scene>(
    digests: &'scene [u8],
    cursor: &mut usize,
    algorithm: u32,
    offset: u32,
    count: u32,
    reason: &'static str,
) -> NodeSceneSemanticResult<NodeSceneProfileDigest<'scene>> {
    let algorithm = decode_digest_algorithm(algorithm)?;
    let offset = u32_to_usize(offset)?;
    let count = u32_to_usize(count)?;
    if offset != *cursor || count != 32 {
        return Err(invariant(reason));
    }
    let bytes = slice_range(digests, offset, count, reason)?;
    *cursor = checked_add(offset, count)?;
    Ok(NodeSceneProfileDigest { algorithm, bytes })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn consume_optional_digest<'scene>(
    digests: &'scene [u8],
    cursor: &mut usize,
    presence: u32,
    algorithm: u32,
    offset: u32,
    count: u32,
    reason: &'static str,
) -> NodeSceneSemanticResult<Option<NodeSceneProfileDigest<'scene>>> {
    match presence {
        0 => {
            if algorithm != 0 {
                return Err(invariant(reason));
            }
            require_empty_range(*cursor, offset, count, reason)?;
            Ok(None)
        }
        1 => consume_required_digest(digests, cursor, algorithm, offset, count, reason).map(Some),
        _ => Err(invariant(reason)),
    }
}

pub(super) fn consume_slice<'scene, T>(
    values: &'scene [T],
    cursor: &mut usize,
    offset: usize,
    count: usize,
    expected_count: usize,
    reason: &'static str,
) -> NodeSceneSemanticResult<&'scene [T]> {
    if offset != *cursor || count != expected_count {
        return Err(invariant(reason));
    }
    let result = slice_range(values, offset, count, reason)?;
    *cursor = checked_add(offset, count)?;
    Ok(result)
}

pub(super) fn consume_sequence<'scene, T>(
    values: Option<&'scene [T]>,
    cursor: &mut usize,
    offset: usize,
    count: usize,
    reason: &'static str,
) -> NodeSceneSemanticResult<&'scene [T]> {
    if offset != *cursor {
        return Err(invariant(reason));
    }
    let result = slice_optional_range(values, offset, count, reason)?;
    *cursor = checked_add(offset, count)?;
    Ok(result)
}

pub(super) fn slice_range<'scene, T>(
    values: &'scene [T],
    offset: usize,
    count: usize,
    reason: &'static str,
) -> NodeSceneSemanticResult<&'scene [T]> {
    let end = offset.checked_add(count).ok_or_else(|| invariant(reason))?;
    values.get(offset..end).ok_or_else(|| invariant(reason))
}

pub(super) fn slice_optional_range<'scene, T>(
    values: Option<&'scene [T]>,
    offset: usize,
    count: usize,
    reason: &'static str,
) -> NodeSceneSemanticResult<&'scene [T]> {
    if count == 0 {
        if offset > values.map_or(0, <[T]>::len) {
            return Err(invariant(reason));
        }
        return Ok(&[]);
    }
    slice_range(
        values.ok_or_else(|| invariant(reason))?,
        offset,
        count,
        reason,
    )
}

pub(super) fn slice_utf8<'scene>(
    text: Option<&'scene str>,
    offset: usize,
    count: usize,
    reason: &'static str,
) -> NodeSceneSemanticResult<&'scene str> {
    if count == 0 {
        if offset > text.map_or(0, str::len) {
            return Err(invariant(reason));
        }
        return Ok("");
    }
    let end = offset.checked_add(count).ok_or_else(|| invariant(reason))?;
    text.and_then(|text| text.get(offset..end))
        .ok_or_else(|| invariant(reason))
}

pub(super) fn scalar_row<'values>(
    values: &'values [u32],
    row: usize,
    stride: usize,
    reason: &'static str,
) -> NodeSceneSemanticResult<&'values [u32]> {
    let start = row.checked_mul(stride).ok_or_else(|| invariant(reason))?;
    let end = start.checked_add(stride).ok_or_else(|| invariant(reason))?;
    values.get(start..end).ok_or_else(|| invariant(reason))
}

pub(super) fn require_empty_range(
    cursor: usize,
    offset: u32,
    count: u32,
    reason: &'static str,
) -> NodeSceneSemanticResult<()> {
    if u32_to_usize(offset)? != cursor || count != 0 {
        return Err(invariant(reason));
    }
    Ok(())
}

pub(super) fn next_row(next: usize, expected: usize) -> NodeSceneSemanticResult<Option<usize>> {
    if next == expected {
        return Ok(None);
    }
    if next > expected {
        return Err(invariant(
            "profile row cursor advanced beyond its declared count",
        ));
    }
    Ok(Some(next))
}

pub(super) fn advance(value: usize, reason: &'static str) -> NodeSceneSemanticResult<usize> {
    value.checked_add(1).ok_or_else(|| invariant(reason))
}

pub(super) fn checked_add(left: usize, right: usize) -> NodeSceneSemanticResult<usize> {
    left.checked_add(right)
        .ok_or_else(|| invariant("validated profile count overflows"))
}

pub(super) fn u32_to_usize(value: u32) -> NodeSceneSemanticResult<usize> {
    usize::try_from(value).map_err(|_| invariant("validated profile value does not fit usize"))
}

pub(super) const fn payload_len<T>(payload: Option<&[T]>) -> usize {
    match payload {
        Some(payload) => payload.len(),
        None => 0,
    }
}

pub(super) const fn payload_len_str(payload: Option<&str>) -> usize {
    match payload {
        Some(payload) => payload.len(),
        None => 0,
    }
}
