# Choose an item panel style

On the client, `panel_style(mod_id, style)` chooses how the mod's `panel_items` panel looks.
It is a `WorldOnly` operation: call it through the client host while a world is open, not from
server code or a title-screen callback. The choice is kept for the session even while the panel
is hidden; it is not a saved player preference.

`PanelStyle` has three variants:

- `RecipeBook` (the default) uses recipe-book page arrows and a plain search field.
- `Viewer` uses square page buttons on a bar and the Recipes panel's search field. It changes
  drawing, not layout.
- `Classic { centred_search, window_width }` also changes layout: an 18 px ingredient grid sits
  2 px from the screen's right edge below an always-visible page row with wrapping navigation.
  The gap from the container window can exceed 2 px when the free width is not divisible by 18.
  The search field ends 4 px above the screen bottom by default. With `centred_search` and room
  under the window, it moves under the window and ends 2 px above the screen bottom instead.
  Nonempty searches with no matches are red; other Classic search text is white.

For Classic, `window_width: 0` uses the container's own bounds. A positive value reserves at least
that much width for a centred recipe window. This width option is Horizon's own, not a JEI 1.12
setting. The Classic wrench opens the owning mod's page in **Options > Mods**, not a shared
viewer configuration screen. Only the page arrows are tinted; the wrench retains its own colours.

Switching back to RecipeBook or Viewer restores the normal search text colour, including after
a failed Classic search. Changing the style does not replace the panel's item list or its event
handling.
