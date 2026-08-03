use super::*;

pub(super) fn stability_measure(
    stability: &ResourceIdentityStability,
) -> Result<(u64, u64), ProfileAuthoringSemanticError> {
    match stability {
        ResourceIdentityStability::Stable {
            owner,
            schema,
            version: _,
        } => {
            let owner_bytes = usize_to_u64(owner.as_str().len(), "stability owner bytes")?;
            let schema_bytes = usize_to_u64(schema.as_str().len(), "stability schema bytes")?;
            let text_bytes = owner_bytes.checked_add(schema_bytes).ok_or(
                ProfileAuthoringSemanticError::AccountingOverflow {
                    dimension: "stable identity bytes",
                },
            )?;
            let cells = text_bytes.checked_add(4).ok_or(
                ProfileAuthoringSemanticError::AccountingOverflow {
                    dimension: "stable identity semantic cells",
                },
            )?;
            let bytes = text_bytes
                .checked_add(LOGICAL_U8_BYTES + LOGICAL_U32_BYTES)
                .ok_or(ProfileAuthoringSemanticError::AccountingOverflow {
                    dimension: "stable identity canonical bytes",
                })?;
            Ok((cells, bytes))
        }
        ResourceIdentityStability::SessionOpaque(_) => Ok((2, 2 * LOGICAL_U8_BYTES)),
    }
}

pub(super) fn stability_text_bytes(
    stability: &ResourceIdentityStability,
) -> Result<u64, ProfileAuthoringSemanticError> {
    match stability {
        ResourceIdentityStability::Stable {
            owner,
            schema,
            version: _,
        } => checked_sum(
            [
                usize_to_u64(owner.as_str().len(), "stability owner text bytes")?,
                usize_to_u64(schema.as_str().len(), "stability schema text bytes")?,
            ],
            "stability text bytes",
        ),
        ResourceIdentityStability::SessionOpaque(_) => Ok(0),
    }
}

pub(super) fn account_source_retention(
    source: SourceIdentityAuthoring<'_>,
    metrics: &mut ProfileMetrics,
) -> Result<(), ProfileAuthoringSemanticError> {
    checked_add(
        &mut metrics.lineage_text_bytes,
        usize_to_u64(source.format().as_str().len(), "source format text bytes")?,
        "profile lineage text bytes",
    )?;
    match source.locator() {
        SourceObjectLocator::Indexed { .. } => {
            checked_add(&mut metrics.wide_count, 1, "profile wide values")?;
        }
        SourceObjectLocator::Namespaced(locator) => {
            checked_add(
                &mut metrics.lineage_text_bytes,
                checked_sum(
                    [
                        usize_to_u64(
                            locator.schema_id().name().as_str().len(),
                            "source locator schema text bytes",
                        )?,
                        usize_to_u64(
                            locator.value().as_str().len(),
                            "source locator value text bytes",
                        )?,
                    ],
                    "source locator text bytes",
                )?,
                "profile lineage text bytes",
            )?;
        }
    }
    if let Some(display_name) = source.display_name() {
        checked_add(
            &mut metrics.lineage_text_bytes,
            usize_to_u64(
                display_name.as_str().len(),
                "source display-name text bytes",
            )?,
            "profile lineage text bytes",
        )?;
    }
    if source.content_digest().is_some() {
        checked_add(
            &mut metrics.lineage_digest_bytes,
            32,
            "profile lineage digest bytes",
        )?;
    }
    Ok(())
}

pub(super) fn account_operation_retention(
    operation: ProvenanceOperationAuthoring<'_>,
    metrics: &mut ProfileMetrics,
) -> Result<(), ProfileAuthoringSemanticError> {
    if let OperationKind::Namespaced(name) = operation.kind() {
        checked_add(
            &mut metrics.lineage_text_bytes,
            usize_to_u64(name.as_str().len(), "operation kind text bytes")?,
            "profile lineage text bytes",
        )?;
    }
    checked_add(
        &mut metrics.lineage_digest_bytes,
        32,
        "profile lineage digest bytes",
    )?;
    for evidence in operation.evidence() {
        checked_add(&mut metrics.evidence_count, 1, "profile evidence rows")?;
        checked_add(&mut metrics.wide_count, 1, "profile wide values")?;
        checked_add(
            &mut metrics.lineage_text_bytes,
            usize_to_u64(evidence.code().as_str().len(), "evidence code text bytes")?,
            "profile lineage text bytes",
        )?;
        if evidence.digest().is_some() {
            checked_add(
                &mut metrics.lineage_digest_bytes,
                32,
                "profile lineage digest bytes",
            )?;
        }
    }
    Ok(())
}

pub(super) fn source_identity_measure(
    source: SourceIdentityAuthoring<'_>,
) -> Result<(u64, u64), ProfileAuthoringSemanticError> {
    let format_bytes = usize_to_u64(source.format().as_str().len(), "source format bytes")?;
    let locator_bytes = source_locator_bytes(source.locator())?;
    let display_bytes = match source.display_name() {
        Some(display) => usize_to_u64(display.as_str().len(), "source display-name bytes")?,
        None => 0,
    };
    let digest_bytes = if source.content_digest().is_some() {
        LOGICAL_DIGEST_BYTES
    } else {
        LOGICAL_U8_BYTES
    };
    let variable_bytes = checked_sum(
        [format_bytes, locator_bytes, display_bytes, digest_bytes],
        "source identity bytes",
    )?;
    let bytes = LOGICAL_REFERENCE_BYTES
        .checked_add(3 * LOGICAL_U8_BYTES)
        .and_then(|bytes| bytes.checked_add(variable_bytes))
        .ok_or(ProfileAuthoringSemanticError::AccountingOverflow {
            dimension: "source identity canonical bytes",
        })?;
    let cells = checked_sum(
        [format_bytes, locator_bytes, display_bytes, 4],
        "source identity semantic cells",
    )?;
    Ok((cells, bytes))
}

pub(super) fn source_locator_bytes(
    locator: &SourceObjectLocator,
) -> Result<u64, ProfileAuthoringSemanticError> {
    match locator {
        SourceObjectLocator::Indexed { .. } => Ok(LOGICAL_U8_BYTES + LOGICAL_U64_BYTES),
        SourceObjectLocator::Namespaced(locator) => {
            let schema = usize_to_u64(
                locator.schema_id().name().as_str().len(),
                "source locator schema bytes",
            )?;
            let value = usize_to_u64(locator.value().as_str().len(), "source locator bytes")?;
            checked_sum(
                [LOGICAL_U8_BYTES, LOGICAL_U32_BYTES, schema, value],
                "source locator canonical bytes",
            )
        }
    }
}

pub(super) fn operation_measure(
    operation: ProvenanceOperationAuthoring<'_>,
) -> Result<(u64, u64), ProfileAuthoringSemanticError> {
    let kind_bytes = match operation.kind() {
        OperationKind::Namespaced(name) => usize_to_u64(name.as_str().len(), "operation kind")?,
        _ => LOGICAL_U8_BYTES,
    };
    let parent_count = usize_to_u64(operation.parents().len(), "operation parents")?;
    let parent_bytes = parent_count.checked_mul(LOGICAL_REFERENCE_BYTES).ok_or(
        ProfileAuthoringSemanticError::AccountingOverflow {
            dimension: "operation parent bytes",
        },
    )?;
    let mut evidence_bytes = 0_u64;
    let mut evidence_cells = 0_u64;
    for evidence in operation.evidence() {
        let code_bytes = usize_to_u64(evidence.code().as_str().len(), "operation evidence code")?;
        let digest_bytes = if evidence.digest().is_some() {
            LOGICAL_DIGEST_BYTES
        } else {
            LOGICAL_U8_BYTES
        };
        checked_add(
            &mut evidence_bytes,
            code_bytes
                .checked_add(LOGICAL_U64_BYTES)
                .and_then(|bytes| bytes.checked_add(digest_bytes))
                .ok_or(ProfileAuthoringSemanticError::AccountingOverflow {
                    dimension: "operation evidence bytes",
                })?,
            "operation evidence bytes",
        )?;
        checked_add(
            &mut evidence_cells,
            code_bytes
                .checked_add(3)
                .ok_or(ProfileAuthoringSemanticError::AccountingOverflow {
                    dimension: "operation evidence semantic cells",
                })?,
            "operation evidence semantic cells",
        )?;
    }
    let bytes = checked_sum(
        [
            LOGICAL_U8_BYTES,
            kind_bytes,
            LOGICAL_DIGEST_BYTES,
            LOGICAL_U32_BYTES,
            parent_bytes,
            LOGICAL_U32_BYTES,
            evidence_bytes,
        ],
        "operation canonical bytes",
    )?;
    let cells = checked_sum(
        [kind_bytes, 4, parent_count, evidence_cells],
        "operation semantic cells",
    )?;
    Ok((cells, bytes))
}
