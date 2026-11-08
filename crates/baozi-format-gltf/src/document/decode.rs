use super::schema::*;
use crate::GltfFailure;
use crate::json_syntax::{DecodedScalars, IndexedValue, JsonIndex, ValueId, ValueKind};
use baozi_import::ImportWorkClass;
use baozi_import::{ImportStorageClass, ParseResult, ParseRoot, ParseWorkLayout};

#[cfg(test)]
mod tests;

// Document views may be shorter than the session supplying admitted storage.
// Keep execution explicit so decoded rows never retain an execution-region borrow.
pub(super) struct Decoder<'a, 'scope> {
    pub(super) bytes: &'scope [u8],
    pub(super) index: &'a JsonIndex<'scope>,
}

impl<'a, 'scope> Decoder<'a, 'scope> {
    fn invalid<'session: 'scope>(
        &self,
        reason: &'static str,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> baozi_import::ParseFailure {
        ctx.format_failure(GltfFailure::invalid_document(reason))
    }

    fn value(&self, id: ValueId) -> &IndexedValue {
        &self.index.values.as_slice()[id]
    }

    fn kind<'session: 'scope>(
        &self,
        id: ValueId,
        kind: ValueKind,
        reason: &'static str,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<()> {
        if self.value(id).kind == kind {
            Ok(())
        } else {
            Err(self.invalid(reason, ctx))
        }
    }

    pub(super) fn object<'session: 'scope>(
        &self,
        id: ValueId,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<()> {
        self.kind(
            id,
            ValueKind::Object,
            "glTF schema field requires an object",
            ctx,
        )
    }

    fn key_eq<'op, 'session: 'scope>(
        &self,
        id: ValueId,
        key: &str,
        layout: &mut Option<ParseWorkLayout<'op, 1>>,
        ctx: &mut ParseRoot<'_, 'op, 'session>,
    ) -> ParseResult<bool> {
        let value = self.value(id);
        let amount = (value.key_end - value.key_start) as u64 + 1;
        let layout = match *layout {
            Some(layout) => layout,
            None => {
                let bound = ctx.bind_group([ImportWorkClass::WorkUnits])?;
                *layout = Some(bound);
                bound
            }
        };
        let compared = ctx.child(&layout, [amount], |child| {
            child.run([amount], || {
                Ok::<_, baozi_import::ParseFailure>((|| -> Result<bool, GltfFailure> {
                    let mut scalars =
                        DecodedScalars::new(self.bytes, value.key_start + 1, value.key_end - 1);
                    for expected in key.chars() {
                        if scalars
                            .next_scalar()
                            .map_err(|()| GltfFailure::invalid_document("invalid admitted key"))?
                            != Some(u32::from(expected))
                        {
                            return Ok(false);
                        }
                    }
                    Ok(scalars
                        .next_scalar()
                        .map_err(|()| GltfFailure::invalid_document("invalid admitted key"))?
                        .is_none())
                })())
            })
        })?;
        compared.map_err(|failure| ctx.format_failure(failure))
    }

    pub(super) fn field<'session: 'scope>(
        &self,
        object: ValueId,
        key: &str,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<Option<ValueId>> {
        self.object(object, ctx)?;
        let mut layout = None;
        for id in self.children(object) {
            if self.key_eq(id, key, &mut layout, ctx)? {
                return Ok(Some(id));
            }
        }
        Ok(None)
    }

    fn required<'session: 'scope>(
        &self,
        object: ValueId,
        key: &'static str,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<ValueId> {
        self.field(object, key, ctx)?
            .ok_or_else(|| self.invalid(key, ctx))
    }

    fn children(&self, id: ValueId) -> impl Iterator<Item = ValueId> + '_ {
        std::iter::successors(self.value(id).first_child, |&id| {
            self.value(id).next_sibling
        })
    }

    fn raw_str<'session: 'scope>(
        &self,
        id: ValueId,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<&'scope str> {
        let value = self.value(id);
        std::str::from_utf8(&self.bytes[value.start..value.end])
            .map_err(|_| self.invalid("invalid admitted UTF-8", ctx))
    }

    fn text_range<'session: 'scope>(
        &self,
        start: usize,
        end: usize,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<Text<'scope>> {
        let bytes = &self.bytes[start..end];
        let mut escaped = false;
        let work_group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
        for chunk in bytes.chunks(crate::parser::BYTE_POLL_INTERVAL) {
            ctx.child(&work_group, [chunk.len() as u64], |child| {
                child.run([chunk.len() as u64], || {
                    escaped |= chunk.contains(&b'\\');
                    Ok::<_, baozi_import::ParseFailure>(())
                })
            })?;
        }
        if !escaped {
            return Ok(Text::Borrowed(
                std::str::from_utf8(bytes)
                    .map_err(|_| self.invalid("invalid admitted string", ctx))?,
            ));
        }
        let mut scalars = DecodedScalars::new(self.bytes, start, end);
        let mut decoded_len = 0usize;
        ctx.child(
            &work_group,
            [(end - start) as u64],
            |child| -> ParseResult<()> {
                let mut polled = start;
                let errors = child.error_view();
                loop {
                    // A surrogate escape pair is indivisible and at most 12 raw bytes.
                    if scalars.position() - polled + 12 > crate::parser::BYTE_POLL_INTERVAL {
                        child.poll()?;
                        polled = scalars.position();
                    }
                    if scalars.position() == end {
                        break;
                    }
                    child.run_returned_with_storage(|_storage| {
                        let before = scalars.position();
                        let Some(scalar) = scalars.next_scalar().map_err(|()| {
                            baozi_import::ParsePieceError::Format(GltfFailure::invalid_document(
                                "invalid admitted escape",
                            ))
                        })?
                        else {
                            return Err(baozi_import::ParsePieceError::Format(
                                GltfFailure::invalid_document("invalid admitted escape"),
                            ));
                        };
                        let scalar = char::from_u32(scalar).ok_or_else(|| {
                            baozi_import::ParsePieceError::Format(GltfFailure::invalid_document(
                                "invalid admitted scalar",
                            ))
                        })?;
                        decoded_len =
                            decoded_len.checked_add(scalar.len_utf8()).ok_or_else(|| {
                                errors.allocation_failure("gltf-document-escaped-string")
                            })?;
                        Ok(((), [(scalars.position() - before) as u64]))
                    })?;
                }
                Ok(())
            },
        )?;
        // Admit the decoded length before the first owned byte, including empty strings.
        let mut decoded = ctx.services()?.metered_string(
            ImportStorageClass::BackendDecode,
            "gltf-document-escaped-string",
        );
        decoded.try_reserve(&mut ctx.services()?, decoded_len)?;
        let mut scalars = DecodedScalars::new(self.bytes, start, end);
        ctx.child(
            &work_group,
            [(end - start) as u64],
            |child| -> ParseResult<()> {
                let mut polled = start;
                loop {
                    if scalars.position() - polled + 12 > crate::parser::BYTE_POLL_INTERVAL {
                        child.poll()?;
                        polled = scalars.position();
                    }
                    if scalars.position() == end {
                        break;
                    }
                    child.run_returned_with_storage(|mut storage| {
                        let before = scalars.position();
                        let Some(scalar) = scalars.next_scalar().map_err(|()| {
                            baozi_import::ParsePieceError::Format(GltfFailure::invalid_document(
                                "invalid admitted escape",
                            ))
                        })?
                        else {
                            return Err(baozi_import::ParsePieceError::Format(
                                GltfFailure::invalid_document("invalid admitted escape"),
                            ));
                        };
                        let scalar = char::from_u32(scalar).ok_or_else(|| {
                            baozi_import::ParsePieceError::Format(GltfFailure::invalid_document(
                                "invalid admitted scalar",
                            ))
                        })?;
                        storage.append_string(&mut decoded, scalar.encode_utf8(&mut [0; 4]))?;
                        Ok(((), [(scalars.position() - before) as u64]))
                    })?;
                }
                Ok(())
            },
        )?;
        Ok(Text::Decoded(decoded))
    }

    fn text<'session: 'scope>(
        &self,
        id: ValueId,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<Text<'scope>> {
        self.kind(
            id,
            ValueKind::String,
            "glTF schema field requires a string",
            ctx,
        )?;
        let value = self.value(id);
        self.text_range(value.start + 1, value.end - 1, ctx)
    }

    fn name<'session: 'scope>(
        &self,
        id: ValueId,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<Option<Text<'scope>>> {
        self.field(id, "name", ctx)?
            .map(|id| self.text(id, ctx))
            .transpose()
    }

    pub(super) fn uint<'session: 'scope>(
        &self,
        id: ValueId,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<u64> {
        self.kind(
            id,
            ValueKind::Number,
            "glTF schema field requires an unsigned integer",
            ctx,
        )?;
        let value = self.value(id);
        let bytes = self
            .bytes
            .get(value.start..value.end)
            .ok_or_else(|| self.invalid("invalid admitted number range", ctx))?;
        super::integer::unsigned(ctx, bytes)
    }

    fn size<'session: 'scope>(
        &self,
        id: ValueId,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<u64> {
        let value = self.uint(id, ctx)?;
        usize::try_from(value).map_err(|_| {
            self.invalid(
                "glTF byte length, count or offset exceeds the host index domain",
                ctx,
            )
        })?;
        Ok(value)
    }

    fn id<'session: 'scope>(
        &self,
        id: ValueId,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<Index> {
        let value = self.uint(id, ctx)?;
        Ok(Index(u32::try_from(value).map_err(|_| {
            self.invalid("glTF reference exceeds the U32 domain", ctx)
        })?))
    }

    fn optional_id<'session: 'scope>(
        &self,
        object: ValueId,
        key: &str,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<Option<Index>> {
        self.field(object, key, ctx)?
            .map(|id| self.id(id, ctx))
            .transpose()
    }

    fn required_id<'session: 'scope>(
        &self,
        object: ValueId,
        key: &'static str,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<Index> {
        self.id(self.required(object, key, ctx)?, ctx)
    }

    fn number<'session: 'scope>(
        &self,
        id: ValueId,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<f64> {
        self.kind(
            id,
            ValueKind::Number,
            "glTF schema field requires a number",
            ctx,
        )?;
        let text = self.raw_str(id, ctx)?;
        let layout = ctx.bind_group([ImportWorkClass::WorkUnits])?;
        let value = ctx
            .child(&layout, [text.len() as u64], |child| {
                child.run([text.len() as u64], || {
                    Ok::<_, baozi_import::ParseFailure>(text.parse::<f64>())
                })
            })?
            .map_err(|_| self.invalid("glTF number is invalid", ctx))?;
        if !value.is_finite() {
            return Err(self.invalid("glTF number exceeds the JSON number domain", ctx));
        }
        Ok(value)
    }

    fn float<'session: 'scope>(
        &self,
        id: ValueId,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<f32> {
        Ok(self.number(id, ctx)? as f32)
    }

    fn float_or<'session: 'scope>(
        &self,
        object: ValueId,
        key: &str,
        default: f32,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<f32> {
        self.field(object, key, ctx)?
            .map_or(Ok(default), |id| self.float(id, ctx))
    }

    fn boolean_or<'session: 'scope>(
        &self,
        object: ValueId,
        key: &str,
        default: bool,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<bool> {
        let Some(id) = self.field(object, key, ctx)? else {
            return Ok(default);
        };
        self.kind(
            id,
            ValueKind::Bool,
            "glTF schema field requires a boolean",
            ctx,
        )?;
        Ok(self.raw_str(id, ctx)? == "true")
    }

    fn fixed<'session: 'scope, const N: usize>(
        &self,
        id: ValueId,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<[f32; N]> {
        self.kind(
            id,
            ValueKind::Array,
            "glTF fixed vector requires an array",
            ctx,
        )?;
        if self.value(id).child_count != N {
            return Err(self.invalid("glTF fixed vector has the wrong length", ctx));
        }
        let mut result = [0.; N];
        for (value, id) in result.iter_mut().zip(self.children(id)) {
            *value = self.float(id, ctx)?;
        }
        Ok(result)
    }

    fn fixed_default<'session: 'scope, const N: usize>(
        &self,
        object: ValueId,
        key: &str,
        default: [f32; N],
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<[f32; N]> {
        self.field(object, key, ctx)?
            .map_or(Ok(default), |id| self.fixed(id, ctx))
    }

    fn list<'session: 'scope, T>(
        &self,
        id: Option<ValueId>,
        decode: impl Fn(ValueId, &mut ParseRoot<'_, '_, 'session>) -> ParseResult<T>,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<List<'scope, T>> {
        let mut list = ctx
            .services()?
            .metered_vec(ImportStorageClass::BackendDecode, "gltf-document-table");
        if let Some(id) = id {
            self.kind(
                id,
                ValueKind::Array,
                "glTF schema field requires an array",
                ctx,
            )?;
            let count = self.value(id).child_count;
            u32::try_from(count)
                .map_err(|_| self.invalid("glTF table exceeds the U32 domain", ctx))?;
            list.try_reserve(&mut ctx.services()?, count)?;
            let work_group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
            let mut children = self.children(id);
            for _ordinal in 0..count {
                let id = ctx
                    .child(&work_group, [1], |child| {
                        child.run([1], || Ok::<_, baozi_import::ParseFailure>(children.next()))
                    })?
                    .ok_or_else(|| self.invalid("glTF table child count is inconsistent", ctx))?;
                list.try_push_reserved(decode(id, ctx)?)?;
            }
        }
        Ok(List(list))
    }

    fn weights<'session: 'scope>(
        &self,
        id: ValueId,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<Option<List<'scope, f32>>> {
        self.field(id, "weights", ctx)?
            .map(|id| self.list(Some(id), |id, ctx| self.float(id, ctx), ctx))
            .transpose()
    }

    fn enum_text<'session: 'scope, T>(
        &self,
        id: ValueId,
        choices: &[(&str, T)],
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<T>
    where
        T: Copy,
    {
        let text = self.text(id, ctx)?;
        choices
            .iter()
            .find(|(key, _)| *key == text.as_str())
            .map(|(_, value)| *value)
            .ok_or_else(|| self.invalid("glTF schema string enum is invalid", ctx))
    }

    fn enum_uint<'session: 'scope, T: Copy>(
        &self,
        id: ValueId,
        choices: &[(u64, T)],
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<T> {
        let number = self.uint(id, ctx)?;
        choices
            .iter()
            .find(|(key, _)| *key == number)
            .map(|(_, value)| *value)
            .ok_or_else(|| self.invalid("glTF schema numeric enum is invalid", ctx))
    }

    fn component<'session: 'scope>(
        &self,
        id: ValueId,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<accessor::ComponentType> {
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
            ctx,
        )
    }

    fn bounds<'session: 'scope>(
        &self,
        object: ValueId,
        key: &str,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<Option<accessor::Bounds>> {
        let Some(id) = self.field(object, key, ctx)? else {
            return Ok(None);
        };
        self.kind(
            id,
            ValueKind::Array,
            "glTF accessor bounds require an array",
            ctx,
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
                *slot = self.number(child, ctx)?;
            }
        }
        Ok(Some(bounds))
    }

    fn accessor<'session: 'scope>(
        &self,
        id: ValueId,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<accessor::Accessor> {
        use accessor::Type::*;
        self.object(id, ctx)?;
        let _name = self.name(id, ctx)?;
        Ok(accessor::Accessor {
            buffer_view: self.optional_id(id, "bufferView", ctx)?,
            byte_offset: self
                .field(id, "byteOffset", ctx)?
                .map(|id| self.size(id, ctx))
                .transpose()?,
            count: self.size(self.required(id, "count", ctx)?, ctx)?,
            component_type: self.component(self.required(id, "componentType", ctx)?, ctx)?,
            type_: self.enum_text(
                self.required(id, "type", ctx)?,
                &[
                    ("SCALAR", Scalar),
                    ("VEC2", Vec2),
                    ("VEC3", Vec3),
                    ("VEC4", Vec4),
                    ("MAT2", Mat2),
                    ("MAT3", Mat3),
                    ("MAT4", Mat4),
                ],
                ctx,
            )?,
            normalized: self.boolean_or(id, "normalized", false, ctx)?,
            sparse: self
                .field(id, "sparse", ctx)?
                .map(|id| self.sparse(id, ctx))
                .transpose()?,
            min: self.bounds(id, "min", ctx)?,
            max: self.bounds(id, "max", ctx)?,
        })
    }

    fn sparse<'session: 'scope>(
        &self,
        id: ValueId,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<accessor::Sparse> {
        let indices = self.required(id, "indices", ctx)?;
        let values = self.required(id, "values", ctx)?;
        let component_type = self.component(self.required(indices, "componentType", ctx)?, ctx)?;
        if !matches!(
            component_type,
            accessor::ComponentType::U8
                | accessor::ComponentType::U16
                | accessor::ComponentType::U32
        ) {
            return Err(self.invalid(
                "glTF sparse indices must use unsigned integer components",
                ctx,
            ));
        }
        Ok(accessor::Sparse {
            count: self.size(self.required(id, "count", ctx)?, ctx)?,
            indices: accessor::Indices {
                buffer_view: self.required_id(indices, "bufferView", ctx)?,
                byte_offset: self
                    .field(indices, "byteOffset", ctx)?
                    .map_or(Ok(0), |id| self.size(id, ctx))?,
                component_type,
            },
            values: accessor::Values {
                buffer_view: self.required_id(values, "bufferView", ctx)?,
                byte_offset: self
                    .field(values, "byteOffset", ctx)?
                    .map_or(Ok(0), |id| self.size(id, ctx))?,
            },
        })
    }

    fn semantic<'session: 'scope>(
        &self,
        child: ValueId,
        ordinal: usize,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<mesh::Semantic> {
        use mesh::Semantic::*;
        let value = self.value(child);
        let key = self.text_range(value.key_start + 1, value.key_end - 1, ctx)?;
        Ok(match key.as_str() {
            "POSITION" => Positions,
            "NORMAL" => Normals,
            "TANGENT" => Tangents,
            key if key.starts_with('_') => Extras(
                u32::try_from(ordinal)
                    .map_err(|_| self.invalid("too many custom semantics", ctx))?,
            ),
            key => {
                let (name, set) = key
                    .rsplit_once('_')
                    .ok_or_else(|| self.invalid("glTF attribute semantic is invalid", ctx))?;
                let set = set
                    .parse::<u32>()
                    .map_err(|_| self.invalid("glTF attribute semantic set is invalid", ctx))?;
                match name {
                    "COLOR" => Colors(set),
                    "TEXCOORD" => TexCoords(set),
                    "JOINTS" => Joints(set),
                    "WEIGHTS" => Weights(set),
                    _ => return Err(self.invalid("glTF attribute semantic is invalid", ctx)),
                }
            }
        })
    }

    fn primitive<'session: 'scope>(
        &self,
        id: ValueId,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<mesh::Primitive<'scope>> {
        use mesh::Mode::*;
        let attributes_id = self.required(id, "attributes", ctx)?;
        self.object(attributes_id, ctx)?;
        let mut attributes = ctx.services()?.metered_vec(
            ImportStorageClass::BackendDecode,
            "gltf-document-attributes",
        );
        attributes.try_reserve(&mut ctx.services()?, self.value(attributes_id).child_count)?;
        let work_group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
        for (ordinal, child) in self.children(attributes_id).enumerate() {
            let semantic = self.semantic(child, ordinal, ctx)?;
            let accessor = self.id(child, ctx)?;
            ctx.child(&work_group, [1], |child| -> ParseResult<()> {
                child.run([1], || attributes.try_push_reserved((semantic, accessor)))
            })?;
        }
        ctx.child(&work_group, [0], |child| -> ParseResult<()> {
            let mut comparisons = 0usize;
            attributes.try_sort_unstable_by(|a, b| -> ParseResult<_> {
                if comparisons.is_multiple_of(baozi_import::MAX_GEOMETRY_ACTIONS_BETWEEN_POLLS) {
                    child.poll()?;
                }
                child.replenish(1)?;
                let ordering =
                    child.run([1], || Ok::<_, baozi_import::ParseFailure>(a.0.cmp(&b.0)))?;
                comparisons += 1;
                Ok(ordering)
            })
        })?;
        ctx.child(
            &work_group,
            [attributes.len().saturating_sub(1) as u64],
            |child| -> ParseResult<()> {
                for (ordinal, pair) in attributes.as_slice().windows(2).enumerate() {
                    if ordinal.is_multiple_of(baozi_import::MAX_GEOMETRY_ACTIONS_BETWEEN_POLLS) {
                        child.poll()?;
                    }
                    let duplicate = child.run([1], || {
                        Ok::<_, baozi_import::ParseFailure>(pair[0].0 == pair[1].0)
                    })?;
                    #[cfg(test)]
                    crate::control_tests::semantic_pair_completed(ordinal + 1);
                    if duplicate {
                        return Err(child.services()?.format_failure(
                            GltfFailure::invalid_document("duplicate glTF attribute semantic"),
                        ));
                    }
                }
                Ok(())
            },
        )?;
        Ok(mesh::Primitive {
            attributes: mesh::Attributes(List(attributes)),
            indices: self.optional_id(id, "indices", ctx)?,
            material: self.optional_id(id, "material", ctx)?,
            mode: self.field(id, "mode", ctx)?.map_or(Ok(Triangles), |id| {
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
                    ctx,
                )
            })?,
            targets: self
                .field(id, "targets", ctx)?
                .map(|id| self.list(Some(id), |id, ctx| self.morph_target(id, ctx), ctx))
                .transpose()?,
        })
    }

    fn morph_target<'session: 'scope>(
        &self,
        id: ValueId,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<mesh::MorphTarget<'scope>> {
        self.object(id, ctx)?;
        let mut unknown = ctx.services()?.metered_vec(
            ImportStorageClass::BackendDecode,
            "gltf-document-morph-semantics",
        );
        let mut positions = None;
        let mut normals = None;
        let mut tangents = None;
        for child in self.children(id) {
            let value = self.value(child);
            let key = self.text_range(value.key_start + 1, value.key_end - 1, ctx)?;
            let slot = match key.as_str() {
                "POSITION" => &mut positions,
                "NORMAL" => &mut normals,
                "TANGENT" => &mut tangents,
                _ => {
                    {
                        let value = (key, self.id(child, ctx)?);
                        unknown.try_push(&mut ctx.services()?, value)
                    }?;
                    continue;
                }
            };
            *slot = Some(self.id(child, ctx)?);
        }
        Ok(mesh::MorphTarget {
            positions,
            normals,
            tangents,
            other_attributes: mesh::UnknownAttributes(List(unknown)),
        })
    }

    fn texture_info<'session: 'scope>(
        &self,
        id: ValueId,
        strength_key: Option<&str>,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<texture::Info> {
        let transform = self
            .field(id, "extensions", ctx)?
            .map(|id| self.field(id, "KHR_texture_transform", ctx))
            .transpose()?
            .flatten();
        Ok(texture::Info {
            index: self.required_id(id, "index", ctx)?,
            tex_coord: self
                .field(id, "texCoord", ctx)?
                .map_or(Ok(0), |id| self.id(id, ctx).map(|id| id.0))?,
            strength: strength_key.map_or(Ok(1.), |key| self.float_or(id, key, 1., ctx))?,
            transform,
        })
    }

    fn pbr<'session: 'scope>(
        &self,
        id: Option<ValueId>,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<Pbr> {
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
            base_color_factor: self.fixed_default(id, "baseColorFactor", [1.; 4], ctx)?,
            metallic_factor: self.float_or(id, "metallicFactor", 1., ctx)?,
            roughness_factor: self.float_or(id, "roughnessFactor", 1., ctx)?,
            base_color_texture: self
                .field(id, "baseColorTexture", ctx)?
                .map(|id| self.texture_info(id, None, ctx))
                .transpose()?,
            metallic_roughness_texture: self
                .field(id, "metallicRoughnessTexture", ctx)?
                .map(|id| self.texture_info(id, None, ctx))
                .transpose()?,
        })
    }

    fn material<'session: 'scope>(
        &self,
        id: ValueId,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<Material<'scope>> {
        Ok(Material {
            name: self.name(id, ctx)?,
            pbr_metallic_roughness: self.pbr(self.field(id, "pbrMetallicRoughness", ctx)?, ctx)?,
            emissive_factor: self.fixed_default(id, "emissiveFactor", [0.; 3], ctx)?,
            alpha_mode: self
                .field(id, "alphaMode", ctx)?
                .map_or(Ok(AlphaMode::Opaque), |id| {
                    self.enum_text(
                        id,
                        &[
                            ("OPAQUE", AlphaMode::Opaque),
                            ("MASK", AlphaMode::Mask),
                            ("BLEND", AlphaMode::Blend),
                        ],
                        ctx,
                    )
                })?,
            alpha_cutoff: self
                .field(id, "alphaCutoff", ctx)?
                .map(|id| self.float(id, ctx))
                .transpose()?,
            double_sided: self.boolean_or(id, "doubleSided", false, ctx)?,
            normal_texture: self
                .field(id, "normalTexture", ctx)?
                .map(|id| self.texture_info(id, Some("scale"), ctx))
                .transpose()?,
            occlusion_texture: self
                .field(id, "occlusionTexture", ctx)?
                .map(|id| self.texture_info(id, Some("strength"), ctx))
                .transpose()?,
            emissive_texture: self
                .field(id, "emissiveTexture", ctx)?
                .map(|id| self.texture_info(id, None, ctx))
                .transpose()?,
            unlit: self
                .field(id, "extensions", ctx)?
                .map(|id| self.field(id, "KHR_materials_unlit", ctx))
                .transpose()?
                .flatten(),
            emissive_strength: self
                .field(id, "extensions", ctx)?
                .map(|id| self.field(id, super::KHR_MATERIALS_EMISSIVE_STRENGTH, ctx))
                .transpose()?
                .flatten(),
            clearcoat: self
                .field(id, "extensions", ctx)?
                .map(|id| self.field(id, super::KHR_MATERIALS_CLEARCOAT, ctx))
                .transpose()?
                .flatten(),
            sheen: self
                .field(id, "extensions", ctx)?
                .map(|id| self.field(id, super::KHR_MATERIALS_SHEEN, ctx))
                .transpose()?
                .flatten(),
        })
    }

    fn camera<'session: 'scope>(
        &self,
        id: ValueId,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<Camera<'scope>> {
        let orthographic = self
            .field(id, "orthographic", ctx)?
            .map(|data| -> ParseResult<_> {
                Ok(Projection::Orthographic {
                    xmag: self.float(self.required(data, "xmag", ctx)?, ctx)?,
                    ymag: self.float(self.required(data, "ymag", ctx)?, ctx)?,
                    znear: self.float(self.required(data, "znear", ctx)?, ctx)?,
                    zfar: self.float(self.required(data, "zfar", ctx)?, ctx)?,
                })
            })
            .transpose()?;
        let perspective = self
            .field(id, "perspective", ctx)?
            .map(|data| -> ParseResult<_> {
                Ok(Projection::Perspective {
                    yfov: self.float(self.required(data, "yfov", ctx)?, ctx)?,
                    znear: self.float(self.required(data, "znear", ctx)?, ctx)?,
                    aspect_ratio: self
                        .field(data, "aspectRatio", ctx)?
                        .map(|id| self.float(id, ctx))
                        .transpose()?,
                    zfar: self
                        .field(data, "zfar", ctx)?
                        .map(|id| self.float(id, ctx))
                        .transpose()?,
                })
            })
            .transpose()?;
        let kind = self.text(self.required(id, "type", ctx)?, ctx)?;
        let projection = match kind.as_str() {
            "orthographic" => orthographic,
            "perspective" => perspective,
            _ => return Err(self.invalid("glTF camera type is invalid", ctx)),
        }
        .ok_or_else(|| {
            self.invalid(
                "glTF camera discriminator requires its matching projection object",
                ctx,
            )
        })?;
        Ok(Camera {
            name: self.name(id, ctx)?,
            projection,
        })
    }

    pub(super) fn emissive_strength<'session: 'scope>(
        &self,
        id: ValueId,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<f32> {
        self.object(id, ctx)?;
        let Some(id) = self.field(id, "emissiveStrength", ctx)? else {
            return Ok(1.0);
        };
        let value = self.number(id, ctx)?;
        if value < 0.0 {
            return Err(self.invalid("glTF emissiveStrength must be nonnegative", ctx));
        }
        let narrowed = value as f32;
        if !narrowed.is_finite() || (value != 0.0 && narrowed == 0.0) {
            return Err(self.invalid(
                "glTF emissiveStrength exceeds the canonical F32 domain",
                ctx,
            ));
        }
        // A nonzero JSON mantissa may already have underflowed while parsing f64.
        if value == 0.0 {
            let text = self.raw_str(id, ctx)?;
            let layout = ctx.bind_group([ImportWorkClass::WorkUnits])?;
            let underflows = ctx.child(&layout, [text.len() as u64], |child| {
                child.run([text.len() as u64], || {
                    Ok::<_, baozi_import::ParseFailure>(
                        text.bytes()
                            .take_while(|&byte| !matches!(byte, b'e' | b'E'))
                            .any(|byte| matches!(byte, b'1'..=b'9')),
                    )
                })
            })?;
            if underflows {
                return Err(self.invalid(
                    "glTF emissiveStrength underflows the canonical F32 domain",
                    ctx,
                ));
            }
        }
        Ok(narrowed)
    }

    fn material_extension_float<'session: 'scope>(
        &self,
        id: ValueId,
        unit: bool,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<f32> {
        let value = self.number(id, ctx)?;
        if unit && !(0.0..=1.0).contains(&value) {
            return Err(self.invalid("glTF material extension factor must be in [0,1]", ctx));
        }
        let narrowed = value as f32;
        if !narrowed.is_finite() || (value != 0.0 && narrowed == 0.0) {
            return Err(self.invalid(
                "glTF material extension number exceeds the canonical F32 domain",
                ctx,
            ));
        }
        if value == 0.0 {
            let token = self.value(id);
            let mut mantissa = true;
            let work_group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
            for chunk in
                self.bytes[token.start..token.end].chunks(crate::parser::BYTE_POLL_INTERVAL)
            {
                ctx.child(&work_group, [chunk.len() as u64], |child| -> ParseResult<()> {
                    for &byte in chunk {
                        let underflows = child.run([1], || {
                        if matches!(byte, b'e' | b'E') {
                            mantissa = false;
                        }
                        Ok::<_, baozi_import::ParseFailure>(mantissa && matches!(byte, b'1'..=b'9'))
                        })?;
                        if underflows {
                            return Err(child.services()?.format_failure(GltfFailure::invalid_document(
                                "glTF material extension number underflows the canonical F32 domain",
                            )));
                        }
                    }
                    Ok(())
                })?;
            }
        }
        Ok(narrowed)
    }

    fn material_unit_or<'session: 'scope>(
        &self,
        object: ValueId,
        key: &str,
        default: f32,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<f32> {
        self.field(object, key, ctx)?.map_or(Ok(default), |id| {
            self.material_extension_float(id, true, ctx)
        })
    }

    fn material_extension_texture<'session: 'scope>(
        &self,
        object: ValueId,
        key: &str,
        normal: bool,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<Option<texture::Info>> {
        self.field(object, key, ctx)?
            .map(|id| {
                let mut info = self.texture_info(id, None, ctx)?;
                if normal {
                    info.strength = self
                        .field(id, "scale", ctx)?
                        .map_or(Ok(1.0), |id| self.material_extension_float(id, false, ctx))?;
                }
                Ok(info)
            })
            .transpose()
    }

    pub(super) fn clearcoat<'session: 'scope>(
        &self,
        id: ValueId,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<Clearcoat> {
        self.object(id, ctx)?;
        Ok(Clearcoat {
            factor: self.material_unit_or(id, "clearcoatFactor", 0.0, ctx)?,
            roughness: self.material_unit_or(id, "clearcoatRoughnessFactor", 0.0, ctx)?,
            texture: self.material_extension_texture(id, "clearcoatTexture", false, ctx)?,
            roughness_texture: self.material_extension_texture(
                id,
                "clearcoatRoughnessTexture",
                false,
                ctx,
            )?,
            normal_texture: self.material_extension_texture(
                id,
                "clearcoatNormalTexture",
                true,
                ctx,
            )?,
        })
    }

    pub(super) fn sheen<'session: 'scope>(
        &self,
        id: ValueId,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<Sheen> {
        self.object(id, ctx)?;
        let mut color = [0.0; 3];
        if let Some(id) = self.field(id, "sheenColorFactor", ctx)? {
            self.kind(
                id,
                ValueKind::Array,
                "glTF sheenColorFactor requires an array",
                ctx,
            )?;
            if self.value(id).child_count != 3 {
                return Err(self.invalid("glTF sheenColorFactor requires three components", ctx));
            }
            for (lane, id) in color.iter_mut().zip(self.children(id)) {
                *lane = self.material_extension_float(id, true, ctx)?;
            }
        }
        Ok(Sheen {
            color,
            roughness: self.material_unit_or(id, "sheenRoughnessFactor", 0.0, ctx)?,
            color_texture: self.material_extension_texture(id, "sheenColorTexture", false, ctx)?,
            roughness_texture: self.material_extension_texture(
                id,
                "sheenRoughnessTexture",
                false,
                ctx,
            )?,
        })
    }

    pub(super) fn transform<'session: 'scope>(
        &self,
        id: ValueId,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<extensions::texture::TextureTransform> {
        self.object(id, ctx)?;
        Ok(extensions::texture::TextureTransform {
            offset: self.fixed_default(id, "offset", [0.; 2], ctx)?,
            rotation: self.float_or(id, "rotation", 0., ctx)?,
            scale: self.fixed_default(id, "scale", [1.; 2], ctx)?,
            tex_coord: self.optional_id(id, "texCoord", ctx)?.map(|id| id.0),
        })
    }

    pub(super) fn punctual_light_reference<'session: 'scope>(
        &self,
        id: ValueId,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<Index> {
        self.required_id(id, "light", ctx)
    }

    fn light_number<'session: 'scope>(
        &self,
        id: ValueId,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<f64> {
        let value = self.number(id, ctx)?;
        if value == 0.0 {
            let text = self.raw_str(id, ctx)?;
            let layout = ctx.bind_group([ImportWorkClass::WorkUnits])?;
            // Source signs and nonzero values must not disappear during binary64 parsing.
            let underflows = ctx.child(&layout, [text.len() as u64], |child| {
                child.run([text.len() as u64], || {
                    Ok::<_, baozi_import::ParseFailure>(
                        text.bytes()
                            .take_while(|&byte| !matches!(byte, b'e' | b'E'))
                            .any(|byte| matches!(byte, b'1'..=b'9')),
                    )
                })
            })?;
            if underflows {
                return Err(self.invalid(
                    "glTF light number underflows the binary64 source domain",
                    ctx,
                ));
            }
        }
        Ok(value)
    }

    pub(super) fn light<'session: 'scope>(
        &self,
        id: ValueId,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<Light<'scope>> {
        self.object(id, ctx)?;
        let mut color = [1.0; 3];
        if let Some(id) = self.field(id, "color", ctx)? {
            self.kind(
                id,
                ValueKind::Array,
                "glTF light color requires an array",
                ctx,
            )?;
            if self.value(id).child_count != 3 {
                return Err(self.invalid("glTF light color requires three components", ctx));
            }
            for (component, id) in color.iter_mut().zip(self.children(id)) {
                *component = self.light_number(id, ctx)?;
            }
        }
        let spot = self
            .field(id, "spot", ctx)?
            .map(|id| -> ParseResult<LightSpot> {
                self.object(id, ctx)?;
                Ok(LightSpot {
                    inner_cone_angle: self
                        .field(id, "innerConeAngle", ctx)?
                        .map_or(Ok(0.0), |id| self.light_number(id, ctx))?,
                    outer_cone_angle: self
                        .field(id, "outerConeAngle", ctx)?
                        .map_or(Ok(std::f64::consts::FRAC_PI_4), |id| {
                            self.light_number(id, ctx)
                        })?,
                })
            })
            .transpose()?;
        Ok(Light {
            name: self
                .field(id, "name", ctx)?
                .map(|id| self.text(id, ctx))
                .transpose()?,
            kind: self.text(self.required(id, "type", ctx)?, ctx)?,
            color,
            intensity: self
                .field(id, "intensity", ctx)?
                .map_or(Ok(1.0), |id| self.light_number(id, ctx))?,
            range: self
                .field(id, "range", ctx)?
                .map(|id| self.light_number(id, ctx))
                .transpose()?,
            spot,
        })
    }

    fn lights<'session: 'scope>(
        &self,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<List<'scope, ValueId>> {
        let payload = self
            .field(0, "extensions", ctx)?
            .map(|id| self.field(id, super::KHR_LIGHTS_PUNCTUAL, ctx))
            .transpose()?
            .flatten();
        let lights = payload
            .map(|id| self.required(id, "lights", ctx))
            .transpose()?;
        if lights.is_some_and(|id| {
            self.value(id).kind == ValueKind::Array && self.value(id).child_count == 0
        }) {
            return Err(self.invalid("glTF punctual lights requires a nonempty lights array", ctx));
        }
        self.list(
            lights,
            |id, ctx| {
                self.object(id, ctx)?;
                Ok(id)
            },
            ctx,
        )
    }

    pub(super) fn root<'session: 'scope>(
        &self,
        ctx: &mut ParseRoot<'_, '_, 'session>,
    ) -> ParseResult<Tables<'scope>> {
        self.object(0, ctx)?;
        self.version(ctx)?;
        Ok(Tables {
            scene: self.optional_id(0, "scene", ctx)?,
            scenes: self.list(
                self.field(0, "scenes", ctx)?,
                |id, ctx| {
                    let _name = self.name(id, ctx)?;
                    Ok(Scene {
                        nodes: self.list(
                            self.field(id, "nodes", ctx)?,
                            |id, ctx| self.id(id, ctx),
                            ctx,
                        )?,
                    })
                },
                ctx,
            )?,
            nodes: self.list(
                self.field(0, "nodes", ctx)?,
                |id, ctx| {
                    let matrix_id = self.field(id, "matrix", ctx)?;
                    if matrix_id.is_some() {
                        for property in ["translation", "rotation", "scale"] {
                            if self.field(id, property, ctx)?.is_some() {
                                return Err(self.invalid(
                                    "glTF node matrix and TRS properties are mutually exclusive",
                                    ctx,
                                ));
                            }
                        }
                    }
                    let matrix: Option<[f32; 16]> =
                        matrix_id.map(|id| self.fixed(id, ctx)).transpose()?;
                    if let Some(cells) = matrix.as_ref() {
                        let columns = std::array::from_fn(|column| {
                            std::array::from_fn(|row| cells[column * 4 + row])
                        });
                        let layout = ctx.bind_group([ImportWorkClass::WorkUnits])?;
                        ctx.child(&layout, [22], |child| {
                            child.run([22], || {
                                Ok::<_, baozi_import::ParseFailure>(super::transforms::node_matrix(
                                    &columns,
                                ))
                            })
                        })?
                        .map_err(|reason| self.invalid(reason, ctx))?;
                    }
                    Ok(Node {
                        name: self.name(id, ctx)?,
                        children: self.list(
                            self.field(id, "children", ctx)?,
                            |id, ctx| self.id(id, ctx),
                            ctx,
                        )?,
                        mesh: self.optional_id(id, "mesh", ctx)?,
                        skin: self.optional_id(id, "skin", ctx)?,
                        camera: self.optional_id(id, "camera", ctx)?,
                        punctual_light: self
                            .field(id, "extensions", ctx)?
                            .map(|id| self.field(id, super::KHR_LIGHTS_PUNCTUAL, ctx))
                            .transpose()?
                            .flatten(),
                        matrix,
                        translation: self.fixed_default(id, "translation", [0.; 3], ctx)?,
                        rotation: self.fixed_default(id, "rotation", [0., 0., 0., 1.], ctx)?,
                        scale: self.fixed_default(id, "scale", [1.; 3], ctx)?,
                        weights: self.weights(id, ctx)?,
                    })
                },
                ctx,
            )?,
            meshes: self.list(
                self.field(0, "meshes", ctx)?,
                |id, ctx| {
                    Ok(mesh::Mesh {
                        name: self.name(id, ctx)?,
                        primitives: self.list(
                            Some(self.required(id, "primitives", ctx)?),
                            |id, ctx| self.primitive(id, ctx),
                            ctx,
                        )?,
                        weights: self.weights(id, ctx)?,
                    })
                },
                ctx,
            )?,
            accessors: self.list(
                self.field(0, "accessors", ctx)?,
                |id, ctx| self.accessor(id, ctx),
                ctx,
            )?,
            buffers: self.list(
                self.field(0, "buffers", ctx)?,
                |id, ctx| {
                    let _name = self.name(id, ctx)?;
                    Ok(Buffer {
                        uri: self
                            .field(id, "uri", ctx)?
                            .map(|id| self.text(id, ctx))
                            .transpose()?,
                        byte_length: self
                            .field(id, "byteLength", ctx)?
                            .map_or(Ok(0), |id| self.size(id, ctx))?,
                    })
                },
                ctx,
            )?,
            buffer_views: self.list(
                self.field(0, "bufferViews", ctx)?,
                |id, ctx| {
                    let _name = self.name(id, ctx)?;
                    let stride = self
                        .field(id, "byteStride", ctx)?
                        .map(|id| self.size(id, ctx))
                        .transpose()?;
                    if stride.is_some_and(|size| !(4..=252).contains(&size)) {
                        return Err(self.invalid("glTF byteStride must be in 4..=252", ctx));
                    }
                    Ok(buffer::View {
                        buffer: self.required_id(id, "buffer", ctx)?,
                        byte_length: self.size(self.required(id, "byteLength", ctx)?, ctx)?,
                        byte_offset: self
                            .field(id, "byteOffset", ctx)?
                            .map(|id| self.size(id, ctx))
                            .transpose()?,
                        byte_stride: stride.map(|size| size as usize),
                        target: self
                            .field(id, "target", ctx)?
                            .map(|id| self.enum_uint(id, &[(34962, 34962), (34963, 34963)], ctx))
                            .transpose()?,
                    })
                },
                ctx,
            )?,
            skins: self.list(
                self.field(0, "skins", ctx)?,
                |id, ctx| {
                    Ok(Skin {
                        name: self.name(id, ctx)?,
                        joints: self.list(
                            Some(self.required(id, "joints", ctx)?),
                            |id, ctx| self.id(id, ctx),
                            ctx,
                        )?,
                        skeleton: self.optional_id(id, "skeleton", ctx)?,
                        inverse_bind_matrices: self.optional_id(id, "inverseBindMatrices", ctx)?,
                    })
                },
                ctx,
            )?,
            images: self.list(
                self.field(0, "images", ctx)?,
                |id, ctx| {
                    Ok(Image {
                        name: self.name(id, ctx)?,
                        uri: self
                            .field(id, "uri", ctx)?
                            .map(|id| self.text(id, ctx))
                            .transpose()?,
                        mime_type: self
                            .field(id, "mimeType", ctx)?
                            .map(|id| self.text(id, ctx))
                            .transpose()?,
                        buffer_view: self.optional_id(id, "bufferView", ctx)?,
                    })
                },
                ctx,
            )?,
            textures: self.list(
                self.field(0, "textures", ctx)?,
                |id, ctx| {
                    Ok(Texture {
                        name: self.name(id, ctx)?,
                        source: self.required_id(id, "source", ctx)?,
                        sampler: self.optional_id(id, "sampler", ctx)?,
                    })
                },
                ctx,
            )?,
            samplers: self.list(
                self.field(0, "samplers", ctx)?,
                |id, ctx| {
                    Ok(Sampler {
                        name: self.name(id, ctx)?,
                        mag_filter: self
                            .field(id, "magFilter", ctx)?
                            .map(|id| {
                                self.enum_uint(
                                    id,
                                    &[(9728, MagFilter::Nearest), (9729, MagFilter::Linear)],
                                    ctx,
                                )
                            })
                            .transpose()?,
                        min_filter: self
                            .field(id, "minFilter", ctx)?
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
                                    ctx,
                                )
                            })
                            .transpose()?,
                        wrap_s: self.field(id, "wrapS", ctx)?.map_or(
                            Ok(WrappingMode::Repeat),
                            |id| {
                                self.enum_uint(
                                    id,
                                    &[
                                        (33071, WrappingMode::ClampToEdge),
                                        (33648, WrappingMode::MirroredRepeat),
                                        (10497, WrappingMode::Repeat),
                                    ],
                                    ctx,
                                )
                            },
                        )?,
                        wrap_t: self.field(id, "wrapT", ctx)?.map_or(
                            Ok(WrappingMode::Repeat),
                            |id| {
                                self.enum_uint(
                                    id,
                                    &[
                                        (33071, WrappingMode::ClampToEdge),
                                        (33648, WrappingMode::MirroredRepeat),
                                        (10497, WrappingMode::Repeat),
                                    ],
                                    ctx,
                                )
                            },
                        )?,
                    })
                },
                ctx,
            )?,
            materials: self.list(
                self.field(0, "materials", ctx)?,
                |id, ctx| self.material(id, ctx),
                ctx,
            )?,
            cameras: self.list(
                self.field(0, "cameras", ctx)?,
                |id, ctx| self.camera(id, ctx),
                ctx,
            )?,
            lights: self.lights(ctx)?,
            animations: self.list(
                self.field(0, "animations", ctx)?,
                |id, ctx| {
                    Ok(animation::Animation {
                        name: self.name(id, ctx)?,
                        channels: self.list(
                            Some(self.required(id, "channels", ctx)?),
                            |id, ctx| {
                                let target = self.required(id, "target", ctx)?;
                                Ok(animation::Channel {
                                    sampler: self.required_id(id, "sampler", ctx)?,
                                    target: animation::Target {
                                        node: self
                                            .field(target, "node", ctx)?
                                            .map(|id| self.id(id, ctx))
                                            .transpose()?,
                                        path: self.enum_text(
                                            self.required(target, "path", ctx)?,
                                            &[
                                                ("translation", animation::Property::Translation),
                                                ("rotation", animation::Property::Rotation),
                                                ("scale", animation::Property::Scale),
                                                (
                                                    "weights",
                                                    animation::Property::MorphTargetWeights,
                                                ),
                                            ],
                                            ctx,
                                        )?,
                                    },
                                })
                            },
                            ctx,
                        )?,
                        samplers: self.list(
                            Some(self.required(id, "samplers", ctx)?),
                            |id, ctx| {
                                Ok(animation::Sampler {
                                    input: self.required_id(id, "input", ctx)?,
                                    output: self.required_id(id, "output", ctx)?,
                                    interpolation: self.field(id, "interpolation", ctx)?.map_or(
                                        Ok(animation::Interpolation::Linear),
                                        |id| {
                                            self.enum_text(
                                                id,
                                                &[
                                                    ("LINEAR", animation::Interpolation::Linear),
                                                    ("STEP", animation::Interpolation::Step),
                                                    (
                                                        "CUBICSPLINE",
                                                        animation::Interpolation::CubicSpline,
                                                    ),
                                                ],
                                                ctx,
                                            )
                                        },
                                    )?,
                                })
                            },
                            ctx,
                        )?,
                    })
                },
                ctx,
            )?,
            extensions_used: self.list(
                self.field(0, "extensionsUsed", ctx)?,
                |id, ctx| self.text(id, ctx),
                ctx,
            )?,
            extensions_required: self.list(
                self.field(0, "extensionsRequired", ctx)?,
                |id, ctx| self.text(id, ctx),
                ctx,
            )?,
        })
    }

    fn version<'session: 'scope>(&self, ctx: &mut ParseRoot<'_, '_, 'session>) -> ParseResult<()> {
        let asset = self.required(0, "asset", ctx)?;
        self.object(asset, ctx)?;
        let version_id = self.required(asset, "version", ctx)?;
        self.kind(
            version_id,
            ValueKind::String,
            "asset.version is required and must be a string",
            ctx,
        )?;
        let version_text = self.text(version_id, ctx)?;
        let version = parse_version(version_text.as_str()).ok_or_else(|| {
            self.invalid(
                "asset.version must be two decimal components without leading zeros",
                ctx,
            )
        })?;
        let minimum_text = self
            .field(asset, "minVersion", ctx)?
            .map(|id| {
                self.kind(
                    id,
                    ValueKind::String,
                    "asset.minVersion must be a string",
                    ctx,
                )?;
                self.text(id, ctx)
            })
            .transpose()?;
        let minimum = minimum_text
            .as_ref()
            .map(|text| {
                parse_version(text.as_str()).ok_or_else(|| {
                    self.invalid(
                        "asset.minVersion must be two decimal components without leading zeros",
                        ctx,
                    )
                })
            })
            .transpose()?;
        if minimum.is_some_and(|minimum| minimum > version) {
            return Err(self.invalid("asset.minVersion must not exceed asset.version", ctx));
        }
        if version.0 != 2 {
            return Err(ctx.format_failure(GltfFailure::unsupported(
                "asset.version",
                "asset.version major version is outside the glTF 2.x profile",
            )));
        }
        if minimum.is_some_and(|minimum| minimum != (2, 0)) {
            return Err(ctx.format_failure(GltfFailure::unsupported(
                "asset.minVersion",
                "asset.minVersion requires an unsupported glTF version; supported: 2.0",
            )));
        }
        for field in ["generator", "copyright"] {
            if let Some(id) = self.field(asset, field, ctx)? {
                self.kind(
                    id,
                    ValueKind::String,
                    "glTF asset descriptive field requires a string",
                    ctx,
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
