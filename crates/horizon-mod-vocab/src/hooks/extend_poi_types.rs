//! Extend vanilla point-of-interest types with mods' block states.
pub trait Hooks {
    /// Add block states to an existing POI type (NeoForge `ExtendPoiTypesEvent`): a mod's
    /// bed to `minecraft:home`, its workstation to `minecraft:armorer`. Asked once the block
    /// registry is resolved, before any world's POI tables are built, and again on a
    /// resource reload. Build each predicate on `event.extensions.metadata()` and hand it
    /// to `event.extensions.extend(poi_type, states)`; a block predicate with no
    /// constraints means all of the block's states. The engine resolves the additions
    /// and refuses, one logged warning each naming both types, an unknown POI type and a
    /// state vanilla or an earlier addition already gives to a different type; the rest
    /// of a mod's additions still apply. A state added to the type it already has is a
    /// no-op. A tag predicate is refused: additions name blocks.
    /// Wasm subscriptions refuse at load, naming the borrowed metadata capability;
    /// this hook is not silently skipped when its context cannot cross Wire.
    #[notes = r#"## Extend POI types

`extend_poi_types` runs once the block registry is resolved, before any world's point of
interest tables are built, and again when resources reload. It collects a `PoiTypeExtensions`
registrar: `metadata()` lends the resolved block metadata that `StatePredicate::block` needs,
and `extend(poi_type, states)` adds the states the predicate selects to the vanilla type named
by `poi_type` (`minecraft:home`, `minecraft:armorer`, `minecraft:beehive`, ...). A predicate
with no constraints adds all of the block's states; constraints narrow it (`part=head`).

Added states are saved, located and found by villagers exactly as vanilla's own: the save's
`poi` records, `/locate poi`, and a villager's search for a bed, bell or job site see one table.
Bees are the exception: they find a hive by its block entity and the `minecraft:beehives` tag,
not by its POI type, so a mod's hive also needs that tag.
The engine refuses, with one logged warning naming both types, an unknown POI type and a state
vanilla or an earlier mod already gives to a different type; the other additions still apply,
and adding a state to the type it already has changes nothing. Tag predicates are refused.
There are no new POI types: a mod extends vanilla's, as NeoForge's event does.

The extension table is process-wide, like the loaded mod set: re-collecting replaces
classification for every simultaneously live resource set. Villagers cache only vanilla
classification and consult the current extension table, just as saves and locate do.
Wasm subscriptions to this hook currently refuse at load with a reason naming
`PoiTypeExtensions::metadata`: its borrowed `BlockMetadata` accessor cannot yet cross
Wire. No wasm registration runs partially or silently omits metadata."#]
    #[hook(
        side = "server",
        contract = "collect",
        out = "extensions",
        context = "registration",
        neoforge = "ExtendPoiTypesEvent"
    )]
    fn extend_poi_types();
}
