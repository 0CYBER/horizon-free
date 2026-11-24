use super::*;
use crate::game::api_values::TryIntoApi;

/// Helpers for native mods, kept as stable as the hook points.
// Native mods use these; nothing in Horizon does.
#[allow(dead_code)]
impl Game<'_> {
    /// Playing, with nothing over the world: keys and zoom apply.
    pub(in crate::game) fn playing(&self) -> bool {
        self.screen == Screen::Playing
            && self.title.is_none()
            && self.sign_edit.is_none()
            && self.book.is_none()
    }
    /// Milliseconds on the game's own clock (`Util.getMillis`), counting from the game's start:
    /// the clock the HUD's scrolling labels and the screens' text fields read too.
    pub(in crate::game) fn millis(&self) -> u64 {
        horizon_client::hud_overlays::now_ms().max(0) as u64
    }
    /// The current window modifier keys in the same GLFW encoding as input hooks.
    /// Cleared when the window loses focus; available even outside a world.
    pub(in crate::game) fn modifiers(&self) -> ClientModifiers {
        ClientModifiers::new(crate::game::glfw_modifiers(self.modifiers))
    }
    /// A vanilla mapping (`key.forward`, `key.jump`, `key.attack`…): whether
    /// it is held (`KeyMapping.isDown`), and its key's name as Controls shows
    /// it. None for a mapping it doesn't answer for.
    pub(in crate::game) fn mapping(
        &self,
        name: &str,
    ) -> Result<Option<mod_api::KeyMapping>, mod_api::TextFormError> {
        let Some(&(_, key)) = MAPPINGS.iter().find(|m| m.0 == name) else {
            return Ok(None);
        };
        let down = match key {
            Some(k) => self.movement_key_held(k),
            // The button, or the mapping's own held state (a toggle, a pad, `set_mapping_down`).
            None => {
                let use_key = usize::from(name == "key.use");
                self.mouse_held()[use_key] || [self.left, self.right][use_key]
            }
        };
        Ok(Some(mod_api::KeyMapping {
            down,
            label: keybinds::display_name(self.settings.keys.get(name)).try_into_api()?,
        }))
    }
    /// Every registered key mapping across Minecraft vanilla controls, Horizon built-ins,
    /// and loaded mods, with translated labels, current/default keys, modifiers, and conflict scenes.
    pub(in crate::game) fn all_keybinds(&self) -> Vec<mod_api::RegisteredKeyBind> {
        build_all_keybinds(&self.settings)
    }
    /// Rebinds a mapping (`owner == ""` for Vanilla/Horizon id; non-empty `owner` for a mod's
    /// registration id), saves settings, and syncs live bindings.
    pub(in crate::game) fn set_keybind(
        &mut self,
        owner: &str,
        id: &str,
        key: &str,
        modifiers: u32,
    ) -> bool {
        if !apply_keybind_change(&mut self.settings, owner, id, key, modifiers) {
            return false;
        }
        self.settings.save(&Self::options_path());
        self.settings.keys.sync_action_bindings();
        self.apply_settings();
        self.ui.options.layout_changed();
        true
    }
}

/// Owner titles of the built-in mappings, as translation keys.
const MINECRAFT_TITLE: &str = "horizon.keybinds.owner.minecraft";
const HORIZON_TITLE: &str = "horizon.keybinds.owner.horizon";

fn build_all_keybinds(settings: &crate::game::Settings) -> Vec<mod_api::RegisteredKeyBind> {
    let mut out = Vec::new();
    for (name, default_key, category) in keybinds::all_mappings() {
        let is_horizon = keybinds::HORIZON_MAPPINGS.iter().any(|(n, _, _)| n == name);
        let cat_key = format!("key.category.minecraft.{category}");
        out.push({
            let mut b = mod_api::RegisteredKeyBind::default();
            b.owner_title = horizon_client::lang::get(if is_horizon {
                HORIZON_TITLE
            } else {
                MINECRAFT_TITLE
            });
            b.id = (*name).to_owned();
            b.label = horizon_client::lang::get(name);
            b.category = horizon_client::lang::get(&cat_key);
            b.key = settings.keys.get(name).to_owned();
            b.default_key = (*default_key).to_owned();
            b
        });
    }
    out.push({
        let mut b = mod_api::RegisteredKeyBind::default();
        b.owner_title = horizon_client::lang::get(HORIZON_TITLE);
        b.id = horizon_client::extras::SORT_KEY.to_owned();
        b.label = horizon_client::lang::get("horizon.extras.sort_key");
        b.category = horizon_client::lang::get(HORIZON_TITLE);
        b.key = settings.extras.sort_key.clone();
        b.default_key = "key.keyboard.unknown".to_owned();
        b.context = mod_api::KeyConflictContext::Gui;
        b
    });
    out.push({
        let mut b = mod_api::RegisteredKeyBind::default();
        b.owner_title = horizon_client::lang::get(HORIZON_TITLE);
        b.id = horizon_client::pvp::HUD_EDITOR_KEY.to_owned();
        b.label = horizon_client::lang::get("horizon.pvp.hud_editor_key");
        b.category = horizon_client::lang::get(HORIZON_TITLE);
        b.key = settings.extras.pvp.editor_key.clone();
        b.default_key = "key.keyboard.unknown".to_owned();
        b.context = mod_api::KeyConflictContext::InGame;
        b
    });
    for installed in &settings.mods.installed {
        if settings.mods.disabled.contains(&installed.id) {
            continue;
        }
        let Some(page) = installed.native.as_ref() else {
            continue;
        };
        let owner_title = settings
            .mods
            .info
            .get(&installed.id)
            .map(|info| info.name.clone())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| installed.title.clone());
        for mapping in &page.mappings {
            let (key, modifiers) = settings.mods.binding(
                &installed.id,
                &mapping.id,
                (&mapping.key, mapping.modifiers),
            );
            let category = if mapping.category.is_empty() {
                String::new()
            } else {
                horizon_client::lang::get(&mapping.category)
            };
            let context = match mapping.context {
                horizon_client::mod_list::KeyConflictContext::InGame => {
                    mod_api::KeyConflictContext::InGame
                }
                horizon_client::mod_list::KeyConflictContext::Gui => {
                    mod_api::KeyConflictContext::Gui
                }
                horizon_client::mod_list::KeyConflictContext::Universal => {
                    mod_api::KeyConflictContext::Universal
                }
            };
            out.push({
                let mut b = mod_api::RegisteredKeyBind::default();
                b.owner = installed.id.clone();
                b.owner_title = owner_title.clone();
                b.id = mapping.id.clone();
                b.label = horizon_client::lang::get(&mapping.label);
                b.category = category;
                b.key = key;
                b.modifiers = modifiers;
                b.default_key = mapping.key.clone();
                b.default_modifiers = mapping.modifiers;
                b.context = context;
                b
            });
        }
    }
    out
}

fn apply_keybind_change(
    settings: &mut crate::game::Settings,
    owner: &str,
    id: &str,
    key: &str,
    modifiers: u32,
) -> bool {
    let valid_key = key == "key.keyboard.unknown"
        || keybinds::key_code(key).is_some()
        || keybinds::mouse_button(key).is_some();
    if !valid_key {
        return false;
    }
    if owner.is_empty() {
        if id == horizon_client::extras::SORT_KEY || id == horizon_client::pvp::HUD_EDITOR_KEY {
            return settings.extras.set_key(id, key.to_owned());
        }
        if !keybinds::all_mappings().any(|(name, _, _)| *name == id) {
            return false;
        }
        settings.keys.set(id, key);
        return true;
    }
    let Some((default_key, default_mods)) = settings
        .mods
        .installed
        .iter()
        .find(|m| m.id == owner)
        .and_then(|m| m.native.as_ref())
        .and_then(|p| p.mappings.iter().find(|k| k.id == id))
        .map(|k| (k.key.clone(), k.modifiers))
    else {
        return false;
    };
    let mods = modifiers & 0b0111;
    if key == default_key && mods == default_mods {
        settings.mods.reset_binding(owner, id);
    } else {
        settings.mods.set_binding(owner, id, key, mods);
    }
    true
}

#[cfg(test)]
mod keygrid_host_tests {
    use super::{apply_keybind_change, build_all_keybinds};
    use crate::game::Settings;
    use horizon_client::mod_list::{
        Installed, KeyConflictContext as PkgContext, KeyMapping as PkgKeyMapping, NativePage,
    };

    #[test]
    fn keybinds_enumerate_and_apply_across_vanilla_horizon_and_mods() {
        let mut settings = Settings::default();
        settings.mods.installed.push(Installed {
            id: "demo".into(),
            title: "Demo 1.0".into(),
            description: String::new(),
            problem: None,
            dev: false,
            native: Some(NativePage {
                mappings: vec![PkgKeyMapping {
                    id: "sort".into(),
                    label: "key.demo.sort".into(),
                    key: "key.keyboard.r".into(),
                    modifiers: 0,
                    context: PkgContext::Gui,
                    category: String::new(),
                }],
                settings: Vec::new(),
            }),
        });
        assert!(!apply_keybind_change(
            &mut settings,
            "",
            "key.forward",
            "invalid.key",
            0
        ));
        assert!(!apply_keybind_change(
            &mut settings,
            "",
            "key.does.not.exist",
            "key.keyboard.g",
            0
        ));
        assert!(apply_keybind_change(
            &mut settings,
            "",
            "key.forward",
            "key.mouse.4",
            0
        ));
        assert!(apply_keybind_change(
            &mut settings,
            "",
            horizon_client::extras::SORT_KEY,
            "key.keyboard.k",
            0
        ));
        assert!(apply_keybind_change(
            &mut settings,
            "demo",
            "sort",
            "key.keyboard.g",
            1 | 8
        ));

        let list = build_all_keybinds(&settings);
        let fwd = list
            .iter()
            .find(|b| b.owner.is_empty() && b.id == "key.forward")
            .unwrap();
        assert_eq!(fwd.key, "key.mouse.4");
        assert_eq!(
            fwd.owner_title,
            horizon_client::lang::get(super::MINECRAFT_TITLE)
        );
        let sort = list
            .iter()
            .find(|b| b.id == horizon_client::extras::SORT_KEY)
            .unwrap();
        assert_eq!(sort.key, "key.keyboard.k");
        assert_eq!(
            sort.owner_title,
            horizon_client::lang::get(super::HORIZON_TITLE)
        );
        let demo = list
            .iter()
            .find(|b| b.owner == "demo" && b.id == "sort")
            .unwrap();
        assert_eq!(demo.key, "key.keyboard.g");
        assert_eq!(demo.modifiers, 1);
        assert_eq!(demo.context, mod_api::KeyConflictContext::Gui);

        // Resetting back to default clears mod override map cleanly.
        assert!(apply_keybind_change(
            &mut settings,
            "demo",
            "sort",
            "key.keyboard.r",
            0
        ));
        assert!(
            settings
                .mods
                .keys
                .get("demo")
                .and_then(|m| m.get("sort"))
                .is_none()
        );
    }
}
