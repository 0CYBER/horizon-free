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

Vanilla's screens are all mouse-driven, and Horizon draws them 1:1, so a pad
drives them through a **virtual cursor** (as Bedrock's classic inventory UI
and Java's controller mods do), not focus navigation:

- Left stick moves the cursor with acceleration; in container screens the
  cursor is gently pulled toward the nearest slot centre.
- D-pad steps one slot (18 GUI pixels in containers) or one widget row in
  menus and snaps to it.
- A = left click (take / place stack, press a button), X = right click
  (half stack / place one), Y = quick move (shift-click), B = back / close.
- LB / RB switch creative tabs and recipe book pages; right stick scrolls
  (creative grid, lists, chat).
- Sliders: hold A on the slider and move the stick, or nudge with the D-pad.
- Text fields: the Steam Deck's keyboard (Steam + X) types into them like a
  real keyboard. Other platforms have no on-screen keyboard yet; text input
  needs a keyboard.

The cursor is drawn only while the pad is the last-used input; moving the
mouse hands control back immediately.

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

1. **Input layer and gameplay** (this note, gilrs, defaults, movement,
   camera, actions, hotbar, B/Start to leave screens, `horizon-ctl pad`,
   `horizon-controller.txt` with the look/deadzone settings).
2. **Menus and inventories**: virtual cursor, slot snapping, clicks, quick
   move, tabs, scrolling, sliders.
3. **Glyphs and the Controller options page**: glyph drawing per pad type,
   the options page, remapping UI.
4. **Rumble**.

Each is shippable alone. Testing is with unit tests, the injected pad and
vscreen screenshots; each PR says what still needs a real controller.
