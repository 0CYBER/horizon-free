//! Prepared blockstate transforms. The face basis and inverse arithmetic use
//! the pinned client's default, unfused f32 operations, including UV-lock
//! identity detection and quaternion basis normalization error.
use super::sin_cos;
use crate::model::Direction;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug)]
struct Affine {
    rows: [[f32; 4]; 3],
    identity: bool,
}
impl Affine {
    const IDENTITY: Self = Self {
        rows: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ],
        identity: true,
    };
    fn position(self, v: [f32; 3]) -> [f32; 3] {
        self.rows
            .map(|r| r[0] * v[0] + (r[1] * v[1] + (r[2] * v[2] + r[3])))
    }
    fn mul(self, b: Self) -> Self {
        if self.identity {
            return b;
        }
        if b.identity {
            return self;
        }
        Self {
            rows: std::array::from_fn(|r| {
                std::array::from_fn(|c| {
                    self.rows[r][0] * b.rows[0][c]
                        + (self.rows[r][1] * b.rows[1][c]
                            + if c == 3 {
                                self.rows[r][2] * b.rows[2][c] + self.rows[r][3]
                            } else {
                                self.rows[r][2] * b.rows[2][c]
                            })
                })
            }),
            identity: false,
        }
    }
    fn inverse(self) -> Self {
        let [[a, b, c, x], [d, e, f, y], [g, h, i, z]] = self.rows;
        let ae = a * e;
        let db = d * b;
        let gb = g * b;
        let ah = a * h;
        let dh = d * h;
        let ge = g * e;
        let bi = b * i;
        let bf = b * f;
        let ei = e * i;
        let ec = e * c;
        let hf = h * f;
        let hc = h * c;
        let cg = c * g;
        let cd = c * d;
        let fg = f * g;
        let fa = f * a;
        let id = i * d;
        let ia = i * a;
        let inverse = 1.0 / ((ae - db) * i + (gb - ah) * f + (dh - ge) * c);
        Self {
            rows: [
                [
                    (ei - hf) * inverse,
                    (hc - bi) * inverse,
                    (bf - ec) * inverse,
                    (bi * y - bf * z + ec * z - ei * x + hf * x - hc * y) * inverse,
                ],
                [
                    (fg - id) * inverse,
                    (ia - cg) * inverse,
                    (cd - fa) * inverse,
                    (cg * y - cd * z + fa * z - fg * x + id * x - ia * y) * inverse,
                ],
                [
                    (dh - ge) * inverse,
                    (gb - ah) * inverse,
                    (ae - db) * inverse,
                    (ge * x - dh * x + ah * y - gb * y + db * z - ae * z) * inverse,
                ],
            ],
            identity: false,
        }
    }
    fn face_basis(direction: Direction) -> Self {
        use Direction::*;
        use std::f32::consts::{FRAC_PI_2, PI};
        let (axis, angle) = match direction {
            South => return Self::IDENTITY,
            Down => (0, FRAC_PI_2),
            Up => (0, -FRAC_PI_2),
            East => (1, FRAC_PI_2),
            West => (1, -FRAC_PI_2),
            North => (1, PI),
        };
        let (s, c) = sin_cos(angle * 0.5);
        // Quaternion.rotateX/Y on an identity quaternion retains signed zeros.
        let [x, y, z, w] = if axis == 0 {
            [
                s + 0.0 * c,
                0.0 * c + 0.0 * s,
                0.0 * c - 0.0 * s,
                c - 0.0 * s,
            ]
        } else {
            [
                0.0 * c - 0.0 * s,
                s + 0.0 * c,
                0.0 * s + 0.0 * c,
                c - 0.0 * s,
            ]
        };
        let (xx, yy, zz, ww) = (x * x, y * y, z * z, w * w);
        let (xy, xz, yz, xw, yw, zw) = (x * y, x * z, y * z, x * w, y * w, z * w);
        let (xy, xz, yz, xw, yw, zw) = (xy + xy, xz + xz, yz + yz, xw + xw, yw + yw, zw + zw);
        Self {
            rows: [
                [ww + xx - zz - yy, -zw + xy, yw + xz, 0.0],
                [xy + zw, yy - zz + ww - xx, yz - xw, 0.0],
                [xz - yw, yz + xw, zz - yy - xx + ww, 0.0],
            ],
            identity: false,
        }
    }
}

/// A blockstate's negative XYZ quarter-turns and optional face UV locking.
/// All 64 rotation combinations are prepared once and reused across models.
#[derive(Clone, Copy, Debug)]
pub struct BlockModelTransform {
    orientation: &'static Orientation,
    uv_lock: bool,
}
#[derive(Debug)]
struct Orientation {
    matrix: Affine,
    inverse_uv: [Affine; 6],
}
impl BlockModelTransform {
    pub fn new(quarter_turns: [u8; 3], uv_lock: bool) -> Self {
        static ORIENTATIONS: OnceLock<[Orientation; 64]> = OnceLock::new();
        let orientations = ORIENTATIONS.get_or_init(|| {
            let bases = Direction::ALL.map(Affine::face_basis);
            let inverses = bases.map(|b| if b.identity { b } else { b.inverse() });
            std::array::from_fn(|index| {
                let turns = [index / 16, index / 4 % 4, index % 4];
                let mut rows = [[1i8, 0, 0], [0, 1, 0], [0, 0, 1]];
                for (axis, turns) in turns.into_iter().enumerate() {
                    for _ in 0..turns {
                        let a = (axis + 1) % 3;
                        let b = (axis + 2) % 3;
                        let old = rows[a];
                        rows[a] = rows[b];
                        rows[b] = old.map(|v| -v);
                    }
                }
                let matrix = Affine {
                    identity: rows == [[1, 0, 0], [0, 1, 0], [0, 0, 1]],
                    rows: rows.map(|r| [f32::from(r[0]), f32::from(r[1]), f32::from(r[2]), 0.0]),
                };
                let inverse_uv = bases.map(|b| {
                    if matrix.identity {
                        return matrix.inverse();
                    }
                    let transformed = matrix.mul(b);
                    let normal = transformed.rows.map(|r| r[2]);
                    inverses[nearest(normal) as usize]
                        .mul(transformed)
                        .inverse()
                });
                Orientation { matrix, inverse_uv }
            })
        });
        let [x, y, z] = quarter_turns.map(|n| usize::from(n % 4));
        Self {
            orientation: &orientations[x * 16 + y * 4 + z],
            uv_lock,
        }
    }
    pub fn matrix_rows(self) -> [[f32; 4]; 3] {
        self.orientation.matrix.rows
    }
    pub fn inverse_uv_rows(self, direction: Direction) -> [[f32; 4]; 3] {
        self.uv_matrix(direction).rows
    }
    fn uv_matrix(self, direction: Direction) -> Affine {
        if self.uv_lock {
            self.orientation.inverse_uv[direction as usize]
        } else {
            Affine::IDENTITY
        }
    }
    pub(super) fn position(self, v: [f32; 3]) -> [f32; 3] {
        if self.orientation.matrix.identity {
            return v;
        }
        self.orientation
            .matrix
            .position(v.map(|c| c - 0.5))
            .map(|c| c + 0.5)
    }
    pub(super) fn uv(self, direction: Direction, uv: [f32; 2]) -> [f32; 2] {
        let matrix = self.uv_matrix(direction);
        // MatrixUtil re-determines properties when the identity bit is absent.
        // Its numeric comparison treats signed zeros as equal, without changing
        // the matrix coefficients retained for later use.
        if matrix.rows == Affine::IDENTITY.rows {
            return uv;
        }
        let result = matrix.position([uv[0] - 0.5, uv[1] - 0.5, 0.0]);
        [result[0] + 0.5, result[1] + 0.5]
    }
    pub fn direction(self, direction: Direction) -> Direction {
        let v = match direction {
            Direction::Down => [0.0, -1.0, 0.0],
            Direction::Up => [0.0, 1.0, 0.0],
            Direction::North => [0.0, 0.0, -1.0],
            Direction::South => [0.0, 0.0, 1.0],
            Direction::West => [-1.0, 0.0, 0.0],
            Direction::East => [1.0, 0.0, 0.0],
        };
        nearest(self.orientation.matrix.position(v))
    }
}
fn nearest([x, y, z]: [f32; 3]) -> Direction {
    let mut best = f32::MIN_POSITIVE;
    let mut direction = Direction::North;
    for (candidate, dot) in Direction::ALL.into_iter().zip([-y, y, -z, z, -x, x]) {
        if dot > best {
            best = dot;
            direction = candidate;
        }
    }
    direction
}
