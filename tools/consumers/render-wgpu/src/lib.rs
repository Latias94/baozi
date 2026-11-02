#![forbid(unsafe_code)]

//! A finite native rendering profile over the existing detached asset authority.

pub mod deformation;
pub mod device;
pub mod images;
pub mod material;
pub mod renderer;
pub mod spatial;

use std::{
    collections::TryReserveError,
    fmt,
    sync::atomic::{AtomicBool, Ordering},
};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    Allocation(TryReserveError),
    Cancelled,
    InvalidAsset(&'static str),
    Limit {
        dimension: &'static str,
        requested: u64,
        limit: u64,
    },
    InvalidLimit(&'static str),
    Scalar {
        location: device::ScalarLocation,
        kind: device::ScalarError,
    },
    UnsupportedTopology {
        draw: usize,
    },
    UnsupportedMaterial {
        material: Option<usize>,
        feature: &'static str,
    },
    MissingUv {
        draw: usize,
        set: u32,
    },
    TooManyActiveUvSets,
    IndivisibleElementTooLarge {
        draw: usize,
        element: usize,
        dimension: &'static str,
    },
    ExplicitFramesRequired {
        instance: usize,
        draw: usize,
    },
    InvalidFrame {
        instance: usize,
        draw: usize,
        vertex: usize,
        reason: &'static str,
    },
    MixedFrameOrientation {
        instance: usize,
        draw: usize,
        element: usize,
    },
    DegenerateNormalMapFrame {
        draw: usize,
        element: usize,
    },
    UnsupportedTangentMorphNormalMap {
        draw: usize,
    },
    NonRigidSpatialPose {
        node: Option<usize>,
        entity: &'static str,
    },
    UnknownSceneUnitScale,
    Image {
        image: usize,
        reason: String,
    },
    UnsupportedSampler,
    AdapterSelection {
        matches: usize,
    },
    UnsupportedDeviceProfile(String),
    Gpu(String),
    RenderNumericFailure {
        pixel: usize,
        code: u32,
    },
    Io(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for Error {}
impl From<TryReserveError> for Error {
    fn from(value: TryReserveError) -> Self {
        Self::Allocation(value)
    }
}
impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

#[derive(Clone, Copy, Default)]
pub struct Control<'a> {
    pub cancellation: Option<&'a AtomicBool>,
}

impl Control<'_> {
    pub fn check(self) -> Result<()> {
        if self
            .cancellation
            .is_some_and(|flag| flag.load(Ordering::Relaxed))
        {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
}

pub(crate) fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b)
        .ok_or(Error::InvalidLimit("size arithmetic overflow"))
}
pub(crate) fn mul(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b)
        .ok_or(Error::InvalidLimit("size arithmetic overflow"))
}
pub(crate) fn cap(dimension: &'static str, requested: u64, limit: u64) -> Result<()> {
    if requested > limit {
        Err(Error::Limit {
            dimension,
            requested,
            limit,
        })
    } else {
        Ok(())
    }
}
pub(crate) fn vec_with_capacity<T>(count: usize) -> Result<Vec<T>> {
    let mut out = Vec::new();
    out.try_reserve_exact(count)?;
    Ok(out)
}
pub(crate) fn f32_at(out: &mut [u8], offset: usize, value: f32) {
    out[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
pub(crate) fn u32_at(out: &mut [u8], offset: usize, value: u32) {
    out[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
