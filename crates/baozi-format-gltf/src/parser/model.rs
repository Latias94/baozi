//! Owned glTF meaning and session snapshot references after backend teardown.
//!
//! Every collection uses the import session's admitted storage. No decoder handle or
//! decoder-dependent borrow is retained by this model.

use super::{animation, lights, morph, source_profile, uri};
use baozi_import::authoring::*;
use baozi_import::{DependencySnapshot, MeteredString, MeteredVec, SourceScopeSelectionRequest};

#[allow(
    clippy::large_enum_variant,
    reason = "each buffer owns its admitted session storage directly, without another allocation per buffer"
)]
pub(super) enum BufferData<'scope> {
    Owned(MeteredVec<'scope, u8>),
    External {
        identity: uri::ProfileIdentity<'scope>,
        snapshot: DependencySnapshot<'scope>,
        declared_length: usize,
        provenance: u32,
    },
}

impl BufferData<'_> {
    pub(super) fn as_slice(&self) -> &[u8] {
        match self {
            Self::Owned(bytes) => bytes.as_slice(),
            Self::External {
                snapshot,
                declared_length,
                ..
            } => &snapshot.bytes()[..*declared_length],
        }
    }

    pub(super) fn len(&self) -> usize {
        self.as_slice().len()
    }

    pub(super) const fn provenance(&self) -> u32 {
        match self {
            Self::Owned(_) => 0,
            Self::External { provenance, .. } => *provenance,
        }
    }

    pub(super) fn external_identity(&self) -> Option<&ResourceIdentity> {
        match self {
            Self::Owned(_) => None,
            Self::External { identity, .. } => Some(identity.value()),
        }
    }
}

pub(super) struct BufferSet<'scope> {
    pub(super) values: MeteredVec<'scope, Option<BufferData<'scope>>>,
}

impl BufferSet<'_> {
    pub(super) fn as_slice(&self) -> &[Option<BufferData<'_>>] {
        self.values.as_slice()
    }

    pub(super) fn external_resource_count(&self) -> usize {
        self.values
            .as_slice()
            .iter()
            .filter_map(Option::as_ref)
            .filter(|buffer| buffer.external_identity().is_some())
            .count()
    }
}

/// Sorted unique sources from the base, sparse indices, and sparse values buffers.
/// Inline storage is admitted with the containing metered row, including root source zero.
#[derive(Clone, Copy, Debug)]
pub(super) struct AccessorSources {
    sources: [u32; Self::MAX_SOURCES],
    len: u8,
}

impl AccessorSources {
    pub(super) const MAX_SOURCES: usize = 3;

    /// Prepay buffer lookups and bounded collection/sort/dedup before entering decode work.
    pub(super) const COLLECTION_WORK_BOUND: u64 = 32;

    pub(super) const ROOT: Self = Self {
        sources: [0; Self::MAX_SOURCES],
        len: 1,
    };

    /// Callers prepay COLLECTION_WORK_BOUND; this never opens a nested work scope.
    pub(super) fn new(mut sources: [Option<u32>; Self::MAX_SOURCES]) -> Self {
        sources.sort_unstable();
        let mut provenance = Self {
            sources: [0; Self::MAX_SOURCES],
            len: 0,
        };
        let mut previous = None;
        for source in sources.into_iter().flatten() {
            if previous != Some(source) {
                provenance.sources[usize::from(provenance.len)] = source;
                provenance.len += 1;
                previous = Some(source);
            }
        }
        provenance
    }

    pub(super) fn as_slice(&self) -> &[u32] {
        &self.sources[..self.reference_count()]
    }

    pub(super) fn reference_count(&self) -> usize {
        usize::from(self.len)
    }
}

pub(super) struct NormalizedGltf<'scope> {
    pub(super) selection_request: SourceScopeSelectionRequest,
    pub(super) sources: source_profile::SelectedSourceProfile<'scope>,
    pub(super) appearance: AppearanceStorage<'scope>,
    pub(super) geometry: GeometryStorage<'scope>,
    pub(super) cameras: CameraStorage<'scope>,
    pub(super) lights: lights::LightStorage<'scope>,
    pub(super) nodes: NodeStorage<'scope>,
    pub(super) skins: SkinStorage<'scope>,
    pub(super) instance_deformations: MeteredVec<'scope, InstanceDeformationStorage<'scope>>,
    pub(super) buffers: BufferResources<'scope>,
    pub(super) external_images: ExternalImageResources<'scope>,
    pub(super) animations: animation::AnimationStorage<'scope>,
    pub(super) morphs: morph::MorphStorage<'scope>,
}

pub(super) struct BufferResource<'scope> {
    pub(super) source_index: usize,
    pub(super) identity: uri::ProfileIdentity<'scope>,
}

pub(super) struct BufferResources<'scope> {
    pub(super) values: MeteredVec<'scope, BufferResource<'scope>>,
}

impl BufferResources<'_> {
    pub(super) fn external_resource_count(&self) -> usize {
        self.values.len()
    }

    pub(super) fn external_resources(&self) -> impl Iterator<Item = (usize, &ResourceIdentity)> {
        self.values
            .as_slice()
            .iter()
            .rev()
            .map(|resource| (resource.source_index, resource.identity.value()))
    }
}

pub(super) struct ExternalImageResource<'scope> {
    pub(super) image_index: usize,
    pub(super) identity: uri::ProfileIdentity<'scope>,
    pub(super) snapshot: DependencySnapshot<'scope>,
}

pub(super) struct ExternalImageResources<'scope> {
    pub(super) values: MeteredVec<'scope, ExternalImageResource<'scope>>,
    pub(super) resource_offset: u32,
}

impl ExternalImageResources<'_> {
    pub(super) fn get(
        &self,
        ctx: &baozi_import::ParseScope<'_>,
        image_index: usize,
    ) -> super::Result<Option<(u32, &DependencySnapshot<'_>)>> {
        let values = self.values.as_slice();
        // Collection follows source image order, including gaps for unselected/embedded images.
        // Prepay the binary-search comparison bound; one search takes at most usize::BITS + 1.
        let comparisons = u64::from(usize::BITS - values.len().leading_zeros()) + 1;
        ctx.charge_work(comparisons)?;
        let Ok(index) = values.binary_search_by_key(&image_index, |resource| resource.image_index)
        else {
            return Ok(None);
        };
        let resource = &values[index];
        Ok(u32::try_from(index)
            .ok()
            .and_then(|index| index.checked_add(self.resource_offset))
            .map(|index| (index, &resource.snapshot)))
    }
}

pub(super) enum ImagePayloadStorage {
    Encoded(usize),
    SessionResource(ResourceIdentityLocalId),
}

pub(super) struct ImageStorage {
    pub(super) payload: ImagePayloadStorage,
    pub(super) provenance: u32,
    pub(super) media_type: Option<ExtensionMediaType>,
}

pub(super) struct BindingStorage {
    pub(super) texture: TextureLocalId,
    pub(super) provenance: u32,
    pub(super) role: TextureRole,
    pub(super) uv_set: u16,
    pub(super) channel_mapping: TextureChannelSwizzle,
    pub(super) color_interpretation: ColorInterpretation,
    pub(super) transform: TextureTransform,
    pub(super) strength: TextureStrength,
    pub(super) blend: TextureBlendOperation,
    pub(super) source_key: TextureSourceKey,
}

pub(super) struct MaterialStorage<'scope> {
    pub(super) shading_model: ShadingModel,
    pub(super) name: Option<MeteredString<'scope>>,
    pub(super) bindings: AuthoringRange,
    pub(super) alpha_mode: AlphaMode,
    pub(super) double_sided: bool,
    pub(super) factors: MaterialFactors,
}

pub(super) struct AppearanceStorage<'scope> {
    pub(super) images: MeteredVec<'scope, ImageStorage>,
    pub(super) image_names: MeteredVec<'scope, Option<MeteredString<'scope>>>,
    pub(super) encoded_image_payloads: MeteredVec<'scope, MeteredVec<'scope, u8>>,
    pub(super) samplers: MeteredVec<'scope, SamplerAuthoring>,
    pub(super) sampler_names: MeteredVec<'scope, Option<MeteredString<'scope>>>,
    pub(super) sampler_provenance: MeteredVec<'scope, u32>,
    pub(super) textures: MeteredVec<'scope, TextureAuthoring>,
    pub(super) texture_provenance: MeteredVec<'scope, u32>,
    pub(super) texture_names: MeteredVec<'scope, Option<MeteredString<'scope>>>,
    pub(super) bindings: MeteredVec<'scope, BindingStorage>,
    pub(super) materials: MeteredVec<'scope, MaterialStorage<'scope>>,
    pub(super) image_ids: MeteredVec<'scope, Option<ImageLocalId>>,
    pub(super) sampler_ids: MeteredVec<'scope, Option<SamplerLocalId>>,
    pub(super) texture_ids: MeteredVec<'scope, Option<TextureLocalId>>,
    pub(super) default_sampler: Option<SamplerLocalId>,
    pub(super) default_material: Option<MaterialLocalId>,
    pub(super) material_ids: MeteredVec<'scope, Option<MaterialLocalId>>,
}

pub(super) struct AttributeStorage<'scope> {
    pub(super) semantic: AttributeSemantic,
    pub(super) lanes: u8,
    pub(super) values: MeteredVec<'scope, f32>,
    pub(super) provenance: AccessorSources,
}

pub(super) struct PrimitiveStorage<'scope> {
    pub(super) topology: GeometryTopologyAuthoring<'static>,
    pub(super) has_indices: bool,
    pub(super) indices: MeteredVec<'scope, u32>,
    pub(super) material_assignment: MaterialAssignment<MaterialLocalId>,
    pub(super) attributes: MeteredVec<'scope, AttributeStorage<'scope>>,
    pub(super) influence_offsets: MeteredVec<'scope, u32>,
    pub(super) influences: MeteredVec<'scope, InfluenceAuthoring>,
    pub(super) provenance_offset: usize,
    pub(super) provenance_count: usize,
}

pub(super) struct LogicalMeshStorage<'scope> {
    pub(super) name: Option<MeteredString<'scope>>,
    pub(super) primitives: MeteredVec<'scope, PrimitiveStorage<'scope>>,
}

pub(super) struct GeometryStorage<'scope> {
    pub(super) logical_meshes: MeteredVec<'scope, LogicalMeshStorage<'scope>>,
    pub(super) logical_mesh_ids: MeteredVec<'scope, Option<LogicalMeshLocalId>>,
    pub(super) primitive_provenance: MeteredVec<'scope, u32>,
}

pub(super) struct MeshInstanceStorage {
    pub(super) authored: GeometryMeshInstanceAuthoring,
    pub(super) skin_index: Option<usize>,
    pub(super) source_node_index: usize,
}

pub(super) struct NodeStorage<'scope> {
    pub(super) roots: MeteredVec<'scope, NodeLocalId>,
    pub(super) nodes: MeteredVec<'scope, NodeAuthoringNode<'static>>,
    pub(super) names: MeteredVec<'scope, Option<MeteredString<'scope>>>,
    pub(super) node_ids: MeteredVec<'scope, Option<NodeLocalId>>,
    pub(super) mesh_instances: MeteredVec<'scope, MeshInstanceStorage>,
}

pub(super) struct SkinStorageRow<'scope> {
    pub(super) name: Option<MeteredString<'scope>>,
    pub(super) joints: MeteredVec<'scope, NodeLocalId>,
    pub(super) inverse_bind_matrices: MeteredVec<'scope, Mat4>,
    pub(super) skeleton_root: Option<NodeLocalId>,
    pub(super) provenance: AccessorSources,
}

pub(super) struct SkinStorage<'scope> {
    pub(super) skins: MeteredVec<'scope, SkinStorageRow<'scope>>,
    pub(super) skin_ids: MeteredVec<'scope, Option<SkinLocalId>>,
}

pub(super) struct InstanceDeformationStorage<'scope> {
    pub(super) skin: Presence<SkinLocalId>,
    pub(super) weights: Option<MeteredVec<'scope, f32>>,
}

pub(super) struct CameraStorage<'scope> {
    pub(super) authored: MeteredVec<'scope, CameraProjection>,
    pub(super) names: MeteredVec<'scope, Option<MeteredString<'scope>>>,
    pub(super) camera_ids: MeteredVec<'scope, Option<CameraLocalId>>,
}
