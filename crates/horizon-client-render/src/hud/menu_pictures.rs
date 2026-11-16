//! The menu pictures the GUI atlas leaves out: the title panorama and the world-preset
//! pictures, drawn by the menu on their own.

/// A texture (`assets/<ns>/textures/...`) the menu draws on its own, not from the GUI
/// atlas: the title panorama and the world-preset pictures, except the ones a run-time
/// slot shows. Only those two folders count, so a sprite whose own name holds `presets`
/// (a registered atlas's `gui/atlases/presets/...`) is still stitched.
pub(super) fn menu_picture_only(path: &str) -> bool {
    const SLOT_PICTURES: [&str; 4] = [
        "textures/gui/title/background/panorama_0.png",
        "textures/gui/title/background/panorama_2.png",
        "textures/gui/title/background/panorama_3.png",
        "textures/gui/presets/isles.png",
    ];
    let Some((_, texture)) = path
        .strip_prefix("assets/")
        .and_then(|rest| rest.split_once('/'))
    else {
        return false;
    };
    !SLOT_PICTURES.contains(&texture)
        && (texture.starts_with("textures/gui/title/background/")
            || texture.starts_with("textures/gui/presets/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_menu_picture_folders_stay_out_of_the_gui_atlas() {
        for kept_out in [
            "assets/minecraft/textures/gui/title/background/panorama_1.png",
            "assets/minecraft/textures/gui/presets/desert.png",
        ] {
            assert!(menu_picture_only(kept_out), "{kept_out}");
        }
        for stitched in [
            "assets/minecraft/textures/gui/title/background/panorama_0.png",
            "assets/minecraft/textures/gui/presets/isles.png",
            "assets/mymod/textures/gui/atlases/presets/mymod/heart.png",
            "assets/mymod/textures/gui/atlases/icons/mymod/title/background/sky.png",
            "assets/minecraft/textures/gui/sprites/icon/presets.png",
        ] {
            assert!(!menu_picture_only(stitched), "{stitched}");
        }
    }
}
