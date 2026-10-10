//! Column-major `Matrix4f` arithmetic with the operand order of JOML's
//! default (non-FMA) code paths, for composing item-model transformations.

/// A JOML `Matrix4f`: `m[column][row]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat4(pub [[f32; 4]; 4]);

impl Mat4 {
    pub const IDENTITY: Self = Self([
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]);
    /// `Matrix4f.translation(x, y, z)`.
    pub fn translation(t: [f32; 3]) -> Self {
        let mut m = Self::IDENTITY;
        m.0[3][..3].copy_from_slice(&t);
        m
    }
    /// `ExtraCodecs.MATRIX4F`: sixteen values in row-major order.
    pub fn from_row_major(values: [f32; 16]) -> Self {
        let mut m = Self([[0.0; 4]; 4]);
        for (i, v) in values.into_iter().enumerate() {
            m.0[i & 3][i >> 2] = v;
        }
        m
    }
    /// The matrix as `[row][column]` entries, the order `getRowColumn` reads.
    pub fn rows(&self) -> [[f32; 4]; 4] {
        std::array::from_fn(|r| std::array::from_fn(|c| self.0[c][r]))
    }
    /// `this * right` (`Matrix4f.mul`).
    pub fn mul(&self, right: &Self) -> Self {
        let (m, r) = (&self.0, &right.0);
        let mut out = [[0.0; 4]; 4];
        for c in 0..4 {
            for row in 0..4 {
                out[c][row] = m[0][row] * r[c][0]
                    + m[1][row] * r[c][1]
                    + m[2][row] * r[c][2]
                    + m[3][row] * r[c][3];
            }
        }
        Self(out)
    }
    /// `Matrix4f.translate(x, y, z)` (`translateGeneric`).
    pub fn translate(&self, t: [f32; 3]) -> Self {
        let mut out = *self;
        for row in 0..4 {
            out.0[3][row] = self.0[0][row] * t[0]
                + (self.0[1][row] * t[1] + (self.0[2][row] * t[2] + self.0[3][row]));
        }
        out
    }
    /// `Matrix4f.scale(x, y, z)`: the first three columns are scaled.
    pub fn scale(&self, s: [f32; 3]) -> Self {
        let mut out = *self;
        for c in 0..3 {
            for row in 0..4 {
                out.0[c][row] = self.0[c][row] * s[c];
            }
        }
        out
    }
    /// `Matrix4f.rotate(Quaternionfc)` (`rotateGeneric`).
    pub fn rotate(&self, q: [f32; 4]) -> Self {
        let [x, y, z, w] = q;
        let (w2, x2, y2, z2) = (w * w, x * x, y * y, z * z);
        let (zw, xy, xz, yw, yz, xw) = (z * w, x * y, x * z, y * w, y * z, x * w);
        let (dzw, dxy, dxz, dyw, dyz, dxw) = (zw + zw, xy + xy, xz + xz, yw + yw, yz + yz, xw + xw);
        let r = [
            [w2 + x2 - z2 - y2, dxy + dzw, dxz - dyw],
            [dxy - dzw, y2 - z2 + w2 - x2, dyz + dxw],
            [dyw + dxz, dyz - dxw, z2 - y2 - x2 + w2],
        ];
        let m = &self.0;
        let mut out = *self;
        for c in 0..3 {
            for row in 0..4 {
                out.0[c][row] = m[0][row] * r[c][0] + m[1][row] * r[c][1] + m[2][row] * r[c][2];
            }
        }
        out
    }
    pub fn is_finite(&self) -> bool {
        self.0.iter().flatten().all(|v| v.is_finite())
    }
}

/// `Quaternionf.normalize` as the codec applies it to four components.
pub fn normalize_quaternion(q: [f32; 4]) -> [f32; 4] {
    let [x, y, z, w] = q;
    let inv = 1.0 / (x * x + (y * y + (z * z + w * w))).sqrt();
    [x * inv, y * inv, z * inv, w * inv]
}
/// `Quaternionf.rotationXYZ` from angles in radians.
pub fn quaternion_xyz(x: f32, y: f32, z: f32) -> [f32; 4] {
    let half = |a: f32| -> (f32, f32) {
        let h = a * 0.5;
        ((f64::from(h).sin()) as f32, (f64::from(h).cos()) as f32)
    };
    let ((sx, cx), (sy, cy), (sz, cz)) = (half(x), half(y), half(z));
    let (cx_cy, sx_cy, cx_sy, sx_sy) = (cx * cy, sx * cy, cx * sy, sx * sy);
    [
        sx_cy * cz + cx_sy * sz,
        cx_sy * cz - sx_cy * sz,
        cx_cy * sz + sx_sy * cz,
        cx_cy * cz - sx_sy * sz,
    ]
}
/// `new Quaternionf(new AxisAngle4f(angle, axis))`.
pub fn quaternion_from_axis_angle(angle: f32, axis: [f32; 3]) -> [f32; 4] {
    let half = angle * 0.5;
    let (sin, cos) = (half.sin(), half.cos());
    let norm = 1.0 / (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]).sqrt();
    let s = sin * norm;
    [axis[0] * s, axis[1] * s, axis[2] * s, cos]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn composes_in_joml_order() {
        let t = Mat4::translation([1.0, 2.0, 3.0]);
        let s = Mat4::IDENTITY.scale([2.0, -1.0, 1.0]);
        let m = t.mul(&s);
        assert_eq!(m.0[0][0], 2.0);
        assert_eq!(m.0[1][1], -1.0);
        assert_eq!(m.0[3][..3], [1.0, 2.0, 3.0]);
        assert_eq!(s.mul(&t).0[3][..3], [2.0, -2.0, 3.0]);
    }
    #[test]
    fn quarter_turn_about_z_rotates_x_into_y() {
        let q = quaternion_from_axis_angle(std::f32::consts::FRAC_PI_2, [0.0, 0.0, 1.0]);
        let m = Mat4::IDENTITY.rotate(q);
        assert!((m.0[0][1] - 1.0).abs() < 1e-6 && m.0[0][0].abs() < 1e-6);
    }
}
