use crate::canonical_scene::store::schema::{
    NODE_SCENE_MATRIX_SCALAR_COUNT, NODE_SCENE_TRANSFORM_MATRIX, NODE_SCENE_TRANSFORM_TRS,
    NODE_SCENE_TRS_SCALAR_COUNT,
};
use crate::entity::AuthoredTransform;
use crate::math::{Mat4, Quat, Vec3};

#[derive(Clone, Copy)]
enum StoredAuthoredTransformKind {
    Matrix,
    Trs,
}

impl StoredAuthoredTransformKind {
    const fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            NODE_SCENE_TRANSFORM_MATRIX => Some(Self::Matrix),
            NODE_SCENE_TRANSFORM_TRS => Some(Self::Trs),
            _ => None,
        }
    }

    const fn scalar_count(self) -> usize {
        match self {
            Self::Matrix => NODE_SCENE_MATRIX_SCALAR_COUNT,
            Self::Trs => NODE_SCENE_TRS_SCALAR_COUNT,
        }
    }

    fn decode(self, values: &[f32]) -> Option<AuthoredTransform> {
        if values.len() != self.scalar_count() {
            return None;
        }
        match self {
            Self::Matrix => {
                let values = <&[f32; NODE_SCENE_MATRIX_SCALAR_COUNT]>::try_from(values).ok()?;
                AuthoredTransform::try_matrix(Mat4 {
                    cols: [
                        [values[0], values[1], values[2], values[3]],
                        [values[4], values[5], values[6], values[7]],
                        [values[8], values[9], values[10], values[11]],
                        [values[12], values[13], values[14], values[15]],
                    ],
                })
                .ok()
            }
            Self::Trs => {
                let values = <&[f32; NODE_SCENE_TRS_SCALAR_COUNT]>::try_from(values).ok()?;
                AuthoredTransform::try_trs(
                    Vec3::new(values[0], values[1], values[2]),
                    Quat::from_xyzw(values[3], values[4], values[5], values[6]),
                    Vec3::new(values[7], values[8], values[9]),
                )
                .ok()
            }
        }
    }
}

/// Returns the exact canonical scalar width for one stored transform tag.
pub(in crate::canonical_scene::store) const fn authored_transform_scalar_count(
    kind: u8,
) -> Option<usize> {
    let kind = match StoredAuthoredTransformKind::from_tag(kind) {
        Some(kind) => kind,
        None => return None,
    };
    Some(kind.scalar_count())
}

/// Decodes one authored transform according to the canonical node-scene schema.
pub(in crate::canonical_scene::store) fn decode_authored_transform(
    kind: u8,
    values: &[f32],
) -> Option<AuthoredTransform> {
    StoredAuthoredTransformKind::from_tag(kind)?.decode(values)
}
