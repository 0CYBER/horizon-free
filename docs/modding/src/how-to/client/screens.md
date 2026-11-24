# Screens, the inventory key and the sign editor

A client mod can close the screen a player has open, open the player's inventory, and read which sign
the sign editor is on. The two writes take the same paths as the keys the player uses, so the server
sees only the packets of a normal client.

```rust
// Escape: close whatever is open.
if host.close_screen() {
    // a screen was open and is now closed
}

// The inventory key, key.inventory.
if host.open_inventory() {
    // the inventory screen is up (or the ridden vehicle's screen was asked for)
}

// Which sign the editor is on, if any.
if let Some(sign) = host.sign_editor() {
    let _ = (sign.pos, sign.front);
}
```

`ClientHost::close_screen()` follows the screen's own Escape close. A container screen closes as the
player's container close: the client sends `ServerboundContainerClosePacket` with the menu's id and then
closes its copy of the menu. It never drops a container menu locally alone, which would leave the
server's menu open. A pause menu closes to the game, another screen runs its own close, and the sign
editor sends `ServerboundSignUpdatePacket` with the lines being edited, as Escape does. The death
screen does not close this way. The screen hooks' closing notices fire as they do for Escape. It returns
`false` when no screen is open.

`ClientHost::open_inventory()` does what `key.inventory` does: it opens the player's inventory screen
locally and sends nothing, because the player's menu is always open on the server. Riding a vehicle
with its own inventory (a tamed mount or a chest boat), it sends the open-inventory player command
(`ServerboundPlayerCommandPacket`) at the player's next tick instead, as the key's press is. It returns
`false` with a screen already open, for a spectator, in a portal transition, or with no world.

`ClientHost::sign_editor()` returns `Some(SignEditView { pos, front })` while the sign editor is open:
the sign's block position and whether its front side is being edited. It returns `None` with no sign
editor open, or outside a world. A close of the sign editor always sends the sign update, however it
closes; there is no close that skips it, because the vanilla client never omits that packet.

## Reading and interacting with a container

`host.screen_menu()` returns the live menu only when its container screen is open, including the
normal player inventory but not creative tabs. In `container_mouse_input_pre`, `event.menu` is that
same handle. Button presses and releases, held-button movement and wheel scroll arrive after the
existing read-only screen Pre hook and before the screen acts. This hook has a mutable `ClientHost`.
Returning `Flow::Cancel` replaces the screen's handling of that input, including its usual Post
hook. If you take over a press and its drag, handle the matching release too; don't swallow the
release of a drag the screen itself started.

Use menu-order indices throughout. `menu.slot_count(host)` is the count, `menu.slot(host, index)`
and `menu.carried(host)` read live stacks (`Some(None)` is empty, outer `None` means unavailable),
and `menu.hovered_slot(host)` gives the screen's actual click target, even when empty. It follows
the game's hit margin, layout offsets and hidden slots; a narrow recipe book covering the menu
returns no hover. `menu.slot_kinds(host)` identifies known result slots, such as inventory
crafting slot 0. `menu.slot_holds(host, index, &candidate)` instead gives the maximum count the
slot can hold for that stack, including its components and the slot's own restrictions.
`Some(0)` means it won't accept that stack (e.g. stone into head armour, or into a result slot);
it does not identify the slot's role. This is not remaining space or a server promise.

```rust
use mod_api::{
    ClientHost, ContainerMouseInput, ContainerMouseInputPreEvent, Flow, Hooks, MenuClick,
    ScreenRenderPostEvent,
};

#[derive(Default)]
struct ContainerHelper;

#[mod_api::horizon_mod]
impl Hooks for ContainerHelper {
    fn container_mouse_input_pre(
        &self,
        host: &mut dyn ClientHost,
        event: &mut ContainerMouseInputPreEvent,
    ) -> Flow {
        // Example gesture: Alt + wheel up shift-clicks the hovered slot.
        if event.modifiers & 4 == 0
            || !matches!(event.input, ContainerMouseInput::Scroll([_, y]) if y > 0.0)
        {
            return Flow::Continue;
        }
        let menu = event.menu;
        let Some(slot) = menu.hovered_slot(host) else {
            return Flow::Continue;
        };
        if menu.click(host, &[MenuClick::QuickMove { slot }]) {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }

    fn screen_render_post(&self, host: &dyn ClientHost, event: &mut ScreenRenderPostEvent<'_>) {
        let Some(menu) = host.screen_menu() else { return };
        let Some(index) = menu.hovered_slot(host) else { return };
        let Some(boxes) = menu.slot_boxes(host) else { return };
        if let Some(&Some(rect)) = boxes.get(index) {
            event.draw.fill(rect, 0x66ff_a000);
        }
    }
}
```

For a move plan, submit `Pickup` with `MouseButton::Left` or `Right`, `QuickMove` for
Shift-click, or `PickupAll` to collect matching stacks onto a stack already carried (the
ordinary double-click action). `Drag` submits a complete quick-craft gesture. These are the
ordinary predicted menu operations and server packets, not synthetic mouse input. A successful
submission is not acknowledgement. Reread `slot` and `carried` immediately to see prediction and
again on the next input or tick to see server corrections, rather than assuming the plan's state
is still current. Reacquire the handle and discard a pending plan if the screen closes, its handle
changes, or a read becomes unavailable. A stale handle cannot act on a replacement menu, even
when the server reuses the same window id.

`screen_render_post` already composites above the game's screen. Use the slot boxes in GUI
pixels to draw markers for locked slots as well as the highlight above. A hidden slot has no
box; while a narrow recipe book covers the menu there are no boxes to draw.

## Replacing the title splash

Native and Wasm mods can call `host.title_splash()` from the title's screen render hooks
(`event.screen.id == "title"`). It returns the exact text the client already picked, including
special-date and player-name splashes. It does not pick a new line each frame. If you cancel
`screen_render_pre` to replace the stock title drawing, you can still read the same text in
`screen_render_post`.
The loading frontend does not run screen render hooks until it hands the title to the game.

The result is `None` while another screen is active, when the player enables Hide Splash Texts, or
when no splash was selected.

## GUI-only in-world frames

A full-screen map can return `Flow::Cancel` from `render_frame` while `host.mod_screen()` is its
own screen. Paint the map in `screen_render_post` and use the regular screen input hooks. The
game presents the GUI over black in its usual SDR or HDR output, without preparing or drawing the
world. Full-frame screenshots and video export include the GUI; world icons and scene captures
wait for a world draw. Normal world rendering resumes on the next frame that does not request
GUI-only presentation, with fresh temporal history.

## Decorating Multiplayer rows

Implement the client hook `server_list_row_decorations` to draw on the saved servers in the title's
Multiplayer list. The event gives you the saved `name` and `address`, the full scrolled `rect`, and its
`visible_rect`. Both rectangles use `[x, y, width, height]` in GUI pixels. Position your decoration
relative to `rect`: the game clips the drawing to `visible_rect`, including at the scroll edges.
Hidden and offscreen entries, LAN games, the LAN scanner, and pages such as Add or Edit Server do not
fire the hook.
Native and WebAssembly client mods use the same hook and `DrawContext` methods.

`mouse` gives the pointer in GUI pixels only when it's over the visible part of the row. Append
`Text` lines to `tooltip` then; the screen places them after drawing its contents, outside the row
clip, and keeps its own ping or player tooltip lines first. You can test the pointer against your
sprite's rectangle if you want the tooltip only on the sprite. Keep saved addresses private; do not
log them or send them off the client.

```rust,ignore
fn server_list_row_decorations(
    &self,
    _: &dyn ClientHost,
    event: &mut ServerListRowDecorationsEvent<'_>,
) {
    let [x, y, w, _] = event.rect;
    let [sx, sy, sw, sh] = [x + w - 16.0, y + 23.0, 12.0, 8.0];
    let painted = event.draw.sprite(
        "example:gui/sprites/region_flag",
        [0.0, 0.0, 12.0, 8.0],
        [sx, sy, sw, sh],
        None,
    );
    if painted
        && let Some([mx, my]) = event.mouse
        && (sx..sx + sw).contains(&mx)
        && (sy..sy + sh).contains(&my)
    {
        event.tooltip.push(Text::literal("Server region"));
    }
}
```

## Quit Game

A native client mod with its own Quit Game confirmation can call `host.request_client_quit()`
after the player confirms. The call works from a title input hook's shared `ClientHost`.
It returns `true` when queued, then the client starts its normal shutdown after the hook
dispatch returns. This includes saving and stopping an open integrated server.
It returns `false` if a quit is already queued or underway, or when the host cannot quit
the primary window, for example on a headless client or a splitscreen guest.

## Replacing the saving-world screen

Native and wasm client mods can use `Hooks::closing_save_screen_draw` when the
window prepares a drawable saving-world frame, separately from ordinary
screen, HUD and frame hooks. It does not run on a suspended renderer, startup
or resource-reload frames, leaving a remote server, quitting from the title
without a save, or headless saves.

The event reports `ClosingSaveOperation::LeaveWorld` when closing a local world,
`QuitClient` when saving to exit, and `ExplicitSave` for control-socket `save` and
`save-all` only when they take the client-owned pending-save path and show this
screen. An explicit save already underway stays `ExplicitSave` even if a quit
is queued behind it. Integrated-server saves requested directly by those
commands do not show this screen.

`ClosingSaveStage` reports `Generation`, `Chunks`, `Entities`, `Writes`, or
`Server` (the client is waiting for the integrated server to save and stop).
`event.progress` is the default screen's coarse `[0, 1]` phase marker for
`Chunks`, `Entities`, and `Writes`. It is `None` during `Generation` and
`Server`: there is no measured progress for those waits.

Draw the entire replacement with `event.draw` in GUI pixels and return
`Some(())` to suppress the default drawing. Return `None` without drawing to
leave the decision to the next listener or Horizon. The first replacement
wins. This affects only the frame: mods cannot cancel, complete, or reschedule
the save through this hook. The default widget screen ID remains `saving`.

## Taking a screenshot

`ClientHost::request_screenshot()` takes a screenshot as the screenshot key (F2) does. The next
frame is read back at the size F2 saves, the `screenshot` hook runs on it as it does for F2 (so every
mod's screenshot listener sees mod-started screenshots too, and may change, rename or cancel them), and
the PNG is written to the player's `screenshots` folder under vanilla's file name with F2's chat line.
`Ok(())` means the screenshot is queued, not yet written.

A mod-started screenshot never replaces a file. Each PNG is written to a temporary file in the same
folder and then published under its final name only if no file has that name: a name the hook picked
that already exists keeps the existing file, the new one is deleted, and the chat line reports the
failure. A failed write leaves no partial file. A name the hook picked may create at most one new
folder: a deeper path reports the failure and writes nothing.

It returns `Err(ScreenshotError)` and queues nothing:

| Error | When |
|---|---|
| `Busy` | a capture is queued or being read back, the `screenshot` hook is running, a screenshot is still being written, or a screenshot sequence runs |
| `BudgetExhausted` | mod-started screenshots have written 4 GiB this game session, shared by all mods |
| `Folder` | the `screenshots` folder could not be created |
| `Unavailable` | no window renders here: a server's thread or a client without a renderer |

One screenshot runs at a time, so a mod cannot queue a burst. The session budget counts each written
capture at its uncompressed size after the hook (4 bytes a pixel, 6 for an HDR copy), an upper bound on
the PNGs, and at least 1 MiB: about 500 screenshots at 1920 by 1080, 120 at 3840 by 2160, and never more
than 4096. A capture that would pass the budget once the hook has run is not written, and the chat line
reports the failure. The budget is reset when the game restarts. F2 itself is never limited.
