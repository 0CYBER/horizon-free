use super::*;

fn stand(x: f64, y: f64, z: f64) -> ArmorStand {
    ArmorStand::new(1, Vec3::new(x, y, z), 0.0)
}

/// `LivingEntity.getDimensions`: a server's `minecraft:scale` sizes the stand's box, the
/// one clicks pick and the one drawn, on top of `Small`; a marker has none.
#[test]
fn scale_sizes_the_box() {
    let mut a = stand(0.0, 0.0, 0.0);
    a.scale = 0.5;
    assert_eq!(a.box_size(), (0.25, 0.9875));
    let b = a.bounds();
    assert!((b.max_position().y - f64::from(1.975_f32) * 0.5).abs() < 1e-9);
    assert!((b.max_position().x - 0.125).abs() < 1e-9);
    a.scale = 2.0;
    a.small = true;
    assert_eq!(a.box_size(), (0.5, 1.975));
    a.marker = true;
    assert_eq!(a.box_size(), (0.0, 0.0));
}

/// `ArmorStand.getLightProbePosition` for a stand that is not a marker is
/// `Entity.getEyePosition`: `EntityType.ARMOR_STAND` declares
/// `eyeHeight(1.7775)`, and a small stand (`isBaby()`) is the same type
/// scaled by 0.5 with `withEyeHeight(0.9875)`. Horizon read
/// `floor(base + 0.5)` for every stand, so a stand standing on a lit floor
/// took the floor's light instead of the light at its head.
#[test]
fn a_stand_is_lit_from_the_block_its_eye_is_in() {
    let dark = |_: i32, _: i32, _: i32| 0u8;
    // Full stand on y = 64: eye 65.7775, cell 65 (the old 64.5 was 64).
    assert_eq!(stand(0.5, 64.0, 0.5).light_probe(1.0, dark), [0, 65, 0]);
    assert_eq!((64.0_f64 + 0.5).floor() as i32, 64);
    // Small stand: eye 0.9875 above its base, so y = 64.2 is 65.1875.
    let mut small = stand(0.5, 64.2, 0.5);
    small.small = true;
    assert_eq!(small.light_probe(1.0, dark), [0, 65, 0]);
    assert_eq!((64.2_f64 + 0.5).floor() as i32, 64);
    // A small stand at y = 64.0 stays in its own cell: eye 64.9875.
    let mut low = stand(0.5, 64.0, 0.5);
    low.small = true;
    assert_eq!(low.light_probe(1.0, dark), [0, 64, 0]);
}

/// `getEyePosition(partialTick)` interpolates: a stand falling from 70 to
/// 69 is probed 0.5 down at the middle of the frame, 69.5 + 1.7775 in the
/// cell at 71, while its feet's cell is 69.
#[test]
fn a_stand_probe_follows_the_frame() {
    let dark = |_: i32, _: i32, _: i32| 0u8;
    let mut falling = stand(0.5, 70.0, 0.5);
    falling.interpolate(Some((Vec3::new(0.5, 69.0, 0.5), 0.0)));
    assert_eq!(falling.light_probe(0.0, dark), [0, 71, 0]);
    assert_eq!(falling.light_probe(0.5, dark), [0, 71, 0]);
    assert_eq!(falling.light_probe(1.0, dark), [0, 70, 0]);
    assert_eq!(falling.position.y.floor() as i32, 69);
}

/// A marker walks `BlockPos.betweenClosed` of its own box - the size it has
/// without the marker flag, 0.5 by 1.975 - from `position()`, taking the
/// first cell at brightness 15 and else the brightest. Horizon used the
/// stand's own cell, so a marker (a hologram on a dark ceiling) was lit by
/// its feet's block.
#[test]
fn a_marker_scans_the_cells_of_its_own_box() {
    let at = |want: [i32; 3], level: u8| {
        move |x: i32, y: i32, z: i32| {
            if [x, y, z] == want { level } else { 0 }
        }
    };
    let mut marker = stand(0.5, 64.0, 0.5);
    marker.marker = true;
    // The box is x/z 0.25..0.75 - the cell at 0 - and y 64..65.975.
    assert_eq!(marker.light_probe(1.0, at([0, 65, 0], 15)), [0, 65, 0]);
    assert_eq!(marker.light_probe(1.0, at([0, 65, 0], 9)), [0, 65, 0]);
    // Nothing lit: the first cell walked, x fastest then y then z.
    assert_eq!(marker.light_probe(1.0, |_, _, _| 0u8), [0, 64, 0]);
    // The cell above the box is not scanned, so its 15 does not win.
    assert_eq!(marker.light_probe(1.0, at([0, 66, 0], 15)), [0, 64, 0]);
    // 15 wins over a brighter-looking neighbour, and y is walked before z:
    // (0, 65, 0) comes before (0, 64, 1) though both are at 15.
    let mut wide = stand(0.5, 64.0, 0.9);
    wide.marker = true;
    let two = |x: i32, y: i32, z: i32| {
        if (x, y, z) == (0, 65, 0) || (x, y, z) == (0, 64, 1) {
            15
        } else {
            0
        }
    };
    assert_eq!(wide.light_probe(1.0, two), [0, 65, 0]);
    // `getLightProbePosition`'s marker branch reads `position()`, not the
    // partial-tick position: falling does not move the scan.
    let mut moving = stand(0.5, 64.0, 0.5);
    moving.marker = true;
    moving.interpolate(Some((Vec3::new(0.5, 70.0, 0.5), 0.0)));
    assert_eq!(moving.light_probe(0.0, |_, _, _| 0u8), [0, 70, 0]);
    assert_eq!(moving.light_probe(0.5, |_, _, _| 0u8), [0, 70, 0]);
}

/// `EntityRenderer.getBlockLightLevel`: a stand `isOnFire` is drawn at 15
/// block light whatever the level says, and the sky light is the probe
/// cell's own.
#[test]
fn a_burning_stand_is_drawn_at_full_block_light() {
    let light = |sky: u8, block: u8| (sky << 4) | block;
    let cold = stand(0.5, 64.0, 0.5);
    assert_eq!(cold.render_light(light(15, 0)), (15 * 16) as u32);
    assert_eq!(cold.render_light(light(0, 9)), ((9 * 16) << 8) as u32);
    let burning = ArmorStand {
        fire_ticks: 100,
        ..stand(0.5, 64.0, 0.5)
    };
    assert_eq!(
        burning.render_light(light(15, 0)),
        ((15 * 16) | ((15 * 16) << 8)) as u32
    );
    assert_eq!(burning.render_light(light(0, 0)), ((15 * 16) << 8) as u32);
}
