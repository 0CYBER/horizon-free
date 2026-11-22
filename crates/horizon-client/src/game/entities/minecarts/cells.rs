//! A coarse bitmap of where the carts are ([`CartCells`]), to turn most mobs away from a cart
//! push without a lookup in the carts' grid.
use super::*;
use horizon_core::geometry::Axis;

/// Blocks along a side of a coarse cell.
const SIDE: f64 = 4.0;
/// Bits in the map (a power of two).
const BITS: usize = 1 << 16;
/// A box wider than this many cells in one direction marks everything: no map is worth it.
const MAX_SPAN: i64 = 64;

/// The coarse cells that any cart's box touches, hashed into a small bitmap. A false "maybe"
/// (a hash collision, a neighbouring box) costs only the lookup it was meant to save; a "no"
/// is always right.
pub(in crate::game) struct CartCells {
    bits: Vec<u64>,
    all: bool,
}

/// The cells `area` touches, as an inclusive range, with a margin for rounding; None when the
/// area is not finite or too wide.
fn span(area: Aabb) -> Option<([i32; 3], [i32; 3])> {
    let eps = 1.0e-4;
    let axes = [Axis::X, Axis::Y, Axis::Z];
    let lo = axes.map(|a| area.min(a) - eps);
    let hi = axes.map(|a| area.max(a) + eps);
    if !lo.iter().chain(&hi).all(|v| v.is_finite()) {
        return None;
    }
    let cell = |v: f64| (v / SIDE).floor().clamp(-1.0e9, 1.0e9) as i32;
    let (a, b) = (lo.map(cell), hi.map(cell));
    let wide = (0..3).any(|k| i64::from(b[k]) - i64::from(a[k]) >= MAX_SPAN);
    (!wide).then_some((a, b))
}
fn slot(x: i32, y: i32, z: i32) -> usize {
    let h = (x as u32).wrapping_mul(0x9E37_79B1)
        ^ (z as u32).wrapping_mul(0x85EB_CA6B)
        ^ (y as u32).wrapping_mul(0xC2B2_AE35);
    ((h ^ (h >> 15)).wrapping_mul(0x2C1B_3C6D) >> 16) as usize % BITS
}

impl CartCells {
    pub(in crate::game) fn of(boxes: impl Iterator<Item = Aabb>) -> Self {
        let mut cells = Self {
            bits: vec![0; BITS / 64],
            all: false,
        };
        for b in boxes {
            let Some((lo, hi)) = span(b) else {
                cells.all = true;
                continue;
            };
            for x in lo[0]..=hi[0] {
                for y in lo[1]..=hi[1] {
                    for z in lo[2]..=hi[2] {
                        let s = slot(x, y, z);
                        cells.bits[s / 64] |= 1 << (s % 64);
                    }
                }
            }
        }
        cells
    }
    /// Whether a cart's box may meet `area`. False: none does.
    pub(in crate::game) fn may_meet(&self, area: Aabb) -> bool {
        if self.all {
            return true;
        }
        let Some((lo, hi)) = span(area) else {
            return true;
        };
        (lo[0]..=hi[0]).any(|x| {
            (lo[1]..=hi[1]).any(|y| {
                (lo[2]..=hi[2]).any(|z| {
                    let s = slot(x, y, z);
                    self.bits[s / 64] & (1 << (s % 64)) != 0
                })
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::geometry::Vec3;

    fn at(x: f64, z: f64) -> Aabb {
        Aabb::new(Vec3::new(x, 64.0, z), Vec3::new(x + 1.0, 65.0, z + 1.0))
    }

    #[test]
    fn a_box_that_meets_a_cart_is_never_turned_away() {
        let carts: Vec<Aabb> = (0..50)
            .map(|i| at(f64::from(i) * 7.3 - 90.0, f64::from(i) * -3.1 + 20.0))
            .collect();
        let cells = CartCells::of(carts.iter().copied());
        for c in &carts {
            for (dx, dz) in [(-0.9, 0.0), (0.0, 0.9), (0.99, 0.99), (-0.5, -0.5)] {
                assert!(cells.may_meet(at(c.min(Axis::X) + dx, c.min(Axis::Z) + dz)));
            }
        }
        assert!(!cells.may_meet(at(500.0, 500.0)));
    }

    /// A cart's moves meet the same boxes of other carts, in the same order, whether it asks the
    /// list or the boxes taken when its tick began, over a pass in which the carts tick and move.
    #[test]
    fn the_carts_taken_at_the_start_of_a_tick_give_the_boxes_the_list_gives() {
        let mut state = 0xD1B5_4A32_D192_ED03u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let unit = |v: u64, span: f64| (v % 10_000) as f64 / 10_000.0 * span;
        for round in 0..40 {
            let n = 5 + (next() % 120) as usize;
            let spread = [10.0, 60.0, 300.0][round % 3];
            let mut carts: Vec<Minecart> = (0..n)
                .map(|k| {
                    let at = Vec3::new(
                        unit(next(), spread),
                        64.0 + unit(next(), 4.0),
                        unit(next(), spread),
                    );
                    Minecart::new(k as u32 + 1, CartKind::Rideable, at)
                })
                .collect();
            let mut index = CartIndex {
                pass: true,
                ..Default::default()
            };
            for i in 0..n {
                let c = carts[i].clone();
                let area = c.bounds().inflate(Vec3::new(
                    NEAR_CARTS_REACH,
                    NEAR_CARTS_REACH,
                    NEAR_CARTS_REACH,
                ));
                let taken = index.near_snapshot(&carts, i, area);
                // Every query a move could make: boxes near the start, further along, and past the area.
                for step in 0..6 {
                    let at = Vec3::new(
                        c.position.x + unit(next(), 12.0) - 6.0 * f64::from(step % 3),
                        c.position.y + unit(next(), 3.0) - 1.0,
                        c.position.z + unit(next(), 12.0) - 6.0 * f64::from(step % 2),
                    );
                    let moved = Minecart::new(c.id, CartKind::Rideable, at);
                    let near = moved.bounds().inflate(Vec3::new(2.0, 2.0, 2.0));
                    let by_list: Vec<Aabb> = carts
                        .iter()
                        .filter(|o| o.id != c.id)
                        .map(Minecart::bounds)
                        .filter(|b| b.intersects(near))
                        .collect();
                    if let Some(boxes) = taken.as_ref().and_then(|t| t.meeting(near)) {
                        assert_eq!(boxes, by_list, "round {round} cart {i} step {step}");
                    }
                }
                // The cart ticks and moves, and the index hears of it, as `section_steps::tick` does.
                carts[i].position = Vec3::new(
                    c.position.x + unit(next(), 6.0) - 3.0,
                    c.position.y,
                    c.position.z + unit(next(), 6.0) - 3.0,
                );
                index.moved(i, carts[i].bounds());
            }
        }
    }

    /// A cart taken out and another put on the end keeps the list's length: the grid is not
    /// trusted then, and what the cart takes is what the list holds now.
    #[test]
    fn a_list_that_lost_a_cart_and_gained_another_is_not_read_through_the_old_grid() {
        let at = |k: u32| Vec3::new(f64::from(k) * 3.0, 64.0, 0.0);
        let mut carts: Vec<Minecart> = (1..=12)
            .map(|k| Minecart::new(k, CartKind::Rideable, at(k)))
            .collect();
        let mut index = CartIndex {
            pass: true,
            ..Default::default()
        };
        let area = |c: &Minecart| {
            c.bounds().inflate(Vec3::new(
                NEAR_CARTS_REACH,
                NEAR_CARTS_REACH,
                NEAR_CARTS_REACH,
            ))
        };
        assert!(index.near_snapshot(&carts, 0, area(&carts[0])).is_some());
        // Cart 5 is destroyed and a new cart 13 joins at the end, beside cart 1.
        carts.remove(4);
        carts.push(Minecart::new(
            13,
            CartKind::Rideable,
            Vec3::new(f64::from(1u32) * 3.0 + 1.0, 64.0, 0.0),
        ));
        assert_eq!(carts.len(), 12);
        for i in 0..carts.len() {
            let c = carts[i].clone();
            let want: Vec<Aabb> = carts
                .iter()
                .filter(|o| o.id != c.id)
                .map(Minecart::bounds)
                .filter(|b| b.intersects(area(&c)))
                .collect();
            // The grid is rebuilt and answers the list as it is.
            let taken = index.near_snapshot(&carts, i, area(&c));
            assert_eq!(taken.map(|t| t.boxes), Some(want), "cart slot {i}");
        }
    }
}
