# Observing advancement progress

Subscribe to `player_advancement_progress` to inspect the whole advancement immediately after
one criterion is granted or revoked, before its completion rewards. `event.progress.criteria`
includes obtained and unobtained criteria; `obtained_at` is the original saved grant date or
`None`. `requirements` lists the OR groups, `completed_requirements` counts satisfied groups,
and `done` is the game's completion result. A player-issued `/reload` mutates and
snapshots the acting player's live progress, including during a nested turn.

Keep `event.progress.clone()` if it is needed later: the owned `AdvancementProgress` snapshot
cannot change the player's progress and is not updated by subsequent events. No-op grants and
revokes do not fire. The snapshot uses the same plain-data bindings in native and wasm mods.

## Display icons

An advancement's `display.icon` is an item stack, including its count, damage and
supported component patch. The toast, advancement widgets, tabs and hover draw that stack;
LAN sends it through the same item-stack conversion as inventory synchronization, including
explicit component removals resolved through the peer's component registry.
Remote clients convert it once when receiving the advancement, not each frame.

If a remote icon fails the custom-component permission check (including malformed
nested templates), the advancement remains visible with the same item and count.
The **entire component patch** is discarded: every addition, including damage and
glint overrides; every removal; and cached wire bytes. Item defaults apply instead.

### Trying the icon surfaces

The runnable datapack `crates/horizon-client/tests/fixtures/advancement-icons`
contains two root advancements with wooden-pickaxe icons, count 3 and damage 7:
`horizon_test:glint` explicitly enables glint; `horizon_test:plain` disables it.
Its titles and descriptions use vanilla translations.

Copy the fixture directory into a **fresh test world's** `datapacks/` directory,
then run `/reload`. With cheats/operator permission:

```text
/advancement grant @s only horizon_test:plain
/advancement grant @s only horizon_test:glint
```

Watch the two toasts: only the second pickaxe shimmers, both show count 3 and a
partially depleted durability bar. Press **L** to open Advancements, select the
fixture's glint tab, then hover its pickaxe widget. The tab, widget and hover must
all shimmer and retain the count and damage bar; the plain tab must not shimmer.
Repeat on a second Horizon client connected to the test world's LAN server.
To replay the toast, revoke that advancement and grant it again:

```text
/advancement revoke @s only horizon_test:glint
/advancement grant @s only horizon_test:glint
```

The native regression
`game::advancements::icons::tests::advancement_icons_reach_local_and_remote_ui_surfaces`
checks actual GUI draw submissions at 640×480 with GUI scale 1, including the
two permission-failure cases in isolated child processes. The resource-free
`lan::advancements::tests::advancement_icons_roundtrip_count_damage_and_opposite_glint_overrides`
checks the LAN packet, remote stack conversion and byte differences when only
the same icon's glint override changes.
