//! A higher-resolution pack's sprites in the HUD atlas: the packed pixels keep
//! the pack's resolution while a `Rect` carries the default resources' logical
//! size, so the GUI draws in the units its code uses whatever the pack
//! (issue #19022's Faithful 32x/64x inventory sprites).
use super::*;
use horizon_assets::resource_source::Resource;

/// A 16x16 default replaced by a 32x32 pack texture: the sprite's logical size
/// stays the default's, and the atlas holds the pack's own pixels.
#[test]
fn a_32x_pack_texture_keeps_the_default_sprite_size() {
    let defaults = index(vec![("item/test", 16)]);
    let overlay = [(
        "assets/minecraft/textures/item/test.png".to_owned(),
        Resource::bytes(grey_png(32, 32)),
    )]
    .into_iter()
    .collect();
    let root = ResourceSource::layered(overlay, defaults.clone());
    let atlas = HudAtlas::pack(&root, &defaults, &["item"], false, false).unwrap();
    let rect = atlas.sprites["item/test"];
    let (tw, th) = rect.texels((atlas.width, atlas.height));
    assert_eq!(
        (rect.w, rect.h),
        (16, 16),
        "the sprite's logical size must stay the default's, not the pack's"
    );
    assert_eq!(
        (tw, th),
        (32, 32),
        "the atlas must hold the pack's own 32x32 pixels"
    );
}

/// The production callers pass the resolved overlay as its own `defaults`
/// (`build_over_with_image(&source, &source, ..)`): the sprite's logical size
/// must then still come from the layer the pack resolved over, not from the
/// root — the root answers a replaced texture with the pack's own file, whose
/// size is no logical size at all (#19022).
#[test]
fn a_pack_over_its_own_defaults_keeps_the_default_sprite_size() {
    let base = index(vec![("item/test", 16)]);
    let overlay = [(
        "assets/minecraft/textures/item/test.png".to_owned(),
        Resource::bytes(grey_png(32, 32)),
    )]
    .into_iter()
    .collect();
    let root = ResourceSource::layered(overlay, base);
    let atlas = HudAtlas::pack(&root, &root, &["item"], false, false).unwrap();
    let rect = atlas.sprites["item/test"];
    assert_eq!(
        (rect.w, rect.h),
        (16, 16),
        "the sprite's logical size must be the layer the pack resolved over"
    );
}

/// A pack whose sprites only fit once the atlas halves them toward their
/// logical sizes: the logical-size filter lets a 128x pack's GUI and entity
/// textures in at full resolution, past what one atlas holds (#19022's
/// review), so the layout falls back to halving them down to the logical
/// size before giving up.
#[test]
fn a_pack_that_only_fits_once_its_hires_sprites_are_reduced() {
    let mut images = vec![
        (
            "gui/sprites/container/inventory".to_owned(),
            512,
            512,
            vec![9; 512 * 512 * 4],
        ),
        ("item/test".to_owned(), 64, 64, vec![7; 64 * 64 * 4]),
    ];
    let gui_sizes = HashMap::from([
        ("gui/sprites/container/inventory".to_owned(), (256, 256)),
        ("item/test".to_owned(), (16, 16)),
    ]);
    // The unreduced shelf (512 tall) plus the glyph rows misses a 800-tall
    // atlas; the reduced one (256 tall) fits.
    let (width, placements, glyph_region) =
        HudAtlas::layout_reducing_hires(&mut images, &gui_sizes, 800).unwrap();
    assert_eq!(width, 800);
    assert_eq!(placements.len(), 2);
    assert!(glyph_region + GLYPH_ROWS <= 800);
    let big = images
        .iter()
        .find(|i| i.0 == "gui/sprites/container/inventory")
        .unwrap();
    assert_eq!(
        (big.1, big.2),
        (256, 256),
        "reduced to the logical size, not past it"
    );
    assert_eq!(big.3.len(), 256 * 256 * 4);
    let item = images.iter().find(|i| i.0 == "item/test").unwrap();
    assert_eq!((item.1, item.2), (32, 32), "halved one step toward 16");
}

/// A pack the reduction cannot fit (its sprites are at their logical sizes
/// already) keeps the layout error.
#[test]
fn a_pack_its_reduction_cannot_fit_still_fails() {
    let mut images = vec![("item/test".to_owned(), 16, 16, vec![7; 16 * 16 * 4])];
    let gui_sizes = HashMap::from([("item/test".to_owned(), (16, 16))]);
    let error = HudAtlas::layout_reducing_hires(&mut images, &gui_sizes, 64).unwrap_err();
    assert!(error.to_string().contains("do not fit"), "{error}");
}

/// An index with one grey PNG per entry.
fn index(files: Vec<(&str, u32)>) -> ResourceSource {
    let map = files
        .into_iter()
        .map(|(name, side)| {
            let path = format!("assets/minecraft/textures/{name}.png");
            (path, Resource::bytes(grey_png(side, side)))
        })
        .collect();
    ResourceSource::indexed(map)
}
