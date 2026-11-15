use super::*;

/// An obsidian frame along X around `width` by `height` empty blocks, its bottom-left inner block at
/// (1, 1, 0) (raw writes: nothing reacts), and the shape `PortalShape.findAnyShape` finds from there.
/// None without the prepared resources.
fn shape(width: i32, height: i32) -> Option<portal::PortalShape> {
    let mut w = World::new(fixture()?);
    let obsidian = w.id("obsidian", &[]);
    for x in 1..=width {
        w.put(BlockPos::new(x, 0, 0), obsidian);
        w.put(BlockPos::new(x, height + 1, 0), obsidian);
    }
    for y in 1..=height {
        w.put(BlockPos::new(0, y, 0), obsidian);
        w.put(BlockPos::new(width + 1, y, 0), obsidian);
    }
    let found =
        w.f.behaviours
            .portal_shape(&mut w.level, &mut w.ticks, BlockPos::new(1, 1, 0), true);
    Some(found)
}

/// PortalShape: MIN_WIDTH 2, MAX_WIDTH 21, MIN_HEIGHT 3, MAX_HEIGHT 21. A frame around 2 by 3 or 21
/// by 21 empty blocks is a portal's; one block narrower or wider, lower or taller is not.
#[test]
fn a_portal_frame_holds_2_to_21_by_3_to_21_blocks() {
    for (width, height, valid) in [
        (2, 3, true),
        (21, 21, true),
        (21, 3, true),
        (2, 21, true),
        (1, 3, false),
        (22, 3, false),
        (2, 2, false),
        (2, 22, false),
    ] {
        let Some(found) = shape(width, height) else {
            return;
        };
        assert_eq!(found.is_valid(), valid, "{width} by {height}: {found:?}");
        if valid {
            assert_eq!((found.width, found.height), (width, height));
        }
    }
}
