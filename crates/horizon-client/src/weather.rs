//! Rain and snow columns around the camera (`WeatherEffectRenderer`).
//!
//! Vanilla draws each column as one camera-facing quad whose texture
//! coordinates run past 0..1 (`v` advances a quarter of the texture per block,
//! so `rain.png` repeats every 4 blocks) and relies on a repeating sampler.
//! Horizon samples the sprite from its atlas, so [`tile`] splits a quad at
//! every whole texture repeat into pieces that stay inside the sprite.
use horizon_core::random::{LegacyRandom, RandomSource};

/// `WeatherEffectRenderer.ColumnInstance`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Column {
    pub x: i32,
    pub z: i32,
    pub bottom_y: i32,
    pub top_y: i32,
    pub u_offset: f32,
    pub v_offset: f32,
    /// Vanilla packed light (`LightCoordsUtil.pack`: block << 4 | sky << 20).
    pub light_coords: i32,
}

/// One vertex of a column quad, in vanilla's emission order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColumnVertex {
    /// Camera-relative position.
    pub pos: [f32; 3],
    /// Texture coordinates before wrapping (the sampler repeats).
    pub uv: [f32; 2],
    /// ARGB (`ARGB.white(alpha)`).
    pub color: u32,
    pub light_coords: i32,
}

/// The random seed of a column (`extractRenderState`).
pub fn column_seed(x: i32, z: i32) -> i32 {
    (x.wrapping_mul(x)
        .wrapping_mul(3121)
        .wrapping_add(x.wrapping_mul(45_238_971)))
        ^ (z.wrapping_mul(z)
            .wrapping_mul(418_711)
            .wrapping_add(z.wrapping_mul(13761)))
}

/// The column's random source, seeded as `random.setSeed(seed)`.
pub fn column_random(x: i32, z: i32) -> LegacyRandom {
    LegacyRandom::new(i64::from(column_seed(x, z)))
}

/// `createRainColumnInstance`.
#[allow(clippy::too_many_arguments)]
pub fn rain_column(
    random: &mut impl RandomSource,
    ticks: i64,
    x: i32,
    bottom_y: i32,
    top_y: i32,
    z: i32,
    light_coords: i32,
    partial: f32,
) -> Column {
    let wrapped_ticks = (ticks & 131_071) as i32;
    let tick_offset = x
        .wrapping_mul(x)
        .wrapping_mul(3121)
        .wrapping_add(x.wrapping_mul(45_238_971))
        .wrapping_add(z.wrapping_mul(z).wrapping_mul(418_711))
        .wrapping_add(z.wrapping_mul(13761))
        & 0xFF;
    let speed = 3.0 + random.next_float();
    let offset = -((wrapped_ticks + tick_offset) as f32 + partial) / 32.0 * speed;
    Column {
        x,
        z,
        bottom_y,
        top_y,
        u_offset: 0.0,
        v_offset: offset % 32.0,
        light_coords,
    }
}

/// `createSnowColumnInstance`: snow drifts sideways and is brightened.
#[allow(clippy::too_many_arguments)]
pub fn snow_column(
    random: &mut impl RandomSource,
    ticks: i64,
    x: i32,
    bottom_y: i32,
    top_y: i32,
    z: i32,
    light_coords: i32,
    partial: f32,
) -> Column {
    let wrapped_ticks = (ticks & 131_071) as i32;
    let time = wrapped_ticks as f32 + partial;
    let u = random.next_double();
    let u = (u + f64::from(time * 0.01 * random.next_gaussian() as f32)) as f32;
    let v = random.next_double();
    let v = (v + f64::from(time * random.next_gaussian() as f32 * 0.001)) as f32;
    let v_offset = -((ticks & 511) as f32 + partial) / 512.0;
    let (block, sky) = (light_coords >> 4 & 15, light_coords >> 20 & 15);
    Column {
        x,
        z,
        bottom_y,
        top_y,
        u_offset: u,
        v_offset: v_offset + v,
        light_coords: ((block * 3 + 15) / 4) << 4 | ((sky * 3 + 15) / 4) << 20,
    }
}

/// The constructor's 32x32 table of horizontal column half-widths, indexed
/// relative to the camera block. The camera's own column divides 0 by 0.
fn column_size(index: usize) -> (f32, f32) {
    let delta_x = (index % 32) as f32 - 16.0;
    let delta_z = (index / 32) as f32 - 16.0;
    let distance = f64::from(delta_x * delta_x + delta_z * delta_z).sqrt() as f32;
    (-delta_z / distance, delta_x / distance)
}

/// `prepareInstances` for one column: rain uses `max_alpha` 1, snow 0.8.
pub fn column_quad(
    column: &Column,
    camera: [f64; 3],
    max_alpha: f32,
    radius: i32,
    intensity: f32,
) -> [ColumnVertex; 4] {
    let radius_sq = (radius * radius) as f32;
    let relative_x = (f64::from(column.x) + 0.5 - camera[0]) as f32;
    let relative_z = (f64::from(column.z) + 0.5 - camera[2]) as f32;
    let distance_sq = relative_x * relative_x + relative_z * relative_z;
    let t = (distance_sq / radius_sq).min(1.0);
    let alpha = (max_alpha + t * (0.5 - max_alpha)) * intensity;
    let color = ((alpha * 255.0).floor() as i32 as u32) << 24 | 0xFF_FFFF;
    let index =
        (column.z - camera[2].floor() as i32 + 16) * 32 + column.x - camera[0].floor() as i32 + 16;
    let (size_x, size_z) = column_size(index as usize);
    let (half_x, half_z) = (size_x / 2.0, size_z / 2.0);
    let (x0, x1) = (relative_x - half_x, relative_x + half_x);
    let y1 = (f64::from(column.top_y) - camera[1]) as f32;
    let y0 = (f64::from(column.bottom_y) - camera[1]) as f32;
    let (z0, z1) = (relative_z - half_z, relative_z + half_z);
    let (u0, u1) = (column.u_offset, column.u_offset + 1.0);
    let v0 = column.bottom_y as f32 * 0.25 + column.v_offset;
    let v1 = column.top_y as f32 * 0.25 + column.v_offset;
    let vertex = |pos, uv| ColumnVertex {
        pos,
        uv,
        color,
        light_coords: column.light_coords,
    };
    [
        vertex([x0, y1, z0], [u0, v0]),
        vertex([x1, y1, z1], [u1, v0]),
        vertex([x1, y0, z1], [u1, v1]),
        vertex([x0, y0, z0], [u0, v1]),
    ]
}

/// Split a column quad (vertices as [`column_quad`] emits them: top edge
/// `u0..u1` at `v0`, bottom edge at `v1`) at every whole texture repeat.
/// Each piece gets positions on the quad and texture coordinates in `0..=1`
/// of one repeat, ready to map onto an atlas sprite.
pub fn tile(quad: &[ColumnVertex; 4], mut emit: impl FnMut([[f32; 3]; 4], [[f32; 2]; 4])) {
    let [u0, v0] = quad[0].uv;
    let [u1, v1] = quad[2].uv;
    let (top_left, top_right) = (quad[0].pos, quad[1].pos);
    let (bottom_right, bottom_left) = (quad[2].pos, quad[3].pos);
    let lerp3 = |a: [f32; 3], b: [f32; 3], t: f32| -> [f32; 3] {
        std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
    };
    // Position at fraction s across (u) and t down (v) the quad.
    let at = |s: f32, t: f32| {
        lerp3(
            lerp3(top_left, top_right, s),
            lerp3(bottom_left, bottom_right, s),
            t,
        )
    };
    if !(u1 > u0 && v1 > v0) {
        return;
    }
    for (va, vb) in pieces(v0, v1) {
        let repeat_v = va.floor();
        let (ta, tb) = ((va - v0) / (v1 - v0), (vb - v0) / (v1 - v0));
        let (la, lb) = (va - repeat_v, vb - repeat_v);
        for (ua, ub) in pieces(u0, u1) {
            let repeat_u = ua.floor();
            let (sa, sb) = ((ua - u0) / (u1 - u0), (ub - u0) / (u1 - u0));
            let (ka, kb) = (ua - repeat_u, ub - repeat_u);
            emit(
                [at(sa, ta), at(sb, ta), at(sb, tb), at(sa, tb)],
                [[ka, la], [kb, la], [kb, lb], [ka, lb]],
            );
        }
    }
}

/// `a..b` cut at every whole number in between, as consecutive pairs
/// (without allocating: this runs for every column of every frame).
fn pieces(a: f32, b: f32) -> impl Iterator<Item = (f32, f32)> {
    let first = a.floor() + 1.0;
    let cuts = move || {
        std::iter::once(a)
            .chain(
                (0..)
                    .map(move |k| first + k as f32)
                    .take_while(move |&k| k < b),
            )
            .chain(std::iter::once(b))
    };
    cuts().zip(cuts().skip(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rain_repeats_every_four_blocks() {
        let column = Column {
            x: 3,
            z: 0,
            bottom_y: 60,
            top_y: 80,
            u_offset: 0.0,
            v_offset: 0.0,
            light_coords: 15 << 20,
        };
        let quad = column_quad(&column, [0.5, 70.0, 0.5], 1.0, 10, 1.0);
        // 20 blocks of rain: five repeats of the texture, split at each seam.
        let mut pieces = Vec::new();
        tile(&quad, |pos, uv| pieces.push((pos, uv)));
        assert_eq!(pieces.len(), 5);
        for (pos, uv) in &pieces {
            assert!(((pos[0][1] - pos[3][1]) - 4.0).abs() < 1e-4);
            assert_eq!(uv[0], [0.0, 0.0]);
            assert_eq!(uv[2], [1.0, 1.0]);
        }
    }

    #[test]
    fn snow_drifts_across_one_seam() {
        let column = Column {
            x: 3,
            z: 2,
            bottom_y: 64,
            top_y: 66,
            u_offset: 0.25,
            v_offset: 0.0,
            light_coords: 0,
        };
        let quad = column_quad(&column, [0.5, 70.0, 0.5], 0.8, 10, 1.0);
        let mut pieces = Vec::new();
        tile(&quad, |pos, uv| pieces.push((pos, uv)));
        assert_eq!(pieces.len(), 2);
        assert_eq!(pieces[0].1[0], [0.25, 0.0]);
        assert_eq!(pieces[1].1[2], [0.25, 0.5]);
    }

    #[test]
    fn pieces_cut_at_whole_numbers() {
        let all = |a, b| pieces(a, b).collect::<Vec<_>>();
        assert_eq!(all(0.25, 2.5), [(0.25, 1.0), (1.0, 2.0), (2.0, 2.5)]);
        assert_eq!(all(1.0, 2.0), [(1.0, 2.0)]);
        assert_eq!(all(-0.5, 0.5), [(-0.5, 0.0), (0.0, 0.5)]);
        assert_eq!(all(3.25, 3.75), [(3.25, 3.75)]);
    }

    #[test]
    fn camera_column_is_degenerate() {
        let column = Column {
            x: 0,
            z: 0,
            bottom_y: 60,
            top_y: 80,
            u_offset: 0.0,
            v_offset: 0.0,
            light_coords: 0,
        };
        let quad = column_quad(&column, [0.5, 70.0, 0.5], 1.0, 10, 1.0);
        assert!(quad[0].pos[0].is_nan());
    }
}
