use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `GuiGraphicsExtractor.componentHoverEffect`: the hovered text component's hover, about
    /// to be drawn at the cursor `x`, `y`. Set `cancel` and the game draws nothing for it.
    /// Client-side.
    pub(in crate::game) fn native_component_hover(
        &self,
        call: mod_api::__private::ComponentHoverCall<'_>,
    ) {
        self.fire_component_hover(call);
    }
}

#[cfg(test)]
mod component_hover_tests {
    use horizon_client::hud::HudBatch;
    use horizon_client::text::{Style, Text};
    use mod_api::text::{TextContent, TextHover};
    use mod_api::{ComponentHoverEvent, Hooks, ModEntry};
    use mod_platform::testing::with_mods;
    use std::sync::Mutex;

    /// Every hover the mod was asked about, in order.
    static SEEN: Mutex<Vec<TextHover>> = Mutex::new(Vec::new());

    /// Records each hover and cancels the ones whose text reads `secret`, drawing a square.
    struct Hovers;
    #[mod_api::__subscriptions]
    impl Hooks for Hovers {
        fn component_hover(
            &self,
            _: &dyn mod_api::ClientHost,
            event: &mut ComponentHoverEvent<'_>,
        ) -> mod_api::Flow {
            SEEN.lock().unwrap().push(event.hover.clone());
            match event.hover {
                TextHover::Text(text) if text.content == TextContent::Literal("secret".into()) => {
                    event.draw.fill([1.0, 2.0, 3.0, 4.0], 0xff00ff00);
                    mod_api::Flow::Cancel
                }
                _ => mod_api::Flow::Continue,
            }
        }
    }

    fn style(hover: serde_json::Value) -> Style {
        let text = Text::from(serde_json::json!({ "text": "x", "hover_event": hover }));
        text.spans[0].style.clone()
    }
    fn drawn(game: &mut super::Game<'_>, style: &Style) -> Vec<([f32; 2], u32)> {
        let mut batch = HudBatch::scratch([320, 240]);
        game.component_hover_effect(&mut batch, style, (40.0, 30.0));
        batch.vertices.iter().map(|v| (v.uv, v.color)).collect()
    }

    /// `componentHoverEffect`: a listening mod is asked once for each hovered component with its
    /// typed hover (text, item, and an entity whose UUID came as an int array), the default
    /// drawing is unchanged when it doesn't cancel, and a cancel draws only what the mod drew.
    #[test]
    #[ignore = "daily"]
    fn a_mod_sees_each_hover_and_may_draw_its_own() {
        let text = style(serde_json::json!({ "action": "show_text", "value": "hello" }));
        let secret = style(serde_json::json!({ "action": "show_text", "value": "secret" }));
        let item = style(serde_json::json!({
            "action": "show_item", "id": "minecraft:stick", "count": 2
        }));
        let entity = style(serde_json::json!({
            "action": "show_entity", "id": "minecraft:pig", "uuid": [0, 1, 0, 2], "name": "Babe"
        }));
        let mut unheard = Vec::new();
        crate::game::native_entity_refs::tests::with_game(|game| {
            unheard = [&text, &secret, &item, &entity]
                .map(|style| drawn(game, style))
                .to_vec();
        });
        assert!(unheard[..3].iter().all(|quads| !quads.is_empty()));
        // Without advanced tooltips an entity hover draws nothing, as vanilla's.
        assert!(unheard[3].is_empty());
        SEEN.lock().unwrap().clear();
        let mods: mod_api::Mods = vec![ModEntry::new("hovers", Box::new(Hovers))].into();
        with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                assert_eq!(drawn(game, &text), unheard[0]);
                assert_eq!(drawn(game, &item), unheard[2]);
                assert_eq!(drawn(game, &entity), unheard[3]);
                // Cancelled: only the mod's square.
                let cancelled = drawn(game, &secret);
                assert_eq!(cancelled.len(), 6);
                assert_ne!(cancelled, unheard[1]);
                // A style with no hover asks nothing.
                assert!(drawn(game, &Style::default()).is_empty());
            });
        });
        let seen = SEEN.lock().unwrap();
        assert_eq!(seen.len(), 4);
        let literal = |hover: &TextHover, s: &str| matches!(hover, TextHover::Text(t) if t.content == TextContent::Literal(s.into()));
        assert!(literal(&seen[0], "hello"));
        assert!(
            matches!(&seen[1], TextHover::Item { id, count: 2, .. } if id == "minecraft:stick")
        );
        assert!(matches!(
            &seen[2],
            TextHover::Entity { kind, id: [1, 2], name: Some(_) } if kind == "minecraft:pig"
        ));
        assert!(literal(&seen[3], "secret"));
    }
}
