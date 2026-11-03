//! Consumer intent and preflight; geometry execution belongs to the public preparation owner.

use std::{
    collections::{HashSet, TryReserveError},
    error::Error,
    fmt,
};

use baozi::{
    AttributeSemanticRef, FormatId, ImportReport, PrimitiveId, TopologyKind,
    postprocess::{ExistingNormals, NormalMode, NormalOptions, TangentPolicy},
};

/// The requested normal-field task, independent of topology lowering and space conversion.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum NormalIntent {
    /// Retain authored shading fields and absence without generating normals.
    Preserve,
    /// Complete static glTF surfaces with Flat normals, preserving authored normals.
    /// Other formats need an explicit edit when a nonempty surface has no normal field.
    #[default]
    SourceFormat,
    /// Explicit geometry editing, using the existing algorithm and dependency contracts.
    GeometryEdit {
        normals: NormalOptions,
        tangents: TangentPolicy,
    },
}

/// A consumer-policy rejection before geometry preparation has started.
#[derive(Debug)]
pub enum NormalPolicyCause {
    Allocation(TryReserveError),
    FormatNormalRuleUnavailable { mesh: usize, primitive: usize },
    ImplicitFlatTangentsUnsupported { mesh: usize, primitive: usize },
    ImplicitFlatMorphTargetsUnsupported { mesh: usize, primitive: usize },
    ImplicitFlatSkinUnsupported { mesh: usize, primitive: usize },
}

impl fmt::Display for NormalPolicyCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Allocation(error) => write!(f, "normal-policy allocation failed: {error}"),
            Self::FormatNormalRuleUnavailable { mesh, primitive } => write!(
                f,
                "mesh {mesh} primitive {primitive}: missing normals need an explicit Preserve or geometry-edit intent for this format"
            ),
            Self::ImplicitFlatTangentsUnsupported { mesh, primitive } => write!(
                f,
                "mesh {mesh} primitive {primitive}: implicit Flat completion cannot retain authored tangents; choose an explicit removal edit"
            ),
            Self::ImplicitFlatMorphTargetsUnsupported { mesh, primitive } => write!(
                f,
                "mesh {mesh} primitive {primitive}: implicit Flat completion of morph targets is unsupported"
            ),
            Self::ImplicitFlatSkinUnsupported { mesh, primitive } => write!(
                f,
                "mesh {mesh} primitive {primitive}: implicit Flat completion of a skinned surface is unsupported"
            ),
        }
    }
}

impl Error for NormalPolicyCause {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Allocation(error) => Some(error),
            _ => None,
        }
    }
}

impl From<TryReserveError> for NormalPolicyCause {
    fn from(error: TryReserveError) -> Self {
        Self::Allocation(error)
    }
}

/// Retains the complete original import publication when consumer policy planning rejects.
/// No postprocess operation or success subset has been published.
#[derive(Debug)]
pub struct NormalPolicyFailure {
    report: ImportReport,
    cause: NormalPolicyCause,
}

impl NormalPolicyFailure {
    pub(super) fn new(report: ImportReport, cause: NormalPolicyCause) -> Self {
        Self { report, cause }
    }

    pub fn report(&self) -> &ImportReport {
        &self.report
    }

    pub fn cause(&self) -> &NormalPolicyCause {
        &self.cause
    }

    pub fn into_parts(self) -> (ImportReport, NormalPolicyCause) {
        (self.report, self.cause)
    }
}

impl fmt::Display for NormalPolicyFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "consumer normal policy for {} rejected: {}",
            self.report.format().id(),
            self.cause
        )
    }
}

impl Error for NormalPolicyFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.cause)
    }
}

pub(super) struct PreparationPlan {
    pub targets: Vec<PrimitiveId>,
    pub normals: Option<NormalOptions>,
    pub tangents: TangentPolicy,
}

pub(super) fn plan(
    report: &ImportReport,
    intent: NormalIntent,
) -> Result<PreparationPlan, NormalPolicyCause> {
    // FormatId is a bounded inline value; this compares the selected registered identity,
    // not a source path, filename extension, display name or attribute-presence heuristic.
    let gltf = report.format().id()
        == &FormatId::new("org.baozi.gltf").expect("registered glTF identifier is valid");
    let (normals, tangents) = match intent {
        NormalIntent::Preserve => (None, TangentPolicy::Preserve),
        NormalIntent::SourceFormat => (
            gltf.then_some(NormalOptions {
                mode: NormalMode::Flat,
                existing: ExistingNormals::Preserve,
            }),
            TangentPolicy::Preserve,
        ),
        NormalIntent::GeometryEdit { normals, tangents } => (Some(normals), tangents),
    };
    let scene = report.scene();
    let mut targets = Vec::new();
    targets.try_reserve_exact(scene.primitive_count())?;
    // Built at most once and only for actual missing-field completion. Influence rows by
    // themselves and unused skins do not establish a bound-skin use of a logical mesh.
    let mut skinned_meshes = None;
    for (mesh, source_mesh) in scene.meshes().enumerate() {
        for (primitive, source) in source_mesh.primitives().enumerate() {
            targets.push(source.id());
            if intent != NormalIntent::SourceFormat
                || source.face_count() == 0
                || !matches!(
                    source.topology(),
                    TopologyKind::TriangleList
                        | TopologyKind::TriangleStrip
                        | TopologyKind::TriangleFan
                        | TopologyKind::Polygons
                )
            {
                continue;
            }
            let (mut normal, mut tangent) = (false, false);
            for attribute in source.attributes() {
                match attribute.semantic() {
                    AttributeSemanticRef::Normal => normal = true,
                    AttributeSemanticRef::Tangent => tangent = true,
                    _ => {}
                }
            }
            if normal {
                continue;
            }
            if !gltf {
                return Err(NormalPolicyCause::FormatNormalRuleUnavailable { mesh, primitive });
            }
            if tangent {
                return Err(NormalPolicyCause::ImplicitFlatTangentsUnsupported { mesh, primitive });
            }
            if source.morph_target_count() != 0 {
                return Err(NormalPolicyCause::ImplicitFlatMorphTargetsUnsupported {
                    mesh,
                    primitive,
                });
            }
            if skinned_meshes.is_none() {
                let mut bound = HashSet::new();
                for instance in scene.mesh_instances() {
                    if instance.skin().is_some() && !bound.contains(&instance.mesh().id()) {
                        bound.try_reserve(1)?;
                        bound.insert(instance.mesh().id());
                    }
                }
                skinned_meshes = Some(bound);
            }
            if skinned_meshes
                .as_ref()
                .is_some_and(|bound| bound.contains(&source.mesh().id()))
            {
                return Err(NormalPolicyCause::ImplicitFlatSkinUnsupported { mesh, primitive });
            }
        }
    }
    Ok(PreparationPlan {
        targets,
        normals,
        tangents,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use baozi::Importer;

    #[test]
    fn allocation_cause_and_exact_import_publication_survive_policy_failure() -> crate::Result<()> {
        let report = Importer::new()?
            .read_bytes("allocation.obj", b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n")?;
        let generation = report.scene().generation();
        let positions = report
            .scene()
            .meshes()
            .next()
            .unwrap()
            .primitives()
            .next()
            .unwrap()
            .positions()
            .unwrap()
            .data();
        let baozi::AttributeData::DenseF32 { values, .. } = positions else {
            panic!("F32 positions")
        };
        let pointer = values.as_ptr();
        let error = Vec::<u8>::new().try_reserve(usize::MAX).unwrap_err();
        let failure = NormalPolicyFailure::new(report, error.into());
        assert!(
            failure
                .source()
                .unwrap()
                .source()
                .unwrap()
                .is::<TryReserveError>()
        );
        let (report, cause) = failure.into_parts();
        assert!(matches!(cause, NormalPolicyCause::Allocation(_)));
        assert_eq!(report.scene().generation(), generation);
        let baozi::AttributeData::DenseF32 { values, .. } = report
            .scene()
            .meshes()
            .next()
            .unwrap()
            .primitives()
            .next()
            .unwrap()
            .positions()
            .unwrap()
            .data()
        else {
            panic!("F32 positions")
        };
        assert_eq!(values.as_ptr(), pointer);
        assert!(report.evidence().matches_scene(report.scene()));
        assert_eq!(report.ledger().current_live_bytes(), 0);
        assert_eq!(report.ledger().active_reservations(), 0);
        Ok(())
    }

    #[test]
    fn empty_selection_does_not_require_a_missing_field_rule() -> crate::Result<()> {
        let report = Importer::new()?.read_bytes(
            "empty.payload",
            br#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{}]}"#,
        )?;
        let plan = plan(&report, NormalIntent::SourceFormat)?;
        assert!(plan.targets.is_empty());
        assert_eq!(report.scene().primitive_count(), 0);
        assert!(report.evidence().matches_scene(report.scene()));
        Ok(())
    }
}
