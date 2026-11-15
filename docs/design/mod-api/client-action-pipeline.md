# Client action pipeline design

The window owns input, targeting, animation and prediction. The logical server owns
combat and world changes. A single-player window uses the same client path as a window
connected to a remote server. Client action hooks serve mods such as auto-tool, hold-attack
and mining overlays without changing the server's gameplay events.

## Where an action starts

- `Game::remote_mouse` updates key state and queues presses. Keyboard bindings and
  controller actions should reach the same queue and held state. Raw input events can
  consume a physical input before it becomes a mapping. Action hooks run later, after
  the game has decided that a mapping may act.
- `Game::remote_handle_input` sends any changed held slot, stops a dig when gameplay
  is unavailable and delegates to `InteractionInput::tick`. This is the boundary
  between physical inputs and gameplay. The input crate owns the sequence and should
  keep no dependency on mods.
- `InteractionInput::tick` snapshots the crosshair once, drains attack, use and pick
  presses in order, then tries the held use and attack continuations. While an item is
  being used, it drains the press queues and releases the item when the use key goes up.
  Preserve this order and the same target for every attempt in a tick.

## Input and prediction contracts

Cancellation and `swing_hand` are independent. For an otherwise successful attack,
use (either hand), or held mining attempt, `(cancel, swing)` gives action/swing-request
counts `(false, false) → 1/0`, `(false, true) → 1/1`, `(true, false) → 0/0`, and
`(true, true) → 0/1`. A swing request does not guarantee a new animation: an item
can veto it, and an animation already in progress can prevent a new swing from
starting during held mining. An uncancelled use that naturally does not request a
swing still does not request one when the flag is true. Main-hand cancellation skips
the off-hand event. Pick retains its packet veto and never requests a swing,
including when `swing_hand` is true. Native and wasm listeners share this decision
and the no-listener path retains vanilla behavior.

Extend `interaction_key_mapping_triggered` at its existing call sites. The event should
carry whether the attempt is from a queued press or a held mapping, alongside the
existing key, hand, cancellation and swing controls. A press attempt and a held attempt
can occur in the same tick. The reason travels from `InteractionInput` through the
game's `InteractionWorld` adapter. Deriving it from the current key state would confuse
a queued press with a held continuation. Listeners use the existing `ClientHost`
crosshair query while the tick's target is pinned.

| Attempt | Dispatch point | Cancellation |
| --- | --- | --- |
| Attack press | The current attack-key event in `remote_start_attack`, after eligibility checks | Skip the attack and preserve the event's swing policy. Spectator actions keep their existing separate route. |
| Held attack | The current attack-key event in `remote_continue_attack`, when the game has a non-air block to mine | Skip this tick's continuation. An allowed attack press can still start a dig earlier in the tick. |
| Use press or repeat | The existing main- and off-hand use events, with the same reason propagated through both hands | Keep the current hand and fallback rules, including cancelling the entire use at the main-hand event. |
| Pick press | The existing pick event, before the target packet | Preserve the existing packet veto. |

`client_mining_predict` runs at the distinct prediction boundary inside
`remote_mining`, after target, restriction and delay checks. It describes the block
position, face, whether the accepted attempt starts or advances a dig, and the
proposed progress. Its typed snapshot and client host are read-only. It runs after
the interaction mapping event, so tool selection can settle before progress is
computed. The server still independently validates
the held item, position, reach and break progression.

A mining veto skips prediction, crack updates and the new action packet. For a
continuation it leaves the existing dig intact so a later allowed attempt can resume
it. A veto of a new target leaves the previous target untouched until the normal
target-change or release path stops it. Ordinary release, focus loss, opening a screen
and server correction must still clean up a running dig. These cleanup paths do not
take a cancellable hook. Preserve the current retained-target behavior when a
survival dig resumes after a miss.

### Feature boundary

Auto-tool can distinguish a fresh attack from a mining continuation, inspect the
current target and react before progress is calculated. Held use is distinguishable
from a queued click. A mod can refuse predicted mining for selected blocks and still
let the existing stop path clear the crack and send the abort. Holding attack on an
entity keeps vanilla's attack cadence. Any API for requesting new actions, retrying
attacks against entities, teleport correction or changing the server's validation
belongs to a separate contract.

## Verification

- Extend the input crate's behavioral tests to distinguish press from repeat and prove
  unchanged target lifetime, click ordering and release while using an item.
- At the client packet boundary check allowed and vetoed attack, main/off-hand use,
  active dig continuation and creative instant break. Compare emitted packets,
  sequence allocation, prediction, swing and crack state. Check a tool switch before
  a start and between continuation ticks.
- Exercise release, screen opening, focus loss, target loss and a server correction
  while an action listener refuses progress. Verify at most one abort for a running
  dig and no surviving crack after cleanup.
- Run the same fixture through the single-player window's integrated server connection
  and a dedicated connection. The mod hook should fire only on the window side.
- Compare the no-subscriber path against the pinned vanilla input/mining harness and
  avoid constructing snapshots or scanning subscribers on unsubscribed held ticks.
