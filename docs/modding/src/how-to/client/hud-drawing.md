# Drawing on the HUD and in screens

A HUD layer, a screen or a tooltip hook draws through the `DrawContext` it is handed, in GUI pixels. Each call
is one of vanilla's `GuiGraphics` calls: `fill`, `text`, `sprite`, `item_stack` and the others. `gui_paint`
takes a buffer of `GuiDraw` ops for the same calls, with `Push` / `Pop` for the pose. Native and wasm mods use
the same methods and the same ops.

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
