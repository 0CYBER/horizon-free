# Fake players

`host.fake_player(level, uuid, name)` creates or retrieves a player with no connection in the level in place, as
NeoForge's `FakePlayerFactory` does: a machine that acts as a player (a deployer, a block breaker). It does not participate as an
online player: the tab list, player counts, sleeping, natural spawning, chunk tickets, statistics and saves leave
it out. `player.is_fake(host)` tells one apart; `player.remove_fake(host)` removes it silently.

`player.disconnect(host, reason)` instead kicks the fake with a `Text` reason, as `/kick` by name or selector does.
The first reason stands; removal completes at the next hub poll, after any current action returns.
The shared leave path broadcasts the leave line and fires `player_kicked` and `player_logged_out` (version 2),
even though a fake has no preceding `player_logged_in`. It still writes no player-data save.
Silent `remove_fake` sends no kick, leave line or logout notification.

## Actions

`player.perform_action(host, action)` makes it act as a player does, through the game's own path for the action, as
that player's packet would run it:
- `PlayerAction::UseOn { hand, pos, hit }` is a click on a block (`ServerPlayerGameMode.useItemOn`): reach, the
  hit's place on the block, the build height, spawn protection and the border, then the block's use and the item's
  `useOn`, which is how a block item is placed;
- `PlayerAction::UseItem { hand }` is a use in the air (`ServerPlayerGameMode.useItem`), by the item's own ray from
  the player's eyes and look: a bucket clicked on a block passes, as vanilla's does, and empties when used.
- `PlayerAction::InteractOn { hand, target, location }` interacts with that entity (`Player.interactOn`),
  after the border and interaction-range checks, with the hit relative to its origin. A passed click does not
  fall back to using the item in the air. The fake keeps its own supplied look, rather than aiming a new ray;
- `PlayerAction::Attack { target }` is a hit with the main hand (`handleInteract`'s ATTACK, then `Player.attack`):
  the border and the attack range, the native mods' `attack_entity`, then the blow on a player, a mob or another
  entity, or a fireball, wind charge or shulker bullet turned back. The blow is scaled by the attack strength, as a
  player's is, so a fake that hits every tick does a fraction of a full blow. Mobs do not select the fake as a retaliation target;
- `PlayerAction::BreakBlock { pos }` breaks the block at once (`ServerPlayerGameMode.destroyBlock`): reach, the build
  height, spawn protection and the border, then the game mode's and the held tool's rules (adventure mode can't, a
  sword in creative can't, and outside creative a block with destroy time -1, such as bedrock, cannot be broken), the native mods' `break_block`, `player_destroyed_block` once the block is out (its `state` is read just before the removal, so a sheared tripwire reads disarmed where vanilla passes `Block.destroy` the armed state), the tool's wear and the drops. There is no digging time:
  a machine that digs over time counts its own progress and breaks the block when it is done.

Its items are its own inventory: set the stack in its hand first (through `ENTITY_ITEMS` or an `ItemPlace`), and
read what is left after. The answer says what happened:

| Answer | Meaning |
|---|---|
| `Ok(ActionOutcome::Consumed)` | Vanilla's `InteractionResult` consumed (a success or a consume), the blow landed, or the block broke |
| `Ok(ActionOutcome::Passed)` | Nothing used the click, a mod or the game stopped the blow, or the block stayed |
| `Ok(ActionOutcome::EffectBlocked(effect))` | The click ran, but what it would start was stopped: a menu or screen, sleeping, riding, or going to another dimension (`FakeEffect`). The click's own side effects remain: the swing seen by others, the hooks that run before the use, vanilla's `Success` |
| `Err(ActionRefused::…)` | The action is refused before it runs; see the reasons below. |

| Refusal | Meaning |
|---|---|
| `NotFake` | The handle belongs to a real player. |
| `Stale` | The handle is stale or the fake is removed. |
| `Busy` | Another player's turn, an open transaction or another `perform_action` is running. |
| `OutOfReach` | The target is out of reach, or the hit is not on the supplied block. |
| `Protected` | The target is outside build height or the world border, or under spawn protection. |
| `InvalidSetup` | The supplied eye position, rotation or entity hit is non-finite. |
| `InvalidTarget` | The entity is the fake itself or is not in its level. |
| `InvalidStack(StackWriteError)` | The supplied stack is rejected before player state is swapped or the action runs. |


## Scoped actions

For a machine using an isolated stack, `player.perform_action_scoped(host, action, setup)` composes that same
persistent player's action with temporary state. Build `PlayerActionSetup::new(stack, eye, yaw, pitch,
secondary_use)`: the fake acts in survival with a fresh standing body (eye height 1.62), that stack in its main
hand, and no other inventory or equipment. Its supplied rotation is used for item rays. Secondary use holds
semantic sneak without changing that standing eye height. No second player is created.

The `ScopedActionResult` contains `outcome` (the same `ActionOutcome`), `held` (the nonempty main-hand remainder)
and `other` (other nonempty inventory stacks, then offhand, carried stack and armor). These stacks are returned
to the caller, not kept by the fake. The fake's persistent body, inventory, game mode, look, secondary-use state
and action timers are restored even on refusal; world changes and the usual interaction/Parts hooks remain.
The ordinary fake safety rules apply: no screens, sleep, rides, dimension travel, online participation,
statistics, advancement credit or network connection. Neither action method can run inside another player turn
or an open transaction; the action's own ordinary write paths run transactionally.

## Secondary use

On either side, `player.entity().is_shift_key_down(host)` reads semantic secondary use rather than a physical
key mapping or a crouching pose. On the client obtain the reachable local handle with `host.local_player()`
(or `host.entity(id)`); it reads live secondary use even before the next movement tick updates the body's
shift flag. On the server the existing entity flag is read, including a scoped fake's setup. Stale handles and
clients without a joined local player return `None`.

What a machine does beyond a player's action stays in the mod: its own mining progress and catching world drops.
Scoped actions retain player reach, protection and border checks; eye placement is supplied by the mod.
