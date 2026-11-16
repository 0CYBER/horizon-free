# Measuring styled text outside drawing

Measure styled text without a draw context. Use `ClientHost::text_width(&Text)` and `text_split(&Text, width)`:
these are reads of the client's active font in unscaled GUI pixels, also usable
in client setup. Native and wasm mods use the same methods.

| Method on `ClientHost` | Result |
| --- | --- |
| `text_width(&Text)` | `Result<i32, TextFormError>`: styled width in GUI pixels. |
| `text_split(&Text, i32)` | `Result<Vec<Text>, TextFormError>`: wrapped, styled lines. |
| `text_string(&Text)` | `Result<String, TextFormError>`: resolved plain characters. |
| `text_visual_order(&Text)` | `Result<Vec<VisualTextCharacter>, TextFormError>`: styled characters in display order. |

Construct an owned `Text` with its content, style and children. A bridge with a
flat component representation can build a `TextForm` and call
`Text::try_from(form)` once at its boundary; the host reads accept that validated
`Text`, not JSON or engine text types. Both methods return a `TextFormError` if
the supplied component cannot be represented at the engine boundary.

Width is the same styled and translated width used by the engine's tooltip
layout. Wrapping follows `Font.split(FormattedText, width)`: it keeps styles,
breaks at newlines and fitting spaces, and uses the vanilla one-pixel minimum
for widths below one. Returned lines are API-owned `Text` values. Translation
uses the current client language; a later language or resource-pack font change
is reflected by the next call, so do not retain dimensions across reloads.

NeoForge's `ClientTooltipComponent` receives a `Font` in its width and drawing
methods. These reads use that same role—the active client font—without passing
an engine font object or requiring a `DrawContext`. They do not draw anything
and do not expose a full rendering context.

## Resolved strings

For vanilla `Component.getString()` behavior, use
`ClientHost::text_string(&Text) -> Result<String, TextFormError>`. This client
setup read also works outside drawing on native and wasm. It uses the same
component conversion as measurement: translation arguments, key names and nested
siblings resolve against the current language on every call. The result contains
plain characters, not serialized JSON or translation keys. Styles do not add
characters; literal legacy `§` formatting codes remain in the string, unlike the
font's formatted rendering. Invalid components return the same boundary errors
as the other text reads. Do not cache resolved strings across language changes.

## Visual order

For vanilla `Component.getVisualOrderText()` behavior, use
`ClientHost::text_visual_order(&Text) -> Result<Vec<VisualTextCharacter>, TextFormError>`.
Each API-owned Wire record contains a Unicode scalar (`scalar`), its full
resolved `TextStyle`, and the sink's `source_index`. Characters are already in
visual order, including Arabic shaping and mirrored punctuation; do not apply
bidi a second time. This read uses the engine's existing language and bidi
pipeline, is available outside drawing including client setup, and is identical
on native and wasm.

Sink indices are UTF-16 offsets local to a constant-style substring within a
directional run, not global component positions. For example, `A😀B` yields
indices `0, 1, 3`; Hebrew `שלום` yields `3, 2, 1, 0`. A style or bidi run boundary
restarts the index. Shaping can shorten the string, so these are not guaranteed
to identify original input scalars. Use the records to re-layout existing tooltip
lines without losing click, hover, font or other style information. Like the
other reads, invalid component input returns its boundary error.

Action keybind IDs such as `key.jump` resolve through the client's current key
binding to their assigned physical key display name, matching vanilla's `KeybindContents`.
Physical key names resolve normally.
