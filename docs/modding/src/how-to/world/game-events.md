# Emitting game events with context

Use `LevelRef::game_event(host, event, at, context)` to emit a vanilla game event
at an explicit position. For example, `event` can be `"minecraft:block_change"`
and `at` is an `[f64; 3]` position. This is a server operation; vibration listeners
such as sculk sensors and wardens hear the event through the normal game path.

`GameEventContext` mirrors vanilla's `GameEvent.Context`:

- `source_entity: Option<EntityRef>` identifies the responsible entity. Its current
  spectator, sneaking, vibration-dampening and other listener facts are captured
  when the event is emitted.
- `affected_state: Option<BlockState>` identifies the state affected by the action,
  not necessarily the state currently at the position. Wool, for example, dampens
  the vibration when supplied as the affected state.

Use `GameEventContext::default()` when neither field applies. Both fields are available to native and wasm mods. An unknown event, invalid position,
stale or foreign source, invalid state or inactive level returns `false` without
emitting an event. A source must belong to the level in place.

`EntityRef::game_event(host, event)` is the convenience operation for an event at
the entity's position with that entity as source and no affected state. It calls
the same level operation; use the level form when the position or state differs.
