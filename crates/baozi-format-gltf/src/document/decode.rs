use super::schema::*;
use crate::GltfFailure;
use crate::json_syntax::{DecodedScalars, IndexedValue, JsonIndex, ValueId, ValueKind};
use baozi_import::{ImportStorageClass, ParseResult, ParseScope};

pub(super) struct Decoder<'a, 'scope, 'session> {
    pub(super) ctx: &'scope ParseScope<'session>,
    pub(super) bytes: &'scope [u8],
    pub(super) index: &'a JsonIndex<'scope>,
}

impl<'a, 'scope, 'session> Decoder<'a, 'scope, 'session> {
    fn invalid(&self, reason: &'static str) -> baozi_import::ParseFailure {
        self.ctx
            .format_failure(GltfFailure::invalid_document(reason))
    }

    fn value(&self, id: ValueId) -> &IndexedValue {
        &self.index.values.as_slice()[id]
    }

    fn kind(&self, id: ValueId, kind: ValueKind, reason: &'static str) -> ParseResult<()> {
        if self.value(id).kind == kind {
            Ok(())
        } else {
            Err(self.invalid(reason))
        }
    }

    pub(super) fn object(&self, id: ValueId) -> ParseResult<()> {
        self.kind(
            id,
            ValueKind::Object,
            "glTF schema field requires an object",
        )
    }

    fn key_eq(&self, id: ValueId, key: &str) -> ParseResult<bool> {
        let value = self.value(id);
        self.ctx
            .charge_work((value.key_end - value.key_start) as u64 + 1)?;
        let mut scalars = DecodedScalars::new(self.bytes, value.key_start + 1, value.key_end - 1);
        for expected in key.chars() {
            if scalars
                .next_scalar()
                .map_err(|()| self.invalid("invalid admitted key"))?
                != Some(u32::from(expected))
            {
                return Ok(false);
            }
        }
        Ok(scalars
            .next_scalar()
            .map_err(|()| self.invalid("invalid admitted key"))?
            .is_none())
    }

    pub(super) fn field(&self, object: ValueId, key: &str) -> ParseResult<Option<ValueId>> {
        self.object(object)?;
        for id in self.children(object) {
            if self.key_eq(id, key)? {
                return Ok(Some(id));
            }
        }
        Ok(None)
    }

    fn required(&self, object: ValueId, key: &'static str) -> ParseResult<ValueId> {
        self.field(object, key)?.ok_or_else(|| self.invalid(key))
    }

    fn children(&self, id: ValueId) -> impl Iterator<Item = ValueId> + '_ {
        std::iter::successors(self.value(id).first_child, |&id| {
            self.value(id).next_sibling
        })
    }

    fn raw_str(&self, id: ValueId) -> ParseResult<&'scope str> {
        let value = self.value(id);
        std::str::from_utf8(&self.bytes[value.start..value.end])
            .map_err(|_| self.invalid("invalid admitted UTF-8"))
    }

    fn text_range(&self, start: usize, end: usize) -> ParseResult<Text<'scope>> {
        let bytes = &self.bytes[start..end];
        let mut escaped = false;
        for chunk in bytes.chunks(4096) {
            self.ctx.charge_work(chunk.len() as u64)?;
            escaped |= chunk.contains(&b'\\');
        }
        if !escaped {
            return Ok(Text::Borrowed(
                std::str::from_utf8(bytes).map_err(|_| self.invalid("invalid admitted string"))?,
            ));
        }
        let mut scalars = DecodedScalars::new(self.bytes, start, end);
        let mut decoded_len = 0usize;
        let mut count = 0usize;
        while let Some(scalar) = scalars
            .next_scalar()
            .map_err(|()| self.invalid("invalid admitted escape"))?
        {
            if count.is_multiple_of(256) {
                self.ctx.charge_work(1)?;
            }
            let scalar =
                char::from_u32(scalar).ok_or_else(|| self.invalid("invalid admitted scalar"))?;
            decoded_len = decoded_len
                .checked_add(scalar.len_utf8())
                .ok_or_else(|| self.ctx.allocation_failure("gltf-document-escaped-string"))?;
            count += 1;
        }
        // Admit the decoded length before the first owned byte, including empty strings.
        let mut decoded = self.ctx.metered_string(
            ImportStorageClass::BackendDecode,
            "gltf-document-escaped-string",
        );
        decoded.try_reserve(decoded_len)?;
        let mut scalars = DecodedScalars::new(self.bytes, start, end);
        let mut count = 0usize;
        while let Some(scalar) = scalars
            .next_scalar()
            .map_err(|()| self.invalid("invalid admitted escape"))?
        {
            if count.is_multiple_of(256) {
                self.ctx.charge_work(1)?;
            }
            let scalar =
                char::from_u32(scalar).ok_or_else(|| self.invalid("invalid admitted scalar"))?;
            decoded.try_push_str(scalar.encode_utf8(&mut [0; 4]))?;
            count += 1;
        }
        Ok(Text::Decoded(decoded))
    }

    fn text(&self, id: ValueId) -> ParseResult<Text<'scope>> {
        self.kind(id, ValueKind::String, "glTF schema field requires a string")?;
        let value = self.value(id);
        self.text_range(value.start + 1, value.end - 1)
    }

    fn name(&self, id: ValueId) -> ParseResult<Option<Text<'scope>>> {
        self.field(id, "name")?.map(|id| self.text(id)).transpose()
    }

    pub(super) fn uint(&self, id: ValueId) -> ParseResult<u64> {
        self.kind(
            id,
            ValueKind::Number,
            "glTF schema field requires an unsigned integer",
        )?;
        let value = self.value(id);
        let bytes = self
            .bytes
            .get(value.start..value.end)
            .ok_or_else(|| self.invalid("invalid admitted number range"))?;
        super::integer::unsigned(self.ctx, bytes)
    }

    fn size(&self, id: ValueId) -> ParseResult<u64> {
        let value = self.uint(id)?;
        usize::try_from(value).map_err(|_| {
            self.invalid("glTF byte length, count or offset exceeds the host index domain")
        })?;
        Ok(value)
    }

    fn id(&self, id: ValueId) -> ParseResult<Index> {
        let value = self.uint(id)?;
        Ok(Index(u32::try_from(value).map_err(|_| {
            self.invalid("glTF reference exceeds the U32 domain")
        })?))
    }

    fn optional_id(&self, object: ValueId, key: &str) -> ParseResult<Option<Index>> {
        self.field(object, key)?.map(|id| self.id(id)).transpose()
    }

    fn required_id(&self, object: ValueId, key: &'static str) -> ParseResult<Index> {
        self.id(self.required(object, key)?)
    }

    fn number(&self, id: ValueId) -> ParseResult<f64> {
        self.kind(id, ValueKind::Number, "glTF schema field requires a number")?;
        let text = self.raw_str(id)?;
        self.ctx.charge_work(text.len() as u64)?;
        let value = text
            .parse::<f64>()
            .map_err(|_| self.invalid("glTF number is invalid"))?;
        if !value.is_finite() {
            return Err(self.invalid("glTF number exceeds the JSON number domain"));
        }
        Ok(value)
    }

    fn float(&self, id: ValueId) -> ParseResult<f32> {
        Ok(self.number(id)? as f32)
    }

    fn float_or(&self, object: ValueId, key: &str, default: f32) -> ParseResult<f32> {
        self.field(object, key)?
            .map_or(Ok(default), |id| self.float(id))
    }

    fn boolean_or(&self, object: ValueId, key: &str, default: bool) -> ParseResult<bool> {
        let Some(id) = self.field(object, key)? else {
            return Ok(default);
        };
        self.kind(id, ValueKind::Bool, "glTF schema field requires a boolean")?;
        Ok(self.raw_str(id)? == "true")
    }

    fn fixed<const N: usize>(&self, id: ValueId) -> ParseResult<[f32; N]> {
        self.kind(id, ValueKind::Array, "glTF fixed vector requires an array")?;
        if self.value(id).child_count != N {
            return Err(self.invalid("glTF fixed vector has the wrong length"));
        }
        let mut result = [0.; N];
        for (value, id) in result.iter_mut().zip(self.children(id)) {
            *value = self.float(id)?;
        }
        Ok(result)
    }

    fn fixed_default<const N: usize>(
        &self,
        object: ValueId,
        key: &str,
        default: [f32; N],
    ) -> ParseResult<[f32; N]> {
        self.field(object, key)?
            .map_or(Ok(default), |id| self.fixed(id))
    }

    fn list<T>(
        &self,
        id: Option<ValueId>,
        decode: impl Fn(ValueId) -> ParseResult<T>,
    ) -> ParseResult<List<'scope, T>> {
        let mut list = self
            .ctx
            .metered_vec(ImportStorageClass::BackendDecode, "gltf-document-table");
        if let Some(id) = id {
            self.kind(id, ValueKind::Array, "glTF schema field requires an array")?;
            let count = self.value(id).child_count;
            u32::try_from(count).map_err(|_| self.invalid("glTF table exceeds the U32 domain"))?;
            list.try_reserve(count)?;
            for id in self.children(id) {
                self.ctx.charge_work(1)?;
                list.try_push_reserved(decode(id)?)?;
            }
        }
        Ok(List(list))
    }

    fn weights(&self, id: ValueId) -> ParseResult<Option<List<'scope, f32>>> {
        self.field(id, "weights")?
            .map(|id| self.list(Some(id), |id| self.float(id)))
            .transpose()
    }

    fn enum_text<T>(&self, id: ValueId, choices: &[(&str, T)]) -> ParseResult<T>
    where
        T: Copy,
    {
        let text = self.text(id)?;
        choices
            .iter()
            .find(|(key, _)| *key == text.as_str())
            .map(|(_, value)| *value)
            .ok_or_else(|| self.invalid("glTF schema string enum is invalid"))
    }

    fn enum_uint<T: Copy>(&self, id: ValueId, choices: &[(u64, T)]) -> ParseResult<T> {
        let number = self.uint(id)?;
        choices
            .iter()
            .find(|(key, _)| *key == number)
            .map(|(_, value)| *value)
            .ok_or_else(|| self.invalid("glTF schema numeric enum is invalid"))
    }

    fn component(&self, id: ValueId) -> ParseResult<accessor::ComponentType> {
        use accessor::ComponentType::*;
        self.enum_uint(
            id,
            &[
                (5120, I8),
                (5121, U8),
                (5122, I16),
                (5123, U16),
                (5125, U32),
                (5126, F32),
            ],
        )
    }

    fn bounds(&self, object: ValueId, key: &str) -> ParseResult<Option<accessor::Bounds>> {
        let Some(id) = self.field(object, key)? else {
            return Ok(None);
        };
        self.kind(
            id,
            ValueKind::Array,
            "glTF accessor bounds require an array",
        )?;
        let mut bounds = accessor::Bounds {
            values: [0.; 16],
            len: 0,
        };
        if self.value(id).child_count <= 16 {
            bounds.len = self.value(id).child_count;
            for (slot, child) in bounds.values.iter_mut().zip(self.children(id)) {
                if self.value(child).kind != ValueKind::Number {
                    bounds.len = 0;
                    break;
                }
                *slot = self.number(child)?;
            }
        }
        Ok(Some(bounds))
    }

    fn accessor(&self, id: ValueId) -> ParseResult<accessor::Accessor> {
        use accessor::Type::*;
        self.object(id)?;
        let _name = self.name(id)?;
        Ok(accessor::Accessor {
            buffer_view: self.optional_id(id, "bufferView")?,
            byte_offset: self
                .field(id, "byteOffset")?
                .map(|id| self.size(id))
                .transpose()?,
            count: self.size(self.required(id, "count")?)?,
            component_type: self.component(self.required(id, "componentType")?)?,
            type_: self.enum_text(
                self.required(id, "type")?,
                &[
                    ("SCALAR", Scalar),
                    ("VEC2", Vec2),
                    ("VEC3", Vec3),
                    ("VEC4", Vec4),
                    ("MAT2", Mat2),
                    ("MAT3", Mat3),
                    ("MAT4", Mat4),
                ],
            )?,
            normalized: self.boolean_or(id, "normalized", false)?,
            sparse: self
                .field(id, "sparse")?
                .map(|id| self.sparse(id))
                .transpose()?,
            min: self.bounds(id, "min")?,
            max: self.bounds(id, "max")?,
        })
    }

    fn sparse(&self, id: ValueId) -> ParseResult<accessor::Sparse> {
        let indices = self.required(id, "indices")?;
        let values = self.required(id, "values")?;
        let component_type = self.component(self.required(indices, "componentType")?)?;
        if !matches!(
            component_type,
            accessor::ComponentType::U8
                | accessor::ComponentType::U16
                | accessor::ComponentType::U32
        ) {
            return Err(self.invalid("glTF sparse indices must use unsigned integer components"));
        }
        Ok(accessor::Sparse {
            count: self.size(self.required(id, "count")?)?,
            indices: accessor::Indices {
                buffer_view: self.required_id(indices, "bufferView")?,
                byte_offset: self
                    .field(indices, "byteOffset")?
                    .map_or(Ok(0), |id| self.size(id))?,
                component_type,
            },
            values: accessor::Values {
                buffer_view: self.required_id(values, "bufferView")?,
                byte_offset: self
                    .field(values, "byteOffset")?
                    .map_or(Ok(0), |id| self.size(id))?,
            },
        })
    }

    fn semantic(&self, child: ValueId, ordinal: usize) -> ParseResult<mesh::Semantic> {
        use mesh::Semantic::*;
        let value = self.value(child);
        let key = self.text_range(value.key_start + 1, value.key_end - 1)?;
        Ok(match key.as_str() {
            "POSITION" => Positions,
            "NORMAL" => Normals,
            "TANGENT" => Tangents,
            key if key.starts_with('_') => Extras(
                u32::try_from(ordinal).map_err(|_| self.invalid("too many custom semantics"))?,
            ),
            key => {
                let (name, set) = key
                    .rsplit_once('_')
                    .ok_or_else(|| self.invalid("glTF attribute semantic is invalid"))?;
                let set = set
                    .parse::<u32>()
                    .map_err(|_| self.invalid("glTF attribute semantic set is invalid"))?;
                match name {
                    "COLOR" => Colors(set),
                    "TEXCOORD" => TexCoords(set),
                    "JOINTS" => Joints(set),
                    "WEIGHTS" => Weights(set),
                    _ => return Err(self.invalid("glTF attribute semantic is invalid")),
                }
            }
        })
    }

    fn primitive(&self, id: ValueId) -> ParseResult<mesh::Primitive<'scope>> {
        use mesh::Mode::*;
        let attributes_id = self.required(id, "attributes")?;
        self.object(attributes_id)?;
        let mut attributes = self.ctx.metered_vec(
            ImportStorageClass::BackendDecode,
            "gltf-document-attributes",
        );
        attributes.try_reserve(self.value(attributes_id).child_count)?;
        for (ordinal, child) in self.children(attributes_id).enumerate() {
            self.ctx.charge_work(1)?;
            attributes.try_push_reserved((self.semantic(child, ordinal)?, self.id(child)?))?;
        }
        attributes.try_sort_unstable_by(|a, b| -> ParseResult<_> {
            self.ctx.charge_work(1)?;
            Ok(a.0.cmp(&b.0))
        })?;
        if attributes
            .as_slice()
            .windows(2)
            .any(|pair| pair[0].0 == pair[1].0)
        {
            return Err(self.invalid("duplicate glTF attribute semantic"));
        }
        Ok(mesh::Primitive {
            attributes: mesh::Attributes(List(attributes)),
            indices: self.optional_id(id, "indices")?,
            material: self.optional_id(id, "material")?,
            mode: self.field(id, "mode")?.map_or(Ok(Triangles), |id| {
                self.enum_uint(
                    id,
                    &[
                        (0, Points),
                        (1, Lines),
                        (2, LineLoop),
                        (3, LineStrip),
                        (4, Triangles),
                        (5, TriangleStrip),
                        (6, TriangleFan),
                    ],
                )
            })?,
            targets: self
                .field(id, "targets")?
                .map(|id| self.list(Some(id), |id| self.morph_target(id)))
                .transpose()?,
        })
    }

    fn morph_target(&self, id: ValueId) -> ParseResult<mesh::MorphTarget<'scope>> {
        self.object(id)?;
        let mut unknown = self.ctx.metered_vec(
            ImportStorageClass::BackendDecode,
            "gltf-document-morph-semantics",
        );
        let mut positions = None;
        let mut normals = None;
        let mut tangents = None;
        for child in self.children(id) {
            let value = self.value(child);
            let key = self.text_range(value.key_start + 1, value.key_end - 1)?;
            let slot = match key.as_str() {
                "POSITION" => &mut positions,
                "NORMAL" => &mut normals,
                "TANGENT" => &mut tangents,
                _ => {
                    unknown.try_push((key, self.id(child)?))?;
                    continue;
                }
            };
            *slot = Some(self.id(child)?);
        }
        Ok(mesh::MorphTarget {
            positions,
            normals,
            tangents,
            other_attributes: mesh::UnknownAttributes(List(unknown)),
        })
    }

    fn texture_info(&self, id: ValueId, strength_key: Option<&str>) -> ParseResult<texture::Info> {
        let transform = self
            .field(id, "extensions")?
            .map(|id| self.field(id, "KHR_texture_transform"))
            .transpose()?
            .flatten();
        Ok(texture::Info {
            index: self.required_id(id, "index")?,
            tex_coord: self
                .field(id, "texCoord")?
                .map_or(Ok(0), |id| self.id(id).map(|id| id.0))?,
            strength: strength_key.map_or(Ok(1.), |key| self.float_or(id, key, 1.))?,
            transform,
        })
    }

    fn pbr(&self, id: Option<ValueId>) -> ParseResult<Pbr> {
        let Some(id) = id else {
            return Ok(Pbr {
                base_color_factor: [1.; 4],
                metallic_factor: 1.,
                roughness_factor: 1.,
                base_color_texture: None,
                metallic_roughness_texture: None,
            });
        };
        Ok(Pbr {
            base_color_factor: self.fixed_default(id, "baseColorFactor", [1.; 4])?,
            metallic_factor: self.float_or(id, "metallicFactor", 1.)?,
            roughness_factor: self.float_or(id, "roughnessFactor", 1.)?,
            base_color_texture: self
                .field(id, "baseColorTexture")?
                .map(|id| self.texture_info(id, None))
                .transpose()?,
            metallic_roughness_texture: self
                .field(id, "metallicRoughnessTexture")?
                .map(|id| self.texture_info(id, None))
                .transpose()?,
        })
    }

    fn material(&self, id: ValueId) -> ParseResult<Material<'scope>> {
        Ok(Material {
            name: self.name(id)?,
            pbr_metallic_roughness: self.pbr(self.field(id, "pbrMetallicRoughness")?)?,
            emissive_factor: self.fixed_default(id, "emissiveFactor", [0.; 3])?,
            alpha_mode: self
                .field(id, "alphaMode")?
                .map_or(Ok(AlphaMode::Opaque), |id| {
                    self.enum_text(
                        id,
                        &[
                            ("OPAQUE", AlphaMode::Opaque),
                            ("MASK", AlphaMode::Mask),
                            ("BLEND", AlphaMode::Blend),
                        ],
                    )
                })?,
            alpha_cutoff: self
                .field(id, "alphaCutoff")?
                .map(|id| self.float(id))
                .transpose()?,
            double_sided: self.boolean_or(id, "doubleSided", false)?,
            normal_texture: self
                .field(id, "normalTexture")?
                .map(|id| self.texture_info(id, Some("scale")))
                .transpose()?,
            occlusion_texture: self
                .field(id, "occlusionTexture")?
                .map(|id| self.texture_info(id, Some("strength")))
                .transpose()?,
            emissive_texture: self
                .field(id, "emissiveTexture")?
                .map(|id| self.texture_info(id, None))
                .transpose()?,
            unlit: self
                .field(id, "extensions")?
                .map(|id| self.field(id, "KHR_materials_unlit"))
                .transpose()?
                .flatten(),
            emissive_strength: self
                .field(id, "extensions")?
                .map(|id| self.field(id, super::KHR_MATERIALS_EMISSIVE_STRENGTH))
                .transpose()?
                .flatten(),
            clearcoat: self
                .field(id, "extensions")?
                .map(|id| self.field(id, super::KHR_MATERIALS_CLEARCOAT))
                .transpose()?
                .flatten(),
            sheen: self
                .field(id, "extensions")?
                .map(|id| self.field(id, super::KHR_MATERIALS_SHEEN))
                .transpose()?
                .flatten(),
        })
    }

    fn camera(&self, id: ValueId) -> ParseResult<Camera<'scope>> {
        let orthographic = self
            .field(id, "orthographic")?
            .map(|data| -> ParseResult<_> {
                Ok(Projection::Orthographic {
                    xmag: self.float(self.required(data, "xmag")?)?,
                    ymag: self.float(self.required(data, "ymag")?)?,
                    znear: self.float(self.required(data, "znear")?)?,
                    zfar: self.float(self.required(data, "zfar")?)?,
                })
            })
            .transpose()?;
        let perspective = self
            .field(id, "perspective")?
            .map(|data| -> ParseResult<_> {
                Ok(Projection::Perspective {
                    yfov: self.float(self.required(data, "yfov")?)?,
                    znear: self.float(self.required(data, "znear")?)?,
                    aspect_ratio: self
                        .field(data, "aspectRatio")?
                        .map(|id| self.float(id))
                        .transpose()?,
                    zfar: self
                        .field(data, "zfar")?
                        .map(|id| self.float(id))
                        .transpose()?,
                })
            })
            .transpose()?;
        let kind = self.text(self.required(id, "type")?)?;
        let projection = match kind.as_str() {
            "orthographic" => orthographic,
            "perspective" => perspective,
            _ => return Err(self.invalid("glTF camera type is invalid")),
        }
        .ok_or_else(|| {
            self.invalid("glTF camera discriminator requires its matching projection object")
        })?;
        Ok(Camera {
            name: self.name(id)?,
            projection,
        })
    }

    pub(super) fn emissive_strength(&self, id: ValueId) -> ParseResult<f32> {
        self.object(id)?;
        let Some(id) = self.field(id, "emissiveStrength")? else {
            return Ok(1.0);
        };
        let value = self.number(id)?;
        if value < 0.0 {
            return Err(self.invalid("glTF emissiveStrength must be nonnegative"));
        }
        let narrowed = value as f32;
        if !narrowed.is_finite() || (value != 0.0 && narrowed == 0.0) {
            return Err(self.invalid("glTF emissiveStrength exceeds the canonical F32 domain"));
        }
        // A nonzero JSON mantissa may already have underflowed while parsing f64.
        if value == 0.0 {
            let text = self.raw_str(id)?;
            self.ctx.charge_work(text.len() as u64)?;
            if text
                .bytes()
                .take_while(|&byte| !matches!(byte, b'e' | b'E'))
                .any(|byte| matches!(byte, b'1'..=b'9'))
            {
                return Err(
                    self.invalid("glTF emissiveStrength underflows the canonical F32 domain")
                );
            }
        }
        Ok(narrowed)
    }

    fn material_extension_float(&self, id: ValueId, unit: bool) -> ParseResult<f32> {
        let value = self.number(id)?;
        if unit && !(0.0..=1.0).contains(&value) {
            return Err(self.invalid("glTF material extension factor must be in [0,1]"));
        }
        let narrowed = value as f32;
        if !narrowed.is_finite() || (value != 0.0 && narrowed == 0.0) {
            return Err(
                self.invalid("glTF material extension number exceeds the canonical F32 domain")
            );
        }
        if value == 0.0 {
            let token = self.value(id);
            let mut mantissa = true;
            for chunk in self.bytes[token.start..token.end].chunks(4096) {
                self.ctx.charge_work(chunk.len() as u64)?;
                for &byte in chunk {
                    if matches!(byte, b'e' | b'E') {
                        mantissa = false;
                    }
                    if mantissa && matches!(byte, b'1'..=b'9') {
                        return Err(self.invalid(
                            "glTF material extension number underflows the canonical F32 domain",
                        ));
                    }
                }
            }
        }
        Ok(narrowed)
    }

    fn material_unit_or(&self, object: ValueId, key: &str, default: f32) -> ParseResult<f32> {
        self.field(object, key)?
            .map_or(Ok(default), |id| self.material_extension_float(id, true))
    }

    fn material_extension_texture(
        &self,
        object: ValueId,
        key: &str,
        normal: bool,
    ) -> ParseResult<Option<texture::Info>> {
        self.field(object, key)?
            .map(|id| {
                let mut info = self.texture_info(id, None)?;
                if normal {
                    info.strength = self
                        .field(id, "scale")?
                        .map_or(Ok(1.0), |id| self.material_extension_float(id, false))?;
                }
                Ok(info)
            })
            .transpose()
    }

    pub(super) fn clearcoat(&self, id: ValueId) -> ParseResult<Clearcoat> {
        self.object(id)?;
        Ok(Clearcoat {
            factor: self.material_unit_or(id, "clearcoatFactor", 0.0)?,
            roughness: self.material_unit_or(id, "clearcoatRoughnessFactor", 0.0)?,
            texture: self.material_extension_texture(id, "clearcoatTexture", false)?,
            roughness_texture: self.material_extension_texture(
                id,
                "clearcoatRoughnessTexture",
                false,
            )?,
            normal_texture: self.material_extension_texture(id, "clearcoatNormalTexture", true)?,
        })
    }

    pub(super) fn sheen(&self, id: ValueId) -> ParseResult<Sheen> {
        self.object(id)?;
        let mut color = [0.0; 3];
        if let Some(id) = self.field(id, "sheenColorFactor")? {
            self.kind(
                id,
                ValueKind::Array,
                "glTF sheenColorFactor requires an array",
            )?;
            if self.value(id).child_count != 3 {
                return Err(self.invalid("glTF sheenColorFactor requires three components"));
            }
            for (lane, id) in color.iter_mut().zip(self.children(id)) {
                *lane = self.material_extension_float(id, true)?;
            }
        }
        Ok(Sheen {
            color,
            roughness: self.material_unit_or(id, "sheenRoughnessFactor", 0.0)?,
            color_texture: self.material_extension_texture(id, "sheenColorTexture", false)?,
            roughness_texture: self.material_extension_texture(
                id,
                "sheenRoughnessTexture",
                false,
            )?,
        })
    }

    pub(super) fn transform(
        &self,
        id: ValueId,
    ) -> ParseResult<extensions::texture::TextureTransform> {
        self.object(id)?;
        Ok(extensions::texture::TextureTransform {
            offset: self.fixed_default(id, "offset", [0.; 2])?,
            rotation: self.float_or(id, "rotation", 0.)?,
            scale: self.fixed_default(id, "scale", [1.; 2])?,
            tex_coord: self.optional_id(id, "texCoord")?.map(|id| id.0),
        })
    }

    pub(super) fn punctual_light_reference(&self, id: ValueId) -> ParseResult<Index> {
        self.required_id(id, "light")
    }

    fn light_number(&self, id: ValueId) -> ParseResult<f64> {
        let value = self.number(id)?;
        if value == 0.0 {
            let text = self.raw_str(id)?;
            self.ctx.charge_work(text.len() as u64)?;
            // Source signs and nonzero values must not disappear during binary64 parsing.
            if text
                .bytes()
                .take_while(|&byte| !matches!(byte, b'e' | b'E'))
                .any(|byte| matches!(byte, b'1'..=b'9'))
            {
                return Err(self.invalid("glTF light number underflows the binary64 source domain"));
            }
        }
        Ok(value)
    }

    pub(super) fn light(&self, id: ValueId) -> ParseResult<Light<'scope>> {
        self.object(id)?;
        let mut color = [1.0; 3];
        if let Some(id) = self.field(id, "color")? {
            self.kind(id, ValueKind::Array, "glTF light color requires an array")?;
            if self.value(id).child_count != 3 {
                return Err(self.invalid("glTF light color requires three components"));
            }
            for (component, id) in color.iter_mut().zip(self.children(id)) {
                *component = self.light_number(id)?;
            }
        }
        let spot = self
            .field(id, "spot")?
            .map(|id| -> ParseResult<LightSpot> {
                self.object(id)?;
                Ok(LightSpot {
                    inner_cone_angle: self
                        .field(id, "innerConeAngle")?
                        .map_or(Ok(0.0), |id| self.light_number(id))?,
                    outer_cone_angle: self
                        .field(id, "outerConeAngle")?
                        .map_or(Ok(std::f64::consts::FRAC_PI_4), |id| self.light_number(id))?,
                })
            })
            .transpose()?;
        Ok(Light {
            name: self
                .field(id, "name")?
                .map(|id| self.text(id))
                .transpose()?,
            kind: self.text(self.required(id, "type")?)?,
            color,
            intensity: self
                .field(id, "intensity")?
                .map_or(Ok(1.0), |id| self.light_number(id))?,
            range: self
                .field(id, "range")?
                .map(|id| self.light_number(id))
                .transpose()?,
            spot,
        })
    }

    fn lights(&self) -> ParseResult<List<'scope, ValueId>> {
        let payload = self
            .field(0, "extensions")?
            .map(|id| self.field(id, super::KHR_LIGHTS_PUNCTUAL))
            .transpose()?
            .flatten();
        let lights = payload.map(|id| self.required(id, "lights")).transpose()?;
        if lights.is_some_and(|id| {
            self.value(id).kind == ValueKind::Array && self.value(id).child_count == 0
        }) {
            return Err(self.invalid("glTF punctual lights requires a nonempty lights array"));
        }
        self.list(lights, |id| {
            self.object(id)?;
            Ok(id)
        })
    }

    pub(super) fn root(&self) -> ParseResult<Tables<'scope>> {
        self.object(0)?;
        self.version()?;
        Ok(Tables {
            scene: self.optional_id(0, "scene")?,
            scenes: self.list(self.field(0, "scenes")?, |id| {
                let _name = self.name(id)?;
                Ok(Scene {
                    nodes: self.list(self.field(id, "nodes")?, |id| self.id(id))?,
                })
            })?,
            nodes: self.list(self.field(0, "nodes")?, |id| {
                let matrix_id = self.field(id, "matrix")?;
                if matrix_id.is_some() {
                    for property in ["translation", "rotation", "scale"] {
                        if self.field(id, property)?.is_some() {
                            return Err(self.invalid(
                                "glTF node matrix and TRS properties are mutually exclusive",
                            ));
                        }
                    }
                }
                let matrix: Option<[f32; 16]> = matrix_id.map(|id| self.fixed(id)).transpose()?;
                if let Some(cells) = matrix.as_ref() {
                    let columns = std::array::from_fn(|column| {
                        std::array::from_fn(|row| cells[column * 4 + row])
                    });
                    self.ctx.charge_work(22)?;
                    super::transforms::node_matrix(&columns)
                        .map_err(|reason| self.invalid(reason))?;
                }
                Ok(Node {
                    name: self.name(id)?,
                    children: self.list(self.field(id, "children")?, |id| self.id(id))?,
                    mesh: self.optional_id(id, "mesh")?,
                    skin: self.optional_id(id, "skin")?,
                    camera: self.optional_id(id, "camera")?,
                    punctual_light: self
                        .field(id, "extensions")?
                        .map(|id| self.field(id, super::KHR_LIGHTS_PUNCTUAL))
                        .transpose()?
                        .flatten(),
                    matrix,
                    translation: self.fixed_default(id, "translation", [0.; 3])?,
                    rotation: self.fixed_default(id, "rotation", [0., 0., 0., 1.])?,
                    scale: self.fixed_default(id, "scale", [1.; 3])?,
                    weights: self.weights(id)?,
                })
            })?,
            meshes: self.list(self.field(0, "meshes")?, |id| {
                Ok(mesh::Mesh {
                    name: self.name(id)?,
                    primitives: self.list(Some(self.required(id, "primitives")?), |id| {
                        self.primitive(id)
                    })?,
                    weights: self.weights(id)?,
                })
            })?,
            accessors: self.list(self.field(0, "accessors")?, |id| self.accessor(id))?,
            buffers: self.list(self.field(0, "buffers")?, |id| {
                let _name = self.name(id)?;
                Ok(Buffer {
                    uri: self.field(id, "uri")?.map(|id| self.text(id)).transpose()?,
                    byte_length: self
                        .field(id, "byteLength")?
                        .map_or(Ok(0), |id| self.size(id))?,
                })
            })?,
            buffer_views: self.list(self.field(0, "bufferViews")?, |id| {
                let _name = self.name(id)?;
                let stride = self
                    .field(id, "byteStride")?
                    .map(|id| self.size(id))
                    .transpose()?;
                if stride.is_some_and(|size| !(4..=252).contains(&size)) {
                    return Err(self.invalid("glTF byteStride must be in 4..=252"));
                }
                Ok(buffer::View {
                    buffer: self.required_id(id, "buffer")?,
                    byte_length: self.size(self.required(id, "byteLength")?)?,
                    byte_offset: self
                        .field(id, "byteOffset")?
                        .map(|id| self.size(id))
                        .transpose()?,
                    byte_stride: stride.map(|size| size as usize),
                    target: self
                        .field(id, "target")?
                        .map(|id| self.enum_uint(id, &[(34962, 34962), (34963, 34963)]))
                        .transpose()?,
                })
            })?,
            skins: self.list(self.field(0, "skins")?, |id| {
                Ok(Skin {
                    name: self.name(id)?,
                    joints: self.list(Some(self.required(id, "joints")?), |id| self.id(id))?,
                    skeleton: self.optional_id(id, "skeleton")?,
                    inverse_bind_matrices: self.optional_id(id, "inverseBindMatrices")?,
                })
            })?,
            images: self.list(self.field(0, "images")?, |id| {
                Ok(Image {
                    name: self.name(id)?,
                    uri: self.field(id, "uri")?.map(|id| self.text(id)).transpose()?,
                    mime_type: self
                        .field(id, "mimeType")?
                        .map(|id| self.text(id))
                        .transpose()?,
                    buffer_view: self.optional_id(id, "bufferView")?,
                })
            })?,
            textures: self.list(self.field(0, "textures")?, |id| {
                Ok(Texture {
                    name: self.name(id)?,
                    source: self.required_id(id, "source")?,
                    sampler: self.optional_id(id, "sampler")?,
                })
            })?,
            samplers: self.list(self.field(0, "samplers")?, |id| {
                Ok(Sampler {
                    name: self.name(id)?,
                    mag_filter: self
                        .field(id, "magFilter")?
                        .map(|id| {
                            self.enum_uint(
                                id,
                                &[(9728, MagFilter::Nearest), (9729, MagFilter::Linear)],
                            )
                        })
                        .transpose()?,
                    min_filter: self
                        .field(id, "minFilter")?
                        .map(|id| {
                            self.enum_uint(
                                id,
                                &[
                                    (9728, MinFilter::Nearest),
                                    (9729, MinFilter::Linear),
                                    (9984, MinFilter::NearestMipmapNearest),
                                    (9985, MinFilter::LinearMipmapNearest),
                                    (9986, MinFilter::NearestMipmapLinear),
                                    (9987, MinFilter::LinearMipmapLinear),
                                ],
                            )
                        })
                        .transpose()?,
                    wrap_s: self
                        .field(id, "wrapS")?
                        .map_or(Ok(WrappingMode::Repeat), |id| {
                            self.enum_uint(
                                id,
                                &[
                                    (33071, WrappingMode::ClampToEdge),
                                    (33648, WrappingMode::MirroredRepeat),
                                    (10497, WrappingMode::Repeat),
                                ],
                            )
                        })?,
                    wrap_t: self
                        .field(id, "wrapT")?
                        .map_or(Ok(WrappingMode::Repeat), |id| {
                            self.enum_uint(
                                id,
                                &[
                                    (33071, WrappingMode::ClampToEdge),
                                    (33648, WrappingMode::MirroredRepeat),
                                    (10497, WrappingMode::Repeat),
                                ],
                            )
                        })?,
                })
            })?,
            materials: self.list(self.field(0, "materials")?, |id| self.material(id))?,
            cameras: self.list(self.field(0, "cameras")?, |id| self.camera(id))?,
            lights: self.lights()?,
            animations: self.list(self.field(0, "animations")?, |id| {
                Ok(animation::Animation {
                    name: self.name(id)?,
                    channels: self.list(Some(self.required(id, "channels")?), |id| {
                        let target = self.required(id, "target")?;
                        Ok(animation::Channel {
                            sampler: self.required_id(id, "sampler")?,
                            target: animation::Target {
                                node: self
                                    .field(target, "node")?
                                    .map(|id| self.id(id))
                                    .transpose()?,
                                path: self.enum_text(
                                    self.required(target, "path")?,
                                    &[
                                        ("translation", animation::Property::Translation),
                                        ("rotation", animation::Property::Rotation),
                                        ("scale", animation::Property::Scale),
                                        ("weights", animation::Property::MorphTargetWeights),
                                    ],
                                )?,
                            },
                        })
                    })?,
                    samplers: self.list(Some(self.required(id, "samplers")?), |id| {
                        Ok(animation::Sampler {
                            input: self.required_id(id, "input")?,
                            output: self.required_id(id, "output")?,
                            interpolation: self.field(id, "interpolation")?.map_or(
                                Ok(animation::Interpolation::Linear),
                                |id| {
                                    self.enum_text(
                                        id,
                                        &[
                                            ("LINEAR", animation::Interpolation::Linear),
                                            ("STEP", animation::Interpolation::Step),
                                            ("CUBICSPLINE", animation::Interpolation::CubicSpline),
                                        ],
                                    )
                                },
                            )?,
                        })
                    })?,
                })
            })?,
            extensions_used: self.list(self.field(0, "extensionsUsed")?, |id| self.text(id))?,
            extensions_required: self
                .list(self.field(0, "extensionsRequired")?, |id| self.text(id))?,
        })
    }

    fn version(&self) -> ParseResult<()> {
        let asset = self.required(0, "asset")?;
        self.object(asset)?;
        let version_id = self.required(asset, "version")?;
        self.kind(
            version_id,
            ValueKind::String,
            "asset.version is required and must be a string",
        )?;
        let version_text = self.text(version_id)?;
        let version = parse_version(version_text.as_str()).ok_or_else(|| {
            self.invalid("asset.version must be two decimal components without leading zeros")
        })?;
        let minimum_text = self
            .field(asset, "minVersion")?
            .map(|id| {
                self.kind(id, ValueKind::String, "asset.minVersion must be a string")?;
                self.text(id)
            })
            .transpose()?;
        let minimum = minimum_text
            .as_ref()
            .map(|text| {
                parse_version(text.as_str()).ok_or_else(|| {
                    self.invalid(
                        "asset.minVersion must be two decimal components without leading zeros",
                    )
                })
            })
            .transpose()?;
        if minimum.is_some_and(|minimum| minimum > version) {
            return Err(self.invalid("asset.minVersion must not exceed asset.version"));
        }
        if version.0 != 2 {
            return Err(self.ctx.format_failure(GltfFailure::unsupported(
                "asset.version",
                "asset.version major version is outside the glTF 2.x profile",
            )));
        }
        if minimum.is_some_and(|minimum| minimum != (2, 0)) {
            return Err(self.ctx.format_failure(GltfFailure::unsupported(
                "asset.minVersion",
                "asset.minVersion requires an unsupported glTF version; supported: 2.0",
            )));
        }
        for field in ["generator", "copyright"] {
            if let Some(id) = self.field(asset, field)? {
                self.kind(
                    id,
                    ValueKind::String,
                    "glTF asset descriptive field requires a string",
                )?;
            }
        }
        Ok(())
    }
}

fn parse_version(value: &str) -> Option<(u32, u32)> {
    fn component(value: &str) -> Option<u32> {
        if value.is_empty() || value.len() > 9 || (value.len() > 1 && value.starts_with('0')) {
            return None;
        }
        value.bytes().try_fold(0u32, |value, byte| {
            byte.is_ascii_digit()
                .then(|| value * 10 + u32::from(byte - b'0'))
        })
    }
    let (major, minor) = value.split_once('.')?;
    Some((component(major)?, component(minor)?))
}
