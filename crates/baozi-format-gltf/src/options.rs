use crate::FORMAT_ID;
use baozi_import::authoring::FormatId;
use baozi_import::{
    CompiledFormatOptions, FormatOptionCodec, FormatOptionEnvelope, FormatOptionEnvelopeError,
    FormatOptionKey, FormatOptionLayers, FormatOptionPayload, FormatOptionReader,
    FormatOptionSchema, FormatOptionSchemaId, FormatOptionSchemaKey, OptionCompileError,
    OptionCompileScope, OptionRuntimeTypeKey, RegistryBuildScope,
};
use std::num::NonZeroU32;
use std::sync::Arc;

const PRESENCE_SCENE_SELECTION: u8 = 1 << 0;
const KNOWN_PRESENCE_BITS: u8 = PRESENCE_SCENE_SELECTION;
const SELECTION_DOCUMENT_DEFAULT: u8 = 0;
const SELECTION_SCENE_INDEX: u8 = 1;
const RETAINED_OPTION_BYTES: u64 = size_of::<GltfImportOptions>() as u64;

/// Scene selection applied to one glTF or GLB document.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum GltfSceneSelection {
    /// Selects the declared default, the sole scene, or parentless nodes if no scenes exist.
    ///
    /// Multiple scenes without a declared default require an explicit index.
    #[default]
    DocumentDefault,
    /// Selects the declared scene at this zero-based source index, overriding any default.
    ///
    /// An out-of-range index fails before dependency IO. In particular, a document
    /// with no declared scenes cannot be selected with index zero.
    SceneIndex(u32),
}

/// Immutable options consumed by one glTF parser invocation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct GltfImportOptions {
    scene_selection: GltfSceneSelection,
}

impl GltfImportOptions {
    pub const fn scene_selection(self) -> GltfSceneSelection {
        self.scene_selection
    }

    pub const fn with_scene_selection(mut self, selection: GltfSceneSelection) -> Self {
        self.scene_selection = selection;
        self
    }

    /// Encodes this complete option value into the version-one canonical envelope.
    ///
    /// The payload contains a presence byte followed by a selection tag. An explicit
    /// scene index adds four little-endian bytes, for a maximum of six payload bytes.
    /// `DocumentDefault` is present explicitly and therefore overrides a preset.
    pub fn to_envelope(self) -> Result<FormatOptionEnvelope, FormatOptionEnvelopeError> {
        let key = gltf_import_option_key();
        match self.scene_selection {
            GltfSceneSelection::DocumentDefault => key.envelope_from_canonical_payload([
                PRESENCE_SCENE_SELECTION,
                SELECTION_DOCUMENT_DEFAULT,
            ]),
            GltfSceneSelection::SceneIndex(index) => {
                let [a, b, c, d] = index.to_le_bytes();
                key.envelope_from_canonical_payload([
                    PRESENCE_SCENE_SELECTION,
                    SELECTION_SCENE_INDEX,
                    a,
                    b,
                    c,
                    d,
                ])
            }
        }
    }
}

/// Returns the stable serialized and process-local key for glTF import options.
pub fn gltf_import_option_key() -> FormatOptionKey<GltfImportOptions> {
    FormatOptionKey::new(option_schema())
}

pub(crate) fn option_codec() -> Arc<dyn FormatOptionCodec> {
    Arc::new(GltfOptionCodec)
}

fn option_schema() -> FormatOptionSchema {
    FormatOptionSchema::new(
        FormatOptionSchemaKey::new(
            FormatId::new(FORMAT_ID).expect("static glTF format id must be valid"),
            FormatOptionSchemaId::new("import-options")
                .expect("static glTF option schema id must be valid"),
        ),
        NonZeroU32::new(1).expect("glTF option schema version must be non-zero"),
    )
}

#[derive(Debug)]
struct GltfOptionCodec;

impl FormatOptionCodec for GltfOptionCodec {
    fn schema(&self) -> Option<FormatOptionSchema> {
        Some(option_schema())
    }

    fn runtime_type_key(&self) -> OptionRuntimeTypeKey {
        OptionRuntimeTypeKey::of::<GltfImportOptions>()
    }

    fn compile_defaults(
        &self,
        scope: &mut RegistryBuildScope<'_>,
    ) -> Result<CompiledFormatOptions, OptionCompileError> {
        scope.charge_work(1)?;
        scope.finish(GltfImportOptions::default(), RETAINED_OPTION_BYTES)
    }

    fn compile_layers(
        &self,
        scope: &mut OptionCompileScope<'_>,
        defaults: &CompiledFormatOptions,
        layers: FormatOptionLayers<'_>,
    ) -> Result<CompiledFormatOptions, OptionCompileError> {
        let key = gltf_import_option_key();
        let mut options = *defaults.downcast_ref(&key).ok_or_else(|| {
            OptionCompileError::invalid_payload(
                "default-type",
                "registry supplied an incompatible glTF option default",
            )
        })?;

        if let Some(payload) = layers.preset() {
            apply_layer(scope, payload, &mut options)?;
        }
        if let Some(payload) = layers.caller() {
            apply_layer(scope, payload, &mut options)?;
        }
        scope.finish(options, RETAINED_OPTION_BYTES)
    }
}

fn apply_layer(
    scope: &mut OptionCompileScope<'_>,
    payload: FormatOptionPayload<'_>,
    options: &mut GltfImportOptions,
) -> Result<(), OptionCompileError> {
    let mut reader = scope.reader(payload);
    let presence = read_byte(&mut reader)?;
    if presence & !KNOWN_PRESENCE_BITS != 0 {
        return Err(OptionCompileError::invalid_payload(
            "unknown-presence-bits",
            "glTF option payload contains unknown presence bits",
        ));
    }

    if presence & PRESENCE_SCENE_SELECTION != 0 {
        options.scene_selection = read_selection(&mut reader)?;
    }
    if reader.remaining() != 0 {
        let _trailing = reader.read_remaining()?;
        return Err(OptionCompileError::invalid_payload(
            "trailing-bytes",
            "glTF option payload contains trailing bytes",
        ));
    }
    Ok(())
}

fn read_byte(reader: &mut FormatOptionReader<'_, '_, '_>) -> Result<u8, OptionCompileError> {
    let [value] = reader.read_exact(1)? else {
        return Err(OptionCompileError::invalid_payload(
            "truncated",
            "glTF option payload is truncated",
        ));
    };
    Ok(*value)
}

fn read_selection(
    reader: &mut FormatOptionReader<'_, '_, '_>,
) -> Result<GltfSceneSelection, OptionCompileError> {
    match read_byte(reader)? {
        SELECTION_DOCUMENT_DEFAULT => Ok(GltfSceneSelection::DocumentDefault),
        SELECTION_SCENE_INDEX => {
            let [a, b, c, d] = reader.read_exact(4)? else {
                return Err(OptionCompileError::invalid_payload(
                    "truncated",
                    "glTF scene index is truncated",
                ));
            };
            Ok(GltfSceneSelection::SceneIndex(u32::from_le_bytes([
                *a, *b, *c, *d,
            ])))
        }
        _ => Err(OptionCompileError::invalid_payload(
            "unknown-scene-selection",
            "glTF option payload contains an unknown scene selection tag",
        )),
    }
}
