//! The private selected profile. Containers own their admission; source ordinals never compact.

use baozi_import::{MeteredString, MeteredVec};
use std::{fmt, ops::Deref};

#[allow(
    clippy::large_enum_variant,
    reason = "decoded strings retain their inline admission owner; boxing would allocate outside that owner"
)]
pub(crate) enum Text<'scope> {
    Borrowed(&'scope str),
    Decoded(MeteredString<'scope>),
}

impl Text<'_> {
    pub(crate) fn as_str(&self) -> &str {
        match self {
            Self::Borrowed(text) => text,
            Self::Decoded(text) => text.as_str(),
        }
    }
}

impl Deref for Text<'_> {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Debug for Text<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl fmt::Display for Text<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.as_str(), f)
    }
}

impl PartialEq<str> for Text<'_> {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for Text<'_> {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

pub(crate) struct List<'scope, T>(pub(crate) MeteredVec<'scope, T>);

impl<T> Deref for List<'_, T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        self.0.as_slice()
    }
}

impl<'a, T> IntoIterator for &'a List<'_, T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Index(pub(crate) u32);
impl Index {
    pub(crate) fn value(self) -> usize {
        self.0 as usize
    }
}

pub(crate) struct Tables<'scope> {
    pub(crate) scene: Option<Index>,
    pub(crate) scenes: List<'scope, Scene<'scope>>,
    pub(crate) nodes: List<'scope, Node<'scope>>,
    pub(crate) meshes: List<'scope, mesh::Mesh<'scope>>,
    pub(crate) materials: List<'scope, Material<'scope>>,
    pub(crate) textures: List<'scope, Texture<'scope>>,
    pub(crate) images: List<'scope, Image<'scope>>,
    pub(crate) samplers: List<'scope, Sampler<'scope>>,
    pub(crate) skins: List<'scope, Skin<'scope>>,
    pub(crate) buffers: List<'scope, Buffer<'scope>>,
    pub(crate) buffer_views: List<'scope, buffer::View>,
    pub(crate) accessors: List<'scope, accessor::Accessor>,
    pub(crate) animations: List<'scope, animation::Animation<'scope>>,
    pub(crate) cameras: List<'scope, Camera<'scope>>,
    pub(crate) lights: List<'scope, crate::json_syntax::ValueId>,
    pub(crate) extensions_used: List<'scope, Text<'scope>>,
    pub(crate) extensions_required: List<'scope, Text<'scope>>,
}

pub(crate) struct Scene<'scope> {
    pub(crate) nodes: List<'scope, Index>,
}
pub(crate) struct Node<'scope> {
    pub(crate) name: Option<Text<'scope>>,
    pub(crate) children: List<'scope, Index>,
    pub(crate) mesh: Option<Index>,
    pub(crate) skin: Option<Index>,
    pub(crate) camera: Option<Index>,
    pub(crate) punctual_light: Option<crate::json_syntax::ValueId>,
    pub(crate) matrix: Option<[f32; 16]>,
    pub(crate) translation: [f32; 3],
    pub(crate) rotation: [f32; 4],
    pub(crate) scale: [f32; 3],
    pub(crate) weights: Option<List<'scope, f32>>,
}
pub(crate) struct Buffer<'scope> {
    pub(crate) uri: Option<Text<'scope>>,
    pub(crate) byte_length: u64,
}

/// Source fields remain binary64 until the format's checked lowering operation.
pub(crate) struct Light<'scope> {
    pub(crate) name: Option<Text<'scope>>,
    pub(crate) kind: Text<'scope>,
    pub(crate) color: [f64; 3],
    pub(crate) intensity: f64,
    pub(crate) range: Option<f64>,
    pub(crate) spot: Option<LightSpot>,
}

pub(crate) struct LightSpot {
    pub(crate) inner_cone_angle: f64,
    pub(crate) outer_cone_angle: f64,
}
pub(crate) mod buffer {
    use super::*;
    pub(crate) struct View {
        pub(crate) buffer: Index,
        pub(crate) byte_offset: Option<u64>,
        pub(crate) byte_length: u64,
        pub(crate) byte_stride: Option<usize>,
        pub(crate) target: Option<u32>,
    }
}
pub(crate) mod accessor {
    use super::*;
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum ComponentType {
        I8,
        U8,
        I16,
        U16,
        U32,
        F32,
    }
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum Type {
        Scalar,
        Vec2,
        Vec3,
        Vec4,
        Mat2,
        Mat3,
        Mat4,
    }
    #[derive(Clone, Copy)]
    pub(crate) struct Bounds {
        pub(crate) values: [f64; 16],
        pub(crate) len: usize,
    }
    impl Bounds {
        pub(crate) fn vec3(self) -> Option<[f32; 3]> {
            (self.len == 3).then(|| {
                [
                    self.values[0] as f32,
                    self.values[1] as f32,
                    self.values[2] as f32,
                ]
            })
        }
    }
    pub(crate) struct Accessor {
        pub(crate) buffer_view: Option<Index>,
        pub(crate) byte_offset: Option<u64>,
        pub(crate) component_type: ComponentType,
        pub(crate) count: u64,
        pub(crate) type_: Type,
        pub(crate) normalized: bool,
        pub(crate) sparse: Option<Sparse>,
        pub(crate) min: Option<Bounds>,
        pub(crate) max: Option<Bounds>,
    }
    pub(crate) struct Sparse {
        pub(crate) count: u64,
        pub(crate) indices: Indices,
        pub(crate) values: Values,
    }
    pub(crate) struct Indices {
        pub(crate) buffer_view: Index,
        pub(crate) byte_offset: u64,
        pub(crate) component_type: ComponentType,
    }
    pub(crate) struct Values {
        pub(crate) buffer_view: Index,
        pub(crate) byte_offset: u64,
    }
}
pub(crate) mod mesh {
    use super::*;
    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
    pub(crate) enum Semantic {
        Extras(u32),
        Positions,
        Normals,
        Tangents,
        Colors(u32),
        TexCoords(u32),
        Joints(u32),
        Weights(u32),
    }
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum Mode {
        Points,
        Lines,
        LineLoop,
        LineStrip,
        Triangles,
        TriangleStrip,
        TriangleFan,
    }
    pub(crate) struct Attributes<'scope>(pub(crate) List<'scope, (Semantic, Index)>);
    impl Attributes<'_> {
        pub(crate) fn get(&self, key: &Semantic) -> Option<&Index> {
            self.0
                .binary_search_by_key(key, |entry| entry.0)
                .ok()
                .map(|index| &self.0[index].1)
        }
        pub(crate) fn contains_key(&self, key: &Semantic) -> bool {
            self.get(key).is_some()
        }
        pub(crate) fn len(&self) -> usize {
            self.0.len()
        }
        pub(crate) fn keys(&self) -> impl Iterator<Item = &Semantic> {
            self.0.iter().map(|entry| &entry.0)
        }
        pub(crate) fn values(&self) -> impl Iterator<Item = &Index> {
            self.0.iter().map(|entry| &entry.1)
        }
        pub(crate) fn iter(&self) -> impl ExactSizeIterator<Item = (&Semantic, &Index)> {
            self.0.iter().map(|entry| (&entry.0, &entry.1))
        }
    }
    impl<'a> IntoIterator for &'a Attributes<'_> {
        type Item = (&'a Semantic, &'a Index);
        type IntoIter = std::iter::Map<
            std::slice::Iter<'a, (Semantic, Index)>,
            fn(&'a (Semantic, Index)) -> (&'a Semantic, &'a Index),
        >;
        fn into_iter(self) -> Self::IntoIter {
            self.0.iter().map(|entry| (&entry.0, &entry.1))
        }
    }
    pub(crate) struct Mesh<'scope> {
        pub(crate) name: Option<Text<'scope>>,
        pub(crate) primitives: List<'scope, Primitive<'scope>>,
        pub(crate) weights: Option<List<'scope, f32>>,
    }
    pub(crate) struct Primitive<'scope> {
        pub(crate) attributes: Attributes<'scope>,
        pub(crate) indices: Option<Index>,
        pub(crate) material: Option<Index>,
        pub(crate) mode: Mode,
        pub(crate) targets: Option<List<'scope, MorphTarget<'scope>>>,
    }
    pub(crate) struct MorphTarget<'scope> {
        pub(crate) positions: Option<Index>,
        pub(crate) normals: Option<Index>,
        pub(crate) tangents: Option<Index>,
        pub(crate) other_attributes: UnknownAttributes<'scope>,
    }
    pub(crate) struct UnknownAttributes<'scope>(pub(crate) List<'scope, (Text<'scope>, Index)>);
    impl UnknownAttributes<'_> {
        pub(crate) fn first_key_value(&self) -> Option<(&Text<'_>, &Index)> {
            self.0.first().map(|entry| (&entry.0, &entry.1))
        }
    }
}
pub(crate) mod animation {
    use super::*;
    #[derive(Clone, Copy, Debug)]
    pub(crate) enum Property {
        Translation,
        Rotation,
        Scale,
        MorphTargetWeights,
    }
    #[derive(Clone, Copy, Debug)]
    pub(crate) enum Interpolation {
        Step,
        Linear,
        CubicSpline,
    }
    pub(crate) struct Animation<'scope> {
        pub(crate) name: Option<Text<'scope>>,
        pub(crate) channels: List<'scope, Channel>,
        pub(crate) samplers: List<'scope, Sampler>,
    }
    pub(crate) struct Channel {
        pub(crate) sampler: Index,
        pub(crate) target: Target,
    }
    pub(crate) struct Target {
        pub(crate) node: Option<Index>,
        pub(crate) path: Property,
    }
    pub(crate) struct Sampler {
        pub(crate) input: Index,
        pub(crate) output: Index,
        pub(crate) interpolation: Interpolation,
    }
}
pub(crate) struct Skin<'scope> {
    pub(crate) name: Option<Text<'scope>>,
    pub(crate) joints: List<'scope, Index>,
    pub(crate) skeleton: Option<Index>,
    pub(crate) inverse_bind_matrices: Option<Index>,
}
pub(crate) struct Image<'scope> {
    pub(crate) name: Option<Text<'scope>>,
    pub(crate) uri: Option<Text<'scope>>,
    pub(crate) mime_type: Option<Text<'scope>>,
    pub(crate) buffer_view: Option<Index>,
}
pub(crate) struct Texture<'scope> {
    pub(crate) name: Option<Text<'scope>>,
    pub(crate) source: Index,
    pub(crate) sampler: Option<Index>,
}
#[derive(Clone, Copy)]
pub(crate) enum MagFilter {
    Nearest,
    Linear,
}
#[derive(Clone, Copy)]
pub(crate) enum MinFilter {
    Nearest,
    Linear,
    NearestMipmapNearest,
    LinearMipmapNearest,
    NearestMipmapLinear,
    LinearMipmapLinear,
}
#[derive(Clone, Copy)]
pub(crate) enum WrappingMode {
    ClampToEdge,
    MirroredRepeat,
    Repeat,
}
pub(crate) struct Sampler<'scope> {
    pub(crate) name: Option<Text<'scope>>,
    pub(crate) mag_filter: Option<MagFilter>,
    pub(crate) min_filter: Option<MinFilter>,
    pub(crate) wrap_s: WrappingMode,
    pub(crate) wrap_t: WrappingMode,
}
#[derive(Clone, Copy)]
pub(crate) enum AlphaMode {
    Opaque,
    Mask,
    Blend,
}
pub(crate) struct Material<'scope> {
    pub(crate) name: Option<Text<'scope>>,
    pub(crate) pbr_metallic_roughness: Pbr,
    pub(crate) emissive_factor: [f32; 3],
    pub(crate) alpha_mode: AlphaMode,
    pub(crate) alpha_cutoff: Option<f32>,
    pub(crate) double_sided: bool,
    pub(crate) normal_texture: Option<texture::Info>,
    pub(crate) occlusion_texture: Option<texture::Info>,
    pub(crate) emissive_texture: Option<texture::Info>,
    pub(crate) unlit: Option<crate::json_syntax::ValueId>,
    pub(crate) emissive_strength: Option<crate::json_syntax::ValueId>,
}
pub(crate) struct Pbr {
    pub(crate) base_color_factor: [f32; 4],
    pub(crate) metallic_factor: f32,
    pub(crate) roughness_factor: f32,
    pub(crate) base_color_texture: Option<texture::Info>,
    pub(crate) metallic_roughness_texture: Option<texture::Info>,
}
pub(crate) mod texture {
    use super::*;
    pub(crate) struct Info {
        pub(crate) index: Index,
        pub(crate) tex_coord: u32,
        pub(crate) strength: f32,
        pub(crate) transform: Option<crate::json_syntax::ValueId>,
    }
}
pub(crate) mod extensions {
    pub(crate) mod texture {
        pub(crate) struct TextureTransform {
            pub(crate) offset: [f32; 2],
            pub(crate) rotation: f32,
            pub(crate) scale: [f32; 2],
            pub(crate) tex_coord: Option<u32>,
        }
    }
}
pub(crate) struct Camera<'scope> {
    pub(crate) name: Option<Text<'scope>>,
    pub(crate) projection: Projection,
}
#[derive(Clone, Copy)]
pub(crate) enum Projection {
    Orthographic {
        xmag: f32,
        ymag: f32,
        znear: f32,
        zfar: f32,
    },
    Perspective {
        yfov: f32,
        aspect_ratio: Option<f32>,
        znear: f32,
        zfar: Option<f32>,
    },
}
