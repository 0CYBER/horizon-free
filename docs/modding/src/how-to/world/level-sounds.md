# Level sounds

Implement `play_level_sound_at_position` or `play_level_sound_at_entity` to edit
or cancel a level sound before it is broadcast (server) or played (client).
Match the hook's `HostSide` to distinguish the requests; the event's sound,
source, volume and pitch describe that side's request, and original volume and
pitch remain available after edits. Returning `Flow::Cancel` suppresses that
request. A server cancellation sends no packet, so there is no client request.

Incoming sound packets, including `/playsound`, ask the client level hooks.
An entity-sound packet stays bound to its emitter. UI clicks, engine-local
sounds and `ClientHost::play_local_sound` do not ask these level hooks; the
sound engine's `play_sound` hook remains a separate, later interception point.

Client-predicted level sounds and `LocalPlayer.playSound` use those same hooks.
Canceling their sound does not cancel the player action or undo a predicted
block change. A local player's ordinary sound is an `AtEntity` request but
plays at the requested position; only a genuinely entity-bound sound
(such as a goat horn) follows its emitter after playback starts.

| Client route | Level event | Vanilla call being represented |
| --- | --- | --- |
| Placement confirmation, predicted doors/buttons/levers | AtPosition | `BlockItem.place`, block `useWithoutItem` → `Level.playSound` |
| Waxing, tool transforms, bucket use | AtPosition | `HoneycombItem.useOn`, tool `useOn`, `BucketItem` → `Level.playSound` |
| Game-event arrow hit (Players), pufferfish sting (Neutral), elder-guardian curse (Hostile) | AtPosition | `ClientPacketListener.handleGameEvent` → `Level.playSound` |
| Own steps, swimming, splash, amethyst chime, honey slide, fall | AtEntity | `Entity`/`LivingEntity` → virtual `LocalPlayer.playSound` |
| Own hurt/death, bundle menu clicks, consume effects | AtEntity | `LivingEntity`, `BundleItem`, `Consumable` → `LocalPlayer.playSound` |
| Player-bound sounds, including the local goat horn | AtEntity | `Level.playSound(except, entity, ...)` |
| Block-breaking/level-event effects, explosions, pickup packets | None | Client-level `playLocalSound` or direct sound manager playback |
| Dragon/fang/sniffer local effects, ticking ambience, UI | None | Entity-local effects, sound instances or widgets, not `playSeededSound` |

## Engine playback and live parameters

`play_sound` also observes transient playback updates. `event.starting` is true for a
new sound or loop replay and false for a live playback update. `event.instance` identifies the same engine-local
instance across those calls. At an update only volume and pitch edits apply;
`None` stops playback. Invalid edited volume (negative or non-finite) or pitch
(non-positive or non-finite) refuses that refresh. Updates do not replay the sound,
resolve a different source, or modify the instance's stored settings.

## Sounds of viewed levels

A client mod that shows another level, as Immersive Portals shows a portal's far side, hears it
through `viewed_level_sound`, a client-only Horizon hook with no NeoForge counterpart. The server
sends a level's sounds to a client that views it, from `horizon:levels` revision 3: only to one
whose chunks of that level hold the sound's chunk, and whose area there has its centre chunk within
the sound's broadcast distance plus 16 blocks. The engine plays none of them by itself. The hook
gets the viewed level's handle, the sound, its source, its position in that level, its volume,
pitch and seed, and `play_at`, which starts as `None`:

```rust,ignore
fn viewed_level_sound(&self, _: &mut dyn ClientHost, e: &mut ViewedLevelSoundEvent) {
    // Through the nearest portal: where it opens in the player's level, and quieter with distance.
    if let Some((opening, distance)) = self.portal_towards(e.level, e.position) {
        *e.play_at = Some(opening);
        e.volume = (e.volume - distance as f32 / 16.0).max(0.0);
    }
}
```

Left `None`, the sound is not played. Set, it plays at that position in the player's level as a
sound packet of that level plays (no distance delay), with the volume and pitch the listeners left
and the server's seed. A `play_at` that is not finite, or a volume or pitch that is not finite and at
least 0, plays nothing. Which portal and how loud are the mod's to decide. Sounds of the player's own
level never ask this hook. A sound bound to an entity of a viewed level comes with that entity's
position there, is sent only while the client tracks the entity there, and plays at `play_at`
without following it. With no listener a viewed sound costs one check.
