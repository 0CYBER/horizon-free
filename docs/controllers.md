# Controllers

Horizon plays with PlayStation (DualShock 4, DualSense), Xbox (360, One,
Series) and Nintendo Switch Pro controllers, and on the Steam Deck with its
built-in controls. Keyboard and mouse keep working exactly as before: with no
controller connected, or with controllers turned off, nothing changes.

## What vanilla has

Nothing. Minecraft Java 26.3 never calls GLFW's gamepad or joystick API, has
no controller options and ships no button glyphs. Its only non-mouse UI input
is keyboard focus navigation (`InputType.KEYBOARD_ARROW` / `KEYBOARD_TAB`:
arrows and Tab move a focus ring between widgets). So controller support is
Horizon's own feature. It follows what players already know from Minecraft on
consoles (Bedrock Edition's default controller layout) and Horizon's menu
design for any new screen.

## Input layer

[gilrs](https://crates.io/crates/gilrs) reads the pads: evdev on Linux (with
libudev for hot-plugging), IOKit on macOS, Windows.Gaming.Input on Windows.
Its SDL mapping database normalizes every pad to one positional layout
(South/East/West/North face buttons, bumpers, triggers, sticks, D-pad,
Select/Start). No second windowing or event stack: gilrs is polled once per
loop iteration next to winit's events.

- The pure logic (mapping table, deadzones and curves, button edges, the
  settings file, glyph choice) lives in `crates/horizon-client/src/gamepad.rs`
  and is unit-tested. The game owns a thin gilrs backend and applies actions.
- Controller actions go through the same path a keyboard key takes after
  rebinding: an action names a vanilla key mapping (`key.jump`, `key.attack`,
  ...), and the game handles it as that mapping's default key or mouse button.
  Toggle sneak/sprint/attack/use, screens releasing held keys, and the
  multiplayer client all behave as they do on the keyboard.
- Movement is analog: the left stick's direction and tilt feed the player's
  move vector directly (a half-tilted stick walks slowly, like Bedrock),
  instead of being rounded to WASD. The forward/back/left/right flags other
  code reads (sprint start, boats, minecarts, the server's input packet) are
  set when the stick passes a threshold.
- The pad last touched is the active one. Several pads are not merged.
- `horizon-ctl pad ...` injects a virtual pad (buttons, sticks, triggers) into
  a running game, for tests without hardware.

## Default mapping

| Action | Xbox | PlayStation | Mapping |
|---|---|---|---|
| Move | Left stick | Left stick | analog `key.forward/back/left/right` |
| Look | Right stick | Right stick | camera |
| Jump / fly up | A | ✕ | `key.jump` |
| Drop item | B | ○ | `key.drop` |
| Swap offhand | X | □ | `key.swapOffhand` |
| Inventory | Y | △ | `key.inventory` |
| Hotbar left / right | LB / RB | L1 / R1 | Horizon `pad.hotbarPrevious/Next` |
| Use / place | LT | L2 | `key.use` |
| Attack / break | RT | R2 | `key.attack` |
| Sprint | Left stick click | L3 | `key.sprint` |
| Sneak (toggles) | Right stick click | R3 | `key.sneak` |
| Change camera | D-pad up | D-pad up | `key.togglePerspective` |
| Pick block | D-pad left | D-pad left | `key.pickItem` |
| Chat | D-pad right | D-pad right | `key.chat` |
| Advancements | D-pad down | D-pad down | `key.advancements` |
| Player list (hold) | View | Create / Share | `key.playerlist` |
| Pause / back | Menu | Options | Escape (fixed) |

The Guide / PS / Steam button is left to the system. Sneak on a stick click
toggles by default (holding a stick in while steering the camera is
awkward, which is why Bedrock does the same); the controls options can make it
hold-to-sneak.

## Menus and inventories

Vanilla's screens are all mouse-driven, and Horizon draws them 1:1. A pad
drives them in one of two ways, picked in the controller options ("Menu
Navigation: Focus / Pointer", `menuNavigation:focus|pointer`):

- **Focus** (the default): a console-style highlight that the D-pad and the
  left stick move between buttons, sliders, switches, text fields, list
  rows, tabs and inventory slots.
- **Pointer**: a virtual cursor the left stick moves, as Bedrock's classic
  inventory UI and Java's controller mods do.

Focus is the default because it is what a pad player expects from a console
game, it needs no aiming (every widget is one D-pad press away, which also
suits the Steam Deck's small screen), and it works from the couch. The
pointer stays one option away, and a **right stick click swaps the two for
the open screen**, for the few places a pointer is better (dragging the
advancements map, painting a crafting grid with a held stack).

Both modes feed the screens the same pointer events a mouse sends: in focus
mode the (hidden) pointer sits on the focused widget. So every screen keeps
its mouse behaviour (drags, sliders, scrollbars, hover highlights) with no
controller code of its own, and both modes reach every screen. Moving the
real mouse hands control back at once.

### Focus navigation

Every interactive rectangle a screen draws registers itself as a focus
target with its kind (`widgets::target_as`): classic buttons, sliders, edit
boxes and list rows (worlds, servers, languages), every Horizon-style widget
(`ui_theme::Ui::interact`: sidebar items, switches, segments, sliders,
fields, buttons), inventory and container slots, creative and recipe-book
tabs, and the stations' own buttons (trades, loom patterns, beacon effects,
enchantments, stonecutter recipes, the recipe book's recipes, pages and
filter). A screen that registers nothing falls back to the pointer.

The step itself is **vanilla's arrow-key navigation**
(`ContainerEventHandler.nextFocusPathInDirection`, 26.3), applied to every
target of the screen at once: from a strip one pixel past the focused edge,
the nearest target that overlaps it on the other axis wins (ties go to the
leftmost or topmost); without one, the target whose facing edge's centre is
nearest the focused edge's centre (`nextFocusPathVaguelyInDirection`). That
gives vanilla-like jumps in containers too: hotbar to inventory, the grid to
the crafting result, armour to the player's grid.

| Input | Focus mode |
|---|---|
| D-pad, left stick | move focus (held: repeats after 0.35 s, every 0.09 s) |
| A | press the focused widget (held: drag); on a text field, type with the on-screen keyboard |
| B, Menu | back (Escape) |
| X | right click (inventories: take half, place one) |
| Y | quick move (shift-click) in inventories |
| View | throw one item from the focused slot (Q) |
| Left stick click | throw the focused stack (Ctrl+Q) |
| LB / RB | previous / next tab (creative, recipe book, the Horizon options' sections) |
| Right stick | scroll |
| Right stick click | swap focus and pointer for this screen |

- **Sliders**: left and right move the value (one pixel per press like
  vanilla's arrow keys, faster while held); A does nothing on a slider, as a
  click would jump the handle.
- **Lists** follow focus: a step past the last visible row scrolls the list
  and lands on the next row once it is drawn.
- **Inventories**: the carried stack is drawn on the focused slot (vanilla
  draws it under the pointer). A takes or places, X splits or places one, Y
  quick-moves, View and the left stick click throw.
- **Where focus lands** when a screen opens: the held hotbar slot in an
  inventory, else the first button.

How it looks follows the menu style:

- **Classic screens** look as vanilla's own keyboard focus does: buttons,
  sliders and slots show their highlighted sprite (`isHoveredOrFocused`,
  drawn because the hidden pointer is over them), and list rows and text
  fields get vanilla's white one-pixel outline (`AbstractSelectionList`'s
  focused selection, `EditBox`'s highlighted border).
- **Horizon-style screens** get the design system's focus colour: a two-dp
  ring in the accent colour around the focused widget, pill-shaped on
  buttons, tabs and sliders.

### Pointer

- Left stick moves the cursor with acceleration; in container screens the
  cursor is gently pulled toward the nearest slot centre.
- D-pad steps to the nearest target that way (a slot, a button) and snaps to
  it.
- A = left click, X = right click, Y = quick move, B = back; View and the left
  stick click throw, as in focus mode.
- LB / RB switch tabs; right stick scrolls.
- Sliders: hold A on the slider and move the stick, or nudge with the D-pad.

The cursor is drawn only while the pad is the last-used input.

### Text fields

A on a focused text field (A in pointer mode clicks into it) opens the
**on-screen keyboard**: a QWERTY grid and a symbols grid in Horizon's design
language, at the bottom of the screen (at the top when the field is in the
lower half, as chat is). D-pad or stick to move, A types, X deletes, Y is a
space, LT is Shift (twice: caps lock), RB switches to symbols and accents,
Menu is Enter, B closes it. It opens by itself when the pad opens chat or a
sign. It types into every field the keyboard reaches: chat (local and
server), signs, books, the title screens' fields, the options' searches, the
creative and recipe-book searches, the anvil, the LAN port.

On a Steam Deck under Steam (`SteamDeck=1`), A asks Steam for its own
keyboard instead (`steam://open/keyboard`), which types like a real keyboard;
Steam + X opens it at any time. "On-Screen Keyboard: Auto / Horizon / Steam"
(`keyboard:`) picks one.

### Screens covered

Both modes reach every screen through the pointer events. Focus mode
registers targets on: the title menus (main, world list and its search, create
and edit world, delete, multiplayer list and its edit screens), every options
page (classic pages and the Horizon-style sections, key and controller binds,
language list), pause, death, chat, signs and books (their buttons), the
survival and creative inventories, every container and station (chests,
furnaces, brewing, crafter, crafting table with its recipe book, anvil,
grindstone, smithing, stonecutter, loom, cartography, enchanting, beacon,
villager and wandering-trader trades, mount inventories), and the LAN /
world options. Where a screen draws something clickable no target names
(the advancements map, the statistics lists, the pack screen's rows, the
book's page text), the right stick click gives the pointer for that screen.

## Glyphs

Horizon draws its own button glyphs in the HUD and screens (no Mojang
assets, nothing new from the player's jar): a small round or pill shape with
the label in the game font, sized to the GUI scale, with two styles:

- **Xbox / Steam Deck / generic**: A B X Y in their usual colours, LB RB LT
  RT, View, Menu, stick icons.
- **PlayStation**: × ○ □ △ in their colours (× rather than ✕, which the game fonts lack), L1 R1 L2 R2,
  Create/Share, Options.
- **Nintendo**: B A Y X by position.

The style comes from the pad's USB vendor (Sony 054c, Microsoft 045e,
Nintendo 057e, Valve 28de) unless the controls options force one. Glyphs
replace key names where a prompt names a key (the controls list, tutorial
hints, "Press X to dismount") only while the pad is the last-used input.

## Options and remapping

Controller settings are Horizon's own and live in `horizon-controller.txt`
next to `options.txt`, like `horizon-graphics.txt`: `options.txt` stays
exactly vanilla's, so a vanilla client can share the game folder.

```
enabled:true
vibration:true
lookSensitivity:0.5
invertX:false
invertY:false
moveDeadzone:0.15
lookDeadzone:0.12
triggerThreshold:0.4
toggleSneak:true
glyphs:auto
bind_key.jump:south
bind_pad.hotbarNext:rb
```

Only bindings that differ from the defaults are written. The controls
options get a Controller page: on/off, vibration, look sensitivity, invert
X/Y, deadzones, glyph style, sneak toggle, and a binding list like the key
binds screen (pick an action, press a button; conflicts marked as vanilla
does; reset).

Deadzones are radial (the stick's length, so diagonals aren't clipped) and the
remaining range is rescaled to 0..1. Look uses a squared curve for precise
aim near the centre, up to 200°/s yaw and 140°/s pitch at the default
sensitivity, slower while scoping with a spyglass.

## Rumble

Vibration (on by default, off in the options) through gilrs force feedback:
taking damage (stronger with the damage), a block broken, an explosion nearby
(scaled by distance), a totem of undying, an elytra boost, a fish biting.
Short, low-intensity pulses, as on Bedrock. Linux (evdev force feedback,
including Steam Input's virtual pad) and Windows support it; macOS gilrs has
no force feedback, so it is silently skipped there.

## Steam Deck

- In Game Mode, or any game launched through Steam, Steam Input presents the
  Deck's controls as a virtual Xbox pad: Horizon sees it like any Xbox pad
  and uses Xbox-letter glyphs (the Deck's face buttons are A B X Y). Steam's
  default "Gamepad" layout works with no setup; the trackpads stay mouse.
- Started outside Steam on the Deck, the kernel's `hid-steam` driver exposes
  the controls as a gamepad too (while Steam isn't holding them), so it also
  works out of the box.
- Text entry uses Steam's keyboard (Steam + X).

## PR plan

The first round (input layer, menus, glyphs and options, rumble) shipped in
PRs #120, #129, #145 and #149. The second round, each shippable alone:

1. **Focus navigation** (this section above): the menu navigation option,
   focus targets and vanilla's arrow-key step, the focus highlight in both
   menu styles, the on-screen keyboard.
2. **Button prompts**: contextual hints with the pad's glyphs in menus and
   optionally in game, and rebinding from them.
3. **DualSense output**: adaptive triggers, lightbar, player LEDs and richer
   rumble over HID, off-safe.
4. **Launcher**: the same focus navigation in the gpui launcher.

Each is tested with unit tests (focus steps, prompt contexts, report bytes),
the injected pad and vscreen screenshots; each PR says what still needs a
real controller.
