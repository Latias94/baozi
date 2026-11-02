//! Admission of authored glTF node matrices without decomposing or rewriting them.

const ORTHOGONAL_REL_TOLERANCE: f64 = 64.0 * f32::EPSILON as f64;

pub(super) fn node_matrix(matrix: &[[f32; 4]; 4]) -> Result<(), &'static str> {
    for column in matrix {
        for cell in column {
            if !cell.is_finite() {
                return Err("glTF node matrix must contain only finite F32 values");
            }
        }
    }
    if matrix[0][3] != 0.0 || matrix[1][3] != 0.0 || matrix[2][3] != 0.0 || matrix[3][3] != 1.0 {
        return Err("glTF node matrix must have the exact affine bottom row [0, 0, 0, 1]");
    }

    let mut columns = [[0.0_f64; 3]; 3];
    let mut norm2 = [0.0_f64; 3];
    for column in 0..3 {
        let [x, y, z, _] = matrix[column];
        if x == 0.0 && y == 0.0 && z == 0.0 {
            return Err("glTF node matrix must have three nonzero spatial columns");
        }
        let [x, y, z] = [f64::from(x), f64::from(y), f64::from(z)];
        columns[column] = [x, y, z];
        // F64 keeps even squared subnormal F32 columns nonzero. Fix x/y/z order.
        norm2[column] = (x * x + y * y) + z * z;
    }

    let tolerance2 = ORTHOGONAL_REL_TOLERANCE * ORTHOGONAL_REL_TOLERANCE;
    for (left, right) in [(0, 1), (0, 2), (1, 2)] {
        let [lx, ly, lz] = columns[left];
        let [rx, ry, rz] = columns[right];
        let dot = (lx * rx + ly * ry) + lz * rz;
        if dot * dot > tolerance2 * norm2[left] * norm2[right] {
            return Err("glTF node matrix spatial columns must be orthogonal");
        }
    }
    Ok(())
}
