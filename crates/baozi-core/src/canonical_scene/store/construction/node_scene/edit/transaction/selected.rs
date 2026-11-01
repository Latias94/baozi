//! Selected request inspection and the bounded per-target evidence wire format.

#[cfg(test)]
mod tests;

use super::*;
use crate::attribute::AttributeSemanticKind as Kind;
use crate::canonical_scene::store::typed_storage::StoredAttributePayload as Payload;
use crate::canonical_scene::store::typed_storage::StoredAttributeSemantic as Semantic;
use crate::canonical_scene::store::typed_storage::edit::selected::GeometrySelectionPlan;

pub use crate::canonical_scene::store::typed_storage::edit::selected::{
    SceneGeometryPlanScratch, SceneGeometryPlanStorage, SceneGeometryReplacement,
    SceneTriangleOrigins,
};

pub(super) const TARGET_WORDS: usize = 18;

impl SceneEditAdmission {
    pub fn into_geometry_transaction<'a>(
        self,
        input: &'a ValidatedScene,
        replacements: &'a [SceneGeometryReplacement<'a>],
        scratch: SceneGeometryPlanScratch<'a>,
        digest: OptionsDigest,
        limits: SceneValidationLimits,
    ) -> SceneEditTransaction<'a> {
        let mut transaction = self.transaction(
            input,
            EditRequest::Selection {
                replacements,
                digest,
            },
            limits,
        );
        transaction.selection_scratch = Some(scratch);
        transaction
    }
}

pub(super) fn plan<'a, C: TypedStorageBuildControl<Error = StoreRuntimeError>>(
    input: &'a ValidatedScene,
    replacements: &'a [SceneGeometryReplacement<'a>],
    scratch: SceneGeometryPlanScratch<'a>,
    digest: OptionsDigest,
    control: &mut C,
) -> Result<(EditStoragePlan<'a>, BoundPendingNodeSceneEdit<'a>), SceneEditRejection> {
    if replacements.is_empty() {
        return Err(SceneEditCompileError::InvalidTarget.into());
    }
    let (corners, faces) =
        checked_counts(replacements).ok_or(SceneEditCompileError::SizeOverflow)?;
    u32::try_from(
        corners
            .checked_add(schema::SELECTION_CORNER_PREFIX_FIELDS)
            .ok_or(SceneEditCompileError::SizeOverflow)?,
    )
    .map_err(|_| SceneEditCompileError::SizeOverflow)?;
    u32::try_from(faces).map_err(|_| SceneEditCompileError::SizeOverflow)?;
    let source = input.as_node_scene();
    for replacement in replacements {
        control
            .run_unit(|| input.primitive(replacement.target))?
            .map_err(|_| SceneEditCompileError::InvalidTarget)?;
        let target = control
            .run_unit(|| {
                let mesh = source
                    .read_geometry()
                    .logical_meshes
                    .get(replacement.target.logical_mesh().index() as usize)?;
                mesh.primitives
                    .start()
                    .checked_add(replacement.target.local() as usize)
            })?
            .ok_or(SceneEditCompileError::InvalidTarget)?;
        if replacement.normals.is_none()
            && replacement.tangents.is_none()
            && !replacement.remove_tangents
        {
            continue;
        }
        check_morph_dependencies(
            source.read_deformation(),
            target,
            |semantic| match semantic {
                crate::deform::MorphDeltaSemantic::Position => false,
                crate::deform::MorphDeltaSemantic::Normal => {
                    replacement.normals.is_some() || replacement.tangents.is_some()
                }
                crate::deform::MorphDeltaSemantic::Tangent => {
                    replacement.normals.is_some()
                        || replacement.tangents.is_some()
                        || replacement.remove_tangents
                }
            },
            control,
        )?;
        let attributes = source.read_geometry().primitives[target]
            .attributes
            .get(&source.read_geometry().attributes)
            .ok_or(SceneEditCompileError::InvalidSource)?;
        for attribute in attributes {
            if control.run_unit(|| {
                replacement.normals.is_some()
                    && replacement.tangents.is_none()
                    && !replacement.remove_tangents
                    && attribute.semantic == Semantic::Tangent
            })? {
                return Err(SceneEditRejection::TangentDependency);
            }
            let wide = control.run_unit(|| {
                matches!(
                    attribute.payload,
                    Payload::DenseF64(_) | Payload::SparseF64 { .. }
                )
            })?;
            let normal_dependency =
                matches!(attribute.semantic, Semantic::Position | Semantic::Normal);
            let tangent_dependency =
                matches!(attribute.semantic, Semantic::Position | Semantic::Tangent)
                    || attribute.semantic == Semantic::Normal
                        && replacement.normals.is_none()
                        && attribute.domain == replacement.normal_domain
                    || attribute.semantic == Semantic::Texcoord(replacement.uv_set)
                        && attribute.domain == replacement.uv_domain;
            if wide
                && (normal_dependency && replacement.normals.is_some_and(|v| !v.is_f64())
                    || tangent_dependency && replacement.tangents.is_some_and(|v| !v.is_f64()))
            {
                return Err(SceneEditRejection::PrecisionLoss {
                    semantic: match attribute.semantic {
                        Semantic::Position => Kind::Position,
                        Semantic::Normal => Kind::Normal,
                        Semantic::Tangent => Kind::Tangent,
                        _ => Kind::Texcoord,
                    },
                });
            }
        }
        if replacement.normals.is_some_and(VectorValues::is_f64)
            && replacement.tangents.is_some_and(|v| !v.is_f64())
        {
            return Err(SceneEditRejection::PrecisionLoss {
                semantic: Kind::Normal,
            });
        }
        if replacement.tangents.is_some() {
            let has_normal = replacement.normals.is_some()
                || attributes.iter().any(|a| {
                    a.semantic == Semantic::Normal && a.domain == replacement.normal_domain
                });
            let has_uv = attributes.iter().any(|a| {
                a.semantic == Semantic::Texcoord(replacement.uv_set)
                    && a.domain == replacement.uv_domain
                    && a.lanes == 2
            });
            if !has_normal || !has_uv {
                return Err(SceneEditCompileError::InvalidSource.into());
            }
        }
    }
    let geometry =
        GeometrySelectionPlan::try_new(source.read_geometry(), replacements, scratch, control)
            .map_err(map_build)?;
    let pending =
        BoundPendingNodeSceneEdit::try_selected_geometry(source, geometry, digest, &mut || {
            control
                .run_unit(|| ())
                .map_err(NodeSceneEditPlanError::from)
        })
        .map_err(map_plan)?;
    Ok((EditStoragePlan::Selection(geometry), pending))
}

fn checked_counts(replacements: &[SceneGeometryReplacement<'_>]) -> Option<(usize, usize)> {
    replacements.iter().try_fold(
        (0_usize, replacements.len().checked_mul(TARGET_WORDS)?),
        |(corners, faces), replacement| match replacement.origins {
            Some(origins) => Some((
                corners.checked_add(origins.triangles.len().checked_mul(3)?)?,
                faces.checked_add(origins.face_offsets.len())?,
            )),
            None => Some((corners, faces)),
        },
    )
}
pub(super) fn corner_count(replacements: &[SceneGeometryReplacement<'_>]) -> usize {
    checked_counts(replacements)
        .expect("sealed selected evidence cardinalities")
        .0
}
pub(super) fn face_count(replacements: &[SceneGeometryReplacement<'_>]) -> usize {
    checked_counts(replacements)
        .expect("sealed selected evidence cardinalities")
        .1
}

pub(super) fn target_words(plan: GeometrySelectionPlan<'_>, index: usize) -> [u32; TARGET_WORDS] {
    let edit = &plan.edits()[index];
    let m = plan.mapping(index);
    let primitive = &plan.source_geometry().primitives[m.row];
    let corners = m.corner_offset + schema::SELECTION_CORNER_PREFIX_FIELDS;
    let faces = plan.edits().len() * TARGET_WORDS + m.face_offset;
    [
        edit.target.logical_mesh().index(),
        edit.target.local(),
        m.row as u32,
        primitive.vertex_count,
        primitive.corner_count,
        primitive.face_count,
        edit.origins
            .map_or(primitive.corner_count, |o| (o.triangles.len() * 3) as u32),
        edit.origins
            .map_or(primitive.face_count, |o| o.triangles.len() as u32),
        m.output_normal
            .filter(|_| edit.normals.is_some())
            .unwrap_or(u32::MAX),
        m.output_tangent
            .filter(|_| edit.tangents.is_some())
            .unwrap_or(u32::MAX),
        u32::from(edit.origins.is_some())
            | (u32::from(edit.normals.is_some()) * 2)
            | (u32::from(edit.tangents.is_some()) * 4)
            | (u32::from(edit.remove_tangents && m.tangent.is_some() && edit.tangents.is_none())
                * 8)
            | (u32::from(edit.normals.is_some() && m.normal.is_some()) * 16)
            | (u32::from(edit.tangents.is_some() && m.tangent.is_some()) * 32)
            | (u32::from(edit.smooth) * 64),
        corners as u32,
        faces as u32,
        edit.origins.map_or(0, |o| o.face_offsets.len()) as u32,
        schema::tangent_domain_word(edit.normal_domain).unwrap_or(u32::MAX),
        edit.uv_set,
        schema::tangent_domain_word(edit.uv_domain).unwrap_or(u32::MAX),
        m.output_count as u32,
    ]
}
