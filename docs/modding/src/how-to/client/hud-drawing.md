# Drawing on the HUD and in screens

A HUD layer, a screen or a tooltip hook draws through the `DrawContext` it is handed, in GUI pixels. Each call
is one of vanilla's `GuiGraphics` calls: `fill`, `text`, `sprite`, `item_stack` and the others. `gui_paint`
takes a buffer of `GuiDraw` ops for the same calls, with `Push` / `Pop` for the pose. Native and wasm mods use
the same methods and the same ops.

## Active control prompts

Resolve a glyph in the draw hook rather than caching it. `ClientHost::input_glyph` reads the
current vanilla mapping for the device the player used last; changing Jump in Controls affects the
next draw. On a controller it uses that player's binding, artwork family and Nintendo letter
setting. An unknown or unbound mapping returns `None`; a stick tilt is not a button binding.

```rust,ignore
use mod_api::{ClientHost, ControllerButton, GuiRenderPostEvent, prelude::*};

fn gui_render_post(&self, host: &dyn ClientHost, event: &mut GuiRenderPostEvent<'_>) {
    let mut x = 20.0;
    for glyph in [
        host.input_glyph("key.jump"),
        host.mod_input_glyph("mymod", "zoom"),
        host.key_input_glyph("key.mouse.left"),
        host.controller_input_glyph(ControllerButton::South),
    ].into_iter().flatten() {
        event.batch.input_glyph(&glyph, [x, 20.0], 0xFFFF_FFFF);
        x += glyph.size()[0] + 4.0;
    }
}
```

`mod_input_glyph` uses the registered mapping's actual assigned key or mouse button, including
modifiers. It has no controller binding. Explicit physical queries don't follow the last input
method: a physical key names itself, and `South` denotes the bottom face button even when Nintendo
letter mode binds Jump to the right face button instead. A controller must be available for the
controller query. `glyph.label()` gives the displayed label for text alternatives.

`DrawContext::input_glyph` takes the top-left of the glyph's layout box in GUI pixels and an ARGB
colour multiplier. Opaque white preserves the artwork or key label's original colours. The game
draws through its own atlas and the current clip; the mod never needs a sprite name or UV.

## Inventory and hotbar animation

Use `screen_render_background` to draw between the container panel and its slots, or
`screen_render_foreground` to draw over the slots but under the carried stack and tooltips.
`screen_render_pre` can cancel stock screen drawing; `screen_render_post` runs after
tooltips even if Pre cancelled. These are the actual container render stages.

`recipe_book_render` draws on the same batch after the **open stock** recipe book's panel,
current page and alternatives, before Foreground and deferred tooltips. `event.rect` is
the panel in GUI pixels (`[x, y, width, height]`); `event.covers_menu` marks the narrow
mode in which the book owns input over the container slots. A closed book, a cancelled
screen, or a book supplied by a native mod screen does not fire this hook.

For an item redraw, save the callback's translation with `draw.push_translation()`,
apply a GUI-pixel offset with `draw.translate([dx, dy])`, and restore it with
`draw.pop_translation()`. `draw.translation()` reads the cumulative offset. It starts
at zero for each hook call, and a clip follows the translation **when opened**, then
stays fixed on screen. Translate the drawing without offsetting its input coordinates again.
Every listener of a hook call shares the translation and the clips, so pop and close what
you open: the game discards what is left open only when the hook call returns.

`gui_draw_item_with` draws both the icon and its decorations. Use
`gui_draw_item_decorations_with` to redraw **only** the bar, cooldown and count over an
already drawn icon, with `Some(text)` to override the count or `None` for the usual count.
The decoration-only helper does not re-enter `item_decorations`.

`draw.game_time_delta_ticks()` gives the gameplay timer's advance for the frame, not
`ScreenView.partial_tick` or the GUI easing delta. While the stock hotbar's items draw,
and in `gui_layer_post` for `minecraft:hotbar`, `draw.hotbar_origin()` gives the panel's
absolute GUI top-left. It is absent in other layers, on container slots, and when the
stock hotbar is hidden, cancelled, or replaced by the spectator hotbar.

On the engine side, `Game::draw_slot_item` and the HUD's `Game::draw_item` use the same
item-decoration path, with absolute GUI coordinates. `Game::draw_recipe_book` routes an
open stock book through its own painter, while native screens keep their own book draw
path. None of these draw stages changes the menu or sends a click.

## Replace the ping bars in the player list

Use the client hooks `tab_list_latency_width` and `tab_list_latency_draw` together. The first runs
before the visible list is laid out. Set `event.width` to the width your drawing needs in GUI pixels;
the default is 13, the maximum is 128, and a narrow screen can leave less space.

The second runs at each visible row's ping-icon slot. It provides `player_uuid`, the profile's
`player_name`, and `latency_ms` (negative means unknown). The game clips your drawing to `bounds`,
given as `[left, top, right, bottom]` with exclusive right and bottom edges. The rectangle may
be narrower than requested to keep the name and objective score clear. Return `Flow::Cancel` to
omit that row's ping bars, or `Flow::Continue` to leave them in place.

```rust,ignore
fn tab_list_latency_width(&self, _: &dyn ClientHost, event: &mut TabListLatencyWidthEvent) {
    event.width = 54;
}

fn tab_list_latency_draw(
    &self,
    _: &dyn ClientHost,
    event: &mut TabListLatencyDrawEvent<'_>,
) -> Flow {
    let [left, top, right, _] = event.bounds;
    if right <= left || event.latency_ms < 0 {
        return Flow::Continue;
    }
    let label = format!("{} ms", event.latency_ms);
    event.draw.text(&label, [left as f32 + 1.0, top as f32], 0xFFFF_FFFF, false);
    Flow::Cancel
}
```

## Gradient fills

`DrawContext::fill_gradient(rect, top, bottom)` is vanilla's `GuiGraphics.fillGradient`, the call screen
backgrounds and tooltips are drawn with. `rect` is `[x, y, w, h]` in GUI pixels, as `fill` takes it. `top` and
`bottom` are straight-alpha ARGB colours (`0xAARRGGBB`): `top` at `y`, blending to `bottom` at `y + h`.
With positive extents these are the visual top and bottom edges. Alpha blends with the colours, so a gradient can
fade out. The current HUD shader discards interpolated alpha below 0.004, not just exact zero; near-transparent
fragments can therefore differ from vanilla's exact-zero discard.

```rust,ignore
fn gui_render_post(&self, _: &dyn ClientHost, event: &mut GuiRenderPostEvent<'_>) {
    // A panel shaded from dark purple at the top to black at the bottom.
    event.batch.fill_gradient([20.0, 20.0, 120.0, 60.0], 0xF010_0010, 0xF000_0000);
    // A shade that fades out downwards.
    event.batch.fill_gradient([20.0, 80.0, 120.0, 10.0], 0x8000_0000, 0x0000_0000);
}
```

In a `gui_paint` buffer the same call is the op `GuiDraw::FillGradient { x0, y0, x1, y1, top, bottom }`, with
vanilla's corner arguments: `top` stays on the `y0` edge and `bottom` on the `y1` edge.

- **Pose and clip.** A gradient follows the current pose (a `gui_paint` buffer's `Push`) and the hook call's
  clip (`push_clip`), as `fill` does. A clip that cuts the rectangle keeps the colour ramp: the cut edge has the
  colour that was there.
- **Empty or inverted.** Zero width or height draws nothing, as does inversion on exactly one axis.
  Inverting both axes (`w < 0` and `h < 0`, or `x1 < x0` and `y1 < y0`) draws sorted bounds with the
  colours flipped: the first colour stays on its input y edge, now the lower edge. This preserves vanilla's
  visible winding and colour placement. A `fill_gradient` rectangle with a NaN or infinite coordinate draws nothing.
  Zero-area calls are culled here; vanilla can retain their layering bounds without drawing pixels.
- **Cost.** One quad on the GUI batch, as `fill`; nothing is drawn or kept when a mod doesn't call it.
- **Wire layout.** `GuiDraw` gained the `FillGradient` variant: rebuild wasm mods against the new SDK.

## Branded loading overlay (native client)

Implement `ModHooks::loading_overlay_style` to change the branded startup,
resource-pack reload, or settings preview. `LoadingOverlayStyleEvent` carries the
phase, normalized progress, and the mutable style for that frame. The hook is
not called for the unbranded bootstrap or world-joining screens.

```rust,ignore
fn loading_overlay_style(&self, event: &mut LoadingOverlayStyleEvent<'_>) {
    event.style.background = 0xFF10_1420;
    event.style.bar_background = Some(0xFF1A_2234);
    event.style.bar_fill = 0xFF85_D7FF;
    event.style.bar_border = 0xFFB4_D8F5;
    event.style.logo = LoadingOverlayLogo::ChannelDeltas {
        darken: 0xFF00_0010,
        lighten: 0xFF50_759B,
    };
}
```

Colors are straight-alpha ARGB. The two logo sprites keep their original
positions and UVs. `ChannelDeltas` subtracts RGB first, then adds RGB; each pass
is scaled by the texel and the supplied alpha. `Tint` uses ordinary alpha
blending, `Hidden` omits the logo, and `Original` leaves it unchanged.

From an in-game settings callback, call `host.preview_loading_overlay(ms)` to
show the same overlay with synthetic progress. It returns `false` when a preview
cannot start, for example when a real reload is underway. Its lifetime is
bounded; Escape or the controller's Back action dismisses it and returns to the
underlying settings screen. A preview does not reload resources or save settings.
