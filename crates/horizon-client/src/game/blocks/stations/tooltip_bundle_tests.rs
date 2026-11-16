use super::*;
use mod_api::{ClientHost, Hooks, ModEntry, Mods, RenderTooltipGatherComponentsEvent};

struct Edit(bool);
#[mod_api::__subscriptions]
impl Hooks for Edit {
    fn render_tooltip_gather_components(
        &self,
        _: &dyn ClientHost,
        e: &mut RenderTooltipGatherComponentsEvent<'_>,
    ) -> mod_api::Flow {
        assert!(matches!(
            &e.elements[1],
            TooltipElement::Image(TooltipImage {
                source: mod_api::TooltipImageSource::Bundle(_),
                ..
            })
        ));
        let image = e.elements.remove(1);
        if self.0 {
            e.elements.push(image);
        }
        mod_api::Flow::Continue
    }
}

fn run(mode: Option<bool>, test: impl FnOnce(&mut Game<'_>)) {
    let mods: Mods = mode
        .map(|mode| ModEntry::new("bundle-test", Box::new(Edit(mode))))
        .into_iter()
        .collect::<Vec<_>>()
        .into();
    run_mods(mods, test);
}

fn run_mods(mods: Mods, test: impl FnOnce(&mut Game<'_>)) {
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.gui.atlas.sprites.insert(
                "gui/sprites/container/bundle/slot_background".into(),
                horizon_client::hud::Rect {
                    u0: 0.25,
                    v0: 0.25,
                    u1: 0.5,
                    v1: 0.5,
                    w: 24,
                    h: 24,
                },
            );
            test(game);
        })
    });
}
fn bundle() -> ItemStack {
    let mut stack = ItemStack::new("minecraft:bundle", 1);
    stack.data_mut().bundle_contents = vec![ItemStack::new("minecraft:stone", 1)];
    stack
}
fn draw(game: &mut Game<'_>, stack: &ItemStack) -> HudBatch {
    let mut batch = HudBatch::scratch([320, 240]);
    game.draw_stack_tooltip_with(
        &mut batch,
        stack,
        false,
        10.0,
        20.0,
        vec![(horizon_client::text::Text::literal("Rest"), u32::MAX)],
    );
    batch
}
fn grid(game: &Game<'_>, batch: &HudBatch, y: f32, present: bool) {
    // Positioner: x + 12, y - 12. Slot right-aligned: 22 + 72 = 94.
    let mut expected = HudBatch::scratch([320, 240]);
    horizon_client::ui::nine_slice(
        &mut expected,
        &game.gui.atlas,
        "gui/sprites/container/bundle/slot_background",
        94.0,
        y,
        24.0,
        24.0,
        u32::MAX,
    );
    assert!(!expected.vertices.is_empty());
    let found = batch
        .vertices
        .windows(expected.vertices.len())
        .any(|window| {
            window
                .iter()
                .zip(&expected.vertices)
                .all(|(a, b)| a.pos == b.pos && a.uv == b.uv && a.color == b.color)
        });
    assert_eq!(found, present);
}
fn bottom(batch: &HudBatch) -> f32 {
    batch.vertices.iter().map(|v| v.pos[1]).fold(0.0, f32::max)
}
#[test]
#[ignore = "daily"]
fn bundle_no_listener_order_and_draw_position() {
    run(None, |game| {
        let stack = bundle();
        let lines = vec![
            (horizon_client::text::Text::literal("Name"), u32::MAX),
            (horizon_client::text::Text::literal("Rest"), u32::MAX),
        ];
        let elements = game
            .gather_tooltip_components(&stack, lines, vec![], (320.0, 240.0))
            .unwrap();
        assert!(matches!(
            &elements[..],
            [
                Element::Text(..),
                Element::Image(TooltipImage {
                    source: mod_api::TooltipImageSource::Bundle(_),
                    size: [96.0, 45.0]
                }),
                Element::Text(..)
            ]
        ));
        let batch = draw(game, &stack);
        // top 8 + name 10 + first-component padding 2 = 20.
        grid(game, &batch, 20.0, true);
    });
}
#[test]
#[ignore = "daily"]
fn bundle_gather_moves_image_to_end() {
    run(Some(true), |game| {
        let batch = draw(game, &bundle());
        // top 8 + name 10 + padding 2 + rest 10 = 30.
        grid(game, &batch, 30.0, true);
        grid(game, &batch, 20.0, false);
    });
}
#[test]
#[ignore = "daily"]
fn bundle_gather_removes_grid_and_height() {
    let mut full_bottom = 0.0;
    run(None, |game| {
        full_bottom = bottom(&draw(game, &bundle()));
    });
    run(Some(false), |game| {
        let batch = draw(game, &bundle());
        grid(game, &batch, 20.0, false);
        // One 24px row + 13px bar + 8px gap = 45px.
        assert_eq!(full_bottom - bottom(&batch), 45.0);
    });
}
#[test]
#[ignore = "daily"]
fn non_bundle_keeps_text_only_geometry() {
    run(None, |game| {
        let stack = ItemStack::new("minecraft:stick", 1);
        let batch = draw(game, &stack);
        grid(game, &batch, 20.0, false);
        assert_eq!(game.stack_tooltip_lines(&stack, false, false).len(), 1);
        // Two 10px lines: top 8, content height 20, outer border extends through y + h + 4.
        assert_eq!(bottom(&batch), 32.0);
    });
}

#[test]
#[ignore = "daily"]
fn bundle_empty_text_gathers_image_at_zero() {
    run(None, |game| {
        let elements = game
            .gather_tooltip_components(&bundle(), vec![], vec![], (320.0, 240.0))
            .unwrap();
        assert!(matches!(
            &elements[..],
            [Element::Image(TooltipImage {
                source: mod_api::TooltipImageSource::Bundle(_),
                size: [96.0, 45.0],
            })]
        ));
    });
}

#[test]
#[ignore = "daily"]
fn bundle_empty_contents_keeps_image() {
    run(None, |game| {
        let stack = ItemStack::new("minecraft:bundle", 1);
        let lines = vec![(horizon_client::text::Text::literal("Name"), u32::MAX)];
        let elements = game
            .gather_tooltip_components(&stack, lines, vec![], (320.0, 240.0))
            .unwrap();
        // Vanilla maps the present bundle-contents component even when empty.
        // Empty description: two 9px lines + 13px bar + 8px gap = 39px.
        assert!(matches!(
            &elements[..],
            [
                Element::Text(..),
                Element::Image(TooltipImage {
                    source: mod_api::TooltipImageSource::Bundle(_),
                    size: [96.0, 39.0],
                })
            ]
        ));
        let batch = draw(game, &stack);
        grid(game, &batch, 20.0, false);
        // Content top 8 + two text lines 20 + image 39 + outer extent 4.
        assert_eq!(bottom(&batch), 71.0);
    });
}

struct Boundary {
    insert: bool,
    observed: std::sync::Arc<std::sync::Mutex<Vec<TooltipElement>>>,
}
#[mod_api::__subscriptions]
impl Hooks for Boundary {
    fn render_tooltip_gather_components(
        &self,
        _: &dyn ClientHost,
        event: &mut RenderTooltipGatherComponentsEvent<'_>,
    ) -> mod_api::Flow {
        if self.insert {
            event.elements.insert(
                0,
                TooltipElement::Text(mod_api::Text::literal("Inserted"), u32::MAX),
            );
        } else {
            let image = event.elements.remove(1);
            event.elements.push(image);
        }
        mod_api::Flow::Continue
    }

    fn render_tooltip_pre(
        &self,
        _: &dyn ClientHost,
        event: &mut mod_api::RenderTooltipPreEvent<'_>,
    ) -> mod_api::Flow {
        *self.observed.lock().unwrap() = event.elements.to_vec();
        mod_api::Flow::Continue
    }
}

fn boundary(insert: bool) {
    let observed = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mods: Mods = vec![ModEntry::new(
        "bundle-boundary",
        Box::new(Boundary {
            insert,
            observed: observed.clone(),
        }),
    )]
    .into();
    run_mods(mods, |game| {
        let stack = bundle();
        let lines = game.stack_tooltip_lines(&stack, false, false);
        assert_eq!(lines.len(), 1);
        let name = TooltipElement::Text(lines[0].0.try_into_api().unwrap(), lines[0].1);
        let rest = TooltipElement::Text(mod_api::Text::literal("Rest"), u32::MAX);
        let image = TooltipElement::Image(TooltipImage {
            source: mod_api::TooltipImageSource::Bundle(stack.try_into_api().unwrap()),
            size: [96.0, 45.0],
        });
        let expected = if insert {
            vec![
                TooltipElement::Text(mod_api::Text::literal("Inserted"), u32::MAX),
                name,
                image,
                rest,
            ]
        } else {
            vec![name, rest, image]
        };
        let batch = draw(game, &stack);
        assert!(*observed.lock().unwrap() == expected);
        // Top 8 + first line 10 + first-element gap 2 + second line 10 = 30.
        // Inserting before the name transfers the gap to the inserted line.
        grid(game, &batch, 30.0, true);
        grid(game, &batch, 20.0, false);
    });
}

#[test]
#[ignore = "daily"]
fn bundle_listener_inserts_before_name() {
    boundary(true);
}

#[test]
#[ignore = "daily"]
fn bundle_pre_observes_gather_reordered_list() {
    boundary(false);
}

struct ClearLines(std::sync::Arc<std::sync::atomic::AtomicBool>);
#[mod_api::__subscriptions]
impl Hooks for ClearLines {
    fn item_tooltip(&self, _: &dyn ClientHost, event: &mut mod_api::ItemTooltipEvent<'_>) {
        event.lines.clear();
    }

    fn render_tooltip_pre(
        &self,
        _: &dyn ClientHost,
        event: &mut mod_api::RenderTooltipPreEvent<'_>,
    ) -> mod_api::Flow {
        assert!(matches!(
            event.elements,
            [TooltipElement::Image(TooltipImage {
                source: mod_api::TooltipImageSource::Bundle(_),
                size: [96.0, 45.0],
            })]
        ));
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
        mod_api::Flow::Continue
    }
}

#[test]
#[ignore = "daily"]
fn bundle_cleared_lines_still_draws_image_at_content_top() {
    let observed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mods: Mods = vec![ModEntry::new(
        "clear-lines",
        Box::new(ClearLines(observed.clone())),
    )]
    .into();
    run_mods(mods, |game| {
        let mut batch = HudBatch::scratch([320, 240]);
        game.draw_stack_tooltip_with(&mut batch, &bundle(), false, 10.0, 20.0, vec![]);
        // Positioner: cursor y 20 - 12 = content top 8. No name or gap above the image.
        grid(game, &batch, 8.0, true);
        assert!(observed.load(std::sync::atomic::Ordering::Relaxed));
    });
}

#[test]
#[ignore = "daily"]
fn bundle_hidden_tooltip_still_draws_nothing() {
    run(None, |game| {
        let mut stack = bundle();
        stack.data_mut().set_other(
            "minecraft:tooltip_display",
            Some(horizon_core::nbt::Tag::compound([(
                "hide_tooltip",
                horizon_core::nbt::Tag::Byte(1),
            )])),
        );
        // Also pins that the supplied after-name text cannot revive a hidden tooltip.
        let batch = draw(game, &stack);
        assert!(batch.vertices.is_empty());
    });
}
