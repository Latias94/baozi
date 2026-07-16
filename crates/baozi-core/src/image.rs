//! Canonical encoded images, sampler state, and texture resource pairings.
//!
//! Images own their encoded bytes independently from import-session resources.
//! Samplers contain only sampling behavior, while textures contain only typed
//! scene-local image and sampler indexes plus provenance. Material role and
//! color interpretation deliberately live outside this module.

use crate::extension::{ExtensionMediaType, ExtensionMediaTypeError};
use crate::identity::{
    ImageId, ImageIndex, SamplerId, SamplerIndex, SceneGenerationId, SceneLookupError,
    validate_storage_index,
};
use crate::provenance::{ContentDigest, Provenance};
use baozi_operation::CapacityPlan;
use std::fmt;
use std::num::NonZeroU32;
use thiserror::Error;

/// Maximum encoded bytes owned by one canonical image.
pub const MAX_ENCODED_IMAGE_BYTES: usize = 256 * 1024 * 1024;

/// Maximum encoded bytes in one canonical image-encoding hint.
pub const MAX_IMAGE_ENCODING_HINT_BYTES: usize = 32;

/// Maximum value accepted for one declared encoded-image dimension.
///
/// This bound retains very large source metadata while preventing absurd
/// decompression claims from entering the canonical model unchecked.
pub const MAX_IMAGE_DIMENSION: u32 = 1_048_576;

/// A bounded lowercase ASCII hint for an encoded image representation.
///
/// Examples include `png`, `jpeg`, `ktx2`, and `openexr`. The hint is
/// descriptive only; it neither decodes bytes nor overrides the media type.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ImageEncodingHint {
    bytes: [u8; MAX_IMAGE_ENCODING_HINT_BYTES],
    length: u8,
}

impl ImageEncodingHint {
    /// Validates and stores a canonical encoding hint without allocation.
    pub fn new(value: impl AsRef<str>) -> Result<Self, ImageEncodingHintError> {
        let value = value.as_ref();
        validate_encoding_hint(value)?;

        let mut bytes = [0; MAX_IMAGE_ENCODING_HINT_BYTES];
        bytes[..value.len()].copy_from_slice(value.as_bytes());
        let length = u8::try_from(value.len()).map_err(|_| ImageEncodingHintError::TooLong {
            actual: value.len(),
            maximum: MAX_IMAGE_ENCODING_HINT_BYTES,
        })?;
        Ok(Self { bytes, length })
    }

    /// Returns the canonical encoding hint.
    pub fn as_str(&self) -> &str {
        let length = usize::from(self.length);
        std::str::from_utf8(&self.bytes[..length])
            .expect("ImageEncodingHint construction accepts only ASCII bytes")
    }
}

impl AsRef<str> for ImageEncodingHint {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Debug for ImageEncodingHint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ImageEncodingHint")
            .field(&self.as_str())
            .finish()
    }
}

impl fmt::Display for ImageEncodingHint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Why an image-encoding hint was rejected.
///
/// Rejected text is never retained or echoed.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum ImageEncodingHintError {
    #[error("image encoding hint is empty")]
    Empty,

    #[error("image encoding hint is {actual} bytes; the maximum is {maximum}")]
    TooLong { actual: usize, maximum: usize },

    #[error("image encoding hint contains a non-canonical byte at offset {offset}")]
    InvalidByte { offset: usize },
}

fn validate_encoding_hint(value: &str) -> Result<(), ImageEncodingHintError> {
    if value.is_empty() {
        return Err(ImageEncodingHintError::Empty);
    }
    if value.len() > MAX_IMAGE_ENCODING_HINT_BYTES {
        return Err(ImageEncodingHintError::TooLong {
            actual: value.len(),
            maximum: MAX_IMAGE_ENCODING_HINT_BYTES,
        });
    }

    for (offset, byte) in value.bytes().enumerate() {
        let valid = byte.is_ascii_lowercase()
            || byte.is_ascii_digit()
            || matches!(byte, b'+' | b'-' | b'.' | b'_');
        if !valid {
            return Err(ImageEncodingHintError::InvalidByte { offset });
        }
    }

    let first = value.as_bytes()[0];
    let last = value.as_bytes()[value.len() - 1];
    if !first.is_ascii_alphanumeric() {
        return Err(ImageEncodingHintError::InvalidByte { offset: 0 });
    }
    if !last.is_ascii_alphanumeric() {
        return Err(ImageEncodingHintError::InvalidByte {
            offset: value.len() - 1,
        });
    }

    Ok(())
}

/// One axis reported by encoded image metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageDimensionAxis {
    Width,
    Height,
}

/// Validated two-dimensional encoded-image metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ImageDimensions {
    width: NonZeroU32,
    height: NonZeroU32,
}

impl ImageDimensions {
    /// Validates non-zero bounded dimensions.
    pub fn new(width: u32, height: u32) -> Result<Self, ImageDimensionsError> {
        let width = validate_dimension(ImageDimensionAxis::Width, width)?;
        let height = validate_dimension(ImageDimensionAxis::Height, height)?;
        Ok(Self { width, height })
    }

    /// Returns the declared encoded width.
    pub const fn width(self) -> NonZeroU32 {
        self.width
    }

    /// Returns the declared encoded height.
    pub const fn height(self) -> NonZeroU32 {
        self.height
    }
}

fn validate_dimension(
    axis: ImageDimensionAxis,
    value: u32,
) -> Result<NonZeroU32, ImageDimensionsError> {
    let value = NonZeroU32::new(value).ok_or(ImageDimensionsError::Zero { axis })?;
    if value.get() > MAX_IMAGE_DIMENSION {
        return Err(ImageDimensionsError::TooLarge {
            axis,
            actual: value.get(),
            maximum: MAX_IMAGE_DIMENSION,
        });
    }
    Ok(value)
}

/// Why encoded image dimensions were rejected.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum ImageDimensionsError {
    #[error("image {axis:?} must be non-zero")]
    Zero { axis: ImageDimensionAxis },

    #[error("image {axis:?} is {actual}; the maximum is {maximum}")]
    TooLarge {
        axis: ImageDimensionAxis,
        actual: u32,
        maximum: u32,
    },
}

/// Immutable owned bytes and metadata for one encoded image.
#[derive(PartialEq, Eq)]
pub struct EncodedImagePayload {
    bytes: Vec<u8>,
    media_type: Option<ExtensionMediaType>,
    encoding_hint: Option<ImageEncodingHint>,
    dimensions: Option<ImageDimensions>,
    content_digest: Option<ContentDigest>,
}

impl EncodedImagePayload {
    /// Fallibly copies one bounded encoded payload after validating its hints.
    pub fn try_new(
        bytes: impl AsRef<[u8]>,
        media_type: Option<&str>,
        encoding_hint: Option<&str>,
        dimensions: Option<ImageDimensions>,
        content_digest: Option<ContentDigest>,
    ) -> Result<Self, ImageBuildError> {
        let media_type = media_type
            .map(ExtensionMediaType::new)
            .transpose()
            .map_err(ImageBuildError::InvalidMediaType)?;
        let encoding_hint = encoding_hint
            .map(ImageEncodingHint::new)
            .transpose()
            .map_err(ImageBuildError::InvalidEncodingHint)?;
        Self::try_new_with_hints(bytes, media_type, encoding_hint, dimensions, content_digest)
    }

    /// Fallibly copies one bounded payload with prevalidated hints.
    pub fn try_new_with_hints(
        bytes: impl AsRef<[u8]>,
        media_type: Option<ExtensionMediaType>,
        encoding_hint: Option<ImageEncodingHint>,
        dimensions: Option<ImageDimensions>,
        content_digest: Option<ContentDigest>,
    ) -> Result<Self, ImageBuildError> {
        let bytes = try_copy_encoded_bytes(bytes.as_ref())?;
        Ok(Self {
            bytes,
            media_type,
            encoding_hint,
            dimensions,
            content_digest,
        })
    }

    /// Borrows the scene-owned encoded bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the optional canonical media type.
    pub const fn media_type(&self) -> Option<&ExtensionMediaType> {
        self.media_type.as_ref()
    }

    /// Returns the optional encoding hint.
    pub const fn encoding_hint(&self) -> Option<&ImageEncodingHint> {
        self.encoding_hint.as_ref()
    }

    /// Returns optional encoded dimensions.
    pub const fn dimensions(&self) -> Option<ImageDimensions> {
        self.dimensions
    }

    /// Returns the optional digest of the exact encoded bytes.
    pub const fn content_digest(&self) -> Option<&ContentDigest> {
        self.content_digest.as_ref()
    }

    /// Returns the number of byte cells retained by canonical heap storage.
    pub(crate) fn retained_cell_capacity(&self) -> usize {
        self.bytes.capacity()
    }

    /// Returns the bytes retained by canonical heap storage.
    ///
    /// Media type, encoding hint, dimensions, and digest storage are fixed
    /// inline values and therefore add no heap capacity here.
    pub(crate) fn retained_allocation_bytes(&self) -> usize {
        self.retained_cell_capacity()
    }

    /// Returns an allocation-free read facade.
    pub const fn view(&self) -> EncodedImageView<'_> {
        EncodedImageView { payload: self }
    }
}

impl fmt::Debug for EncodedImagePayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EncodedImagePayload")
            .field("byte_len", &self.bytes.len())
            .field("media_type", &self.media_type)
            .field("encoding_hint", &self.encoding_hint)
            .field("dimensions", &self.dimensions)
            .field("content_digest", &self.content_digest)
            .finish()
    }
}

/// The owned payload of one canonical image.
#[derive(Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ImagePayload {
    Encoded(EncodedImagePayload),
}

impl ImagePayload {
    pub(crate) fn retained_cell_capacity(&self) -> usize {
        match self {
            Self::Encoded(payload) => payload.retained_cell_capacity(),
        }
    }

    pub(crate) fn retained_allocation_bytes(&self) -> usize {
        match self {
            Self::Encoded(payload) => payload.retained_allocation_bytes(),
        }
    }

    /// Returns an allocation-free read facade.
    pub const fn view(&self) -> ImagePayloadView<'_> {
        match self {
            Self::Encoded(payload) => ImagePayloadView::Encoded(payload.view()),
        }
    }
}

/// One canonical image with mandatory provenance.
#[derive(Debug, PartialEq, Eq)]
pub struct Image {
    payload: ImagePayload,
    provenance: Provenance,
}

impl Image {
    /// Fallibly copies one bounded encoded image and validates text hints.
    pub fn try_encoded(
        bytes: impl AsRef<[u8]>,
        media_type: Option<&str>,
        encoding_hint: Option<&str>,
        dimensions: Option<ImageDimensions>,
        content_digest: Option<ContentDigest>,
        provenance: Provenance,
    ) -> Result<Self, ImageBuildError> {
        let payload = EncodedImagePayload::try_new(
            bytes,
            media_type,
            encoding_hint,
            dimensions,
            content_digest,
        )?;
        Ok(Self::from_encoded(payload, provenance))
    }

    /// Owns a previously validated encoded payload.
    pub const fn from_encoded(payload: EncodedImagePayload, provenance: Provenance) -> Self {
        Self {
            payload: ImagePayload::Encoded(payload),
            provenance,
        }
    }

    /// Returns the immutable payload.
    pub const fn payload(&self) -> &ImagePayload {
        &self.payload
    }

    /// Returns the mandatory provenance attachment.
    pub const fn provenance(&self) -> Provenance {
        self.provenance
    }

    pub(crate) fn retained_cell_capacity(&self) -> usize {
        self.payload.retained_cell_capacity()
    }

    pub(crate) fn retained_allocation_bytes(&self) -> usize {
        self.payload.retained_allocation_bytes()
    }

    /// Returns an allocation-free read facade.
    pub const fn view(&self) -> ImageView<'_> {
        ImageView { image: self }
    }
}

/// A borrowed view of one scene-owned encoded payload.
#[derive(Debug, Clone, Copy)]
pub struct EncodedImageView<'a> {
    payload: &'a EncodedImagePayload,
}

impl<'a> EncodedImageView<'a> {
    /// Borrows bytes owned by the published scene.
    pub fn bytes(self) -> &'a [u8] {
        self.payload.bytes()
    }

    /// Returns the optional canonical media type.
    pub const fn media_type(self) -> Option<&'a ExtensionMediaType> {
        self.payload.media_type()
    }

    /// Returns the optional encoding hint.
    pub const fn encoding_hint(self) -> Option<&'a ImageEncodingHint> {
        self.payload.encoding_hint()
    }

    /// Returns optional encoded dimensions.
    pub const fn dimensions(self) -> Option<ImageDimensions> {
        self.payload.dimensions()
    }

    /// Returns the optional digest of the exact encoded bytes.
    pub const fn content_digest(self) -> Option<&'a ContentDigest> {
        self.payload.content_digest()
    }
}

/// A borrowed view of one image payload.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum ImagePayloadView<'a> {
    Encoded(EncodedImageView<'a>),
}

/// A borrowed view of one canonical image.
#[derive(Debug, Clone, Copy)]
pub struct ImageView<'a> {
    image: &'a Image,
}

impl<'a> ImageView<'a> {
    /// Returns the image payload view.
    pub const fn payload(self) -> ImagePayloadView<'a> {
        self.image.payload.view()
    }

    /// Returns the mandatory provenance attachment.
    pub const fn provenance(self) -> Provenance {
        self.image.provenance
    }

    /// Returns the encoded payload view.
    pub const fn encoded(self) -> EncodedImageView<'a> {
        match &self.image.payload {
            ImagePayload::Encoded(payload) => payload.view(),
        }
    }
}

/// Why an image payload could not enter canonical storage.
///
/// Errors contain only categories, sizes, and offsets. Rejected text and bytes
/// are never retained or echoed.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum ImageBuildError {
    #[error("encoded image payload is empty")]
    EmptyEncodedPayload,

    #[error("encoded image payload is {actual} bytes; the maximum is {maximum}")]
    EncodedPayloadTooLong { actual: usize, maximum: usize },

    #[error(transparent)]
    InvalidMediaType(ExtensionMediaTypeError),

    #[error(transparent)]
    InvalidEncodingHint(ImageEncodingHintError),

    #[error("image allocation failed for {resource:?}")]
    AllocationFailed { resource: ImageAllocation },
}

/// Heap-backed image resource whose fallible allocation failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageAllocation {
    EncodedPayload,
}

fn validate_encoded_byte_len(length: usize) -> Result<(), ImageBuildError> {
    if length == 0 {
        return Err(ImageBuildError::EmptyEncodedPayload);
    }
    if length > MAX_ENCODED_IMAGE_BYTES {
        return Err(ImageBuildError::EncodedPayloadTooLong {
            actual: length,
            maximum: MAX_ENCODED_IMAGE_BYTES,
        });
    }
    Ok(())
}

fn try_copy_encoded_bytes(bytes: &[u8]) -> Result<Vec<u8>, ImageBuildError> {
    validate_encoded_byte_len(bytes.len())?;
    let mut owned = Vec::new();
    try_reserve_encoded_bytes(&mut owned, bytes.len())?;
    owned.extend_from_slice(bytes);
    Ok(owned)
}

fn try_reserve_encoded_bytes(
    bytes: &mut Vec<u8>,
    additional: usize,
) -> Result<(), ImageBuildError> {
    let desired = bytes
        .len()
        .checked_add(additional)
        .ok_or(ImageBuildError::AllocationFailed {
            resource: ImageAllocation::EncodedPayload,
        })?;
    let plan =
        CapacityPlan::for_type::<u8>(bytes.len(), bytes.capacity(), desired).map_err(|_| {
            ImageBuildError::AllocationFailed {
                resource: ImageAllocation::EncodedPayload,
            }
        })?;
    if desired > bytes.capacity() {
        bytes
            .try_reserve_exact(plan.allocator_additional())
            .map_err(|_| ImageBuildError::AllocationFailed {
                resource: ImageAllocation::EncodedPayload,
            })?;
    }
    plan.finish(bytes.len(), bytes.capacity())
        .map_err(|_| ImageBuildError::AllocationFailed {
            resource: ImageAllocation::EncodedPayload,
        })?;
    Ok(())
}

/// Magnification or minification filtering within one mip level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TexelFilter {
    Nearest,
    Linear,
}

/// Filtering between mip levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MipmapFilter {
    None,
    Nearest,
    Linear,
}

/// Addressing behavior for one texture coordinate axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TextureAddressMode {
    Repeat,
    MirroredRepeat,
    ClampToEdge,
    ClampToBorder,
}

/// Provenance-free sampler behavior used inside a canonical sampler owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SamplerState {
    minification: TexelFilter,
    magnification: TexelFilter,
    mipmap: MipmapFilter,
    address_u: TextureAddressMode,
    address_v: TextureAddressMode,
    address_w: TextureAddressMode,
}

impl SamplerState {
    /// The canonical behavior used when a source omits sampler state.
    pub const CANONICAL_DEFAULT: Self = Self {
        minification: TexelFilter::Linear,
        magnification: TexelFilter::Linear,
        mipmap: MipmapFilter::Linear,
        address_u: TextureAddressMode::Repeat,
        address_v: TextureAddressMode::Repeat,
        address_w: TextureAddressMode::Repeat,
    };

    /// Creates explicit independent filter and address state.
    pub const fn new(
        minification: TexelFilter,
        magnification: TexelFilter,
        mipmap: MipmapFilter,
        address_u: TextureAddressMode,
        address_v: TextureAddressMode,
        address_w: TextureAddressMode,
    ) -> Self {
        Self {
            minification,
            magnification,
            mipmap,
            address_u,
            address_v,
            address_w,
        }
    }

    pub const fn minification(self) -> TexelFilter {
        self.minification
    }

    pub const fn magnification(self) -> TexelFilter {
        self.magnification
    }

    pub const fn mipmap(self) -> MipmapFilter {
        self.mipmap
    }

    pub const fn address_u(self) -> TextureAddressMode {
        self.address_u
    }

    pub const fn address_v(self) -> TextureAddressMode {
        self.address_v
    }

    pub const fn address_w(self) -> TextureAddressMode {
        self.address_w
    }
}

/// One canonical sampler with mandatory source or generated provenance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Sampler {
    state: SamplerState,
    provenance: Provenance,
}

impl Sampler {
    /// Creates a sampler from explicit state and mandatory provenance.
    pub const fn new(state: SamplerState, provenance: Provenance) -> Self {
        Self { state, provenance }
    }

    /// Creates the explicit canonical default with mandatory default-operation
    /// provenance.
    ///
    /// The scene validator remains responsible for resolving the attachment
    /// and proving that generated defaults use a valid arena node.
    pub const fn canonical_default(provenance: Provenance) -> Self {
        Self::new(SamplerState::CANONICAL_DEFAULT, provenance)
    }

    /// Returns independent sampler behavior.
    pub const fn state(self) -> SamplerState {
        self.state
    }

    /// Returns the mandatory provenance attachment.
    pub const fn provenance(self) -> Provenance {
        self.provenance
    }

    /// Returns an allocation-free read facade.
    pub const fn view(&self) -> SamplerView<'_> {
        SamplerView { sampler: self }
    }
}

/// A borrowed view of one canonical sampler.
#[derive(Debug, Clone, Copy)]
pub struct SamplerView<'a> {
    sampler: &'a Sampler,
}

impl SamplerView<'_> {
    pub const fn state(self) -> SamplerState {
        self.sampler.state
    }

    pub const fn minification(self) -> TexelFilter {
        self.sampler.state.minification()
    }

    pub const fn magnification(self) -> TexelFilter {
        self.sampler.state.magnification()
    }

    pub const fn mipmap(self) -> MipmapFilter {
        self.sampler.state.mipmap()
    }

    pub const fn address_u(self) -> TextureAddressMode {
        self.sampler.state.address_u()
    }

    pub const fn address_v(self) -> TextureAddressMode {
        self.sampler.state.address_v()
    }

    pub const fn address_w(self) -> TextureAddressMode {
        self.sampler.state.address_w()
    }

    pub const fn provenance(self) -> Provenance {
        self.sampler.provenance
    }
}

/// One scene-local image/sampler pairing with no usage interpretation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Texture {
    image: ImageIndex,
    sampler: SamplerIndex,
    provenance: Provenance,
}

/// Generation-qualified input used to assemble one canonical texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct CanonicalTextureDescriptor {
    image: ImageId,
    sampler: SamplerId,
    provenance: Provenance,
}

impl CanonicalTextureDescriptor {
    pub(crate) const fn new(image: ImageId, sampler: SamplerId, provenance: Provenance) -> Self {
        Self {
            image,
            sampler,
            provenance,
        }
    }

    pub(crate) fn resolve(
        self,
        generation: SceneGenerationId,
        image_count: usize,
        sampler_count: usize,
    ) -> Result<Texture, SceneLookupError> {
        let image = ImageIndex::from_handle(generation, self.image)?;
        let sampler = SamplerIndex::from_handle(generation, self.sampler)?;
        let image = validate_storage_index(image, image_count)?;
        let sampler = validate_storage_index(sampler, sampler_count)?;
        Ok(Texture::from_resolved_references(
            image,
            sampler,
            self.provenance,
        ))
    }
}

impl Texture {
    const fn from_resolved_references(
        image: ImageIndex,
        sampler: SamplerIndex,
        provenance: Provenance,
    ) -> Self {
        Self {
            image,
            sampler,
            provenance,
        }
    }

    #[cfg(test)]
    pub(crate) const fn from_unchecked_references_for_test(
        image: ImageIndex,
        sampler: SamplerIndex,
        provenance: Provenance,
    ) -> Self {
        Self::from_resolved_references(image, sampler, provenance)
    }

    pub(crate) const fn image_index(self) -> ImageIndex {
        self.image
    }

    pub(crate) const fn sampler_index(self) -> SamplerIndex {
        self.sampler
    }

    /// Returns the mandatory provenance attachment.
    pub const fn provenance(self) -> Provenance {
        self.provenance
    }

    /// Qualifies the stored private indexes with the owning generation.
    pub(crate) const fn view(self, generation: SceneGenerationId) -> TextureView {
        TextureView {
            image: self.image.handle(generation),
            sampler: self.sampler.handle(generation),
            provenance: self.provenance,
        }
    }
}

/// A read-only texture pairing with generation-qualified public handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextureView {
    image: ImageId,
    sampler: SamplerId,
    provenance: Provenance,
}

impl TextureView {
    /// Returns the image handle in the owning scene generation.
    pub const fn image(self) -> ImageId {
        self.image
    }

    /// Returns the sampler handle in the owning scene generation.
    pub const fn sampler(self) -> SamplerId {
        self.sampler
    }

    /// Returns the mandatory provenance attachment.
    pub const fn provenance(self) -> Provenance {
        self.provenance
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::SceneStorageIndex;
    use crate::provenance::{
        OperationIdentity, OperationKind, OptionsDigest, ProvenanceArenaBuilder,
        ProvenanceNodeView, test_provenance_fixture,
    };

    #[test]
    fn encoded_payload_outlives_its_input_storage() {
        let (_arena, provenance) = test_provenance_fixture();
        let image = {
            let input = vec![0x89, b'P', b'N', b'G'];
            Image::try_encoded(
                &input,
                Some("image/png"),
                Some("png"),
                Some(ImageDimensions::new(16, 8).unwrap()),
                Some(ContentDigest::sha256([7; 32])),
                provenance,
            )
            .unwrap()
        };

        let encoded = image.view().encoded();
        assert_eq!(encoded.bytes(), [0x89, b'P', b'N', b'G']);
        assert_eq!(encoded.media_type().unwrap().as_str(), "image/png");
        assert_eq!(encoded.encoding_hint().unwrap().as_str(), "png");
        assert_eq!(encoded.dimensions().unwrap().width().get(), 16);
        assert_eq!(encoded.content_digest().unwrap().as_bytes(), &[7; 32]);
    }

    #[test]
    fn encoded_payload_reports_destination_capacity_without_inheriting_source_spare() {
        let (_arena, provenance) = test_provenance_fixture();
        let mut source = Vec::with_capacity(4_096);
        source.extend_from_slice(&[0x89, b'P', b'N', b'G']);
        let source_capacity = source.capacity();

        let compact = Image::try_encoded(
            &source,
            Some("image/png"),
            Some("png"),
            None,
            None,
            provenance,
        )
        .unwrap();
        assert!(compact.retained_allocation_bytes() < source_capacity);
        assert_eq!(
            compact.retained_cell_capacity(),
            compact.retained_allocation_bytes()
        );

        let mut destination = Vec::with_capacity(128);
        destination.extend_from_slice(&[1, 2, 3, 4]);
        let destination_capacity = destination.capacity();
        let encoded = EncodedImagePayload {
            bytes: destination,
            media_type: Some(ExtensionMediaType::new("image/png").unwrap()),
            encoding_hint: Some(ImageEncodingHint::new("png").unwrap()),
            dimensions: None,
            content_digest: None,
        };
        assert!(destination_capacity > encoded.bytes().len());
        assert_eq!(encoded.retained_cell_capacity(), destination_capacity);
        assert_eq!(encoded.retained_allocation_bytes(), destination_capacity);

        let image = Image::from_encoded(encoded, provenance);
        assert_eq!(
            image.payload().retained_cell_capacity(),
            destination_capacity
        );
        assert_eq!(
            image.payload().retained_allocation_bytes(),
            destination_capacity
        );
        assert_eq!(image.retained_cell_capacity(), destination_capacity);
        assert_eq!(image.retained_allocation_bytes(), destination_capacity);
    }

    #[test]
    fn one_image_can_back_multiple_sampler_texture_pairs() {
        let (_arena, provenance) = test_provenance_fixture();
        let image = Image::try_encoded(
            [1, 2, 3, 4],
            Some("image/png"),
            Some("png"),
            None,
            None,
            provenance,
        )
        .unwrap();
        let image_bytes = image.view().encoded().bytes();

        let first_sampler = Sampler::canonical_default(provenance);
        let second_sampler = Sampler::new(
            SamplerState::new(
                TexelFilter::Nearest,
                TexelFilter::Nearest,
                MipmapFilter::None,
                TextureAddressMode::ClampToEdge,
                TextureAddressMode::ClampToEdge,
                TextureAddressMode::ClampToEdge,
            ),
            provenance,
        );
        let first_texture = Texture::from_unchecked_references_for_test(
            ImageIndex::from_u32(0),
            SamplerIndex::from_u32(0),
            provenance,
        );
        let second_texture = Texture::from_unchecked_references_for_test(
            ImageIndex::from_u32(0),
            SamplerIndex::from_u32(1),
            provenance,
        );
        let generation = SceneGenerationId::allocate().unwrap();

        let first = first_texture.view(generation);
        let second = second_texture.view(generation);
        assert_eq!(first.image(), second.image());
        assert_ne!(first.sampler(), second.sampler());
        assert_eq!(
            image_bytes.as_ptr(),
            image.view().encoded().bytes().as_ptr()
        );
        assert_eq!(first_sampler.view().mipmap(), MipmapFilter::Linear);
        assert_eq!(second_sampler.view().mipmap(), MipmapFilter::None);
    }

    #[test]
    fn canonical_default_sampler_retains_default_operation_provenance() {
        let mut builder = ProvenanceArenaBuilder::try_new().unwrap();
        let provenance = builder
            .add_operation(
                OperationIdentity::without_evidence(
                    OperationKind::Default,
                    OptionsDigest::sha256([0; 32]),
                ),
                std::iter::empty::<Provenance>(),
            )
            .unwrap();
        let arena = builder.finish().unwrap();
        let sampler = Sampler::canonical_default(provenance);

        let ProvenanceNodeView::Operation(operation) =
            arena.view(&sampler.provenance()).unwrap().node()
        else {
            panic!("default sampler provenance must be an operation");
        };
        assert_eq!(operation.identity().kind(), &OperationKind::Default);
        assert_eq!(sampler.state(), SamplerState::CANONICAL_DEFAULT);
    }

    #[test]
    fn dimensions_reject_zero_and_excess_values() {
        assert_eq!(
            ImageDimensions::new(0, 1),
            Err(ImageDimensionsError::Zero {
                axis: ImageDimensionAxis::Width,
            })
        );
        assert_eq!(
            ImageDimensions::new(1, MAX_IMAGE_DIMENSION + 1),
            Err(ImageDimensionsError::TooLarge {
                axis: ImageDimensionAxis::Height,
                actual: MAX_IMAGE_DIMENSION + 1,
                maximum: MAX_IMAGE_DIMENSION,
            })
        );
    }

    #[test]
    fn rejects_invalid_media_and_encoding_without_echoing_hostile_text() {
        let (_arena, provenance) = test_provenance_fixture();
        let hostile_media = "Image/private-secret";
        let media_error =
            Image::try_encoded([1], Some(hostile_media), None, None, None, provenance).unwrap_err();
        assert!(matches!(
            media_error,
            ImageBuildError::InvalidMediaType(ExtensionMediaTypeError::InvalidByte { .. })
        ));
        assert!(!media_error.to_string().contains(hostile_media));

        let hostile_encoding = "PNG/private-secret";
        let encoding_error =
            Image::try_encoded([1], None, Some(hostile_encoding), None, None, provenance)
                .unwrap_err();
        assert!(matches!(
            encoding_error,
            ImageBuildError::InvalidEncodingHint(ImageEncodingHintError::InvalidByte { .. })
        ));
        assert!(!encoding_error.to_string().contains(hostile_encoding));
    }

    #[test]
    fn encoded_payload_limits_and_allocation_failures_are_typed() {
        assert_eq!(
            validate_encoded_byte_len(0),
            Err(ImageBuildError::EmptyEncodedPayload)
        );
        assert_eq!(
            validate_encoded_byte_len(MAX_ENCODED_IMAGE_BYTES + 1),
            Err(ImageBuildError::EncodedPayloadTooLong {
                actual: MAX_ENCODED_IMAGE_BYTES + 1,
                maximum: MAX_ENCODED_IMAGE_BYTES,
            })
        );

        let mut bytes = Vec::new();
        assert_eq!(
            try_reserve_encoded_bytes(&mut bytes, usize::MAX),
            Err(ImageBuildError::AllocationFailed {
                resource: ImageAllocation::EncodedPayload,
            })
        );
        assert!(bytes.is_empty());
    }

    #[test]
    fn texture_storage_uses_independent_typed_indexes() {
        let (_arena, provenance) = test_provenance_fixture();
        let texture = Texture::from_unchecked_references_for_test(
            ImageIndex::try_from(12usize).unwrap(),
            SamplerIndex::try_from(7usize).unwrap(),
            provenance,
        );
        assert_eq!(texture.image_index().try_to_usize().unwrap(), 12);
        assert_eq!(texture.sampler_index().try_to_usize().unwrap(), 7);

        let generation = SceneGenerationId::allocate().unwrap();
        let view = texture.view(generation);
        assert_eq!(view.image().generation(), generation);
        assert_eq!(view.sampler().generation(), generation);
        assert_eq!(view.image().index(), 12);
        assert_eq!(view.sampler().index(), 7);
    }
}
