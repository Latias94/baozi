use std::path::PathBuf;

use baozi::{
    AssetPath, AttributeData, AttributeDomain, AttributeSemanticRef, Importer, TangentBasisRef,
    TangentEncodingRef,
    postprocess::{
        PostProcessControl, PostProcessRunDisposition, SpaceConversionOptions, TangentOptions,
        convert_scene_space, generate_tangents,
    },
};
use baozi_asset_consumer::{ApplicationAsset, Options, Result};
use baozi_native_io_candidate::{FileSystemAssetIo, NativeDirectoryCapability};

#[test]
fn textured_box_generated_tangents_survive_reflection_and_scene_destruction() -> Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let io = FileSystemAssetIo::from_capability(NativeDirectoryCapability::open_ambient(root)?);
    let (scene, import_evidence) = Importer::new()?
        .read_asset(
            &io,
            AssetPath::new("upstream/Models/BoxTextured/glTF/BoxTextured.gltf")?,
        )?
        .into_parts();
    let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
    assert!(
        primitive
            .attributes()
            .all(|a| a.semantic() != AttributeSemanticRef::Tangent)
    );
    assert!(primitive.normals(AttributeDomain::Vertex).is_some());
    assert!(primitive.texcoords(0, AttributeDomain::Vertex).is_some());
    let target = primitive.id();
    let (scene, generated) = generate_tangents(
        scene,
        target,
        TangentOptions::new(AttributeDomain::Vertex, 0, AttributeDomain::Vertex),
        PostProcessControl::default(),
    )?
    .into_parts();
    assert_eq!(generated.disposition(), PostProcessRunDisposition::Changed);
    assert!(generated.matches_scene(&scene));
    assert_eq!(generated.resources().active_reservations(), 0);
    let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
    let tangent = primitive
        .attribute(AttributeSemanticRef::Tangent, AttributeDomain::Corner)
        .unwrap();
    let conventions = tangent.tangent_conventions().unwrap();
    assert_eq!(conventions.basis, TangentBasisRef::TriangleUvDerivative);
    assert_eq!(conventions.encoding, TangentEncodingRef::SignInW);
    let AttributeData::DenseF32 { lanes: 4, values } = tangent.data() else {
        panic!("generated Tangents must be dense Corner F32x4");
    };
    let before: Vec<[f32; 4]> = values
        .chunks_exact(4)
        .map(|v| v.try_into().unwrap())
        .collect();
    assert_eq!(before.len(), 36);
    let (scene, converted) = convert_scene_space(
        scene,
        SpaceConversionOptions {
            target: Options::default().target,
            source_assumptions: None,
        },
        PostProcessControl::default(),
    )?
    .into_parts();
    assert!(
        converted
            .space_conversion_effects()
            .unwrap()
            .reverses_triangle_winding()
    );
    let asset = ApplicationAsset::from_scene(&scene)?;
    drop(scene);
    assert_eq!(asset.draws.len(), 1);
    let draw = &asset.draws[0];
    assert_eq!(draw.vertices.len(), 24);
    assert_eq!(draw.indices.len(), before.len());
    for (corner, &index) in draw.indices.iter().enumerate() {
        let source = before[(corner / 3) * 3 + [0, 2, 1][corner % 3]];
        let tangent = draw.vertices[index as usize].tangent.unwrap();
        assert_eq!(
            tangent,
            [source[0], source[1], -source[2], -source[3]].map(f64::from)
        );
        let normal = draw.vertices[index as usize].normal.unwrap();
        assert!((tangent[..3].iter().map(|v| v * v).sum::<f64>() - 1.0).abs() < 1e-6);
        assert!(
            normal
                .iter()
                .zip(tangent)
                .map(|(n, t)| n * t)
                .sum::<f64>()
                .abs()
                < 1e-6
        );
    }
    assert_eq!(
        generated.tangent_generation().unwrap().input_target(),
        target
    );
    assert!(
        !generated
            .tangent_generation()
            .unwrap()
            .replaced_authored_tangents()
    );
    assert!(import_evidence.loss_evidence().is_empty());
    Ok(())
}
