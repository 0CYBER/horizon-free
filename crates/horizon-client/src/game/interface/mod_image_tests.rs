//! A client mod's own image (`ClientHost::mod_image`): its atlas slot follows the loaded mods,
//! native or wasm, with no mixin (arismoko/horizon#4437).
use super::resource_tables::mod_image_slot;
use horizon_client::{
    gfx::entity_pixel_test::render_hud,
    hud::{HudBatch, MOD_IMAGE_SPRITE},
};
use mod_api::{Hooks, ModEntry};
use mod_platform::testing::with_mods;

const SIZE: [u32; 2] = [64, 64];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLACK: [u8; 4] = [0, 0, 0, 255];

/// A mod with no hook and no mixin: it only uses the API.
struct ApiOnly;

#[mod_api::__subscriptions]
impl Hooks for ApiOnly {}

#[test]
fn the_slot_follows_the_loaded_client_mods_native_or_wasm() {
    assert!(!with_mods(Vec::new().into(), mod_image_slot), "no mod");
    let native = vec![ModEntry::new("api-only", Box::new(ApiOnly))];
    assert!(
        with_mods(native.into(), mod_image_slot),
        "a native mod with no mixin"
    );
    let wasm = vec![ModEntry::wasm("wasm-mod", Box::new(ApiOnly))];
    assert!(with_mods(wasm.into(), mod_image_slot), "a wasm mod");
}

fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    pixels[((y * SIZE[0] + x) * 4) as usize..][..4]
        .try_into()
        .unwrap()
}

/// The game's own atlas, built with a native mod that has no mixin loaded, has the slot; the
/// mod's image uploads into it and draws through `native_sprite` on the real HUD pipeline.
#[test]
#[ignore = "daily"]
fn an_api_only_mods_image_draws_with_no_mixin() {
    let mods = vec![ModEntry::new("api-only", Box::new(ApiOnly))];
    with_mods(mods.into(), || {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            assert!(game.gui.atlas.sprites.contains_key(MOD_IMAGE_SPRITE));
            assert!(game.mod_image(16, 16, &GREEN.repeat(16 * 16)));
            let mut batch = HudBatch::scratch(SIZE);
            batch.patches.append(&mut game.map_patches);
            let (src, dst) = ([0.0, 0.0, 16.0, 16.0], [16.0, 16.0, 32.0, 32.0]);
            assert!(game.native_sprite(&mut batch, MOD_IMAGE_SPRITE, src, dst, None));
            let pixels = render_hud(&batch, &game.gui.atlas, SIZE).expect("software Vulkan draw");
            assert_eq!(pixel(&pixels, 32, 32), GREEN, "the mod's image");
            assert_eq!(pixel(&pixels, 4, 4), BLACK, "outside the sprite");
        });
    });
}
