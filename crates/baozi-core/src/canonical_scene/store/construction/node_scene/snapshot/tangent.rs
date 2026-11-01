use super::super::semantic::geometry::{
    NodeSceneGeometryTangentBasis, NodeSceneGeometryTangentConventions,
    NodeSceneGeometryTangentEncoding,
};
use super::{
    CanonicalDigestEncoder, NodeSceneSnapshotError, PreviewSink, SnapshotWorkMeter,
    write_metered_bytes,
};

const TAG_TANGENT_CONVENTIONS: u8 = 224;
const TAG_TANGENT_BASIS: u8 = 225;
const TAG_TANGENT_ENCODING: u8 = 226;
const TAG_TANGENT_SCHEMA_NAME: u8 = 227;
const TAG_TANGENT_SCHEMA_VERSION: u8 = 228;

/// Commits only the declarations owned by this Tangent descriptor, including explicit Unknown.
pub(super) fn write_tangent_conventions(
    conventions: NodeSceneGeometryTangentConventions<'_>,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
    render: bool,
) -> Result<usize, NodeSceneSnapshotError> {
    work.observe()?;
    transcript.write_tag(TAG_TANGENT_CONVENTIONS)?;
    let (basis, basis_name, basis_schema) = match conventions.basis {
        NodeSceneGeometryTangentBasis::Unknown => (0, "unknown", None),
        NodeSceneGeometryTangentBasis::MikkTspace => (1, "mikk_tspace", None),
        NodeSceneGeometryTangentBasis::TriangleUvDerivative => (2, "triangle_uv_derivative", None),
        NodeSceneGeometryTangentBasis::Namespaced { name, version } => {
            (3, "namespaced", Some((name, version)))
        }
    };
    transcript.tagged_u8(TAG_TANGENT_BASIS, basis)?;
    preview.geometry_field_word(render, "tangent_basis", basis_name);
    let mut schema_bytes = 0;
    if let Some((name, version)) = basis_schema {
        write_metered_bytes(transcript, work, TAG_TANGENT_SCHEMA_NAME, name.as_bytes())?;
        transcript.tagged_u32(TAG_TANGENT_SCHEMA_VERSION, version.get())?;
        preview.geometry_field_word(render, "tangent_basis_schema_name", name);
        preview.geometry_field_u32(render, "tangent_basis_schema_version", version.get());
        schema_bytes += name.len();
    }
    let (encoding, encoding_name, encoding_schema) = match conventions.encoding {
        NodeSceneGeometryTangentEncoding::Unknown => (0, "unknown", None),
        NodeSceneGeometryTangentEncoding::SignInW => (1, "sign_in_w", None),
        NodeSceneGeometryTangentEncoding::Namespaced { name, version } => {
            (2, "namespaced", Some((name, version)))
        }
    };
    transcript.tagged_u8(TAG_TANGENT_ENCODING, encoding)?;
    preview.geometry_field_word(render, "tangent_handedness_encoding", encoding_name);
    if let Some((name, version)) = encoding_schema {
        write_metered_bytes(transcript, work, TAG_TANGENT_SCHEMA_NAME, name.as_bytes())?;
        transcript.tagged_u32(TAG_TANGENT_SCHEMA_VERSION, version.get())?;
        preview.geometry_field_word(render, "tangent_encoding_schema_name", name);
        preview.geometry_field_u32(render, "tangent_encoding_schema_version", version.get());
        schema_bytes += name.len();
    }
    Ok(schema_bytes)
}
