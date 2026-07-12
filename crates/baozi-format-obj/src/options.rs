use crate::FORMAT_ID;
use baozi_core::FormatId;
use baozi_import::{
    CompiledFormatOptions, FormatOptionCodec, FormatOptionEnvelope, FormatOptionEnvelopeError,
    FormatOptionKey, FormatOptionLayers, FormatOptionPayload, FormatOptionReader,
    FormatOptionSchema, FormatOptionSchemaId, FormatOptionSchemaKey, OptionCompileError,
    OptionCompileScope, OptionRuntimeTypeKey, RegistryBuildScope,
};
use std::num::NonZeroU32;
use std::sync::Arc;

const PRESENCE_TEXT_DECODING_MODE: u8 = 1 << 0;
const KNOWN_PRESENCE_BITS: u8 = PRESENCE_TEXT_DECODING_MODE;
const MODE_REJECT_INVALID: u8 = 0;
const MODE_REPLACE_INVALID_UTF8: u8 = 1;
const RETAINED_OPTION_BYTES: u64 = 1;

/// UTF-8 behavior selected for OBJ and its MTL dependencies.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum ObjTextDecodingMode {
    #[default]
    RejectInvalid,
    ReplaceInvalidUtf8,
}

/// Immutable options consumed by one OBJ parser invocation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct ObjImportOptions {
    text_decoding_mode: ObjTextDecodingMode,
}

impl ObjImportOptions {
    pub const fn text_decoding_mode(self) -> ObjTextDecodingMode {
        self.text_decoding_mode
    }

    pub const fn with_text_decoding_mode(mut self, mode: ObjTextDecodingMode) -> Self {
        self.text_decoding_mode = mode;
        self
    }

    /// Encodes this complete option value into the version-one canonical envelope.
    pub fn to_envelope(self) -> Result<FormatOptionEnvelope, FormatOptionEnvelopeError> {
        obj_import_option_key().envelope_from_canonical_payload([
            PRESENCE_TEXT_DECODING_MODE,
            encode_mode(self.text_decoding_mode),
        ])
    }
}

/// Returns the stable serialized and process-local key for OBJ import options.
pub fn obj_import_option_key() -> FormatOptionKey<ObjImportOptions> {
    FormatOptionKey::new(option_schema())
}

pub(crate) fn option_codec() -> Arc<dyn FormatOptionCodec> {
    Arc::new(ObjOptionCodec)
}

fn option_schema() -> FormatOptionSchema {
    FormatOptionSchema::new(
        FormatOptionSchemaKey::new(
            FormatId::new(FORMAT_ID).expect("static OBJ format id must be valid"),
            FormatOptionSchemaId::new("import-options")
                .expect("static OBJ option schema id must be valid"),
        ),
        NonZeroU32::new(1).expect("OBJ option schema version must be non-zero"),
    )
}

#[derive(Debug)]
struct ObjOptionCodec;

impl FormatOptionCodec for ObjOptionCodec {
    fn schema(&self) -> Option<FormatOptionSchema> {
        Some(option_schema())
    }

    fn runtime_type_key(&self) -> OptionRuntimeTypeKey {
        OptionRuntimeTypeKey::of::<ObjImportOptions>()
    }

    fn compile_defaults(
        &self,
        scope: &mut RegistryBuildScope<'_>,
    ) -> Result<CompiledFormatOptions, OptionCompileError> {
        scope.charge_work(1)?;
        scope.finish(ObjImportOptions::default(), RETAINED_OPTION_BYTES)
    }

    fn compile_layers(
        &self,
        scope: &mut OptionCompileScope<'_>,
        defaults: &CompiledFormatOptions,
        layers: FormatOptionLayers<'_>,
    ) -> Result<CompiledFormatOptions, OptionCompileError> {
        let key = obj_import_option_key();
        let mut options = *defaults.downcast_ref(&key).ok_or_else(|| {
            OptionCompileError::invalid_payload(
                "default-type",
                "registry supplied an incompatible OBJ option default",
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
    options: &mut ObjImportOptions,
) -> Result<(), OptionCompileError> {
    let mut reader = scope.reader(payload);
    let presence = read_byte(&mut reader)?;
    if presence & !KNOWN_PRESENCE_BITS != 0 {
        return Err(OptionCompileError::invalid_payload(
            "unknown-presence-bits",
            "OBJ option payload contains unknown presence bits",
        ));
    }

    if presence & PRESENCE_TEXT_DECODING_MODE != 0 {
        options.text_decoding_mode = decode_mode(read_byte(&mut reader)?)?;
    }
    if reader.remaining() != 0 {
        let _trailing = reader.read_remaining()?;
        return Err(OptionCompileError::invalid_payload(
            "trailing-bytes",
            "OBJ option payload contains trailing bytes",
        ));
    }
    Ok(())
}

fn read_byte(reader: &mut FormatOptionReader<'_, '_, '_>) -> Result<u8, OptionCompileError> {
    let [value] = reader.read_exact(1)? else {
        return Err(OptionCompileError::invalid_payload(
            "truncated",
            "OBJ option payload is truncated",
        ));
    };
    Ok(*value)
}

const fn encode_mode(mode: ObjTextDecodingMode) -> u8 {
    match mode {
        ObjTextDecodingMode::RejectInvalid => MODE_REJECT_INVALID,
        ObjTextDecodingMode::ReplaceInvalidUtf8 => MODE_REPLACE_INVALID_UTF8,
    }
}

fn decode_mode(value: u8) -> Result<ObjTextDecodingMode, OptionCompileError> {
    match value {
        MODE_REJECT_INVALID => Ok(ObjTextDecodingMode::RejectInvalid),
        MODE_REPLACE_INVALID_UTF8 => Ok(ObjTextDecodingMode::ReplaceInvalidUtf8),
        _ => Err(OptionCompileError::invalid_payload(
            "unknown-text-decoding-mode",
            "OBJ option payload contains an unknown text decoding mode",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_complete_payload_is_presence_then_mode() {
        assert_eq!(
            [
                PRESENCE_TEXT_DECODING_MODE,
                encode_mode(ObjTextDecodingMode::RejectInvalid)
            ],
            [1, 0]
        );
        assert_eq!(
            [
                PRESENCE_TEXT_DECODING_MODE,
                encode_mode(ObjTextDecodingMode::ReplaceInvalidUtf8)
            ],
            [1, 1]
        );
    }
}
