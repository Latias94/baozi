use super::{ContentError, Result, enforce_usize_limit, to_u64};
use crate::GltfFailure;
use baozi_import::{
    FormatFailureDetailSink, FormatFailureEvidenceProducer, ImportStorageClass, ImportWorkClass,
    MeteredVec, ParseScope,
};
use std::fmt;

/// A canonical logical label and the allowance for its actual backing allocation.
pub(super) struct ProfileIdentity<'scope> {
    value: baozi_import::authoring::ResourceIdentity,
    _storage: baozi_import::MeteredOpaqueAllowance<'scope>,
}

impl ProfileIdentity<'_> {
    pub(super) fn value(&self) -> &baozi_import::authoring::ResourceIdentity {
        &self.value
    }
}

/// Resource identities are logical labels, not URIs. Escape literal delimiters reversibly so
/// a decoded filename containing '#' or '?' does not acquire URI fragment/query semantics.
pub(super) fn profile_identity<'scope>(
    ctx: &'scope ParseScope<'_>,
    path: &str,
) -> Result<ProfileIdentity<'scope>> {
    use baozi_import::CapacityPlan;
    use baozi_import::authoring::ResourceIdentity;

    let mut label = ctx.metered_string(
        ImportStorageClass::FormatNormalization,
        "gltf-profile-identity-label",
    );
    ctx.with_work(
        ImportWorkClass::WorkUnits,
        to_u64(path.len()),
        |work| -> Result<()> {
            let mut start = 0;
            let mut next_poll = 0;
            for (offset, character) in path.char_indices() {
                if offset >= next_poll {
                    work.poll()?;
                    next_poll = offset.saturating_add(super::PARSER_POLL_INTERVAL);
                }
                let escape = match character {
                    '%' => "%25",
                    '#' => "%23",
                    '?' => "%3F",
                    _ => continue,
                };
                label.try_push_str(&path[start..offset])?;
                label.try_push_str(escape)?;
                start = offset + character.len_utf8();
            }
            label.try_push_str(&path[start..])?;
            work.complete_all()?;
            Ok(())
        },
    )?;
    let plan = CapacityPlan::for_type::<u8>(0, 0, label.len())
        .map_err(|_| ctx.allocation_failure("gltf-profile-identity"))?;
    let envelope = plan
        .envelope_delta_bytes()
        .map_err(|_| ctx.allocation_failure("gltf-profile-identity"))?;
    let storage =
        ctx.metered_opaque_allowance(ImportStorageClass::FormatNormalization, envelope)?;
    let value = ResourceIdentity::new(label.as_str()).map_err(|cause| match cause {
        baozi_import::authoring::ResourceIdentityError::AllocationFailed => {
            ContentError::from(ctx.allocation_failure("gltf-profile-identity"))
        }
        _ => ContentError::from(GltfFailure::invalid_document(
            "resource identity is outside the bounded canonical profile",
        )),
    })?;
    plan.finish(0, value.capacity_bytes()).map_err(|_| {
        ctx.format_failure(GltfFailure::internal(
            "gltf-profile-identity-capacity-envelope",
        ))
    })?;
    Ok(ProfileIdentity {
        value,
        _storage: storage,
    })
}

#[derive(Clone, Copy)]
pub(super) enum Resource {
    Buffer(usize),
    Image(usize),
}

impl Resource {
    fn subject(self) -> &'static str {
        match self {
            Self::Buffer(_) => "buffer-uri",
            Self::Image(_) => "texture-uri",
        }
    }

    pub(super) fn index(self) -> usize {
        match self {
            Self::Buffer(index) | Self::Image(index) => index,
        }
    }

    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Buffer(_) => "buffer",
            Self::Image(_) => "image",
        }
    }

    fn invalid(self, reason: &'static str) -> ContentError {
        GltfFailure::invalid_reference(self.subject(), Some(self.index()), reason).into()
    }

    pub(super) fn unsupported(self, ctx: &ParseScope<'_>, reason: &'static str) -> ContentError {
        let failure = GltfFailure::unsupported(self.subject(), format_args!("{self}: {reason}"));
        ctx.format_failure(IndexedUriFailure {
            failure,
            index: self.index(),
        })
        .into()
    }
}

struct IndexedUriFailure {
    failure: GltfFailure,
    index: usize,
}

impl FormatFailureEvidenceProducer for IndexedUriFailure {
    fn code(&self) -> &str {
        self.failure.code().as_str()
    }

    fn visit_details(&self, sink: &mut dyn FormatFailureDetailSink) {
        self.failure.visit_details(sink);
        sink.unsigned("index", to_u64(self.index));
    }
}

impl fmt::Display for Resource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Buffer(index) => write!(formatter, "buffers[{index}].uri"),
            Self::Image(index) => write!(formatter, "images[{index}].uri"),
        }
    }
}

#[allow(
    clippy::large_enum_variant,
    reason = "decoded paths retain their admitted buffer inline; literal paths need no allocation"
)]
pub(super) enum RelativePath<'scope, 'uri> {
    Literal(&'uri str),
    Decoded(MeteredVec<'scope, u8>),
}

impl RelativePath<'_, '_> {
    pub(super) fn as_str(&self) -> &str {
        match self {
            Self::Literal(value) => value,
            Self::Decoded(value) => std::str::from_utf8(value.as_slice())
                .expect("relative URI bytes were validated before storage"),
        }
    }
}

/// Decodes an IRI path once, before the import scope normalizes and authorizes it.
/// Encoded separators and encoded dot segments are outside this bounded profile;
/// they must never acquire path structure as a side effect of decoding.
pub(super) fn relative_path<'scope, 'uri>(
    ctx: &'scope ParseScope<'_>,
    value: &'uri str,
    resource: Resource,
) -> Result<RelativePath<'scope, 'uri>> {
    enforce_usize_limit(
        ctx,
        "max_string_bytes",
        value.len(),
        ctx.limits().max_string_bytes,
    )?;
    enforce_usize_limit(
        ctx,
        "max_path_length",
        value.len(),
        ctx.limits().max_path_length,
    )?;
    ctx.with_work(
        ImportWorkClass::WorkUnits,
        to_u64(value.len()),
        |work| -> Result<_> {
            if value.is_empty() {
                return Err(resource.invalid("external URI path is empty"));
            }
            if value.starts_with('/') {
                return Err(
                    resource.unsupported(ctx, "absolute paths and authorities are unsupported")
                );
            }
            if value
                .split('/')
                .next()
                .is_some_and(|segment| segment.contains(':'))
            {
                return Err(resource.unsupported(ctx, "URI schemes are unsupported"));
            }

            let mut decoded =
                ctx.metered_vec(ImportStorageClass::FormatNormalization, "gltf-uri-path");
            let encoded = value.as_bytes();
            let has_escape = encoded.contains(&b'%');
            if has_escape {
                decoded.try_reserve(encoded.len())?;
            }
            let mut cursor = 0;
            let mut next_poll = 0;
            let mut component_start = 0;
            let mut component_escaped = false;
            while cursor < encoded.len() {
                if cursor >= next_poll {
                    work.poll()?;
                    next_poll = cursor.saturating_add(super::PARSER_POLL_INTERVAL);
                }
                let byte = encoded[cursor];
                if matches!(byte, b'?' | b'#' | b'\\') {
                    return Err(
                        resource.unsupported(ctx, "query, fragment or backslash is unsupported")
                    );
                }
                if byte.is_ascii_control() || byte == b' ' {
                    return Err(resource.invalid("URI contains unescaped whitespace or controls"));
                }
                if byte == b'%' {
                    let escaped = encoded
                        .get(cursor + 1..cursor + 3)
                        .and_then(|digits| Some(hex(digits[0])? * 16 + hex(digits[1])?))
                        .ok_or_else(|| {
                            resource.invalid("URI contains an invalid percent escape")
                        })?;
                    if matches!(escaped, b'/' | b'\\') {
                        return Err(
                            resource.unsupported(ctx, "percent-encoded separators are unsupported")
                        );
                    }
                    if escaped == b':' {
                        return Err(
                            resource.unsupported(ctx, "percent-encoded colons are unsupported")
                        );
                    }
                    if escaped.is_ascii_control() {
                        return Err(resource.invalid("URI decodes to a control character"));
                    }
                    decoded.try_push_reserved(escaped)?;
                    component_escaped = true;
                    cursor += 3;
                } else {
                    if byte == b'/' {
                        if component_escaped
                            && is_dot_segment(&decoded.as_slice()[component_start..])
                        {
                            return Err(resource
                                .unsupported(ctx, "percent-encoded dot segments are unsupported"));
                        }
                        component_start = decoded.len() + 1;
                        component_escaped = false;
                    }
                    if has_escape {
                        decoded.try_push_reserved(byte)?;
                    }
                    cursor += 1;
                }
            }
            if component_escaped && is_dot_segment(&decoded.as_slice()[component_start..]) {
                return Err(
                    resource.unsupported(ctx, "percent-encoded dot segments are unsupported")
                );
            }
            if has_escape {
                let path = std::str::from_utf8(decoded.as_slice())
                    .map_err(|_| resource.invalid("URI percent escapes are not valid UTF-8"))?;
                reject_controls(path, resource)?;
            } else {
                reject_controls(value, resource)?;
            }
            work.complete_all()?;
            if has_escape {
                Ok(RelativePath::Decoded(decoded))
            } else {
                Ok(RelativePath::Literal(value))
            }
        },
    )
}

fn reject_controls(value: &str, resource: Resource) -> Result<()> {
    if value.chars().any(char::is_control) {
        return Err(resource.invalid("URI contains or decodes to a control character"));
    }
    Ok(())
}

fn is_dot_segment(value: &[u8]) -> bool {
    matches!(value, b"." | b"..")
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
