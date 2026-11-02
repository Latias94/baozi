//! Finite core/quantization encoding rules, shared by global and selected mesh checks.

use super::schema::{
    accessor::{ComponentType as C, Type},
    mesh::Semantic,
};

#[derive(Clone, Copy)]
pub(crate) enum MeshAttributeUse {
    Base(Semantic),
    Morph(Semantic),
}

impl MeshAttributeUse {
    pub(crate) fn semantic(self) -> Semantic {
        match self {
            Self::Base(semantic) | Self::Morph(semantic) => semantic,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self.semantic() {
            Semantic::Positions => "POSITION",
            Semantic::Normals => "NORMAL",
            Semantic::Tangents => "TANGENT",
            Semantic::TexCoords(_) => "TEXCOORD",
            Semantic::Colors(_) => "COLOR",
            Semantic::Joints(_) => "JOINTS",
            Semantic::Weights(_) => "WEIGHTS",
            Semantic::Extras(_) => "custom",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum MeshEncodingClass {
    Core,
    Quantization,
}

pub(crate) struct MeshAttributeEncoding {
    pub(crate) dimensions: &'static [Type],
    pub(crate) class: MeshEncodingClass,
}

pub(crate) fn mesh_attribute_encoding(
    usage: MeshAttributeUse,
    component: C,
    normalized: bool,
) -> Option<MeshAttributeEncoding> {
    use MeshAttributeUse::{Base, Morph};
    use MeshEncodingClass::{Core, Quantization};
    use Semantic::{Colors, Joints, Normals, Positions, Tangents, TexCoords, Weights};

    let dimensions: &'static [Type] = match usage {
        Base(Positions | Normals) | Morph(Positions | Normals | Tangents) => &[Type::Vec3],
        Base(Tangents | Joints(_) | Weights(_)) => &[Type::Vec4],
        Base(TexCoords(_)) | Morph(TexCoords(_)) => &[Type::Vec2],
        Base(Colors(_)) | Morph(Colors(_)) => &[Type::Vec3, Type::Vec4],
        _ => return None,
    };
    let class = match (usage, component, normalized) {
        (Base(Joints(_)), C::U8 | C::U16, false) => Core,
        (Base(Joints(_)), _, _) => return None,
        (_, C::F32, false) => Core,
        (Base(Positions), C::I8 | C::U8 | C::I16 | C::U16, _) => Quantization,
        (Morph(Positions), C::I8 | C::I16, _) => Quantization,
        (Base(Normals | Tangents) | Morph(Normals | Tangents), C::I8 | C::I16, true) => {
            Quantization
        }
        (Base(TexCoords(_) | Colors(_) | Weights(_)), C::U8 | C::U16, true) => Core,
        (Base(TexCoords(_)), C::I8 | C::I16, _)
        | (Base(TexCoords(_)), C::U8 | C::U16, false)
        | (Morph(TexCoords(_)), C::I8 | C::I16, false) => Quantization,
        (Morph(TexCoords(_) | Colors(_)), C::I8 | C::U8 | C::I16 | C::U16, true) => Core,
        _ => return None,
    };
    Some(MeshAttributeEncoding { dimensions, class })
}

pub(super) fn optional_morph_semantic(name: &str) -> Option<Semantic> {
    if let Some(set) = name.strip_prefix("TEXCOORD_") {
        return set.parse().ok().map(Semantic::TexCoords);
    }
    name.strip_prefix("COLOR_")
        .and_then(|set| set.parse().ok())
        .map(Semantic::Colors)
}
