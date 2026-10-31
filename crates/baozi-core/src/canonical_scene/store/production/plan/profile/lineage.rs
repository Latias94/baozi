use super::*;

pub(super) fn stability_cells(
    stability: &ResourceIdentityStability,
) -> Result<u64, ProfileAuthoringSemanticError> {
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
            text_bytes
                .checked_add(4)
                .ok_or(ProfileAuthoringSemanticError::AccountingOverflow {
                    dimension: "stable identity semantic cells",
                })
        }
        ResourceIdentityStability::SessionOpaque(_) => Ok(2),
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

pub(super) fn source_identity_cells(
    source: SourceIdentityAuthoring<'_>,
) -> Result<u64, ProfileAuthoringSemanticError> {
    let format_bytes = usize_to_u64(source.format().as_str().len(), "source format bytes")?;
    let locator_cells = source_locator_cells(source.locator())?;
    let display_bytes = match source.display_name() {
        Some(display) => usize_to_u64(display.as_str().len(), "source display-name bytes")?,
        None => 0,
    };
    checked_sum(
        [format_bytes, locator_cells, display_bytes, 4],
        "source identity semantic cells",
    )
}

fn source_locator_cells(
    locator: &SourceObjectLocator,
) -> Result<u64, ProfileAuthoringSemanticError> {
    // Preserve the semantic work contract: one tag cell and one cell per numeric byte or text byte.
    // These counts do not describe the inline StoredSourceLocator layout.
    match locator {
        SourceObjectLocator::Indexed { .. } => Ok(9),
        SourceObjectLocator::Namespaced(locator) => {
            let schema = usize_to_u64(
                locator.schema_id().name().as_str().len(),
                "source locator schema bytes",
            )?;
            let value = usize_to_u64(locator.value().as_str().len(), "source locator bytes")?;
            checked_sum([5, schema, value], "source locator semantic cells")
        }
    }
}

pub(super) fn operation_cells(
    operation: ProvenanceOperationAuthoring<'_>,
) -> Result<u64, ProfileAuthoringSemanticError> {
    let kind_cells = match operation.kind() {
        OperationKind::Namespaced(name) => usize_to_u64(name.as_str().len(), "operation kind")?,
        _ => 1,
    };
    let parent_count = usize_to_u64(operation.parents().len(), "operation parents")?;
    let mut evidence_cells = 0_u64;
    for evidence in operation.evidence() {
        let code_bytes = usize_to_u64(evidence.code().as_str().len(), "operation evidence code")?;
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
    checked_sum(
        [kind_cells, 4, parent_count, evidence_cells],
        "operation semantic cells",
    )
}
