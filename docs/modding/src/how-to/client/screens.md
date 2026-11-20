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

## Quit Game

A native client mod with its own Quit Game confirmation can call `host.request_client_quit()`
after the player confirms. The call works from a title input hook's shared `ClientHost`.
It returns `true` when queued, then the client starts its normal shutdown after the hook
dispatch returns. This includes saving and stopping an open integrated server.
It returns `false` if a quit is already queued or underway, or when the host cannot quit
the primary window, for example on a headless client or a splitscreen guest.

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
