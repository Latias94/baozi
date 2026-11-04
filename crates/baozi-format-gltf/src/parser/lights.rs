//! The single lowering owner for selected glTF punctual-light definitions.

use super::{
    GltfFailure, Result, authoring_u32, enforce_usize_limit, gltf_parse_error, owned_name,
    selected_count, try_vec_capacity, try_vec_filled,
};
use crate::document::Document;
use crate::source_map::{SourceEntityId, SourceEntityKind};
use baozi_import::ImportWorkClass;
use baozi_import::ParseServices;
use baozi_import::authoring::{Color, LightDefinition, LightKind, LightLocalId, LocalAxes};
use baozi_import::{ImportStorageClass, MeteredString, MeteredVec, ParseRoot};

pub(super) struct LightStorage<'scope> {
    pub(super) authored: MeteredVec<'scope, LightDefinition>,
    pub(super) names: MeteredVec<'scope, Option<MeteredString<'scope>>>,
    pub(super) light_ids: MeteredVec<'scope, Option<LightLocalId>>,
}

pub(super) fn prepare<'scope>(
    ctx: &mut ParseRoot<'_, '_, 'scope>,
    document: &Document<'scope>,
    selected: &[bool],
) -> Result<LightStorage<'scope>> {
    if selected.len() != document.tables().lights.len() {
        return Err(GltfFailure::internal("glTF light selection length changed").into());
    }
    let count = selected_count(ctx, selected)?;
    {
        let argument_0 = ctx.services()?.limits().max_records;
        enforce_usize_limit(&mut ctx.services()?, "max_records", count, argument_0)
    }?;
    let mut storage = LightStorage {
        authored: try_vec_capacity(
            &mut ctx.services()?,
            ImportStorageClass::SceneConstruction,
            count,
            "gltf-authoring-lights",
        )?,
        names: try_vec_capacity(
            &mut ctx.services()?,
            ImportStorageClass::FormatNormalization,
            count,
            "gltf-light-names",
        )?,
        light_ids: try_vec_filled(
            &mut ctx.services()?,
            ImportStorageClass::FormatNormalization,
            selected.len(),
            None,
            "gltf-light-id-map",
        )?,
    };
    let work_group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
    for (index, &selected) in selected.iter().enumerate() {
        let selected = ctx.child(&work_group, [1], |child| {
            child.run([1], || Ok::<_, baozi_import::ParseFailure>(selected))
        })?;
        if !selected {
            continue;
        }
        let source = document.light(ctx, document.tables().lights[index])?;
        let unsupported = |reason| -> super::ContentError {
            let id = SourceEntityId {
                kind: SourceEntityKind::Light,
                parent_index: None,
                source_index: index,
            };
            match document.source_map().get(id) {
                Some(entity) => GltfFailure::unsupported_at(
                    "light",
                    entity.range.byte_offset,
                    entity.range.byte_length,
                    reason,
                )
                .into(),
                None => GltfFailure::internal("selected glTF light has no source range").into(),
            }
        };
        let comparisons =
            u64::from(usize::BITS - document.source_map().entities().len().leading_zeros()) + 1;
        let supported = ctx.child(&work_group, [comparisons], |child| {
            child.run([comparisons], || {
                Ok::<_, baozi_import::ParseFailure>(matches!(
                    source.kind.as_str(),
                    "directional" | "point" | "spot"
                ))
            })
        })?;
        if !supported {
            return Err(unsupported(
                "glTF light type is outside the admitted profile",
            ));
        }
        if source.kind.as_str() != "spot" && source.spot.is_some() {
            return Err(unsupported(
                "glTF spot payload on a non-spot light has no admitted mapping",
            ));
        }
        let mut color = [0.0; 3];
        for (component, value) in color.iter_mut().zip(source.color) {
            if !(0.0..=1.0).contains(&value) {
                return Err(gltf_parse_error("glTF light color must be in [0, 1]"));
            }
            *component = narrow(&mut ctx.services()?, value)?;
        }
        let [red, green, blue] = color;
        if source.intensity < 0.0 {
            return Err(gltf_parse_error("glTF light intensity must be nonnegative"));
        }
        let intensity = narrow(&mut ctx.services()?, source.intensity)?;
        let range = source
            .range
            .map(|range| {
                if range <= 0.0 {
                    return Err(gltf_parse_error("glTF light range must be positive"));
                }
                narrow(&mut ctx.services()?, range)
            })
            .transpose()?;
        let kind = match source.kind.as_str() {
            "directional" => {
                if range.is_some() {
                    return Err(gltf_parse_error(
                        "glTF directional light cannot have a range",
                    ));
                }
                LightKind::directional()
            }
            "point" => LightKind::point(range)
                .map_err(|_| gltf_parse_error("glTF light range is invalid"))?,
            "spot" => {
                let spot = source
                    .spot
                    .ok_or_else(|| gltf_parse_error("glTF spot light requires a spot object"))?;
                if spot.inner_cone_angle < 0.0
                    || spot.inner_cone_angle >= spot.outer_cone_angle
                    || spot.outer_cone_angle > std::f64::consts::FRAC_PI_2
                {
                    return Err(gltf_parse_error(
                        "glTF spot requires 0 <= inner < outer <= pi/2",
                    ));
                }
                let inner = narrow(&mut ctx.services()?, spot.inner_cone_angle)?;
                let outer = narrow(&mut ctx.services()?, spot.outer_cone_angle)?;
                if inner >= outer {
                    return Err(gltf_parse_error(
                        "glTF spot cone angles collapse in canonical F32",
                    ));
                }
                LightKind::spot(range, inner, outer)
                    .map_err(|_| gltf_parse_error("glTF spot cone is invalid"))?
            }
            _ => return Err(GltfFailure::internal("checked glTF light kind changed").into()),
        };
        let definition = ctx
            .child(&work_group, [1], |child| {
                child.run([1], || {
                    Ok::<_, baozi_import::ParseFailure>(
                        LightDefinition::try_from_scaled_linear_srgb(
                            kind,
                            LocalAxes::STANDARD,
                            Color::linear_rgba(red, green, blue, 1.0),
                            intensity,
                        ),
                    )
                })
            })?
            .map_err(|_| {
                gltf_parse_error("glTF light emission is not representable in canonical F32")
            })?;
        let name = source.name.as_ref().map(|name| name.as_str());
        if let Some(name) = name {
            {
                let argument_0 = ctx.services()?.limits().max_string_bytes;
                enforce_usize_limit(
                    &mut ctx.services()?,
                    "max_string_bytes",
                    name.len(),
                    argument_0,
                )
            }?;
        }
        let name = match name {
            Some(name) if !name.is_empty() => {
                let layout = ctx.bind_group([ImportWorkClass::CanonicalStringBytes])?;
                ctx.child(&layout, [super::to_u64(name.len())], |child| {
                    child.run([super::to_u64(name.len())], || {
                        Ok::<_, baozi_import::ParseFailure>(
                            source.name.as_ref().map(|name| name.as_str()),
                        )
                    })
                })?
            }
            name => name,
        };
        let id = LightLocalId::new(authoring_u32(
            &mut ctx.services()?,
            storage.authored.len(),
            "gltf-authoring-light-id",
        )?);
        storage
            .names
            .try_push_reserved(owned_name(ctx, name, "gltf-light-name")?)?;
        storage.authored.try_push_reserved(definition)?;
        storage.light_ids.as_mut_slice()[index] = Some(id);
    }
    Ok(storage)
}

/// Validate source bounds first; narrowing must not erase positive source values.
fn narrow(_ctx: &mut ParseServices<'_, '_, '_>, value: f64) -> Result<f32> {
    let narrowed = value as f32;
    if !value.is_finite() || !narrowed.is_finite() || (value > 0.0 && narrowed == 0.0) {
        return Err(gltf_parse_error(
            "glTF light value exceeds the canonical F32 domain",
        ));
    }
    Ok(narrowed)
}
