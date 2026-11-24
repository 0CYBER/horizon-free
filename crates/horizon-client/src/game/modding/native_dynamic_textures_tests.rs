//! A client mod's dynamic textures through the game (#11138): with a native mod loaded and no
//! mixin, an image the mod creates draws under its id on the real HUD pipeline from the next
//! frame's patches, with no resource reload, goes up again for a new renderer, and lasts until
//! the mod frees it.
use horizon_client::{gfx::entity_pixel_test::render_hud, hud::HudBatch};
use mod_api::{
    DynamicTextureError, Hooks, Identifier, ModEntry,
    content::crash::{ModCall, ModGuard},
};
use mod_platform::testing::with_mods;

const SIZE: [u32; 2] = [64, 64];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLACK: [u8; 4] = [0, 0, 0, 255];

/// A mod with no hook and no mixin: it only uses the API.
struct ApiOnly;

#[mod_api::__subscriptions]
impl Hooks for ApiOnly {}

fn in_mod(id: &'static str) -> ModGuard {
    mod_platform::content::crash::enter_native_mod(ModCall {
        id,
        hook: "client_tick_post",
        part: None,
    })
}

fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    pixels[((y * SIZE[0] + x) * 4) as usize..][..4]
        .try_into()
        .unwrap()
}

#[test]
#[ignore = "daily"]
fn a_mods_dynamic_texture_draws_under_its_id_until_it_is_freed() {
    let mods = vec![ModEntry::new("api-only", Box::new(ApiOnly))];
    with_mods(mods.into(), || {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            let (id, name) = (
                Identifier::parse("api-only:emoji/green").unwrap(),
                "api-only:emoji/green",
            );
            let rgba = GREEN.repeat(16 * 16);
            assert_eq!(
                game.create_dynamic_texture(&id, 16, 16, &rgba),
                Err(DynamicTextureError::NotOwned),
                "outside a mod's call no mod owns the id"
            );
            let _call = in_mod("api-only");
            game.create_dynamic_texture(&id, 16, 16, &rgba).unwrap();
            let mut batch = HudBatch::scratch(SIZE);
            game.dynamic_textures
                .patches(&mut game.gui.atlas, &mut batch.patches);
            let (src, dst) = ([0.0, 0.0, 16.0, 16.0], [16.0, 16.0, 32.0, 32.0]);
            assert!(game.native_sprite(&mut batch, name, src, dst, None));
            let pixels = render_hud(&batch, &game.gui.atlas, SIZE).expect("software Vulkan draw");
            assert_eq!(pixel(&pixels, 32, 32), GREEN, "the mod's image");
            assert_eq!(pixel(&pixels, 4, 4), BLACK, "outside the sprite");
            // A new renderer's atlas texture holds only the packed pixels: the image goes up
            // again with the next frame.
            game.forget_atlas_slots();
            let mut again = Vec::new();
            game.dynamic_textures
                .patches(&mut game.gui.atlas, &mut again);
            assert_eq!(again.len(), 1, "a new renderer gets the image again");
            game.free_dynamic_texture(&id).unwrap();
            let mut batch = HudBatch::scratch(SIZE);
            assert!(!game.native_sprite(&mut batch, name, src, dst, None));
        });
    });
}
