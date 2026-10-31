//! Fundamental animation curve declarations shared by authoring, storage, and read APIs.
//!
//! These domain values are independent of a Scene generation, storage layout,
//! importer, or curve evaluation backend.

/// Authored units for duration, key times, and cubic tangent derivatives.
///
/// Seconds carry no rate. Frame rates and declared tick rates must be finite and
/// positive when validated. An unknown tick rate stays None; no default is invented.
/// Key times may be negative; canonical validation requires finite, strictly increasing
/// keys and a finite, non-negative clip duration. Conversion to another time domain
/// is an explicit operation, not a read behavior.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AnimationTimeDomain {
    Seconds,
    Frames { rate: f64 },
    Ticks { rate: Option<f64> },
}

/// Retained curve interpolation. Cubic tangents are derivatives in the clip's
/// declared time domain, not normalized quaternion samples or per-second guesses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationInterpolation {
    Step,
    Linear,
    CubicSpline,
}

/// Retained behavior before the first key or after the last key.
/// SourceDefault preserves the source declaration without evaluating a default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationExtrapolation {
    SourceDefault,
    Constant,
    Linear,
    Repeat,
}
