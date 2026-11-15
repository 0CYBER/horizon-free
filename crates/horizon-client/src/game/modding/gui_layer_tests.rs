//! Mod HUD layers in a real game: where `register_gui_layers` puts them among vanilla's layers
//! as the HUD pass draws, and another mod hiding one as it hides a vanilla layer.
use super::{
    execution_role::ExecutionRole,
    mods,
    native_entity_refs::tests::{run_fresh_cases, with_client_game},
};
use horizon_client::hud::HudBatch;
use mod_api::{
    ClientHost, Flow, GuiLayer, GuiLayerDrawEvent, GuiLayerPlacement, GuiLayerPostEvent,
    GuiLayerPreEvent, Hooks, ModEntry, RegisterGuiLayersEvent,
};
use std::sync::Mutex;

/// What the HUD pass did, in order: `pre <id>`, `draw <id>`, `post <id>`, and `vanilla <id>`
/// for a vanilla layer's own drawing.
static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn log(entry: String) {
    LOG.lock().unwrap().push(entry);
}

/// Registers its layers around the hotbar and above everything, and draws them.
struct Compass;
#[mod_api::__subscriptions]
impl Hooks for Compass {
    fn register_gui_layers(
        &self,
        _: &mod_api::RegistrationContext,
        event: &mut RegisterGuiLayersEvent<'_>,
    ) {
        use GuiLayerPlacement::*;
        for (id, placement) in [
            (
                "compass:under_crosshair",
                Below("minecraft:crosshair".into()),
            ),
            ("compass:bar", Above("minecraft:hotbar".into())),
            ("compass:top", AboveAll),
        ] {
            event.layers.push(GuiLayer {
                id: id.into(),
                placement,
            });
        }
    }
    fn gui_layer_pre(&self, _: &dyn ClientHost, event: &mut GuiLayerPreEvent<'_>) -> Flow {
        log(format!("pre {}", event.layer));
        Flow::Continue
    }
    fn gui_layer_draw(&self, _: &dyn ClientHost, event: &mut GuiLayerDrawEvent<'_>) {
        if event.layer.starts_with("compass:") {
            log(format!("draw {}", event.layer));
        }
    }
    fn gui_layer_post(&self, _: &dyn ClientHost, event: &mut GuiLayerPostEvent<'_>) {
        log(format!("post {}", event.layer));
    }
}

/// Hides the compass's bar, as it would hide a vanilla layer.
struct Hider;
#[mod_api::__subscriptions]
impl Hooks for Hider {
    fn gui_layer_pre(&self, _: &dyn ClientHost, event: &mut GuiLayerPreEvent<'_>) -> Flow {
        if event.layer == "compass:bar" {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

/// One vanilla layer through `gui_layer`, as the HUD pass draws it.
fn vanilla(game: &mut super::Game<'_>, batch: &mut HudBatch, id: &'static str) {
    game.gui_layer(id, batch, |_, _| log(format!("vanilla {id}")));
}

fn take() -> Vec<String> {
    std::mem::take(&mut *LOG.lock().unwrap())
}

#[test]
#[ignore = "daily"]
fn mod_layers_draw_in_their_place_and_hide_like_vanilla_layers() {
    // The layer order is kept once per process.
    if run_fresh_cases(
        "game::gui_layer_tests::mod_layers_draw_in_their_place_and_hide_like_vanilla_layers",
        "HORIZON_GUI_LAYER_FIXTURE",
        &["run"],
    ) {
        return;
    }
    with_client_game(|game| {
        mod_platform::__private::install(
            vec![
                ModEntry::new("compass", Box::new(Compass)),
                ModEntry::new("hider", Box::new(Hider)),
            ]
            .into(),
        );
        mods::register_gui_layers(ExecutionRole::Client);
        let mut batch = HudBatch::new([320, 240], game.settings.gui_scale_inputs());

        // A frame where the crosshair is not drawn (third person): the layer under it still
        // draws, before the hotbar. The bar is hidden by the other mod, so it is not drawn,
        // but its `post` still fires, as a vanilla layer's does.
        game.hud_layer_next = Some(0);
        vanilla(game, &mut batch, "minecraft:hotbar");
        vanilla(game, &mut batch, "minecraft:effects");
        vanilla(game, &mut batch, "minecraft:chat");
        game.draw_mod_gui_layers(None, &mut batch);
        game.hud_layer_next = None;
        assert_eq!(
            take(),
            [
                "pre compass:under_crosshair",
                "draw compass:under_crosshair",
                "post compass:under_crosshair",
                "pre minecraft:hotbar",
                "vanilla minecraft:hotbar",
                "post minecraft:hotbar",
                "pre compass:bar",
                "post compass:bar",
                "pre minecraft:effects",
                "vanilla minecraft:effects",
                "post minecraft:effects",
                "pre minecraft:chat",
                "vanilla minecraft:chat",
                "post minecraft:chat",
                "pre compass:top",
                "draw compass:top",
                "post compass:top",
            ]
        );

        // With the HUD hidden (F1), vanilla's sleep overlay still draws, and no mod layer.
        vanilla(game, &mut batch, "minecraft:sleep_overlay");
        game.draw_mod_gui_layers(None, &mut batch);
        assert_eq!(
            take(),
            [
                "pre minecraft:sleep_overlay",
                "vanilla minecraft:sleep_overlay",
                "post minecraft:sleep_overlay",
            ]
        );
    });
}
