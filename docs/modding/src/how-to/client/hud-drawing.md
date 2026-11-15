# Drawing on the HUD and in screens

A HUD layer, a screen or a tooltip hook draws through the `DrawContext` it is handed, in GUI pixels. Each call
is one of vanilla's `GuiGraphics` calls: `fill`, `text`, `sprite`, `item_stack` and the others. `gui_paint`
takes a buffer of `GuiDraw` ops for the same calls, with `Push` / `Pop` for the pose. Native and wasm mods use
the same methods and the same ops.

## Gradient fills

`DrawContext::fill_gradient(rect, top, bottom)` is vanilla's `GuiGraphics.fillGradient`, the call screen
backgrounds and tooltips are drawn with. `rect` is `[x, y, w, h]` in GUI pixels, as `fill` takes it. `top` and
`bottom` are ARGB colours (`0xAARRGGBB`): `top` along the rectangle's top edge, blending to `bottom` along its
bottom edge. Alpha blends with the colours, so a gradient can fade out.

```rust,ignore
fn gui_render_post(&self, _: &dyn ClientHost, event: &mut GuiRenderPostEvent<'_>) {
    // A panel shaded from dark purple at the top to black at the bottom.
    event.batch.fill_gradient([20.0, 20.0, 120.0, 60.0], 0xF010_0010, 0xF000_0000);
    // A shade that fades out downwards.
    event.batch.fill_gradient([20.0, 80.0, 120.0, 10.0], 0x8000_0000, 0x0000_0000);
}
```

In a `gui_paint` buffer the same call is the op `GuiDraw::FillGradient { x0, y0, x1, y1, top, bottom }`, with
vanilla's corner arguments: `(x0, y0)` the top-left corner and `(x1, y1)` the bottom-right one.

- **Pose and clip.** A gradient follows the current pose (a `gui_paint` buffer's `Push`) and the hook call's
  clip (`push_clip`), as `fill` does. A clip that cuts the rectangle keeps the colour ramp: the cut edge has the
  colour that was there.
- **Empty or reversed.** Unlike `GuiDraw::Fill`, the corners are not sorted. A rectangle with no width or
  height, or a reversed one (`w <= 0` or `h <= 0`, `x1 <= x0` or `y1 <= y0`), draws nothing. So does a
  `fill_gradient` rectangle with a NaN or infinite coordinate.
- **Cost.** One quad on the GUI batch, as `fill`; nothing is drawn or kept when a mod doesn't call it.
- **Wire layout.** `GuiDraw` gained the `FillGradient` variant: rebuild wasm mods against the new SDK.
