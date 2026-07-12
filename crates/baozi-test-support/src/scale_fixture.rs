use std::collections::TryReserveError;
use std::error::Error;
use std::fmt;

const OBJ_HEADER: &[u8] = b"o baozi_scale\nv 0 0 0\nv 1 0 0\nv 0 1 0\n";
const OBJ_FACE: &[u8] = b"f 1 2 3\n";

/// The logical size multiplier used by the deterministic import scale fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ObjScale {
    One,
    Ten,
    Hundred,
    Thousand,
}

impl ObjScale {
    pub const CORRECTNESS: [Self; 3] = [Self::One, Self::Ten, Self::Hundred];
    pub const PROFILE: [Self; 4] = [Self::One, Self::Ten, Self::Hundred, Self::Thousand];

    pub const fn multiplier(self) -> usize {
        match self {
            Self::One => 1,
            Self::Ten => 10,
            Self::Hundred => 100,
            Self::Thousand => 1000,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::One => "1x",
            Self::Ten => "10x",
            Self::Hundred => "100x",
            Self::Thousand => "1000x",
        }
    }

    pub const fn from_multiplier(multiplier: usize) -> Option<Self> {
        match multiplier {
            1 => Some(Self::One),
            10 => Some(Self::Ten),
            100 => Some(Self::Hundred),
            1000 => Some(Self::Thousand),
            _ => None,
        }
    }
}

/// Failure to construct a bounded deterministic scale fixture.
#[derive(Debug)]
pub enum ObjScaleFixtureError {
    SizeOverflow,
    Allocation {
        requested_bytes: usize,
        source: TryReserveError,
    },
}

impl fmt::Display for ObjScaleFixtureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SizeOverflow => formatter.write_str("OBJ scale fixture size overflowed"),
            Self::Allocation {
                requested_bytes, ..
            } => write!(
                formatter,
                "OBJ scale fixture could not reserve {requested_bytes} bytes"
            ),
        }
    }
}

impl Error for ObjScaleFixtureError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::SizeOverflow => None,
            Self::Allocation { source, .. } => Some(source),
        }
    }
}

/// A generated OBJ whose parser and output work grow affinely with its face count.
#[derive(Debug)]
pub struct ObjScaleFixture {
    scale: ObjScale,
    face_count: usize,
    bytes: Vec<u8>,
}

impl ObjScaleFixture {
    pub const CASE_ID: &'static str = "obj_repeated_triangle_faces";
    pub const BASE_FACE_COUNT: usize = 256;

    // This reviewed envelope is intentionally loose enough for allocator capacity rounding while
    // remaining affine. Changes require scale evidence and policy review.
    pub const PEAK_FIXED_BYTES: u64 = 8 * 1024 * 1024;
    pub const PEAK_BYTES_PER_INPUT_BYTE: u64 = 16;
    pub const PEAK_BYTES_PER_FACE: u64 = 2048;

    pub fn try_new(scale: ObjScale) -> Result<Self, ObjScaleFixtureError> {
        let face_count = Self::BASE_FACE_COUNT
            .checked_mul(scale.multiplier())
            .ok_or(ObjScaleFixtureError::SizeOverflow)?;
        let face_bytes = OBJ_FACE
            .len()
            .checked_mul(face_count)
            .ok_or(ObjScaleFixtureError::SizeOverflow)?;
        let requested_bytes = OBJ_HEADER
            .len()
            .checked_add(face_bytes)
            .ok_or(ObjScaleFixtureError::SizeOverflow)?;

        let mut bytes = Vec::new();
        bytes.try_reserve_exact(requested_bytes).map_err(|source| {
            ObjScaleFixtureError::Allocation {
                requested_bytes,
                source,
            }
        })?;
        bytes.extend_from_slice(OBJ_HEADER);
        for _ in 0..face_count {
            bytes.extend_from_slice(OBJ_FACE);
        }
        debug_assert_eq!(bytes.len(), requested_bytes);

        Ok(Self {
            scale,
            face_count,
            bytes,
        })
    }

    pub const fn scale(&self) -> ObjScale {
        self.scale
    }

    pub const fn face_count(&self) -> usize {
        self.face_count
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn expected_record_count(&self) -> u64 {
        self.face_count as u64 + 4
    }

    pub fn expected_token_count(&self) -> u64 {
        self.face_count as u64 * 4 + 14
    }

    pub fn reviewed_peak_live_envelope(&self) -> u64 {
        Self::PEAK_FIXED_BYTES
            + self.bytes.len() as u64 * Self::PEAK_BYTES_PER_INPUT_BYTE
            + self.face_count as u64 * Self::PEAK_BYTES_PER_FACE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_size_and_affine_expectations_are_exact() {
        for scale in ObjScale::PROFILE {
            let fixture = ObjScaleFixture::try_new(scale).expect("bounded fixture should allocate");
            let expected_faces = ObjScaleFixture::BASE_FACE_COUNT * scale.multiplier();

            assert_eq!(fixture.face_count(), expected_faces);
            assert_eq!(
                fixture.bytes().len(),
                OBJ_HEADER.len() + OBJ_FACE.len() * expected_faces
            );
            assert_eq!(fixture.expected_record_count(), expected_faces as u64 + 4);
            assert_eq!(
                fixture.expected_token_count(),
                expected_faces as u64 * 4 + 14
            );
        }
    }

    #[test]
    fn unsupported_multiplier_is_rejected() {
        assert_eq!(ObjScale::from_multiplier(0), None);
        assert_eq!(ObjScale::from_multiplier(11), None);
    }
}
