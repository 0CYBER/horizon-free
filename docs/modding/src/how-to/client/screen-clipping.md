# Clipping what a screen draws

A scrolling list, a label that scrolls when it is too long, or a viewport into a larger page draws more than fits
and shows only the part inside a box. `DrawContext::push_clip(rect)` opens such a box (vanilla's
`GuiGraphics.enableScissor`) and `pop_clip()` closes it (`disableScissor`). Everything the hook call draws
between the two is cut to `rect`, `[x, y, w, h]` in GUI pixels as `fill` takes it: the visible part of a sprite,
a glyph or an item stays. Native and wasm mods use the same methods.

```rust,ignore
fn gui_render_post(&self, _: &dyn ClientHost, event: &mut GuiRenderPostEvent<'_>) {
    let [x, y, w, h] = self.list_box;
    if event.batch.push_clip([x, y, w, h]).is_ok() {
        for (row, line) in self.lines.iter().enumerate() {
            let top = y + row as f32 * 12.0 - self.scroll;
            event.batch.text(line, [x + 2.0, top], 0xFFFF_FFFF, false);
        }
        let _ = event.batch.pop_clip();
    }
}
```

- **Nesting.** Clips nest: an inner clip is cut to the clip around it, so it never shows more than that one.
  A negative width or height, or an inner clip outside the outer one, clips everything.
- **One hook call.** The clips belong to the hook call that opened them. One still open when the call returns
  is closed then, so it also cuts what later listeners of the same call draw: close what you open.
- **Depth.** Item models and scenes from `draw_scene` are cut too, to both their own box and the clip, and
  keep their depth: a cut face still draws in front of or behind its neighbours as it did uncut.
- **Refusals.** Both return `Result<(), GuiClipRefusal>` and change nothing when they refuse:
  `NotFinite` (a coordinate or size is NaN or infinite), `TooDeep` (64 clips are already open in this call)
  and `NoneOpen` (`pop_clip` with nothing open). The enum is `#[non_exhaustive]`.
- **Cost.** A call that never clips allocates nothing; a clip is cut on the GUI batch when it closes, as the
  game's own scrolling screens cut theirs.
