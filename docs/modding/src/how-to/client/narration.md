# Narrating a line

A client mod has the player's narrator say a line with `ClientNarratorClientHost::narrate`,
as Jade's "Narrate Target" reads the block or entity under the crosshair (`GameNarrator.saySystemNow`
in Java). It goes through the game's own narrator, so the Narrator option, the voice volume and
the narrator's log of recent lines apply as to the game's own narration. Nothing reaches the server.

```rust
use mod_api::prelude::*;
use mod_api::{NarrateError, Narration, Text};

fn target_changed(host: &mut dyn ClientHost, name: &Text) {
    match host.narrate(name, Narration::Now) {
        Ok(()) => {}
        // The player turned narration off, or chose Chat only: nothing to do.
        Err(NarrateError::Off) => {}
        Err(other) => log::debug!("not narrated: {other}"),
    }
}
```

## When

`Narration::Now` (`saySystemNow`) cuts off what is being said; `Narration::Queued`
(`saySystemQueued`) waits for it. The line is the component's plain string (`Component.getString`)
in the current language. There is no length cap beyond the component's own form limits.

## Refusals

Each refusal says and logs nothing, and has its own name: `Text` (the component was refused at the
boundary), `Empty` (its text is empty), `Off` (the option is Off or Chat only) and `Unavailable`
(no speech on this platform, or no client narrator). Match `NarrateError` non-exhaustively.
