//! Selected source identities and their canonical owners, independent of decoder lifetimes.

use super::{
    ContentError, GltfFailure, PreparedDocument, Result, authoring_u32, extras::SourceExtras,
    gltf_parse_error, model::NormalizedGltf,
};
use crate::document::schema as json;
use crate::source_map::{
    GltfSourceMap, SourceEntity, SourceEntityId, SourceEntityKind, SourcePayloadKind,
};
use baozi_import::ImportWorkClass;
use baozi_import::authoring::{
    AttributeSemantic, ExtensionSchemaId, ProfileOwner, SourceLocatorValue, SourceObjectLocator,
};
use baozi_import::{
    CapacityPlan, ImportStorageClass, MeteredOpaqueAllowance, MeteredVec, ParseParser, ParseResult,
    ParseRoot, ParseServices,
};

const SOURCE_SCHEMA: &str = "org.baozi.gltf:source-object";
const COMPARISONS_BETWEEN_POLLS: u64 = 64;

#[cfg(test)]
mod tests;

// Each comparison is one WorkUnit. Production callers compare fixed owner/entry ordinals
// and validated NamespacedName keys of at most 64 ASCII bytes.
struct ProfileComparisonWork<'meter, 'borrow, 'frame, 'ctx, 'op, 'session> {
    work: &'meter mut ParseParser<'borrow, 'frame, 'ctx, 'op, 'session, 1, true>,
    comparisons_until_poll: u64,
}

impl<'meter, 'borrow, 'frame, 'ctx, 'op, 'session>
    ProfileComparisonWork<'meter, 'borrow, 'frame, 'ctx, 'op, 'session>
{
    fn new(work: &'meter mut ParseParser<'borrow, 'frame, 'ctx, 'op, 'session, 1, true>) -> Self {
        Self {
            work,
            comparisons_until_poll: COMPARISONS_BETWEEN_POLLS,
        }
    }

    fn run<T>(&mut self, compare: impl FnOnce() -> T) -> ParseResult<T> {
        // Admit only this comparison; no later work precedes its physical completion.
        self.work.replenish(1)?;
        let value = self
            .work
            .run([1], || Ok::<_, baozi_import::ParseFailure>(compare()))?;
        #[cfg(test)]
        tests::comparison_completed();
        self.comparisons_until_poll -= 1;
        if self.comparisons_until_poll == 0 {
            self.work.poll()?;
            self.comparisons_until_poll = COMPARISONS_BETWEEN_POLLS;
        }
        Ok(value)
    }
}

/// Schema v1 has five ordered fields: `kind;parent;index;offset;length`.
/// Fields are labelled, decimal integers use no leading zeroes, and `-` means no parent.
/// Offsets include the GLB JSON chunk's raw base, so every range addresses primary bytes.
/// Object identities never contain names or canonical compact ordinals.
pub(super) struct SelectedSource<'scope> {
    pub(super) entity: SourceEntity,
    pub(super) locator: SourceObjectLocator,
    // The locator's String drops before the allowance for its actual capacity.
    _storage: MeteredOpaqueAllowance<'scope>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct SelectedEntryOrdinal(usize);

#[derive(Clone, Copy)]
struct OwnerSourceRange {
    owner: ProfileOwner,
    start: usize,
    length: usize,
}

const OWNER_CLASSES: usize = 19;

fn owner_key(owner: ProfileOwner) -> (usize, u32) {
    match owner {
        ProfileOwner::Scene => (0, 0),
        ProfileOwner::SyntheticRoot => (1, 0),
        ProfileOwner::Node(id) => (2, id.index()),
        ProfileOwner::Camera(index) => (3, index),
        ProfileOwner::Light(index) => (4, index),
        ProfileOwner::Image(index) => (5, index),
        ProfileOwner::Sampler(index) => (6, index),
        ProfileOwner::Texture(index) => (7, index),
        ProfileOwner::TextureBinding(index) => (8, index),
        ProfileOwner::Material(index) => (9, index),
        ProfileOwner::LogicalMesh(index) => (10, index),
        ProfileOwner::Primitive(index) => (11, index),
        ProfileOwner::Attribute(index) => (12, index),
        ProfileOwner::MeshInstance(index) => (13, index),
        ProfileOwner::Skin(index) => (14, index),
        ProfileOwner::MorphDefinition(index) => (15, index),
        ProfileOwner::MorphDelta(index) => (16, index),
        ProfileOwner::AnimationClip(index) => (17, index),
        ProfileOwner::AnimationChannel(index) => (18, index),
    }
}

/// Addressing for the compact owner keys actually observed during range coverage.
/// Canonical owner validity and completeness remain the Core validator's responsibility.
struct OwnerDirectoryShape {
    offsets: [usize; OWNER_CLASSES + 1],
}

impl OwnerDirectoryShape {
    fn from_maxima(
        maxima: [usize; OWNER_CLASSES],
        invalid_domain: bool,
        owners: usize,
    ) -> Result<Self> {
        if invalid_domain {
            return Err(GltfFailure::internal("glTF source owner directory exceeds usize").into());
        }
        let mut offsets = [0_usize; OWNER_CLASSES + 1];
        for (class, length) in maxima.into_iter().enumerate() {
            offsets[class + 1] = offsets[class].checked_add(length).ok_or_else(|| {
                GltfFailure::internal("glTF source owner directory exceeds usize")
            })?;
        }
        if offsets[OWNER_CLASSES] > owners {
            return Err(GltfFailure::internal("glTF source owner directory is not compact").into());
        }
        Ok(Self { offsets })
    }

    fn len(&self) -> usize {
        self.offsets[OWNER_CLASSES]
    }

    fn slot(&self, owner: ProfileOwner) -> Option<usize> {
        let (class, local) = owner_key(owner);
        let slot = self.offsets[class].checked_add(usize::try_from(local).ok()?)?;
        (slot < self.offsets[class + 1]).then_some(slot)
    }
}

struct OwnerDirectory<'scope> {
    shape: OwnerDirectoryShape,
    // Missing slots are usize::MAX. Each real value indexes an append-order range.
    ranges: MeteredVec<'scope, usize>,
}

pub(super) struct OwnerSourceRun<'scope> {
    owner: Option<ProfileOwner>,
    ordinals: MeteredVec<'scope, SelectedEntryOrdinal>,
}

impl<'scope> OwnerSourceRun<'scope> {
    pub(super) fn new(ctx: &mut ParseRoot<'_, '_, 'scope>) -> Result<Self> {
        Ok(Self {
            owner: None,
            ordinals: ctx.services()?.metered_vec(
                ImportStorageClass::FormatNormalization,
                "gltf-selected-source-owner-scratch",
            ),
        })
    }

    fn order(&mut self, ctx: &mut ParseRoot<'_, '_, '_>) -> Result<()> {
        ctx.poll()?;
        let group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
        if self.ordinals.len() > 1 {
            ctx.child(&group, [0], |ledger_work| {
                let mut work = ProfileComparisonWork::new(ledger_work);
                #[cfg(test)]
                tests::comparison_scope(tests::ComparisonSite::RunSort);
                self.ordinals
                    .try_sort_unstable_by(|left, right| work.run(|| left.cmp(right)))
            })?;
        }
        ctx.poll()?;
        let retained = if self.ordinals.len() > 1 {
            ctx.child(&group, [0], |ledger_work| {
                let mut work = ProfileComparisonWork::new(ledger_work);
                #[cfg(test)]
                tests::comparison_scope(tests::ComparisonSite::RunDedup);
                self.ordinals
                    .try_partition_dedup_by(|left, right| work.run(|| left == right))
            })?
        } else {
            self.ordinals.len()
        };
        ctx.poll()?;
        self.ordinals.truncate(retained);
        Ok(())
    }
}

/// Only selected declarations survive normalization. The session owns the primary snapshot;
/// this inventory owns the locators and their admitted allocations until authoring completes.
pub(super) struct SelectedSourceProfile<'scope> {
    entries: MeteredVec<'scope, SelectedSource<'scope>>,
    ordinals: MeteredVec<'scope, SelectedEntryOrdinal>,
    owners: MeteredVec<'scope, OwnerSourceRange>,
    directory: Option<OwnerDirectory<'scope>>,
    finished: bool,
    extras: MeteredVec<'scope, SourceExtras>,
}

pub(super) const fn entity(
    kind: SourceEntityKind,
    parent_index: Option<usize>,
    source_index: usize,
) -> SourceEntityId {
    SourceEntityId {
        kind,
        parent_index,
        source_index,
    }
}

pub(super) const DOCUMENT: SourceEntityId = entity(SourceEntityKind::Document, None, 0);

impl<'scope> SelectedSourceProfile<'scope> {
    pub(super) fn from_source_map(
        ctx: &mut ParseRoot<'_, '_, 'scope>,
        map: &GltfSourceMap<'_>,
        selected: impl Fn(SourceEntityId) -> bool,
    ) -> Result<Self> {
        let mut entries = ctx.services()?.metered_vec(
            ImportStorageClass::FormatNormalization,
            "gltf-selected-source-identities",
        );
        let mut extras = ctx.services()?.metered_vec(
            ImportStorageClass::FormatNormalization,
            "gltf-selected-source-extras",
        );
        let work_group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
        for &source in map.entities() {
            let selected_source = ctx.child(&work_group, [1], |child| {
                child.run([1], || {
                    Ok::<_, baozi_import::ParseFailure>(selected(source.id))
                })
            })?;
            if selected_source {
                {
                    let value = source_locator(&mut ctx.services()?, source)?;
                    entries.try_push(&mut ctx.services()?, value)
                }?;
                let payloads = {
                    let argument_0 = ctx.services()?.primary_bytes().len();
                    lookup(ctx, argument_0, 2, || map.payloads_for(source.id))
                }?;
                for payload in payloads {
                    let kind = ctx.child(&work_group, [1], |child| {
                        child.run([1], || Ok::<_, baozi_import::ParseFailure>(payload.kind))
                    })?;
                    if kind == SourcePayloadKind::Extras {
                        {
                            let value = SourceExtras::new(payload)?;
                            extras.try_push(&mut ctx.services()?, value)
                        }?;
                    }
                }
            }
        }
        ctx.poll()?;
        if extras.len() > 1 {
            ctx.child(&work_group, [0], |ledger_work| {
                let mut work = ProfileComparisonWork::new(ledger_work);
                #[cfg(test)]
                tests::comparison_scope(tests::ComparisonSite::ExtrasSort);
                extras.try_sort_unstable_by(|left, right| {
                    work.run(|| {
                        left.source
                            .cmp(&right.source)
                            .then_with(|| left.key.cmp(&right.key))
                    })
                })
            })?;
        }
        ctx.poll()?;
        let profile = Self {
            entries,
            ordinals: ctx.services()?.metered_vec(
                ImportStorageClass::FormatNormalization,
                "gltf-selected-source-owner-ordinals",
            ),
            owners: ctx.services()?.metered_vec(
                ImportStorageClass::FormatNormalization,
                "gltf-selected-source-owner-ranges",
            ),
            directory: None,
            finished: false,
            extras,
        };
        profile.required(ctx, DOCUMENT)?;
        Ok(profile)
    }

    pub(super) fn entries(&self) -> &[SelectedSource<'scope>] {
        self.entries.as_slice()
    }

    pub(super) fn link_count(&self) -> usize {
        self.ordinals.len()
    }

    pub(super) fn extras(&self) -> &[SourceExtras] {
        self.extras.as_slice()
    }

    pub(super) fn required(
        &self,
        ctx: &mut ParseRoot<'_, '_, '_>,
        id: SourceEntityId,
    ) -> Result<&SelectedSource<'scope>> {
        let ordinal = self.required_ordinal(ctx, id)?;
        self.source(ctx, ordinal)
    }

    fn find_ordinal(
        &self,
        ctx: &mut ParseRoot<'_, '_, '_>,
        id: SourceEntityId,
    ) -> Result<Option<SelectedEntryOrdinal>> {
        lookup(ctx, self.entries.len(), 1, || {
            self.entries
                .as_slice()
                .binary_search_by_key(&id, |source| source.entity.id)
                .ok()
                .map(SelectedEntryOrdinal)
        })
    }

    fn required_ordinal(
        &self,
        ctx: &mut ParseRoot<'_, '_, '_>,
        id: SourceEntityId,
    ) -> Result<SelectedEntryOrdinal> {
        self.find_ordinal(ctx, id)?.ok_or_else(|| {
            ContentError::from(GltfFailure::internal(
                "selected glTF owner has no admitted source identity",
            ))
        })
    }

    pub(super) fn link(
        &self,
        ctx: &mut ParseRoot<'_, '_, 'scope>,
        run: &mut OwnerSourceRun<'scope>,
        owner: ProfileOwner,
        source: SourceEntityId,
    ) -> Result<()> {
        let ordinal = self.required_ordinal(ctx, source)?;
        run.ordinals.try_reserve(&mut ctx.services()?, 1)?;
        let layout = ctx.bind_group([ImportWorkClass::WorkUnits])?;
        ctx.child(&layout, [1], |child| {
            child.run([1], || {
                if self.finished || run.owner.is_some_and(|active| active != owner) {
                    return Err(GltfFailure::internal("glTF source owner run is not open").into());
                }
                run.ordinals.try_push_reserved(ordinal)?;
                run.owner = Some(owner);
                Ok::<_, ContentError>(())
            })
        })
    }

    pub(super) fn complete_owner(
        &mut self,
        ctx: &mut ParseRoot<'_, '_, 'scope>,
        run: &mut OwnerSourceRun<'scope>,
        owner: ProfileOwner,
    ) -> Result<()> {
        ctx.poll()?;
        let group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
        ctx.child(&group, [1], |child| {
            child.run([1], || {
                if self.finished || run.owner != Some(owner) || run.ordinals.is_empty() {
                    return Err(GltfFailure::internal("glTF source owner run is incomplete").into());
                }
                Ok::<_, ContentError>(())
            })
        })?;
        run.order(ctx)?;
        let range = ctx.child(&group, [1], |child| {
            child.run([1], || {
                let range = OwnerSourceRange {
                    owner,
                    start: self.ordinals.len(),
                    length: run.ordinals.len(),
                };
                range.start.checked_add(range.length).ok_or_else(|| {
                    GltfFailure::internal("glTF source owner range exceeds usize")
                })?;
                Ok::<_, ContentError>(range)
            })
        })?;
        self.ordinals
            .try_reserve(&mut ctx.services()?, range.length)?;
        self.owners.try_reserve(&mut ctx.services()?, 1)?;
        ctx.child(&group, [0], |work| {
            work.poll()?;
            for (index, &ordinal) in run.ordinals.as_slice().iter().enumerate() {
                work.replenish(1)?;
                work.run([1], || {
                    if self.entries.as_slice().get(ordinal.0).is_none() {
                        return Err(
                            GltfFailure::internal("glTF source entry ordinal is invalid").into(),
                        );
                    }
                    self.ordinals.try_push_reserved(ordinal)?;
                    Ok::<_, ContentError>(())
                })?;
                #[cfg(test)]
                tests::ordinal_transferred();
                if (index + 1) % COMPARISONS_BETWEEN_POLLS as usize == 0 {
                    work.poll()?;
                }
            }
            work.poll()?;
            Ok::<_, ContentError>(())
        })?;
        ctx.child(&group, [1], |child| {
            child.run([1], || {
                self.owners.try_push_reserved(range)?;
                run.ordinals.truncate(0);
                run.owner = None;
                Ok::<_, ContentError>(())
            })
        })?;
        ctx.poll()?;
        Ok(())
    }

    pub(super) fn finish(
        &mut self,
        ctx: &mut ParseRoot<'_, '_, 'scope>,
        ordinary: &OwnerSourceRun<'_>,
        suspended: &OwnerSourceRun<'_>,
    ) -> Result<()> {
        ctx.poll()?;
        let group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
        ctx.child(&group, [1], |child| {
            child.run([1], || {
                if self.finished
                    || self.directory.is_some()
                    || ordinary.owner.is_some()
                    || suspended.owner.is_some()
                    || !ordinary.ordinals.is_empty()
                    || !suspended.ordinals.is_empty()
                {
                    return Err(
                        GltfFailure::internal("glTF source owner runs are incomplete").into(),
                    );
                }
                Ok::<_, ContentError>(())
            })
        })?;
        let mut maxima = [0_usize; OWNER_CLASSES];
        let mut invalid_domain = false;
        let covered = ctx.child(&group, [0], |work| {
            work.poll()?;
            let mut covered = 0;
            for (index, range) in self.owners.as_slice().iter().enumerate() {
                work.replenish(1)?;
                covered = work.run([1], || -> Result<_> {
                    let end = range
                        .start
                        .checked_add(range.length)
                        .filter(|&end| {
                            range.start == covered
                                && range.length != 0
                                && end <= self.ordinals.len()
                        })
                        .ok_or_else(|| {
                            ContentError::from(GltfFailure::internal(
                                "glTF source owner range is invalid",
                            ))
                        })?;
                    let (class, local) = owner_key(range.owner);
                    if let Some(length) = usize::try_from(local)
                        .ok()
                        .and_then(|index| index.checked_add(1))
                    {
                        maxima[class] = maxima[class].max(length);
                    } else {
                        // Keep all original range/coverage errors ahead of domain errors.
                        invalid_domain = true;
                    }
                    Ok(end)
                })?;
                #[cfg(test)]
                tests::directory_range_covered();
                if (index + 1) % COMPARISONS_BETWEEN_POLLS as usize == 0 {
                    work.poll()?;
                }
            }
            work.poll()?;
            Ok::<_, ContentError>(covered)
        })?;
        let shape = ctx.child(&group, [1], |child| {
            child.run([1], || {
                if covered != self.ordinals.len() {
                    return Err(
                        GltfFailure::internal("glTF source owner ranges are incomplete").into(),
                    );
                }
                OwnerDirectoryShape::from_maxima(maxima, invalid_domain, self.owners.len())
            })
        })?;
        let mut directory = OwnerDirectory {
            shape,
            ranges: ctx.services()?.metered_vec(
                ImportStorageClass::FormatNormalization,
                "gltf-selected-source-owner-directory",
            ),
        };
        directory
            .ranges
            .try_reserve(&mut ctx.services()?, directory.shape.len())?;
        ctx.child(&group, [0], |work| {
            work.poll()?;
            for index in 0..directory.shape.len() {
                work.replenish(1)?;
                work.run([1], || {
                    directory.ranges.try_push_reserved(usize::MAX)?;
                    Ok::<_, ContentError>(())
                })?;
                #[cfg(test)]
                tests::directory_initialized();
                if (index + 1) % COMPARISONS_BETWEEN_POLLS as usize == 0 {
                    work.poll()?;
                }
            }
            work.poll()?;
            Ok::<_, ContentError>(())
        })?;
        ctx.child(&group, [0], |work| {
            work.poll()?;
            for (index, range) in self.owners.as_slice().iter().enumerate() {
                work.replenish(1)?;
                work.run([1], || {
                    let entry = directory
                        .shape
                        .slot(range.owner)
                        .and_then(|slot| directory.ranges.as_mut_slice().get_mut(slot))
                        .ok_or_else(|| {
                            GltfFailure::internal("glTF source owner directory is invalid")
                        })?;
                    if *entry != usize::MAX {
                        return Err(
                            GltfFailure::internal("glTF source owner was completed twice").into(),
                        );
                    }
                    *entry = index;
                    Ok::<_, ContentError>(())
                })?;
                #[cfg(test)]
                tests::directory_registered();
                if (index + 1) % COMPARISONS_BETWEEN_POLLS as usize == 0 {
                    work.poll()?;
                }
            }
            work.poll()?;
            Ok::<_, ContentError>(())
        })?;
        #[cfg(test)]
        tests::before_directory_publication();
        ctx.poll()?;
        self.directory = Some(directory);
        self.finished = true;
        Ok(())
    }

    pub(super) fn for_owner(
        &self,
        ctx: &mut ParseRoot<'_, '_, '_>,
        owner: ProfileOwner,
    ) -> Result<&[SelectedEntryOrdinal]> {
        let group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
        let range = ctx.child(&group, [1], |child| {
            child.run([1], || {
                self.directory
                    .as_ref()
                    .and_then(|directory| {
                        directory
                            .shape
                            .slot(owner)
                            .and_then(|slot| directory.ranges.as_slice().get(slot))
                    })
                    .and_then(|&index| self.owners.as_slice().get(index))
                    .filter(|range| range.owner == owner)
                    .ok_or_else(|| {
                        ContentError::from(GltfFailure::internal(
                            "glTF canonical owner has no source mapping",
                        ))
                    })
            })
        })?;
        #[cfg(test)]
        tests::directory_query_completed(1);
        ctx.child(&group, [1], |child| {
            child.run([1], || {
                if !self.finished || range.length == 0 {
                    return Err(
                        GltfFailure::internal("glTF source owner directory is incomplete").into(),
                    );
                }
                range
                    .start
                    .checked_add(range.length)
                    .and_then(|end| self.ordinals.as_slice().get(range.start..end))
                    .ok_or_else(|| {
                        GltfFailure::internal("glTF source owner range is invalid").into()
                    })
            })
        })
    }

    pub(super) fn source(
        &self,
        ctx: &mut ParseRoot<'_, '_, '_>,
        ordinal: SelectedEntryOrdinal,
    ) -> Result<&SelectedSource<'scope>> {
        let group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
        ctx.child(&group, [1], |child| {
            child.run([1], || {
                self.entries.as_slice().get(ordinal.0).ok_or_else(|| {
                    GltfFailure::internal("glTF source entry ordinal is invalid").into()
                })
            })
        })
    }

    /// Resource sources retain their existing indices. The document reuses source zero; other
    /// declarations follow the resource sources. Normalized source provenance leaves use this
    /// same order, while metadata explicitly references a SourceIdentityLocalId.
    pub(super) fn source_index(
        &self,
        ctx: &mut ParseRoot<'_, '_, '_>,
        resource_count: usize,
        id: SourceEntityId,
    ) -> Result<u32> {
        let ordinal = self.find_ordinal(ctx, id)?.ok_or_else(|| {
            GltfFailure::internal("glTF source mapping references a missing identity")
        })?;
        self.ordinal_source_index(ctx, resource_count, ordinal)
    }

    pub(super) fn ordinal_source_index(
        &self,
        ctx: &mut ParseRoot<'_, '_, '_>,
        resource_count: usize,
        ordinal: SelectedEntryOrdinal,
    ) -> Result<u32> {
        let group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
        ctx.child(&group, [1], |child| {
            child.run([1], || {
                if ordinal.0 >= self.entries.len() {
                    return Err(
                        GltfFailure::internal("glTF source entry ordinal is invalid").into(),
                    );
                }
                if ordinal.0 == 0 {
                    return Ok(0);
                }
                resource_count
                    .checked_add(ordinal.0)
                    .and_then(|index| index.checked_sub(1))
                    .and_then(|index| u32::try_from(index).ok())
                    .ok_or_else(|| {
                        GltfFailure::internal("glTF source identity count exceeds u32").into()
                    })
            })
        })
    }
}

/// Binary search and partition_point perform at most bit_length(len) + 1 comparisons.
/// Charge that bound before entering the bounded, allocation-free standard-library search.
fn lookup<T>(
    ctx: &mut ParseRoot<'_, '_, '_>,
    length: usize,
    searches: u64,
    search: impl FnOnce() -> T,
) -> Result<T> {
    let comparisons = u64::from(usize::BITS - length.leading_zeros()) + 1;
    let layout = ctx.bind_group([ImportWorkClass::WorkUnits])?;
    ctx.child(&layout, [comparisons * searches], |child| {
        child.run([comparisons * searches], || Ok::<_, ContentError>(search()))
    })
}

fn source_locator<'scope>(
    ctx: &mut ParseServices<'_, '_, 'scope>,
    source: SourceEntity,
) -> Result<SelectedSource<'scope>> {
    let end = source
        .range
        .byte_offset
        .checked_add(source.range.byte_length);
    if source.range.byte_length == 0
        || end.is_none_or(|end| end > super::to_u64(ctx.primary_bytes().len()))
    {
        return Err(gltf_parse_error(
            "selected glTF source range is outside primary bytes",
        ));
    }
    let kind = match source.id.kind {
        SourceEntityKind::Document => "document",
        SourceEntityKind::Scene => "scene",
        SourceEntityKind::Node => "node",
        SourceEntityKind::Mesh => "mesh",
        SourceEntityKind::Primitive => "primitive",
        SourceEntityKind::Material => "material",
        SourceEntityKind::Texture => "texture",
        SourceEntityKind::Image => "image",
        SourceEntityKind::Sampler => "sampler",
        SourceEntityKind::Skin => "skin",
        SourceEntityKind::Buffer => "buffer",
        SourceEntityKind::BufferView => "buffer-view",
        SourceEntityKind::Accessor => "accessor",
        SourceEntityKind::Animation => "animation",
        SourceEntityKind::AnimationChannel => "animation-channel",
        SourceEntityKind::AnimationSampler => "animation-sampler",
        SourceEntityKind::Camera => "camera",
        SourceEntityKind::Light => "light",
    };
    let mut label = ctx.metered_string(
        ImportStorageClass::FormatNormalization,
        "gltf-source-object-locator-label",
    );
    label.try_write_fmt(ctx, format_args!("kind={kind};parent="))?;
    match source.id.parent_index {
        Some(index) => label.try_push_usize(ctx, index)?,
        None => label.try_push_str(ctx, "-")?,
    }
    label.try_write_fmt(
        ctx,
        format_args!(
            ";index={};offset={};length={}",
            source.id.source_index, source.range.byte_offset, source.range.byte_length,
        ),
    )?;
    let plan = CapacityPlan::for_type::<u8>(0, 0, label.len())
        .map_err(|_| ctx.allocation_failure("gltf-source-object-locator"))?;
    let envelope = plan
        .envelope_delta_bytes()
        .map_err(|_| ctx.allocation_failure("gltf-source-object-locator"))?;
    let storage =
        ctx.metered_opaque_allowance(ImportStorageClass::FormatNormalization, envelope)?;
    let value = SourceLocatorValue::new(label.as_str())
        .map_err(|_| ctx.allocation_failure("gltf-source-object-locator"))?;
    plan.finish(0, value.capacity_bytes()).map_err(|_| {
        ctx.format_failure(GltfFailure::internal(
            "gltf-source-object-locator-capacity-envelope",
        ))
    })?;
    let schema = ExtensionSchemaId::new(SOURCE_SCHEMA, 1)
        .map_err(|_| gltf_parse_error("static glTF source locator schema is invalid"))?;
    let locator = SourceObjectLocator::try_namespaced(Some(schema), value)
        .map_err(|_| gltf_parse_error("static glTF source locator schema is missing"))?;
    Ok(SelectedSource {
        entity: source,
        locator,
        _storage: storage,
    })
}

/// Called before decoder teardown. Only source ordinals, raw ranges, and canonical local owner
/// IDs cross this boundary; no JSON or backend object is retained by the normalized model.
pub(super) fn associate<'session>(
    ctx: &mut ParseRoot<'_, '_, 'session>,
    document: &crate::document::Document<'_>,
    prepared: &PreparedDocument<'_, '_>,
    model: &mut NormalizedGltf<'session>,
) -> Result<()> {
    let profile = &mut model.sources;
    let mut ordinary = OwnerSourceRun::new(ctx)?;
    let mut suspended = OwnerSourceRun::new(ctx)?;
    let root = document.tables();
    let scene_source = prepared
        .node_graph
        .selection_outcome
        .selected_scene_index()
        .map_or(DOCUMENT, |index| {
            entity(SourceEntityKind::Scene, None, index)
        });
    profile.link(ctx, &mut ordinary, ProfileOwner::Scene, scene_source)?;
    profile.complete_owner(ctx, &mut ordinary, ProfileOwner::Scene)?;
    profile.link(
        ctx,
        &mut ordinary,
        ProfileOwner::SyntheticRoot,
        scene_source,
    )?;
    profile.complete_owner(ctx, &mut ordinary, ProfileOwner::SyntheticRoot)?;

    let work_group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
    for (index, id) in model.nodes.node_ids.as_slice().iter().enumerate() {
        let id = ctx.child(&work_group, [1], |child| {
            child.run([1], || Ok::<_, baozi_import::ParseFailure>(id.as_ref()))
        })?;
        if let Some(id) = id {
            profile.link(
                ctx,
                &mut ordinary,
                ProfileOwner::Node(*id),
                entity(SourceEntityKind::Node, None, index),
            )?;
            profile.complete_owner(ctx, &mut ordinary, ProfileOwner::Node(*id))?;
        }
    }
    for (index, id) in model.cameras.camera_ids.as_slice().iter().enumerate() {
        let id = ctx.child(&work_group, [1], |child| {
            child.run([1], || Ok::<_, baozi_import::ParseFailure>(id.as_ref()))
        })?;
        if let Some(id) = id {
            profile.link(
                ctx,
                &mut ordinary,
                ProfileOwner::Camera(id.index()),
                entity(SourceEntityKind::Camera, None, index),
            )?;
            profile.complete_owner(ctx, &mut ordinary, ProfileOwner::Camera(id.index()))?;
        }
    }
    for (index, id) in model.lights.light_ids.as_slice().iter().enumerate() {
        let id = ctx.child(&work_group, [1], |child| {
            child.run([1], || Ok::<_, baozi_import::ParseFailure>(id.as_ref()))
        })?;
        if let Some(id) = id {
            profile.link(
                ctx,
                &mut ordinary,
                ProfileOwner::Light(id.index()),
                entity(SourceEntityKind::Light, None, index),
            )?;
            profile.complete_owner(ctx, &mut ordinary, ProfileOwner::Light(id.index()))?;
        }
    }
    for (index, instance) in model.nodes.mesh_instances.as_slice().iter().enumerate() {
        let owner = ProfileOwner::MeshInstance(authoring_u32(
            &mut ctx.services()?,
            index,
            "gltf-source-mesh-instance",
        )?);
        profile.link(
            ctx,
            &mut ordinary,
            owner,
            entity(SourceEntityKind::Node, None, instance.source_node_index),
        )?;
        let mesh = root
            .nodes
            .get(instance.source_node_index)
            .and_then(|node| node.mesh)
            .ok_or_else(|| gltf_parse_error("glTF instance source mesh is missing"))?;
        profile.link(
            ctx,
            &mut ordinary,
            owner,
            entity(SourceEntityKind::Mesh, None, mesh.value()),
        )?;
        profile.complete_owner(ctx, &mut ordinary, owner)?;
    }

    for (index, id) in model.appearance.image_ids.as_slice().iter().enumerate() {
        let id = ctx.child(&work_group, [1], |child| {
            child.run([1], || Ok::<_, baozi_import::ParseFailure>(id.as_ref()))
        })?;
        let Some(id) = id else { continue };
        let owner = ProfileOwner::Image(id.index());
        profile.link(
            ctx,
            &mut ordinary,
            owner,
            entity(SourceEntityKind::Image, None, index),
        )?;
        if let Some(view) = root.images.get(index).and_then(|image| image.buffer_view) {
            link_view(ctx, profile, &mut ordinary, root, owner, view.value())?;
        }
        profile.complete_owner(ctx, &mut ordinary, owner)?;
    }
    for (index, id) in model.appearance.sampler_ids.as_slice().iter().enumerate() {
        let id = ctx.child(&work_group, [1], |child| {
            child.run([1], || Ok::<_, baozi_import::ParseFailure>(id.as_ref()))
        })?;
        if let Some(id) = id {
            profile.link(
                ctx,
                &mut ordinary,
                ProfileOwner::Sampler(id.index()),
                entity(SourceEntityKind::Sampler, None, index),
            )?;
            profile.complete_owner(ctx, &mut ordinary, ProfileOwner::Sampler(id.index()))?;
        }
    }
    for (index, id) in model.appearance.texture_ids.as_slice().iter().enumerate() {
        let id = ctx.child(&work_group, [1], |child| {
            child.run([1], || Ok::<_, baozi_import::ParseFailure>(id.as_ref()))
        })?;
        let Some(id) = id else { continue };
        let source = entity(SourceEntityKind::Texture, None, index);
        let owner = ProfileOwner::Texture(id.index());
        profile.link(ctx, &mut ordinary, owner, source)?;
        if root
            .textures
            .get(index)
            .is_some_and(|texture| texture.sampler.is_none())
        {
            let sampler = model
                .appearance
                .default_sampler
                .ok_or_else(|| gltf_parse_error("glTF implicit sampler source is missing"))?;
            profile.link(
                ctx,
                &mut suspended,
                ProfileOwner::Sampler(sampler.index()),
                source,
            )?;
        }
        profile.complete_owner(ctx, &mut ordinary, owner)?;
    }
    if let Some(sampler) = model.appearance.default_sampler {
        profile.complete_owner(ctx, &mut suspended, ProfileOwner::Sampler(sampler.index()))?;
    }
    if let Some(material) = model.appearance.default_material {
        let owner = ProfileOwner::Material(material.index());
        profile.link(ctx, &mut ordinary, owner, DOCUMENT)?;
        profile.complete_owner(ctx, &mut ordinary, owner)?;
    }
    let mut binding_index = 0;
    for (index, id) in model.appearance.material_ids.as_slice().iter().enumerate() {
        let id = ctx.child(&work_group, [1], |child| {
            child.run([1], || Ok::<_, baozi_import::ParseFailure>(id.as_ref()))
        })?;
        let Some(id) = id else { continue };
        let source = entity(SourceEntityKind::Material, None, index);
        let owner = ProfileOwner::Material(id.index());
        profile.link(ctx, &mut ordinary, owner, source)?;
        profile.complete_owner(ctx, &mut ordinary, owner)?;
        for binding in prepared
            .material_contracts
            .bindings(&mut ctx.services()?, index)?
        {
            let texture = ctx.child(&work_group, [1], |child| {
                child.run([1], || {
                    Ok::<_, baozi_import::ParseFailure>(binding.source_texture)
                })
            })?;
            let owner = ProfileOwner::TextureBinding(authoring_u32(
                &mut ctx.services()?,
                binding_index,
                "gltf-source-binding",
            )?);
            profile.link(ctx, &mut ordinary, owner, source)?;
            profile.link(
                ctx,
                &mut ordinary,
                owner,
                entity(SourceEntityKind::Texture, None, texture),
            )?;
            profile.complete_owner(ctx, &mut ordinary, owner)?;
            binding_index += 1;
        }
    }
    if binding_index != model.appearance.bindings.len() {
        return Err(gltf_parse_error("glTF binding source map is incomplete"));
    }

    let mut primitive_index = 0;
    let mut attribute_index = 0;
    let mut mesh_index = 0;
    let mut definition_index = 0;
    let mut delta_index = 0;
    for (source_index, selected) in model
        .geometry
        .logical_mesh_ids
        .as_slice()
        .iter()
        .enumerate()
    {
        let selected = ctx.child(&work_group, [1], |child| {
            child.run([1], || {
                Ok::<_, baozi_import::ParseFailure>(selected.is_some())
            })
        })?;
        if !selected {
            continue;
        }
        let mesh = root
            .meshes
            .get(source_index)
            .ok_or_else(|| gltf_parse_error("glTF mesh source is missing"))?;
        let normalized = model
            .geometry
            .logical_meshes
            .as_slice()
            .get(mesh_index)
            .ok_or_else(|| gltf_parse_error("glTF normalized mesh is missing"))?;
        let source = entity(SourceEntityKind::Mesh, None, source_index);
        {
            let argument_0 = ProfileOwner::LogicalMesh(authoring_u32(
                &mut ctx.services()?,
                mesh_index,
                "gltf-source-mesh",
            )?);
            profile.link(ctx, &mut ordinary, argument_0, source)?;
            profile.complete_owner(ctx, &mut ordinary, argument_0)
        }?;
        mesh_index += 1;
        let mut width = 0;
        for primitive in &mesh.primitives {
            let targets = ctx.child(&work_group, [1], |child| {
                child.run([1], || {
                    Ok::<_, baozi_import::ParseFailure>(primitive.targets.as_ref())
                })
            })?;
            if let Some(targets) = targets {
                width = targets.len();
                break;
            }
        }
        for _ in 0..width {
            let owner = ProfileOwner::MorphDefinition(authoring_u32(
                &mut ctx.services()?,
                definition_index,
                "gltf-source-morph-definition",
            )?);
            profile.link(ctx, &mut ordinary, owner, source)?;
            profile.complete_owner(ctx, &mut ordinary, owner)?;
            definition_index += 1;
        }
        if mesh.primitives.len() != normalized.primitives.len() {
            return Err(gltf_parse_error("glTF primitive source map is incomplete"));
        }
        for (index, (primitive, normalized)) in mesh
            .primitives
            .iter()
            .zip(normalized.primitives.as_slice())
            .enumerate()
        {
            let source = entity(SourceEntityKind::Primitive, Some(source_index), index);
            let owner = ProfileOwner::Primitive(authoring_u32(
                &mut ctx.services()?,
                primitive_index,
                "gltf-source-primitive",
            )?);
            profile.link(ctx, &mut ordinary, owner, source)?;
            primitive_index += 1;
            // Includes the JOINTS/WEIGHTS inputs retained as influences rather than attributes.
            for accessor in primitive
                .attributes
                .values()
                .copied()
                .chain(primitive.indices)
            {
                link_accessor(ctx, profile, &mut ordinary, root, owner, accessor.value())?;
            }
            profile.complete_owner(ctx, &mut ordinary, owner)?;
            for attribute in normalized.attributes.as_slice() {
                let semantic = match &attribute.semantic {
                    AttributeSemantic::Position => json::mesh::Semantic::Positions,
                    AttributeSemantic::Normal => json::mesh::Semantic::Normals,
                    AttributeSemantic::Tangent => json::mesh::Semantic::Tangents,
                    AttributeSemantic::Texcoord(set) => json::mesh::Semantic::TexCoords(*set),
                    AttributeSemantic::Color(set) => json::mesh::Semantic::Colors(*set),
                    _ => {
                        return Err(gltf_parse_error(
                            "glTF attribute source semantic is missing",
                        ));
                    }
                };
                let accessor = lookup(ctx, primitive.attributes.len(), 1, || {
                    primitive.attributes.get(&semantic)
                })?
                .ok_or_else(|| gltf_parse_error("glTF attribute source accessor is missing"))?;
                let owner = ProfileOwner::Attribute(authoring_u32(
                    &mut ctx.services()?,
                    attribute_index,
                    "gltf-source-attribute",
                )?);
                profile.link(ctx, &mut ordinary, owner, source)?;
                link_accessor(ctx, profile, &mut ordinary, root, owner, accessor.value())?;
                profile.complete_owner(ctx, &mut ordinary, owner)?;
                attribute_index += 1;
            }
            for target in primitive.targets.iter().flatten() {
                let accessors = ctx.child(&work_group, [1], |child| {
                    child.run([1], || {
                        Ok::<_, baozi_import::ParseFailure>([
                            target.positions,
                            target.normals,
                            target.tangents,
                        ])
                    })
                })?;
                for accessor in accessors.into_iter().flatten() {
                    let owner = ProfileOwner::MorphDelta(authoring_u32(
                        &mut ctx.services()?,
                        delta_index,
                        "gltf-source-morph-delta",
                    )?);
                    profile.link(ctx, &mut ordinary, owner, source)?;
                    link_accessor(ctx, profile, &mut ordinary, root, owner, accessor.value())?;
                    profile.complete_owner(ctx, &mut ordinary, owner)?;
                    delta_index += 1;
                }
            }
        }
    }
    if mesh_index != model.geometry.logical_meshes.len()
        || definition_index != model.morphs.definitions.len()
        || delta_index != model.morphs.deltas.len()
    {
        return Err(gltf_parse_error("glTF geometry source map is incomplete"));
    }

    let mut skin_index = 0;
    for (index, selected) in model.skins.skin_ids.as_slice().iter().enumerate() {
        let selected = ctx.child(&work_group, [1], |child| {
            child.run([1], || {
                Ok::<_, baozi_import::ParseFailure>(selected.is_some())
            })
        })?;
        if !selected {
            continue;
        }
        let owner = ProfileOwner::Skin(authoring_u32(
            &mut ctx.services()?,
            skin_index,
            "gltf-source-skin",
        )?);
        skin_index += 1;
        profile.link(
            ctx,
            &mut ordinary,
            owner,
            entity(SourceEntityKind::Skin, None, index),
        )?;
        let skin = root
            .skins
            .get(index)
            .ok_or_else(|| gltf_parse_error("glTF skin source is missing"))?;
        if let Some(accessor) = skin.inverse_bind_matrices {
            link_accessor(ctx, profile, &mut ordinary, root, owner, accessor.value())?;
        }
        for node in skin.joints.iter().copied().chain(skin.skeleton) {
            profile.link(
                ctx,
                &mut ordinary,
                owner,
                entity(SourceEntityKind::Node, None, node.value()),
            )?;
        }
        profile.complete_owner(ctx, &mut ordinary, owner)?;
    }

    let mut clip_index = 0;
    let mut channel_index = 0;
    let mut source_channel_index = 0;
    for (index, animation) in root.animations.iter().enumerate() {
        let selected = ctx.child(&work_group, [1], |child| {
            child.run([1], || {
                Ok::<_, baozi_import::ParseFailure>(
                    prepared
                        .animation_contracts
                        .selected_animations
                        .as_slice()
                        .get(index)
                        .copied(),
                )
            })
        })?;
        let selected = selected
            .ok_or_else(|| gltf_parse_error("glTF animation source selection is missing"))?;
        let owner = ProfileOwner::AnimationClip(authoring_u32(
            &mut ctx.services()?,
            clip_index,
            "gltf-source-animation",
        )?);
        if selected {
            profile.link(
                ctx,
                &mut suspended,
                owner,
                entity(SourceEntityKind::Animation, None, index),
            )?;
            clip_index += 1;
        }
        for (local_channel, channel) in animation.channels.iter().enumerate() {
            let has_target = ctx.child(&work_group, [1], |child| {
                child.run([1], || {
                    Ok::<_, baozi_import::ParseFailure>(channel.target.node.is_some())
                })
            })?;
            let retained = prepared
                .animation_contracts
                .source_channels
                .as_slice()
                .get(source_channel_index)
                .is_some_and(|disposition| disposition.is_retained());
            source_channel_index += 1;
            if !selected || !has_target {
                continue;
            }
            let sampler_id = entity(
                SourceEntityKind::AnimationSampler,
                Some(index),
                channel.sampler.value(),
            );
            let sampler = animation
                .samplers
                .get(channel.sampler.value())
                .ok_or_else(|| gltf_parse_error("glTF animation source sampler is missing"))?;
            // Omitted targets may contribute duration, but never receive a canonical channel owner.
            profile.link(ctx, &mut suspended, owner, sampler_id)?;
            link_accessor(
                ctx,
                profile,
                &mut suspended,
                root,
                owner,
                sampler.input.value(),
            )?;
            if retained {
                let owner = ProfileOwner::AnimationChannel(authoring_u32(
                    &mut ctx.services()?,
                    channel_index,
                    "gltf-source-animation-channel",
                )?);
                channel_index += 1;
                profile.link(
                    ctx,
                    &mut ordinary,
                    owner,
                    entity(
                        SourceEntityKind::AnimationChannel,
                        Some(index),
                        local_channel,
                    ),
                )?;
                profile.link(ctx, &mut ordinary, owner, sampler_id)?;
                link_accessor(
                    ctx,
                    profile,
                    &mut ordinary,
                    root,
                    owner,
                    sampler.input.value(),
                )?;
                link_accessor(
                    ctx,
                    profile,
                    &mut ordinary,
                    root,
                    owner,
                    sampler.output.value(),
                )?;
                profile.complete_owner(ctx, &mut ordinary, owner)?;
            }
        }
        if selected {
            profile.complete_owner(ctx, &mut suspended, owner)?;
        }
    }
    if clip_index != model.animations.clip_count()
        || channel_index != model.animations.channel_count()
        || source_channel_index != prepared.animation_contracts.source_channels.len()
    {
        return Err(gltf_parse_error("glTF animation source map is incomplete"));
    }
    profile.finish(ctx, &ordinary, &suspended)
}

fn link_accessor<'session>(
    ctx: &mut ParseRoot<'_, '_, 'session>,
    profile: &SelectedSourceProfile<'session>,
    run: &mut OwnerSourceRun<'session>,
    root: &json::Tables<'_>,
    owner: ProfileOwner,
    index: usize,
) -> Result<()> {
    profile.link(
        ctx,
        run,
        owner,
        entity(SourceEntityKind::Accessor, None, index),
    )?;
    let accessor = root
        .accessors
        .get(index)
        .ok_or_else(|| gltf_parse_error("glTF source accessor is missing"))?;
    if let Some(view) = accessor.buffer_view {
        link_view(ctx, profile, run, root, owner, view.value())?;
    }
    if let Some(sparse) = &accessor.sparse {
        link_view(
            ctx,
            profile,
            run,
            root,
            owner,
            sparse.indices.buffer_view.value(),
        )?;
        link_view(
            ctx,
            profile,
            run,
            root,
            owner,
            sparse.values.buffer_view.value(),
        )?;
    }
    Ok(())
}

fn link_view<'session>(
    ctx: &mut ParseRoot<'_, '_, 'session>,
    profile: &SelectedSourceProfile<'session>,
    run: &mut OwnerSourceRun<'session>,
    root: &json::Tables<'_>,
    owner: ProfileOwner,
    index: usize,
) -> Result<()> {
    profile.link(
        ctx,
        run,
        owner,
        entity(SourceEntityKind::BufferView, None, index),
    )?;
    let view = root
        .buffer_views
        .get(index)
        .ok_or_else(|| gltf_parse_error("glTF source buffer view is missing"))?;
    profile.link(
        ctx,
        run,
        owner,
        entity(SourceEntityKind::Buffer, None, view.buffer.value()),
    )
}
