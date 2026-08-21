mod common;

use baozi_import::ResourceLimits;
use common::{
    TestResult, import_assets, import_bytes, scene_snapshot, snapshot_line,
    snapshot_line_containing,
};

#[test]
fn imports_one_triangle_obj() -> TestResult<()> {
    let bytes = b"o triangle\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";

    let report = import_bytes("triangle.obj", bytes)?;
    let snapshot = scene_snapshot(&report)?;

    assert!(report.diagnostics().is_empty());
    assert_eq!(snapshot.node_count(), 1);
    assert_eq!(snapshot.root_count(), 1);
    assert_eq!(snapshot.logical_mesh_count(), 1);
    assert_eq!(snapshot.primitive_count(), 1);
    assert_eq!(snapshot.attribute_count(), 1);
    assert_eq!(snapshot.mesh_instance_count(), 1);
    assert_eq!(snapshot.primitive_index_count(), 3);
    assert_eq!(snapshot.polygon_boundary_count(), 0);
    assert!(snapshot_line(&snapshot, "node[0]").contains(" name=triangle "));
    assert!(snapshot_line(&snapshot, "logical_mesh[0]").contains(" name=triangle "));
    assert!(snapshot_line(&snapshot, "primitive[0]").contains(concat!(
        "topology=triangle_list vertices=3 corners=3 faces=1 ",
        "polygon_boundaries(absent,total=0)=[] indices(present,total=3)=[0,1,2]"
    )));
    assert!(snapshot_line_containing(&snapshot, "semantic=position").contains("elements=3"));
    Ok(())
}

#[test]
fn imports_quad_as_polygon_with_face_counts() -> TestResult<()> {
    let bytes = b"g quad\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1 2 3 4\n";

    let report = import_bytes("quad.obj", bytes)?;
    let snapshot = scene_snapshot(&report)?;

    assert!(report.diagnostics().is_empty());
    assert_eq!(snapshot.primitive_index_count(), 4);
    assert_eq!(snapshot.polygon_boundary_count(), 1);
    assert!(snapshot_line(&snapshot, "logical_mesh[0]").contains(" name=quad "));
    assert!(snapshot_line(&snapshot, "primitive[0]").contains(concat!(
        "topology=polygons vertices=4 corners=4 faces=1 ",
        "polygon_boundaries(present,total=1)=[4] ",
        "indices(present,total=4)=[0,1,2,3]"
    )));
    Ok(())
}

#[test]
fn preserves_corner_uv_and_normal_domains_without_duplicating_positions() -> TestResult<()> {
    let bytes = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nvt 1 1\nvt 0.25 0.25\nvn 0 0 1\nvn 0 0 -1\nvn 1 0 0\nf 1/1/1 2/2/1 3/3/1\nf 1/4/2 3/3/2 2/2/2\n";

    let report = import_bytes("remap.obj", bytes)?;
    let snapshot = scene_snapshot(&report)?;

    assert!(report.diagnostics().is_empty());
    assert_eq!(snapshot.attribute_count(), 3);
    assert_eq!(snapshot.primitive_index_count(), 6);
    assert_eq!(snapshot.f32_scalar_count(), 39);
    let primitive = snapshot_line(&snapshot, "primitive[0]");
    assert!(
        primitive.contains("topology=triangle_list vertices=3 corners=6 faces=2")
            && primitive.contains("indices(present,total=6)=[0,1,2,0,2,1]"),
        "unexpected canonical primitive: {primitive}"
    );
    let position = snapshot_line_containing(&snapshot, "semantic=position");
    assert!(position.contains("domain=vertex scalar=f32 lanes=3"));
    assert!(position.contains("payload=dense elements=3 explicit=3 canonical_scalars=9"));

    let texcoord = snapshot_line_containing(&snapshot, "semantic=texcoord");
    assert!(texcoord.contains("domain=corner scalar=f32 lanes=2"));
    assert!(texcoord.contains("payload=dense elements=6 explicit=6 canonical_scalars=12"));
    assert!(texcoord.contains(concat!(
        "values(present,total=12)=[",
        "0x00000000,0x00000000,0x3f800000,0x00000000,",
        "0x00000000,0x3f800000,0x3f800000,0x3f800000,",
        "0x00000000,0x3f800000,0x3f800000,0x00000000]"
    )));

    let normal = snapshot_line_containing(&snapshot, "semantic=normal");
    assert!(normal.contains("domain=corner scalar=f32 lanes=3"));
    assert!(normal.contains("payload=dense elements=6 explicit=6 canonical_scalars=18"));
    assert!(normal.contains(concat!(
        "values(present,total=18)=[",
        "0x00000000,0x00000000,0x3f800000,",
        "0x00000000,0x00000000,0x3f800000,",
        "0x00000000,0x00000000,0x3f800000,",
        "0x00000000,0x00000000,0xbf800000,",
        "0x00000000,0x00000000,0xbf800000,",
        "0x00000000,0x00000000,0xbf800000]"
    )));
    Ok(())
}

#[test]
fn isolates_corner_source_projection_between_segments() -> TestResult<()> {
    let bytes = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 1 0\nvt 2 0\nvt 3 0\nvt 4 0\nvn 1 0 0\nvn 2 0 0\nvn 3 0 0\nvn 4 0 0\ng first\nf 1/1/1 2/2/2 3/3/3\ng second\nf 1/3/3 3/4/4 2/1/1\n";

    let report = import_bytes("segment-remap.obj", bytes)?;
    let snapshot = scene_snapshot(&report)?;

    assert!(report.diagnostics().is_empty());
    assert_eq!(snapshot.logical_mesh_count(), 2);
    assert_eq!(snapshot.attribute_count(), 6);
    assert!(snapshot_line(&snapshot, "attribute[1]").contains(concat!(
        "semantic=normal semantic_set=0 domain=corner scalar=f32 lanes=3 ",
        "normalization=none payload=dense elements=3 explicit=3 canonical_scalars=9 ",
        "values(present,total=9)=[",
        "0x3f800000,0x00000000,0x00000000,",
        "0x40000000,0x00000000,0x00000000,",
        "0x40400000,0x00000000,0x00000000]"
    )));
    assert!(snapshot_line(&snapshot, "attribute[2]").contains(concat!(
        "semantic=texcoord semantic_set=0 domain=corner scalar=f32 lanes=2 ",
        "normalization=none payload=dense elements=3 explicit=3 canonical_scalars=6 ",
        "values(present,total=6)=[",
        "0x3f800000,0x00000000,0x40000000,0x00000000,",
        "0x40400000,0x00000000]"
    )));
    assert!(snapshot_line(&snapshot, "attribute[4]").contains(concat!(
        "semantic=normal semantic_set=0 domain=corner scalar=f32 lanes=3 ",
        "normalization=none payload=dense elements=3 explicit=3 canonical_scalars=9 ",
        "values(present,total=9)=[",
        "0x40400000,0x00000000,0x00000000,",
        "0x40800000,0x00000000,0x00000000,",
        "0x3f800000,0x00000000,0x00000000]"
    )));
    assert!(snapshot_line(&snapshot, "attribute[5]").contains(concat!(
        "semantic=texcoord semantic_set=0 domain=corner scalar=f32 lanes=2 ",
        "normalization=none payload=dense elements=3 explicit=3 canonical_scalars=6 ",
        "values(present,total=6)=[",
        "0x40400000,0x00000000,0x40800000,0x00000000,",
        "0x3f800000,0x00000000]"
    )));
    Ok(())
}

#[test]
fn retains_complete_texcoords_when_normals_are_partial() -> TestResult<()> {
    let bytes = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nvn 0 0 1\nf 1/1/1 2/2 3/3/1\n";

    let report = import_bytes("partial-normal.obj", bytes)?;
    let snapshot = scene_snapshot(&report)?;

    assert_eq!(snapshot.attribute_count(), 2);
    assert!(snapshot.preview().contains("semantic=texcoord"));
    assert!(!snapshot.preview().contains("semantic=normal"));
    assert!(
        snapshot_line_containing(&snapshot, "semantic=texcoord").contains(concat!(
            "domain=corner scalar=f32 lanes=2 normalization=none ",
            "payload=dense elements=3 explicit=3 canonical_scalars=6"
        ))
    );
    assert_eq!(report.diagnostics().len(), 1);
    assert_eq!(
        report.diagnostics()[0].code().as_str(),
        "obj.partial_normal_channel"
    );
    Ok(())
}

#[test]
fn preserves_corner_uv_and_normal_cardinality_for_polygons() -> TestResult<()> {
    let bytes = b"v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nvn 0 0 1\nf 1/1/1 2/2/1 3/3/1 4/4/1\n";

    let report = import_bytes("corner-polygon.obj", bytes)?;
    let snapshot = scene_snapshot(&report)?;

    assert!(report.diagnostics().is_empty());
    assert_eq!(snapshot.attribute_count(), 3);
    assert_eq!(snapshot.f32_scalar_count(), 32);
    assert!(snapshot_line(&snapshot, "primitive[0]").contains(concat!(
        "topology=polygons vertices=4 corners=4 faces=1 ",
        "polygon_boundaries(present,total=1)=[4] ",
        "indices(present,total=4)=[0,1,2,3]"
    )));
    let texcoord = snapshot_line_containing(&snapshot, "semantic=texcoord");
    assert!(texcoord.contains("domain=corner scalar=f32 lanes=2"));
    assert!(texcoord.contains("payload=dense elements=4"));
    let normal = snapshot_line_containing(&snapshot, "semantic=normal");
    assert!(normal.contains("domain=corner scalar=f32 lanes=3"));
    assert!(normal.contains("payload=dense elements=4"));
    Ok(())
}

#[test]
fn supports_negative_face_indices() -> TestResult<()> {
    let bytes = b"v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf -4 -3 -2 -1\n";

    let report = import_bytes("negative.obj", bytes)?;
    let snapshot = scene_snapshot(&report)?;

    assert!(report.diagnostics().is_empty());
    assert_eq!(snapshot.primitive_index_count(), 4);
    assert_eq!(snapshot.polygon_boundary_count(), 1);
    assert!(snapshot_line(&snapshot, "primitive[0]").contains(concat!(
        "topology=polygons vertices=4 corners=4 faces=1 ",
        "polygon_boundaries(present,total=1)=[4] ",
        "indices(present,total=4)=[0,1,2,3]"
    )));
    Ok(())
}

#[test]
fn partial_uv_and_normal_channels_are_diagnosed_and_omitted() -> TestResult<()> {
    let bytes = b"g mixed\nv 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nvn 0 0 1\nf 1/1/1 2/2/1 3/3/1\nf 1 2 3\n";

    let report = import_bytes("partial.obj", bytes)?;
    let snapshot = scene_snapshot(&report)?;
    let diagnostics = report.diagnostics();

    assert_eq!(snapshot.logical_mesh_count(), 1);
    assert_eq!(snapshot.attribute_count(), 1);
    assert!(snapshot_line(&snapshot, "primitive[0]").contains("vertices=3 corners=6 faces=2"));
    assert!(!snapshot.preview().contains("semantic=texcoord"));
    assert!(!snapshot.preview().contains("semantic=normal"));
    assert_eq!(diagnostics.len(), 2);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "obj.partial_texcoord_channel")
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "obj.partial_normal_channel")
    );
    Ok(())
}

#[test]
fn state_changes_split_meshes_and_preserve_material_bindings() -> TestResult<()> {
    let bytes = b"o first-object\ng first-group\nusemtl red\ns 1\n\
        v 0 0 0\nv 1 0 0\nv 0 1 0\nv 2 0 0\nv 3 0 0\nv 2 1 0\n\
        f 1 2 3\no second-object\ng second-group\nusemtl blue\nf 4 5 6\n";

    let report = import_bytes("state-switch.obj", bytes)?;
    let snapshot = scene_snapshot(&report)?;

    assert_eq!(snapshot.logical_mesh_count(), 2);
    assert_eq!(snapshot.primitive_count(), 2);
    assert_eq!(snapshot.mesh_instance_count(), 2);
    assert_eq!(snapshot.material_count(), 2);
    assert_eq!(report.diagnostics().len(), 2);
    assert!(snapshot_line(&snapshot, "logical_mesh[0]").contains(" name=first-group "));
    assert!(snapshot_line(&snapshot, "logical_mesh[1]").contains(" name=second-group "));
    assert!(snapshot_line(&snapshot, "primitive[0]").contains(" material=0 "));
    assert!(snapshot_line(&snapshot, "primitive[1]").contains(" material=1 "));
    assert!(snapshot_line(&snapshot, "material[0]").contains(" name=red"));
    assert!(snapshot_line(&snapshot, "material[1]").contains(" name=blue"));
    assert!(
        snapshot.preview().lines().any(|line| {
            line.contains("key=obj:object") && line.contains("string=first-object")
        })
    );
    let smoothing_attributes = snapshot
        .preview()
        .lines()
        .filter(|line| line.contains("semantic=smoothing_group"))
        .collect::<Vec<_>>();
    assert_eq!(smoothing_attributes.len(), 2);
    assert!(smoothing_attributes[0].contains("values(present,total=1)=[1]"));
    assert!(smoothing_attributes[1].contains("values(present,total=1)=[1]"));
    assert!(!snapshot.preview().contains("key=obj:smoothing"));
    Ok(())
}

#[test]
fn repeating_the_current_state_does_not_split_the_mesh() -> TestResult<()> {
    let bytes = b"o same\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\no same\nf 1 2 3\n";

    let report = import_bytes("same-state.obj", bytes)?;
    let snapshot = scene_snapshot(&report)?;

    assert!(report.diagnostics().is_empty());
    assert_eq!(snapshot.logical_mesh_count(), 1);
    assert_eq!(snapshot.primitive_count(), 1);
    assert_eq!(snapshot.primitive_index_count(), 6);
    assert!(
        snapshot_line(&snapshot, "primitive[0]").contains("indices(present,total=6)=[0,1,2,0,1,2]")
    );
    Ok(())
}

#[test]
fn smoothing_changes_are_preserved_as_one_face_domain_attribute() -> TestResult<()> {
    let obj = b"mtllib shared.mtl\no shared-object\ng shared-group\nusemtl shared\n\
        v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\n\
        f 1 2 3\ns 7\nf 1 3 4\ns off\nf 1 4 2\n";
    let mtl = b"newmtl shared\nKd 1 1 1\n";

    let report = import_assets(
        "smoothing-groups.obj",
        [
            ("smoothing-groups.obj", obj.as_slice()),
            ("shared.mtl", mtl.as_slice()),
        ],
        ResourceLimits::default(),
    )?;
    let snapshot = scene_snapshot(&report)?;

    assert!(report.diagnostics().is_empty());
    assert_eq!(snapshot.logical_mesh_count(), 1);
    assert_eq!(snapshot.primitive_count(), 1);
    assert_eq!(snapshot.attribute_count(), 2);
    assert_eq!(snapshot.material_count(), 1);
    assert!(snapshot_line(&snapshot, "primitive[0]").contains(" material=0 "));
    assert!(!snapshot.preview().contains("key=obj:smoothing"));

    let smoothing = snapshot_line_containing(&snapshot, "semantic=smoothing_group");
    assert!(smoothing.contains("domain=face scalar=u32 lanes=1"));
    assert!(smoothing.contains("payload=dense elements=3 explicit=3 canonical_scalars=3"));
    assert!(smoothing.contains("values(present,total=3)=[0,7,0]"));
    Ok(())
}

#[test]
fn zero_and_off_smoothing_literals_are_canonically_equivalent() -> TestResult<()> {
    let zero_report = import_bytes(
        "zero-smoothing.obj",
        b"v 0 0 0\nv 1 0 0\nv 0 1 0\ns 0\nf 1 2 3\n",
    )?;
    let off_report = import_bytes(
        "off-smoothing.obj",
        b"v 0 0 0\nv 1 0 0\nv 0 1 0\ns off\nf 1 2 3\n",
    )?;
    let zero_snapshot = scene_snapshot(&zero_report)?;
    let off_snapshot = scene_snapshot(&off_report)?;

    assert!(zero_report.diagnostics().is_empty());
    assert!(off_report.diagnostics().is_empty());
    let zero_smoothing = snapshot_line_containing(&zero_snapshot, "semantic=smoothing_group");
    let off_smoothing = snapshot_line_containing(&off_snapshot, "semantic=smoothing_group");
    assert_eq!(zero_smoothing, off_smoothing);
    assert!(zero_smoothing.contains("values(present,total=1)=[0]"));
    assert!(!zero_snapshot.preview().contains("key=obj:smoothing"));
    assert!(!off_snapshot.preview().contains("key=obj:smoothing"));
    Ok(())
}

#[test]
fn maximum_u32_smoothing_group_is_preserved() -> TestResult<()> {
    let report = import_bytes(
        "maximum-smoothing.obj",
        b"v 0 0 0\nv 1 0 0\nv 0 1 0\ns 4294967295\nf 1 2 3\n",
    )?;
    let snapshot = scene_snapshot(&report)?;

    assert!(report.diagnostics().is_empty());
    assert!(
        snapshot_line_containing(&snapshot, "semantic=smoothing_group")
            .contains("values(present,total=1)=[4294967295]")
    );
    Ok(())
}
