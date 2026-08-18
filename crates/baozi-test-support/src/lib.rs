#![forbid(unsafe_code)]

pub mod scale_fixture;
pub mod support_matrix;

pub use scale_fixture::{ObjScale, ObjScaleFixture, ObjScaleFixtureError};
pub use support_matrix::{SupportMatrixColumn, assert_support_matrix_row};
