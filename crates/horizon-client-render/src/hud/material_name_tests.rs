//! Names ending in `_n` or `_s` in the HUD atlas: ordinary images unless the LabPBR maps are
//! read, when the ones beside a texture are its maps.
use super::*;
use horizon_assets::resource_source::Resource;

fn source(names: &[&str]) -> ResourceSource {
    ResourceSource::indexed(
        names
            .iter()
            .map(|name| {
                (
                    format!("assets/minecraft/textures/{name}.png"),
                    Resource::bytes(grey_png(2, 2)),
                )
            })
            .collect(),
    )
}

/// With discovery off, `gui/icon_n` and `gui/icon_s` are images like any other, beside their
/// base or alone.
#[test]
fn ordinary_suffix_names_are_packed_beside_their_base() {
    let names = [
        "gui/icon",
        "gui/icon_n",
        "gui/icon_s",
        "gui/lone_n",
        "entity/figure",
        "entity/figure_n",
    ];
    let root = source(&names);
    let atlas = HudAtlas::pack(&root, &root, &["gui", "entity"], false, false).unwrap();
    for name in names {
        assert!(atlas.sprites.contains_key(name), "missing sprite {name}");
    }
}

/// With discovery on, a map beside its texture is not drawn as an image (it would take the
/// atlas twice); a suffix name with no texture beside it still is.
#[test]
fn maps_beside_a_texture_are_not_ordinary_images_when_they_are_read() {
    let root = source(&["gui/icon", "gui/icon_n", "gui/icon_s", "gui/lone_n"]);
    assert!(is_companion_map(&root, "gui/icon_n", true));
    assert!(is_companion_map(&root, "gui/icon_s", true));
    assert!(!is_companion_map(&root, "gui/lone_n", true));
    assert!(!is_companion_map(&root, "gui/icon", true));
    for name in ["gui/icon_n", "gui/icon_s", "gui/lone_n"] {
        assert!(!is_companion_map(&root, name, false), "{name}");
    }
}
