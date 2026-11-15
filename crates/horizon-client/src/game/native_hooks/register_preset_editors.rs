use super::*;

impl Game<'_> {
    /// Collect world-preset editor registrations once at client startup.
    pub(in crate::game) fn native_register_preset_editors(
        context: &mod_api::RegistrationContext,
        editors: &mut Vec<mod_api::PresetEditor>,
    ) {
        Self::fire_register_preset_editors(context, editors);
    }

    fn preset_editor_title(&self) -> Option<&horizon_client::title::Title> {
        if self.screen != Screen::Playing || self.screen_over_playing() || self.remote_loading() {
            return None;
        }
        self.title
            .as_ref()
            .filter(|title| title.preset_editor().is_some())
    }

    pub(in crate::game) fn preset_editor(&self) -> Option<mod_api::PresetEditor> {
        self.preset_editor_title()?.preset_editor().cloned()
    }

    pub(in crate::game) fn preset_editor_dimensions(&self) -> Option<mod_api::Compound> {
        let horizon_core::nbt::Tag::Compound(dimensions) =
            self.preset_editor_title()?.preset_editor_dimensions()?
        else {
            return None;
        };
        Some(dimensions)
    }

    pub(in crate::game) fn set_preset_editor_dimensions(
        &self,
        dimensions: mod_api::Compound,
    ) -> bool {
        self.preset_editor_title().is_some_and(|title| {
            title.set_preset_editor_dimensions(horizon_core::nbt::Tag::Compound(dimensions))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::title::{Mode, WorldOptions};
    use mod_api::{
        Hooks, ModEntry, RegisterPresetEditorsEvent, ScreenKeyPressedPreEvent,
        ScreenRenderPostEvent,
    };

    struct EditorMod;
    #[mod_api::__subscriptions]
    impl Hooks for EditorMod {
        fn register_preset_editors(
            &self,
            _: &mod_api::RegistrationContext,
            event: &mut RegisterPresetEditorsEvent<'_>,
        ) {
            event.editors.push(mod_api::PresetEditor {
                preset: "example:islands".into(),
                screen: "example:editor".into(),
            });
        }
        fn screen_render_post(
            &self,
            host: &dyn mod_api::ClientHost,
            event: &mut ScreenRenderPostEvent<'_>,
        ) {
            if host
                .preset_editor()
                .is_some_and(|editor| editor.screen == "example:editor")
            {
                event.draw.fill([20.0, 20.0, 50.0, 20.0], 0xFF00FF00);
            }
        }
        fn screen_key_pressed_pre(
            &self,
            host: &dyn mod_api::ClientHost,
            event: &mut ScreenKeyPressedPreEvent<'_>,
        ) -> mod_api::Flow {
            if event.key != "key.keyboard.space" || host.preset_editor().is_none() {
                return mod_api::Flow::Continue;
            }
            let mut dimensions = host.preset_editor_dimensions().unwrap();
            let horizon_core::nbt::Tag::Compound(overworld) =
                dimensions.get_mut(&"minecraft:overworld".into()).unwrap()
            else {
                unreachable!()
            };
            overworld.insert(
                "type".into(),
                horizon_core::nbt::Tag::String("minecraft:overworld_caves".into()),
            );
            assert!(host.set_preset_editor_dimensions(dimensions));
            mod_api::Flow::Cancel
        }
    }

    #[test]
    #[ignore = "daily"]
    fn preset_editor_test_mod_draws_and_edits_through_real_client_hooks() {
        mod_platform::testing::with_mods(
            vec![ModEntry::new("example", Box::new(EditorMod))].into(),
            || {
                crate::game::native_entity_refs::tests::with_client_game(|game| {
                    use mod_api::__private::game::RegistrationContextNew;
                    let mut editors = Vec::new();
                    Game::native_register_preset_editors(
                        &mod_api::RegistrationContext::new(),
                        &mut editors,
                    );
                    assert_eq!(editors.len(), 1);
                    let mut title = game.new_title(game.save.root.join("editor-saves"));
                    title.set_preset_data_source(game.pack.data_changes.clone());
                    let dimensions = horizon_core::nbt::Tag::compound([(
                        "minecraft:overworld",
                        horizon_core::nbt::Tag::compound([
                            (
                                "type",
                                horizon_core::nbt::Tag::String("minecraft:overworld".into()),
                            ),
                            (
                                "generator",
                                horizon_core::nbt::Tag::compound([(
                                    "type",
                                    horizon_core::nbt::Tag::String("minecraft:debug".into()),
                                )]),
                            ),
                        ]),
                    )]);
                    horizon_client::title::preset_editors::register(editors);
                    title.mode = Mode::Create {
                        name: "Test".into(),
                        seed: "1".into(),
                        creative: false,
                        focus: 0,
                        options: WorldOptions {
                            custom_preset: Some("example:islands".into()),
                            presets: [("example:islands".into(), dimensions)].into(),
                            preset_order: vec!["example:islands".into()],
                            tab: 1,
                            ..Default::default()
                        },
                    };
                    title.focused_button = Some(26);
                    title.confirm_key_size((320.0, 240.0));
                    assert_eq!(title.mode.id(), "world_preset_editor");
                    game.title = Some(title);
                    game.screen = Screen::Playing;
                    let mut batch = HudBatch::new([640, 480], game.settings.gui_scale_inputs());
                    game.title.as_ref().unwrap().draw(
                        &game.gui.atlas,
                        &mut batch,
                        (0.0, 0.0),
                        false,
                        &mut |phase, batch| game.draw_screen_phase(batch, phase),
                    );
                    let stock_vertices = batch.vertices.len();
                    let view = game.title_screen_render_view(&batch).unwrap();
                    assert_eq!(view.id, "world_preset_editor");
                    game.native_screen_render_post(&mut batch, &view);
                    assert!(batch.vertices.len() > stock_vertices);
                    let mut cancelled = false;
                    game.native_screen_key_pressed_pre(
                        &view,
                        "key.keyboard.space",
                        false,
                        0,
                        &mut cancelled,
                    );
                    assert!(cancelled);
                    let dimensions = game.preset_editor_dimensions().unwrap();
                    assert_eq!(
                        dimensions
                            .get(&"minecraft:overworld".into())
                            .unwrap()
                            .get("type"),
                        Some(&horizon_core::nbt::Tag::String(
                            "minecraft:overworld_caves".into()
                        ))
                    );
                    game.screen = Screen::Options;
                    assert!(game.preset_editor().is_none());
                    assert!(game.preset_editor_dimensions().is_none());
                    assert!(!game.set_preset_editor_dimensions(dimensions));
                });
            },
        );
    }
}
