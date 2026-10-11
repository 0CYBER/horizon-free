//! Column-major `Matrix4f` arithmetic as JOML 1.10.9 computes it, for composing
//! item-model transformations: each operation takes the path JOML picks from
//! the matrix properties (identity, translation, affine...), and `Math.fma` is
//! `a * b + c` since Minecraft leaves `joml.useMathFma` off.

/// `Matrix4fc.PROPERTY_*` bits.
const PERSPECTIVE: u8 = 1;
const AFFINE: u8 = 2;
const IDENTITY: u8 = 4;
const TRANSLATION: u8 = 8;
const ORTHONORMAL: u8 = 16;

/// JOML `Math.fma` without `joml.useMathFma`.
#[inline]
fn fma(a: f32, b: f32, c: f32) -> f32 {
    a * b + c
}
/// JOML `Math.absEqualsOne`.
fn abs_equals_one(r: f32) -> bool {
    r.to_bits() & 0x7FFF_FFFF == 0x3F80_0000
}

/// A JOML `Matrix4f`: `m[column][row]`, with its property bits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat4(pub [[f32; 4]; 4], pub u8);

impl Mat4 {
    /// `new Matrix4f()`.
    pub const IDENTITY: Self = Self(
        [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
        IDENTITY | AFFINE | TRANSLATION | ORTHONORMAL,
    );
    /// `Matrix4f.translation(x, y, z)`.
    pub fn translation(t: [f32; 3]) -> Self {
        let mut m = Self::IDENTITY;
        m.0[3][..3].copy_from_slice(&t);
        m.1 = AFFINE | TRANSLATION | ORTHONORMAL;
        m
    }
    /// `ExtraCodecs.MATRIX4F`: sixteen values in row-major order, then
    /// `determineProperties`.
    pub fn from_row_major(values: [f32; 16]) -> Self {
        let mut m = Self([[0.0; 4]; 4], 0);
        for (i, v) in values.into_iter().enumerate() {
            m.0[i & 3][i >> 2] = v;
        }
        m.1 = m.determine_properties();
        m
    }
    /// `Matrix4f.determineProperties`.
    fn determine_properties(&self) -> u8 {
        let m = &self.0;
        let mut properties = 0;
        if m[0][3] == 0.0 && m[1][3] == 0.0 {
            if m[2][3] == 0.0 && m[3][3] == 1.0 {
                properties |= AFFINE;
                if m[0] == [1.0, 0.0, 0.0, 0.0]
                    && m[1] == [0.0, 1.0, 0.0, 0.0]
                    && m[2] == [0.0, 0.0, 1.0, 0.0]
                {
                    properties |= TRANSLATION | ORTHONORMAL;
                    if m[3][0] == 0.0 && m[3][1] == 0.0 && m[3][2] == 0.0 {
                        properties |= IDENTITY;
                    }
                }
            } else if m[0][1] == 0.0
                && m[0][2] == 0.0
                && m[1][0] == 0.0
                && m[1][2] == 0.0
                && m[2][0] == 0.0
                && m[2][1] == 0.0
                && m[3][0] == 0.0
                && m[3][1] == 0.0
                && m[3][3] == 0.0
            {
                properties |= PERSPECTIVE;
            }
        }
        properties
    }
    fn has(&self, property: u8) -> bool {
        self.1 & property != 0
    }
    /// The matrix as `[row][column]` entries, the order `getRowColumn` reads.
    pub fn rows(&self) -> [[f32; 4]; 4] {
        std::array::from_fn(|r| std::array::from_fn(|c| self.0[c][r]))
    }
    /// `this * right` (`Matrix4f.mul`).
    pub fn mul(&self, right: &Self) -> Self {
        let (m, r) = (&self.0, &right.0);
        if self.has(IDENTITY) {
            return *right;
        }
        if right.has(IDENTITY) {
            return *self;
        }
        let mut out = [[0.0; 4]; 4];
        if self.has(TRANSLATION) && right.has(AFFINE) {
            // mulTranslationAffine
            for c in 0..4 {
                out[c] = [r[c][0], r[c][1], r[c][2], m[c][3]];
            }
            for row in 0..3 {
                out[3][row] = r[3][row] + m[3][row];
            }
            return Self(out, AFFINE | right.1 & ORTHONORMAL);
        }
        if self.has(AFFINE) && right.has(AFFINE) {
            // mulAffine
            for c in 0..3 {
                for row in 0..3 {
                    out[c][row] = fma(
                        m[0][row],
                        r[c][0],
                        fma(m[1][row], r[c][1], m[2][row] * r[c][2]),
                    );
                }
                out[c][3] = m[c][3];
            }
            for row in 0..3 {
                out[3][row] = fma(
                    m[0][row],
                    r[3][0],
                    fma(m[1][row], r[3][1], fma(m[2][row], r[3][2], m[3][row])),
                );
            }
            out[3][3] = m[3][3];
            return Self(out, AFFINE | self.1 & right.1 & ORTHONORMAL);
        }
        if self.has(PERSPECTIVE) && right.has(AFFINE) {
            // mulPerspectiveAffine
            for c in 0..4 {
                out[c] = [
                    m[0][0] * r[c][0],
                    m[1][1] * r[c][1],
                    m[2][2] * r[c][2],
                    m[2][3] * r[c][2],
                ];
            }
            out[3][2] = m[2][2] * r[3][2] + m[3][2];
            out[3][3] = m[2][3] * r[3][2];
            return Self(out, 0);
        }
        if right.has(AFFINE) {
            // mulAffineR
            for c in 0..3 {
                for row in 0..4 {
                    out[c][row] = fma(
                        m[0][row],
                        r[c][0],
                        fma(m[1][row], r[c][1], m[2][row] * r[c][2]),
                    );
                }
            }
            for row in 0..4 {
                out[3][row] = fma(
                    m[0][row],
                    r[3][0],
                    fma(m[1][row], r[3][1], fma(m[2][row], r[3][2], m[3][row])),
                );
            }
            return Self(
                out,
                self.1 & !(PERSPECTIVE | IDENTITY | TRANSLATION | ORTHONORMAL),
            );
        }
        // mul0
        for c in 0..4 {
            for row in 0..4 {
                out[c][row] = fma(
                    m[0][row],
                    r[c][0],
                    fma(
                        m[1][row],
                        r[c][1],
                        fma(m[2][row], r[c][2], m[3][row] * r[c][3]),
                    ),
                );
            }
        }
        Self(out, 0)
    }
    /// `Matrix4f.translate(x, y, z)`.
    pub fn translate(&self, t: [f32; 3]) -> Self {
        if self.has(IDENTITY) {
            return Self::translation(t);
        }
        // translateGeneric
        let mut out = *self;
        for row in 0..4 {
            out.0[3][row] = fma(
                self.0[0][row],
                t[0],
                fma(
                    self.0[1][row],
                    t[1],
                    fma(self.0[2][row], t[2], self.0[3][row]),
                ),
            );
        }
        out.1 = self.1 & !(PERSPECTIVE | IDENTITY);
        out
    }
    /// `Matrix4f.scale(x, y, z)`.
    pub fn scale(&self, s: [f32; 3]) -> Self {
        let one = s.iter().all(|&v| abs_equals_one(v));
        if self.has(IDENTITY) {
            // scaling
            let mut out = Self::IDENTITY;
            for (c, v) in s.into_iter().enumerate() {
                out.0[c][c] = v;
            }
            out.1 = AFFINE | if one { ORTHONORMAL } else { 0 };
            return out;
        }
        // scaleGeneric: the first three columns are scaled.
        let mut out = *self;
        for c in 0..3 {
            for row in 0..4 {
                out.0[c][row] = self.0[c][row] * s[c];
            }
        }
        out.1 =
            self.1 & !(PERSPECTIVE | IDENTITY | TRANSLATION | if one { 0 } else { ORTHONORMAL });
        out
    }
    /// `Matrix4f.rotate(Quaternionfc)`.
    pub fn rotate(&self, q: [f32; 4]) -> Self {
        let [x, y, z, w] = q;
        let (w2, x2, y2, z2) = (w * w, x * x, y * y, z * z);
        let (zw, xy, xz, yw, yz, xw) = (z * w, x * y, x * z, y * w, y * z, x * w);
        let (dzw, dxy, dxz, dyw, dyz, dxw) = (zw + zw, xy + xy, xz + xz, yw + yw, yz + yz, xw + xw);
        let r = [
            [w2 + x2 - z2 - y2, dxy + dzw, dxz - dyw],
            [-dzw + dxy, y2 - z2 + w2 - x2, dyz + dxw],
            [dyw + dxz, dyz - dxw, z2 - y2 - x2 + w2],
        ];
        let m = &self.0;
        let mut out = *self;
        if self.has(IDENTITY) {
            // rotation
            for c in 0..3 {
                out.0[c][..3].copy_from_slice(&r[c]);
            }
            out.1 = AFFINE | ORTHONORMAL;
            return out;
        }
        out.1 = self.1 & !(PERSPECTIVE | IDENTITY | TRANSLATION);
        if self.has(TRANSLATION) {
            // rotateTranslation
            for c in 0..3 {
                out.0[c] = [r[c][0], r[c][1], r[c][2], 0.0];
            }
        } else if self.has(AFFINE) {
            // rotateAffine
            for c in 0..3 {
                for row in 0..3 {
                    out.0[c][row] = m[0][row] * r[c][0] + m[1][row] * r[c][1] + m[2][row] * r[c][2];
                }
                out.0[c][3] = 0.0;
            }
        } else {
            // rotateGeneric
            for c in 0..3 {
                for row in 0..4 {
                    out.0[c][row] = fma(
                        m[0][row],
                        r[c][0],
                        fma(m[1][row], r[c][1], m[2][row] * r[c][2]),
                    );
                }
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
/// JOML `Math.sin(float)` without fast math.
fn joml_sin(rad: f32) -> f32 {
    f64::from(rad).sin() as f32
}
/// JOML `Math.cosFromSin(float, float)` without fast math: the magnitude from
/// the sine, the sign from the angle's quadrant (exact zeros at right angles).
fn cos_from_sin(sin: f32, angle: f32) -> f32 {
    let cos = (1.0 - sin * sin).sqrt();
    let tau = (std::f64::consts::PI * 2.0) as f32;
    let a = angle + std::f32::consts::FRAC_PI_2;
    let mut b = a - (a / tau) as i32 as f32 * tau;
    if b < 0.0 {
        b += tau;
    }
    if b >= std::f32::consts::PI { -cos } else { cos }
}
/// `Quaternionf.rotationXYZ` from angles in radians.
pub fn quaternion_xyz(x: f32, y: f32, z: f32) -> [f32; 4] {
    let half = |a: f32| -> (f32, f32) {
        let h = a * 0.5;
        let s = joml_sin(h);
        (s, cos_from_sin(s, h))
    };
    let ((sx, cx), (sy, cy), (sz, cz)) = (half(x), half(y), half(z));
    let (cycz, sysz, sycz, cysz) = (cy * cz, sy * sz, sy * cz, cy * sz);
    [
        sx * cycz + cx * sysz,
        cx * sycz - sx * cysz,
        cx * cysz + sx * sycz,
        cx * cycz - sx * sysz,
    ]
}
/// `new Quaternionf(new AxisAngle4f(angle, axis))`: the angle is wrapped into
/// [0, 2π) and the axis is used as given.
pub fn quaternion_from_axis_angle(angle: f32, axis: [f32; 3]) -> [f32; 4] {
    let tau = (std::f64::consts::PI * 2.0) as f32;
    let angle = if angle < 0.0 {
        tau + angle % tau
    } else {
        angle
    } % tau;
    let sin = joml_sin(angle * 0.5);
    let cos = cos_from_sin(sin, angle * 0.5);
    [axis[0] * sin, axis[1] * sin, axis[2] * sin, cos]
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
    #[test]
    fn half_turn_has_exact_zeros_like_joml() {
        let half_turn = 180.0 * (std::f64::consts::PI / 180.0) as f32;
        assert_eq!(quaternion_xyz(0.0, half_turn, 0.0), [0.0, 1.0, 0.0, 0.0]);
    }
}
