//! Owned, metered selected-profile rows over the admitted immutable JSON surface.

mod decode;
mod integer;
pub(crate) mod mesh_encoding;
pub(crate) mod read;
pub(crate) mod schema;
mod transforms;
mod validate;

use crate::{
    admission::AdmittedSurfaces,
    json_syntax::{JsonIndex, ValueId},
    source_map::GltfSourceMap,
};
use baozi_import::{ParseResult, ParseScope};
use schema::Text;
pub(crate) use schema::{
    AlphaMode, MagFilter, MinFilter, WrappingMode,
    mesh::{Mode, Semantic},
};

pub(crate) const KHR_TEXTURE_TRANSFORM: &str = "KHR_texture_transform";
pub(crate) const KHR_MATERIALS_UNLIT: &str = "KHR_materials_unlit";
pub(crate) const KHR_MATERIALS_EMISSIVE_STRENGTH: &str = "KHR_materials_emissive_strength";
pub(crate) const KHR_LIGHTS_PUNCTUAL: &str = "KHR_lights_punctual";
pub(crate) const KHR_MATERIALS_CLEARCOAT: &str = "KHR_materials_clearcoat";
pub(crate) const KHR_MATERIALS_SHEEN: &str = "KHR_materials_sheen";
pub(crate) const KHR_MESH_QUANTIZATION: &str = "KHR_mesh_quantization";

pub(crate) fn is_supported_extension(name: &str) -> bool {
    [
        KHR_TEXTURE_TRANSFORM,
        KHR_MATERIALS_UNLIT,
        KHR_MATERIALS_EMISSIVE_STRENGTH,
        KHR_LIGHTS_PUNCTUAL,
        KHR_MATERIALS_CLEARCOAT,
        KHR_MATERIALS_SHEEN,
        KHR_MESH_QUANTIZATION,
    ]
    .contains(&name)
}

pub(crate) struct Document<'scope> {
    tables: schema::Tables<'scope>,
    has_mesh_quantization: bool,
    source_map: GltfSourceMap<'scope>,
    index: JsonIndex<'scope>,
    json: &'scope [u8],
    bin: Option<&'scope [u8]>,
}

impl<'scope> Document<'scope> {
    pub(crate) fn decode(
        ctx: &'scope ParseScope<'_>,
        surfaces: AdmittedSurfaces<'scope>,
    ) -> ParseResult<Self> {
        let (json, bin, source_map, index) = surfaces.into_parts();
        let tables = decode::Decoder {
            ctx,
            bytes: json,
            index: &index,
        }
        .root()?;
        let has_mesh_quantization = validate::validate(ctx, &tables)?;
        Ok(Self {
            tables,
            has_mesh_quantization,
            source_map,
            index,
            json,
            bin,
        })
    }

    pub(crate) fn tables(&self) -> &schema::Tables<'scope> {
        &self.tables
    }
    pub(crate) fn has_mesh_quantization(&self) -> bool {
        self.has_mesh_quantization
    }
    pub(crate) fn source_map(&self) -> &GltfSourceMap<'scope> {
        &self.source_map
    }
    pub(crate) fn bin_bytes(&self) -> Option<&'scope [u8]> {
        self.bin
    }
    pub(crate) fn texture_transform(
        &self,
        ctx: &ParseScope<'_>,
        id: ValueId,
    ) -> ParseResult<schema::extensions::texture::TextureTransform> {
        decode::Decoder {
            ctx,
            bytes: self.json,
            index: &self.index,
        }
        .transform(id)
    }
    pub(crate) fn emissive_strength(&self, ctx: &ParseScope<'_>, id: ValueId) -> ParseResult<f32> {
        decode::Decoder {
            ctx,
            bytes: self.json,
            index: &self.index,
        }
        .emissive_strength(id)
    }
    pub(crate) fn clearcoat(
        &self,
        ctx: &ParseScope<'_>,
        id: ValueId,
    ) -> ParseResult<schema::Clearcoat> {
        decode::Decoder {
            ctx,
            bytes: self.json,
            index: &self.index,
        }
        .clearcoat(id)
    }
    pub(crate) fn sheen(&self, ctx: &ParseScope<'_>, id: ValueId) -> ParseResult<schema::Sheen> {
        decode::Decoder {
            ctx,
            bytes: self.json,
            index: &self.index,
        }
        .sheen(id)
    }
    pub(crate) fn node(&self, index: usize) -> Option<Node<'_>> {
        self.tables
            .nodes
            .get(index)
            .map(|_| Node { doc: self, index })
    }
    pub(crate) fn punctual_light_reference(
        &self,
        ctx: &ParseScope<'_>,
        id: ValueId,
    ) -> ParseResult<schema::Index> {
        decode::Decoder {
            ctx,
            bytes: self.json,
            index: &self.index,
        }
        .punctual_light_reference(id)
    }

    pub(crate) fn light(
        &self,
        ctx: &'scope ParseScope<'_>,
        id: ValueId,
    ) -> ParseResult<schema::Light<'scope>> {
        decode::Decoder {
            ctx,
            bytes: self.json,
            index: &self.index,
        }
        .light(id)
    }
    pub(crate) fn default_scene(&self) -> Option<Scene<'_>> {
        self.tables.scene.map(|id| Scene {
            doc: self,
            index: id.value(),
        })
    }
}

macro_rules! table_views {
    ($($plural:ident: $view:ident => $table:ident),* $(,)?) => {$(
        #[derive(Clone, Copy)]
        pub(crate) struct $view<'a> { pub(crate) doc: &'a Document<'a>, pub(crate) index: usize }
        impl $view<'_> { pub(crate) fn index(&self) -> usize { self.index } }
        impl std::fmt::Debug for $view<'_> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.debug_tuple(stringify!($view)).field(&self.index).finish() }
        }
        impl Document<'_> {
            pub(crate) fn $plural(&self) -> impl ExactSizeIterator<Item = $view<'_>> + '_ {
                (0..self.tables.$table.len()).map(move |index| $view { doc: self, index })
            }
        }
    )*};
}
table_views! {
    nodes: Node => nodes, scenes: Scene => scenes, meshes: Mesh => meshes,
    buffers: Buffer => buffers, accessors: Accessor => accessors, skins: Skin => skins,
    images: Image => images, textures: Texture => textures, cameras: Camera => cameras,
    animations: Animation => animations,
}

impl<'a> Node<'a> {
    pub(crate) fn name(&self) -> Option<&'a str> {
        self.doc.tables.nodes[self.index]
            .name
            .as_ref()
            .map(Text::as_str)
    }
    pub(crate) fn children(&self) -> impl ExactSizeIterator<Item = Node<'a>> + 'a {
        let doc = self.doc;
        doc.tables.nodes[self.index]
            .children
            .iter()
            .map(move |id| Node {
                doc,
                index: id.value(),
            })
    }
    pub(crate) fn mesh(&self) -> Option<Mesh<'a>> {
        self.doc.tables.nodes[self.index].mesh.map(|id| Mesh {
            doc: self.doc,
            index: id.value(),
        })
    }
    pub(crate) fn skin(&self) -> Option<Skin<'a>> {
        self.doc.tables.nodes[self.index].skin.map(|id| Skin {
            doc: self.doc,
            index: id.value(),
        })
    }
    pub(crate) fn camera(&self) -> Option<Camera<'a>> {
        self.doc.tables.nodes[self.index].camera.map(|id| Camera {
            doc: self.doc,
            index: id.value(),
        })
    }
    pub(crate) fn transform(&self) -> NodeTransform {
        let row = &self.doc.tables.nodes[self.index];
        match row.matrix {
            Some(matrix) => NodeTransform::Matrix {
                matrix: std::array::from_fn(|column| {
                    std::array::from_fn(|lane| matrix[column * 4 + lane])
                }),
            },
            None => NodeTransform::Decomposed {
                translation: row.translation,
                rotation: row.rotation,
                scale: row.scale,
            },
        }
    }
}
pub(crate) enum NodeTransform {
    Matrix {
        matrix: [[f32; 4]; 4],
    },
    Decomposed {
        translation: [f32; 3],
        rotation: [f32; 4],
        scale: [f32; 3],
    },
}
impl<'a> Scene<'a> {
    pub(crate) fn nodes(&self) -> impl ExactSizeIterator<Item = Node<'a>> + 'a {
        let doc = self.doc;
        doc.tables.scenes[self.index]
            .nodes
            .iter()
            .map(move |id| Node {
                doc,
                index: id.value(),
            })
    }
}
impl<'a> Mesh<'a> {
    pub(crate) fn name(&self) -> Option<&'a str> {
        self.doc.tables.meshes[self.index]
            .name
            .as_ref()
            .map(Text::as_str)
    }
    pub(crate) fn weights(&self) -> Option<&'a [f32]> {
        self.doc.tables.meshes[self.index].weights.as_deref()
    }
    pub(crate) fn primitives(&self) -> impl ExactSizeIterator<Item = Primitive<'a>> + 'a {
        let doc = self.doc;
        let mesh = self.index;
        (0..doc.tables.meshes[mesh].primitives.len()).map(move |index| Primitive {
            doc,
            mesh,
            index,
        })
    }
}
#[derive(Clone, Copy)]
pub(crate) struct Primitive<'a> {
    pub(crate) doc: &'a Document<'a>,
    pub(crate) mesh: usize,
    pub(crate) index: usize,
}
impl<'a> Primitive<'a> {
    pub(crate) fn index(&self) -> usize {
        self.index
    }
    fn row(&self) -> &'a schema::mesh::Primitive<'a> {
        &self.doc.tables.meshes[self.mesh].primitives[self.index]
    }
    pub(crate) fn get(&self, semantic: &Semantic) -> Option<Accessor<'a>> {
        self.row().attributes.get(semantic).map(|id| Accessor {
            doc: self.doc,
            index: id.value(),
        })
    }
    pub(crate) fn indices(&self) -> Option<Accessor<'a>> {
        self.row().indices.map(|id| Accessor {
            doc: self.doc,
            index: id.value(),
        })
    }
    pub(crate) fn attributes(
        &self,
    ) -> impl ExactSizeIterator<Item = (Semantic, Accessor<'a>)> + 'a {
        let doc = self.doc;
        self.row().attributes.iter().map(move |(semantic, id)| {
            (
                *semantic,
                Accessor {
                    doc,
                    index: id.value(),
                },
            )
        })
    }
}
impl<'a> Buffer<'a> {
    pub(crate) fn length(&self) -> usize {
        self.doc.tables.buffers[self.index].byte_length as usize
    }
    pub(crate) fn source(&self) -> BufferSource<'a> {
        self.doc.tables.buffers[self.index]
            .uri
            .as_ref()
            .map_or(BufferSource::Bin, |uri| BufferSource::Uri(uri.as_str()))
    }
}
pub(crate) enum BufferSource<'a> {
    Bin,
    Uri(&'a str),
}
#[derive(Clone, Copy)]
pub(crate) struct View<'a> {
    doc: &'a Document<'a>,
    index: usize,
}
impl<'a> View<'a> {
    pub(crate) fn index(&self) -> usize {
        self.index
    }
    pub(crate) fn buffer(&self) -> Buffer<'a> {
        Buffer {
            doc: self.doc,
            index: self.doc.tables.buffer_views[self.index].buffer.value(),
        }
    }
    pub(crate) fn offset(&self) -> usize {
        self.doc.tables.buffer_views[self.index]
            .byte_offset
            .map_or(0, |offset| offset as usize)
    }
    pub(crate) fn length(&self) -> usize {
        self.doc.tables.buffer_views[self.index].byte_length as usize
    }
}
impl<'a> Accessor<'a> {
    pub(crate) fn row(&self) -> &'a schema::accessor::Accessor {
        &self.doc.tables.accessors[self.index]
    }
    pub(crate) fn count(&self) -> usize {
        self.row().count as usize
    }
    pub(crate) fn view(&self) -> Option<View<'a>> {
        self.row().buffer_view.map(|id| View {
            doc: self.doc,
            index: id.value(),
        })
    }
    pub(crate) fn sparse(&self) -> Option<Sparse<'a>> {
        self.row()
            .sparse
            .as_ref()
            .map(|row| Sparse { doc: self.doc, row })
    }
}
pub(crate) struct Sparse<'a> {
    doc: &'a Document<'a>,
    row: &'a schema::accessor::Sparse,
}
impl<'a> Sparse<'a> {
    pub(crate) fn count(&self) -> usize {
        self.row.count as usize
    }
    pub(crate) fn indices(&self) -> SparseIndices<'a> {
        SparseIndices {
            doc: self.doc,
            row: &self.row.indices,
        }
    }
    pub(crate) fn values(&self) -> SparseValues<'a> {
        SparseValues {
            doc: self.doc,
            row: &self.row.values,
        }
    }
}
pub(crate) struct SparseIndices<'a> {
    doc: &'a Document<'a>,
    row: &'a schema::accessor::Indices,
}
#[derive(Clone, Copy)]
pub(crate) enum IndexType {
    U8,
    U16,
    U32,
}
impl IndexType {
    pub(crate) fn size(self) -> usize {
        match self {
            Self::U8 => 1,
            Self::U16 => 2,
            Self::U32 => 4,
        }
    }
}
impl<'a> SparseIndices<'a> {
    pub(crate) fn view(&self) -> View<'a> {
        View {
            doc: self.doc,
            index: self.row.buffer_view.value(),
        }
    }
    pub(crate) fn offset(&self) -> usize {
        self.row.byte_offset as usize
    }
    pub(crate) fn index_type(&self) -> IndexType {
        use schema::accessor::ComponentType as C;
        match self.row.component_type {
            C::U8 => IndexType::U8,
            C::U16 => IndexType::U16,
            _ => IndexType::U32,
        }
    }
}
pub(crate) struct SparseValues<'a> {
    doc: &'a Document<'a>,
    row: &'a schema::accessor::Values,
}
impl<'a> SparseValues<'a> {
    pub(crate) fn view(&self) -> View<'a> {
        View {
            doc: self.doc,
            index: self.row.buffer_view.value(),
        }
    }
}
impl<'a> Skin<'a> {
    pub(crate) fn name(&self) -> Option<&'a str> {
        self.doc.tables.skins[self.index]
            .name
            .as_ref()
            .map(Text::as_str)
    }
    pub(crate) fn joints(&self) -> impl ExactSizeIterator<Item = Node<'a>> + 'a {
        let doc = self.doc;
        doc.tables.skins[self.index]
            .joints
            .iter()
            .map(move |id| Node {
                doc,
                index: id.value(),
            })
    }
    pub(crate) fn skeleton(&self) -> Option<Node<'a>> {
        self.doc.tables.skins[self.index].skeleton.map(|id| Node {
            doc: self.doc,
            index: id.value(),
        })
    }
    pub(crate) fn inverse_bind_matrices(&self) -> Option<Accessor<'a>> {
        self.doc.tables.skins[self.index]
            .inverse_bind_matrices
            .map(|id| Accessor {
                doc: self.doc,
                index: id.value(),
            })
    }
}
impl<'a> Image<'a> {
    pub(crate) fn name(&self) -> Option<&'a str> {
        self.doc.tables.images[self.index]
            .name
            .as_ref()
            .map(Text::as_str)
    }
    pub(crate) fn source(&self) -> ImageSource<'a> {
        let row = &self.doc.tables.images[self.index];
        match row.buffer_view {
            Some(view) => ImageSource::View {
                view: View {
                    doc: self.doc,
                    index: view.value(),
                },
                mime_type: row.mime_type.as_ref().map_or("", Text::as_str),
            },
            None => ImageSource::Uri {
                uri: row.uri.as_ref().map_or("", Text::as_str),
                mime_type: row.mime_type.as_ref().map(Text::as_str),
            },
        }
    }
}
pub(crate) enum ImageSource<'a> {
    Uri {
        uri: &'a str,
        mime_type: Option<&'a str>,
    },
    View {
        view: View<'a>,
        mime_type: &'a str,
    },
}
impl<'a> Texture<'a> {
    pub(crate) fn name(&self) -> Option<&'a str> {
        self.doc.tables.textures[self.index]
            .name
            .as_ref()
            .map(Text::as_str)
    }
    pub(crate) fn source(&self) -> Image<'a> {
        Image {
            doc: self.doc,
            index: self.doc.tables.textures[self.index].source.value(),
        }
    }
    pub(crate) fn sampler(&self) -> Sampler<'a> {
        Sampler {
            doc: self.doc,
            index: self.doc.tables.textures[self.index]
                .sampler
                .map(schema::Index::value),
        }
    }
}
#[derive(Clone, Copy)]
pub(crate) struct Sampler<'a> {
    doc: &'a Document<'a>,
    index: Option<usize>,
}
impl Document<'_> {
    pub(crate) fn samplers(&self) -> impl ExactSizeIterator<Item = Sampler<'_>> + '_ {
        (0..self.tables.samplers.len()).map(move |index| Sampler {
            doc: self,
            index: Some(index),
        })
    }
    pub(crate) fn materials(&self) -> impl ExactSizeIterator<Item = Material<'_>> + '_ {
        (0..self.tables.materials.len()).map(move |index| Material { doc: self, index })
    }
}
impl<'a> Sampler<'a> {
    pub(crate) fn index(&self) -> Option<usize> {
        self.index
    }
    pub(crate) fn name(&self) -> Option<&'a str> {
        self.index.and_then(|index| {
            self.doc.tables.samplers[index]
                .name
                .as_ref()
                .map(Text::as_str)
        })
    }
    pub(crate) fn mag_filter(&self) -> Option<MagFilter> {
        self.index
            .and_then(|index| self.doc.tables.samplers[index].mag_filter)
    }
    pub(crate) fn min_filter(&self) -> Option<MinFilter> {
        self.index
            .and_then(|index| self.doc.tables.samplers[index].min_filter)
    }
    pub(crate) fn wrap_s(&self) -> WrappingMode {
        self.index.map_or(WrappingMode::Repeat, |index| {
            self.doc.tables.samplers[index].wrap_s
        })
    }
    pub(crate) fn wrap_t(&self) -> WrappingMode {
        self.index.map_or(WrappingMode::Repeat, |index| {
            self.doc.tables.samplers[index].wrap_t
        })
    }
}
#[derive(Clone, Copy)]
pub(crate) struct Material<'a> {
    doc: &'a Document<'a>,
    index: usize,
}
impl<'a> Material<'a> {
    fn row(&self) -> &'a schema::Material<'a> {
        &self.doc.tables.materials[self.index]
    }
    pub(crate) fn index(&self) -> usize {
        self.index
    }
    pub(crate) fn name(&self) -> Option<&'a str> {
        self.row().name.as_ref().map(Text::as_str)
    }
    pub(crate) fn double_sided(&self) -> bool {
        self.row().double_sided
    }
}
impl<'a> Animation<'a> {
    pub(crate) fn name(&self) -> Option<&'a str> {
        self.doc.tables.animations[self.index]
            .name
            .as_ref()
            .map(Text::as_str)
    }
    pub(crate) fn channels(&self) -> impl ExactSizeIterator<Item = Channel<'a>> + 'a {
        let doc = self.doc;
        let animation = self.index;
        (0..doc.tables.animations[animation].channels.len()).map(move |index| Channel {
            doc,
            animation,
            index,
        })
    }
}
#[derive(Clone, Copy)]
pub(crate) struct Channel<'a> {
    doc: &'a Document<'a>,
    animation: usize,
    index: usize,
}
impl<'a> Channel<'a> {
    pub(crate) fn sampler(&self) -> AnimationSampler<'a> {
        AnimationSampler {
            doc: self.doc,
            row: &self.doc.tables.animations[self.animation].samplers[self.doc.tables.animations
                [self.animation]
                .channels[self.index]
                .sampler
                .value()],
        }
    }
}
pub(crate) struct AnimationSampler<'a> {
    doc: &'a Document<'a>,
    row: &'a schema::animation::Sampler,
}
impl<'a> AnimationSampler<'a> {
    pub(crate) fn input(&self) -> Accessor<'a> {
        Accessor {
            doc: self.doc,
            index: self.row.input.value(),
        }
    }
    pub(crate) fn output(&self) -> Accessor<'a> {
        Accessor {
            doc: self.doc,
            index: self.row.output.value(),
        }
    }
}
impl<'a> Camera<'a> {
    pub(crate) fn name(&self) -> Option<&'a str> {
        self.doc.tables.cameras[self.index]
            .name
            .as_ref()
            .map(Text::as_str)
    }
    pub(crate) fn projection(&self) -> schema::Projection {
        self.doc.tables.cameras[self.index].projection
    }
}

impl std::fmt::Debug for Channel<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Channel")
            .field(&self.animation)
            .field(&self.index)
            .finish()
    }
}
