# Write the code

A mod is a Rust type that implements the `Mod` trait from `horizon-mod-sdk`. The game calls its methods, the hooks, at fixed points, and the mod calls back into the game through the parts of the API its capabilities allow. This page explains both, then builds a complete mod step by step.

## The shape of a mod

```rust
use horizon_mod_sdk::{Mod, Registration, chat};

struct Hello {
    greeted: u32,
}

impl Mod for Hello {
    fn init() -> Result<(Self, Registration), String> {
        Ok((Hello { greeted: 0 }, Registration::new().command("hello")))
    }

    fn command(&mut self, _name: &str, args: &str) -> Result<Option<String>, String> {
        self.greeted += 1;
        chat::show(&format!("Hello {args}! ({} so far)", self.greeted));
        Ok(None)
    }
}

horizon_mod_sdk::export_mod!(Hello);
```

- **The state lives in your type.** `init` creates it, and every hook gets `&mut self`.
- **`export_mod!`** connects the type to the game. A mod has exactly one.
- **The game calls only the hooks you ask for.** `init` returns a `Registration` that lists them. The other methods keep their default, empty bodies, and a hook you didn't ask for costs nothing.

## Hooks

| Hook | Ask for it with | When the game calls it |
| --- | --- | --- |
| `init() -> Result<(Self, Registration), String>` | always called | Once, when the mod loads. An `Err` keeps the mod off and shows the message on its row. |
| `tick(&mut self, tick: u64)` | `.on_tick()` | Every game tick while the player is in a world: 20 times a second. |
| `frame(&mut self, seconds: f32)` | `.on_frame()` | Every frame the game draws, with the time since the last one. |
| `command(&mut self, name: &str, args: &str)` | `.command("name")` | When the player types `/name …` in chat. `args` is everything after the name. |
| `chat(&mut self, text: &str)` | `.on_chat()` | For each chat line the player receives, as plain text. |

**Commands.** Ask for as many as you like with `.command("…")`, without the slash. The chat box colours them and completes them with Tab, like vanilla's commands. Vanilla's names win: a mod can't take `/give`, and when two mods ask for the same name, the first one loaded keeps it. The game logs both cases. The return value says what happens next:

- `Ok(None)`: nothing more. Use it when the mod already answered with `chat::show`.
- `Ok(Some(text))`: the game shows `text` to the player.
- `Err(text)`: the game shows `text` in red, as a failed command.

**Prefer ticks to frames.** A tick happens 20 times a second whatever the frame rate. The HUD panel is kept by the game and drawn every frame for you, so most mods never need `frame`.

## What your code can call

Each module of the SDK is one part of the API, and each needs its capability in the manifest ([the list](/docs/mod-manifest#capabilities)):

| Module | Capability | Calls |
| --- | --- | --- |
| `game` | `game-read` | `get_player() -> Option<Player>` (`x`, `y`, `z`, `yaw`, `pitch`, `health`, `food`, `dimension`), `get_clock() -> Option<Clock>` (`game_time`, `day_time`). `None` outside a world. |
| `chat` | `chat` | `show(text)` |
| `hud` | `hud` | `set_panel(Corner, &[Line])` (`Line { text, color }`, colour as `0xRRGGBB`; corners `TopLeft`, `TopRight`, `BottomLeft`, `BottomRight`), `clear()` |
| `storage` | `storage` | `get(key) -> Option<Vec<u8>>`, `set(key, &[u8]) -> Result<(), String>`, `delete(key)` |
| `log` | none | `horizon_mod_sdk::info(text)` and `warn(text)`: lines in the game log |

What the game reads for you (`game`) is a snapshot taken once a tick, and again before a command runs. It never changes in the middle of a call. Everything else is plain Rust: your own types, maths and text, and the crates you add to `Cargo.toml`, as long as they build for `wasm32-unknown-unknown` without the operating system (no files, threads, network or clock).

## A worked example: Playtime

This mod counts how long you play, shows it in the top-right corner, and keeps the count when you quit. `/playtime` says it in chat; `/playtime hide`, `show` and `reset` do what they say. It uses three capabilities (`hud`, `chat`, `storage`) and two hooks (`tick`, `command`).

**1. Make the mod.**

```sh
cargo horizon-mod new playtime --name "Playtime"
cd playtime
```

**2. Declare what it may do.** In `horizon-mod.toml`, set the description and the capabilities:

```toml
description = "How long you have played, in a corner of the screen"
capabilities = ["hud", "chat", "storage"]
```

**3. The state and a helper.** Replace `src/lib.rs` with the code below, in parts. First the imports, the state and a function that turns ticks into hours and minutes. It touches no API, so an ordinary unit test can check it (step 7).

```rust
use horizon_mod_sdk::{
    Mod, Registration, chat,
    hud::{self, Corner, Line},
    storage,
};

/// Ticks played, kept across sessions in the mod's storage.
struct Playtime {
    ticks: u64,
    shown: bool,
}

/// `1h 05m` from a number of ticks (20 a second).
fn format(ticks: u64) -> String {
    let minutes = ticks / 20 / 60;
    format!("{}h {:02}m", minutes / 60, minutes % 60)
}
```

**4. Saving and drawing.** `storage::set` writes bytes under a key of your choice; it can fail when the mod is over its quota, so the error goes to the log. The panel is set once and the game draws it until it changes, so redraw only when there is something new to show.

```rust
impl Playtime {
    fn save(&self) {
        if let Err(e) = storage::set("ticks", &self.ticks.to_le_bytes()) {
            horizon_mod_sdk::warn(&format!("could not save: {e}"));
        }
    }

    fn draw(&self) {
        if self.shown {
            let line = Line { text: format!("Played {}", format(self.ticks)), color: 0xFFFF55 };
            hud::set_panel(Corner::TopRight, &[line]);
        } else {
            hud::clear();
        }
    }
}
```

**5. The hooks.** `init` reads what an earlier session saved, draws the panel and asks for one command and the tick hook. `tick` counts, and once a minute redraws and saves. `command` reads its arguments.

```rust
impl Mod for Playtime {
    fn init() -> Result<(Self, Registration), String> {
        // What an earlier session saved, or 0 the first time.
        let ticks = storage::get("ticks")
            .and_then(|bytes| bytes.try_into().ok())
            .map_or(0, u64::from_le_bytes);
        let state = Playtime { ticks, shown: true };
        state.draw();
        Ok((state, Registration::new().command("playtime").on_tick()))
    }

    fn tick(&mut self, _tick: u64) {
        self.ticks += 1;
        // Once a minute: redraw the panel and save.
        if self.ticks % (20 * 60) == 0 {
            self.draw();
            self.save();
        }
    }

    fn command(&mut self, _name: &str, args: &str) -> Result<Option<String>, String> {
        match args.trim() {
            "" => chat::show(&format!("You have played {}", format(self.ticks))),
            "hide" | "show" => {
                self.shown = args.trim() == "show";
                self.draw();
            }
            "reset" => {
                self.ticks = 0;
                self.save();
                self.draw();
                return Ok(Some("Playtime reset".into()));
            }
            other => return Err(format!("Unknown option {other}: use hide, show or reset")),
        }
        Ok(None)
    }
}

horizon_mod_sdk::export_mod!(Playtime);
```

**6. Try it.** `cargo horizon-mod build`, then start the game with it as a dev mod ([how](/docs/make-a-mod#see-it-in-the-game)). Open a world: "Played 0h 00m" sits in the top-right corner and goes up each minute. Type `/playtime`, then `/playtime hide`. `/playtime foo` answers in red. Quit and come back: the count goes on from where it was.

**7. Test the helper.** At the end of `src/lib.rs`:

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn formats_hours_and_minutes() {
        assert_eq!(super::format(0), "0h 00m");
        assert_eq!(super::format(20 * 60 * 65), "1h 05m");
    }
}
```

`cargo test` runs it on your computer, with no game. [Test it](/docs/test-a-mod) covers the rest.

**Where to go from here.** The SDK's examples are complete mods to read: [`coords-hud`](https://github.com/ludovic111/horizon/tree/main/crates/horizon-mod-sdk/examples/coords-hud) (coordinates, facing and the time on the HUD, `/where` and `/coords`) and [`ruby-content`](https://github.com/ludovic111/horizon/tree/main/crates/horizon-mod-sdk/examples/ruby-content) (a content mod). The [reference](/docs/api/mods-reference) lists every type and function, with its limits.

Next: [content mods](/docs/content-mods), or straight to [testing](/docs/test-a-mod) if your mod has no blocks or items.
