use std::cell::Cell;
use std::rc::Rc;

use crate::deform::SkinMatrixSpaces;
use crate::math::Mat4;

use super::super::plan::deformation::{
    DeformationAuthoringFailure, DeformationAuthoringSemanticError,
    validate_deformation_batch_with_control,
};
use super::super::plan::geometry::validate_geometry_batch_with_control;
use super::super::{
    AuthoringRange, DeformationAuthoringBatch, GeometryAuthoringBatch,
    GeometryLogicalMeshAuthoring, GeometryPrimitiveAuthoring, GeometryTopologyAuthoring,
    LogicalMorphAuthoring, MorphTargetDefinitionAuthoring, MorphTargetIdentity, NodeLocalId,
    Presence, PrimitiveDeformationAuthoring, PrimitiveMorphTargetAuthoring, SkinAuthoring,
};
use super::{SceneValidationLimits, TestControl, trs, validate};

fn validate_single_skin<'input>(
    skins: &'input [SkinAuthoring<'input>],
    control: &mut TestControl,
) -> super::super::ValidatedDeformationBatch<'input, super::TestReservation> {
    let roots = [NodeLocalId::new(0)];
    let nodes = [trs(Presence::Absent)];
    let mut setup_control = TestControl::default();
    let validated_nodes = validate(
        Presence::Present(&roots),
        &nodes,
        SceneValidationLimits::default(),
        &mut setup_control,
    )
    .expect("the deformation fixture node must validate");
    let geometry = validate_geometry_batch_with_control(
        GeometryAuthoringBatch::empty(),
        nodes.len(),
        0,
        SceneValidationLimits::default(),
        &mut setup_control,
    )
    .expect("empty geometry must validate for the deformation fixture");
    validate_deformation_batch_with_control(
        DeformationAuthoringBatch::new(skins, &[], &[], &[], &[]),
        &validated_nodes,
        &geometry,
        SceneValidationLimits::default(),
        control,
    )
    .expect("the single-skin deformation fixture must validate")
}

#[test]
fn compiled_deformation_facts_do_not_borrow_authored_storage() {
    let mut control = TestControl::default();
    let compiled = {
        let joints = vec![NodeLocalId::new(0)];
        let inverse_bind_matrices = vec![Mat4::IDENTITY];
        let skins = vec![SkinAuthoring::new(
            Presence::Absent,
            &joints,
            &inverse_bind_matrices,
            Presence::Absent,
            SkinMatrixSpaces::canonical(),
        )];

        validate_single_skin(&skins, &mut control).into_compiled_facts_for_test()
    };

    assert_eq!(compiled.0, 1);
    assert_eq!(compiled.1, [1, 0, 0, 0, 0, 0, 0]);

    drop(compiled);
    assert_eq!(control.live.get(), 0);
}

#[test]
fn compiled_deformation_storage_drops_before_its_reservation() {
    let joints = [NodeLocalId::new(0)];
    let inverse_bind_matrices = [Mat4::IDENTITY];
    let skins = [SkinAuthoring::new(
        Presence::Absent,
        &joints,
        &inverse_bind_matrices,
        Presence::Absent,
        SkinMatrixSpaces::canonical(),
    )];
    let stage = Rc::new(Cell::new(0));
    let mut control = TestControl {
        reservation_drop_stage: Some(Rc::clone(&stage)),
        ..TestControl::default()
    };
    let mut validated = validate_single_skin(&skins, &mut control);

    validated.install_storage_drop_probe(Rc::clone(&stage));
    drop(validated);

    assert_eq!(stage.get(), 2);
    assert_eq!(control.live.get(), 0);
}

#[test]
fn borrowed_deformation_names_preserve_empty_unicode_and_bounded_errors() {
    let roots = [NodeLocalId::new(0)];
    let nodes = [trs(Presence::Absent)];
    let mut setup = TestControl::default();
    let validated_nodes = validate(
        Presence::Present(&roots),
        &nodes,
        SceneValidationLimits::default(),
        &mut setup,
    )
    .unwrap();
    let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        Presence::Absent,
        Presence::Absent,
        AuthoringRange::new(0, 0),
    )];
    let geometry = validate_geometry_batch_with_control(
        GeometryAuthoringBatch::new(&meshes, &primitives, &[], &[]),
        nodes.len(),
        0,
        SceneValidationLimits::default(),
        &mut setup,
    )
    .unwrap();
    let logical = [LogicalMorphAuthoring::new(
        AuthoringRange::new(0, 1),
        &[0.0],
    )];
    let targets = [PrimitiveMorphTargetAuthoring::new(&[])];
    let deformation = [PrimitiveDeformationAuthoring::new(
        Presence::Absent,
        &targets,
    )];
    let long = "x".repeat(crate::entity::MAX_ENTITY_NAME_BYTES + 1);
    for name in ["", "骨架 α", "bad\nname", long.as_str()] {
        for skin_name in [true, false] {
            let skins = [SkinAuthoring::new(
                if skin_name {
                    Presence::Present(name)
                } else {
                    Presence::Absent
                },
                &[],
                &[],
                Presence::Absent,
                SkinMatrixSpaces::canonical(),
            )];
            let definitions = [MorphTargetDefinitionAuthoring::new(
                MorphTargetIdentity::new(0),
                if skin_name {
                    Presence::Absent
                } else {
                    Presence::Present(name)
                },
            )];
            let mut control = TestControl::default();
            let result = validate_deformation_batch_with_control(
                DeformationAuthoringBatch::new(&skins, &definitions, &logical, &deformation, &[]),
                &validated_nodes,
                &geometry,
                SceneValidationLimits::default(),
                &mut control,
            );
            if let Err(expected) = crate::entity::validate_entity_name_str(name) {
                match result {
                    Err(DeformationAuthoringFailure::Semantic(
                        DeformationAuthoringSemanticError::InvalidSkinName { skin: 0, source },
                    )) if skin_name => assert_eq!(source, expected),
                    Err(DeformationAuthoringFailure::Semantic(
                        DeformationAuthoringSemanticError::InvalidMorphTargetName {
                            logical_mesh: 0,
                            target: 0,
                            source,
                        },
                    )) if !skin_name => assert_eq!(source, expected),
                    _ => panic!("borrowed name must retain its bounded domain error"),
                }
            } else {
                drop(result.expect("empty and Unicode names must remain accepted"));
            }
            assert_eq!(control.live.get(), 0);
        }
    }
}
