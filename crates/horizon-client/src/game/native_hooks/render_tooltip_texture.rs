use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `RenderTooltipEvent.Texture`: the look of a tooltip box about to be drawn. Set `texture`
    /// to a tooltip style and the box draws its `tooltip/<name>_background` and `_frame` GUI
    /// sprites; left `None`, it is the solid fills in `background` and the `border_top` to
    /// `border_bottom` gradient, as ARGB. Stack is None for tooltips not associated with an item.
    pub(in crate::game) fn native_render_tooltip_texture(
        &self,
        stack: Option<&ItemStack>,
        texture: &mut Option<Identifier>,
        background: &mut u32,
        border_top: &mut u32,
        border_bottom: &mut u32,
    ) {
        self.fire_render_tooltip_texture(stack, texture, background, border_top, border_bottom);
    }
}

#[cfg(test)]
mod render_tooltip_texture_tests {
    use super::ItemStack;
    use horizon_client::hud::{HudBatch, Rect};
    use mod_api::{ClientHost, Hooks, Identifier, ModEntry, RenderTooltipTextureEvent};
    use mod_platform::testing::with_mods;

    /// Gives a stick's tooltip the `lobby:gold` style and a text tooltip a red background.
    struct Gold;
    #[mod_api::__subscriptions]
    impl Hooks for Gold {
        fn render_tooltip_texture(
            &self,
            host: &dyn ClientHost,
            event: &mut RenderTooltipTextureEvent<'_>,
        ) {
            assert_eq!(*event.texture, None);
            assert_eq!(
                (event.background, event.border_top, event.border_bottom),
                (0xf0100010, 0x505000ff, 0x5028007f)
            );
            match event.stack {
                Some(stack) if stack.item().id(host) == "minecraft:stick" => {
                    *event.texture = Some(Identifier::parse("lobby:gold").unwrap());
                }
                Some(_) => {}
                None => event.background = 0xffff0000,
            }
        }
    }

    fn sprite(u0: f32) -> Rect {
        Rect {
            u0,
            v0: 0.0,
            u1: u0 + 0.125,
            v1: 0.125,
            w: 100,
            h: 100,
        }
    }
    fn drawn(game: &super::Game<'_>, stack: Option<&ItemStack>) -> Vec<([f32; 2], u32)> {
        let mut batch = HudBatch::scratch([320, 240]);
        game.tooltip_box(&mut batch, stack, 40.0, 30.0, 60.0, 8.0);
        batch.vertices.iter().map(|v| (v.uv, v.color)).collect()
    }
    fn in_sprite(uv: [f32; 2], r: Rect) -> bool {
        (r.u0..=r.u1).contains(&uv[0]) && (r.v0..=r.v1).contains(&uv[1])
    }

    /// `RenderTooltipEvent.Texture`: a mod's tooltip style draws its background and frame
    /// sprites from the GUI atlas, follows the atlas a resource reload restitches, and falls
    /// back to the fills when no pack has it; its colours change the fills; and an unheard
    /// tooltip draws as before.
    #[test]
    #[ignore = "daily"]
    fn a_mod_gives_a_tooltip_its_own_texture_or_colours() {
        let stick = ItemStack::new("minecraft:stick", 1);
        let stone = ItemStack::new("minecraft:stone", 1);
        let mut unheard = Vec::new();
        crate::game::native_entity_refs::tests::with_game(|game| {
            unheard = [None, Some(&stick)]
                .map(|stack| drawn(game, stack))
                .to_vec();
        });
        // The solid fills: the background's three, the two borders' and their gradient sides.
        assert_eq!(unheard[0].len(), 7 * 6);
        assert_eq!(unheard[0], unheard[1]);
        let mods: mod_api::Mods = vec![ModEntry::new("gold", Box::new(Gold))].into();
        with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                let names = ["background", "frame"]
                    .map(|part| format!("lobby:gui/sprites/tooltip/gold_{part}"));
                // No pack has the style: the stick keeps the fills.
                assert_eq!(drawn(game, Some(&stick)), unheard[1]);
                for (first, then) in [(0.0, 0.5), (0.25, 0.75)] {
                    let rects = [sprite(first), sprite(then)];
                    for (name, rect) in names.iter().zip(rects) {
                        game.gui.atlas.sprites.insert(name.clone(), rect);
                    }
                    let quads = drawn(game, Some(&stick));
                    assert_ne!(quads, unheard[1]);
                    assert!(
                        quads.iter().all(|&(uv, color)| color == u32::MAX
                            && rects.iter().any(|&r| in_sprite(uv, r)))
                    );
                    for rect in rects {
                        assert!(quads.iter().any(|&(uv, _)| in_sprite(uv, rect)));
                    }
                }
                // Another item's tooltip is unchanged; the text tooltip's background is red.
                assert_eq!(drawn(game, Some(&stone)), unheard[1]);
                let red = drawn(game, None);
                assert_eq!(red.len(), unheard[0].len());
                assert_ne!(red, unheard[0]);
            });
        });
    }
}
