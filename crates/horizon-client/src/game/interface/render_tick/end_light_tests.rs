//! `Lightmap.getBrightness(dimensionType, getMaxLocalRawBrightness(eye))`, the
//! brightness `Hud.updateVignetteBrightness` and `LevelExtractor`'s water
//! overlay read: it lerps the curved level with the level's own
//! `DimensionType.ambientLight`, so the end's 0.25 lightens both there.
use super::Game;
use crate::chunk::{Chunk, Light, Section};
use crate::game::Dim;
use std::sync::Arc;

/// A game on the prepared resources whose column has no sky and no block light
/// in its section 8 (y 64..79), or `None` without prepared resources.
fn dark_column_game() -> Option<Game<'static>> {
    std::env::var_os("HORIZON_TEST_RESOURCES")?;
    let mut game = crate::game::native_machines::tests::server_game();
    let mut chunk = Chunk::empty(0, 0);
    chunk.sections[8] = Arc::new(Section {
        light: Light::Uniform(0),
        ..Default::default()
    });
    game.world.chunks.insert((0, 0), Arc::new(chunk));
    Some(game)
}

#[test]
fn the_end_lightmap_brightness_includes_its_ambient_light() {
    let Some(mut game) = dark_column_game() else {
        return;
    };
    let eye = [0.5, 64.5, 0.5];
    let brightness = |game: &mut Game<'_>, dim: Dim| {
        game.dim = dim;
        game.eye_brightness(eye)
    };
    // No light means the curved level is 0, so the brightness is the
    // dimension type's ambient light (`DimensionTypes.END` has 0.25).
    assert_eq!(brightness(&mut game, Dim::End), 0.25, "the end");
    assert_eq!(brightness(&mut game, Dim::Nether), 0.1, "the nether");
    assert_eq!(brightness(&mut game, Dim::Overworld), 0.0, "the overworld");
}
