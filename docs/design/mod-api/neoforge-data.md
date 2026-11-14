# NeoForge data files

This is a scoped audit of server data, not a claim that carrying a file into a
package executes it. The paths and examples below identify the readers and
fixtures checked; the modifier gaps are tracked separately.

| Data | Result and reader | Example or evidence |
| --- | --- | --- |
| Convention item tags | Loaded for content roots: `mod_content::build` in `crates/horizon-client/src/mod_content.rs` merges tag files; `content_namespaces::tag_namespaces` in `crates/horizon-items/src/content_namespaces.rs` includes `c` for content roots, and `items::load_tag_lists` in `crates/horizon-items/src/items.rs` reads and resolves them. Unknown item entries are filtered by `Known::keeps` in `mod_content.rs`. | `data/c/tags/item/gems/glint.json`: `{"values":["minecraft:diamond"]}`. The `glint_mod` fixture and `a_data_mod_loads_unloads_and_loads_again` test in `crates/horizon-client/src/mod_content_tests.rs` check the resulting `c:gems/glint` membership. |
| Structure modifiers | Not loaded by any reader found today. `mod_content::build` copies ordinary files to its overlay; `worldgen_entry` in that file recognizes worldgen bundles, not structure modifiers. A workspace search for `structure_modifier` found no reader. Copying is not applying. | No verified in-tree example. [#1269](https://github.com/arismoko/horizon/issues/1269) tracks loading and applying them. |
| Global loot modifiers | Not loaded by any reader found today. `mod_content::build` copies ordinary files to its overlay, but a workspace search for `loot_modifiers` found no loader. The `native_loot_drops` code hook in `crates/horizon-mod-vocab/src/hooks/base.rs` is not a data-file reader. | No verified current-format in-tree example. [#1270](https://github.com/arismoko/horizon/issues/1270) tracks loading and applying them. The old `global.json` in `carry::tests::claims_the_namespaces_its_files_name` is a carry classification fixture, not proof of current-format loading. |
| Tag `remove` lists | Loaded wherever tag files are read: a file's `remove` entries (ids, `#tags`, `{"id": ...}` objects) follow its `values`, and every entry applies in pack order, as NeoForge 26.3's `TagLoader` does (`values::add` or `values::remove` on the tag's ordered set). A later pack can add an entry back after a lower one removed it; a `replace` file drops lower files' removals too; a removal never fails a tag, so an unknown id or tag removes nothing; a file whose `neoforge:conditions` fail removes nothing. The fold is `TagLayers` in `crates/horizon-world/src/tag_layers.rs`, read by `load_tag_lists` (`crates/horizon-items/src/items.rs`), the loot tags (`crates/horizon-world/src/loot/tags.rs`) and the worldgen loader's `TagDefinitions` (`crates/horizon-world/src/worldgen/tags.rs`). Content mods' tag files carry their `remove` entries into the merged file (`merge_tag` in `crates/horizon-client/src/mod_content/load_conditions.rs`), after its values, and biome and carver modifiers' tag selectors apply them. | `data/test/tags/item/things.json`: `{"values":[],"remove":["minecraft:dirt","#c:ores/gold"]}`. `a_higher_packs_remove_takes_entries_out_of_a_tag` in `items.rs`, `a_removal_takes_out_an_entry_or_a_nested_tags_members` in `loot/tags.rs`, `a_higher_packs_remove_takes_entries_and_tag_members_out` in `worldgen/tags.rs`, `a_pack_tags_remove_list_takes_carvers_out` in `mod_content_carvers.rs`. |
| Title screen splashes | Loaded from the resource stack: `splashes::pool` in `crates/horizon-client/src/splashes.rs` reads every namespace's top `texts/splashes.txt` (vanilla, content mods and enabled resource packs), joined in namespace order with duplicates kept, as NeoForge's `NeoForgeSplashHooks.loadSplashes`, on each resource reload. Lines are trimmed plain text, empty lines are kept and the hidden line is dropped. When a resource pack or a mod provides `minecraft:texts/splashes.txt`, only that file is read while `skipModdedSplashesIfVanillaModified` (in `horizon-extras.txt`, on by default) is on. | `assets/<modid>/texts/splashes.txt`, one splash per line. `a_content_mods_lines_follow_vanillas` and `a_packs_own_minecraft_splashes_replace_the_pool` in `splashes.rs`. |
| Built-in mob, villager and game event data maps | Six maps load through `neoforge_mob_maps::load` during server table preparation, then publish canonical common-store NBT and one immutable world-owned snapshot. Gameplay readers borrow it; each structure finalization pins one Arc, and a composter work batch pins once across callbacks. `acceptable_villager_distances`, `parrot_imitations`, `villager_types`, `raid_hero_gifts`, `vibration_frequencies` and `villager_compostables` use actual registered identities, registry-specific tags and atomic file decoding. No process-global current world and no client network sync. Villager types apply at finalizeSpawn only; bred villagers retain the biome-facts rule. Monster-room data remains on [#3360](https://github.com/arismoko/horizon/issues/3360); remaining query coverage is deferred to [#4199](https://github.com/arismoko/horizon/issues/4199). | `data/neoforge/data_maps/entity_type/acceptable_villager_distances.json`: `{"values":{"test:wraith":{"acceptable_villager_distance":8}}}`. Regressions: `lexical_numeric_files_keep_infinity_and_exact_decimal_truncation_in_both_views`, `hostile_sensor_matches_registered_ids_and_world_snapshot`, `structure_maps_reload_without_cross_world_bleed`, and `vanilla_shoulder_rng_checkpoint_without_data_files`. |

The shared load-condition evaluator in `crates/horizon-world/src/conditions.rs`
applies to recipes, loot tables, item modifiers, predicates, enchantments,
jukebox songs, instruments, damage types, advancements, synchronized registry
JSON, world-table tags (including the direct loot and damage readers,
synchronized registry tags and function tags), and native item/block data maps.
The [conditional-data contract](../../modding/src/how-to/items/conditional-data.md)
describes these file types and their reload lifetimes.

Ordinary JSON resources select the winning pack file before evaluating its
conditions. A false condition omits that winner without exposing a lower pack's
file. [Registered conditions](../../modding/src/how-to/items/load-conditions.md) can also refuse
reload on bad arguments or unknown names in an owned namespace. Tag resources and data maps evaluate each layer before
merge, `replace`, or `remove`; an omitted layer cannot clear lower entries.
Content-root composition follows the same tag rule, including block/biome tag
bundles. The damage registry remains frozen after world open; reloads reevaluate
its tags. Generated item default components retain their prototype lifetime.

Supported forms are `neoforge:mod_loaded`, `neoforge:true`, `neoforge:not`
(`value`), and `neoforge:and` / `neoforge:or` (`values`), inside a
`neoforge:conditions` array. Every condition must hold. An unknown kind in a
namespace where a loaded mod registered conditions refuses reload, naming the
file, condition and owner. Other unknown kinds omit the file with one named
error-level log entry, including beneath `not` or `or`. Files without the array load normally.

`mod_loaded` reads an immutable snapshot of the owning generation's server-enabled
mod IDs, including code-only native and WebAssembly mods, plus `minecraft`.
Client-only, disabled and refused packages are absent. Namespaces do not imply a
loaded mod. Changed enabled IDs participate in cache identity and are published
with successful reloads; failed preparation preserves the old generation, and
retained readers keep their old snapshot. Recipe condition behavior is unchanged.
This shared contract does not add readers for the two modifier kinds above.

## Holder sets

The supported fields vanilla types as a `HolderSet` use one decoder,
`horizon_world::holder_set::resolve`. It takes an id, a `#tag`, a list of ids,
or NeoForge's `neoforge:any`, `neoforge:and` / `neoforge:or` (`values`) and
`neoforge:not` (`value`) objects (`AnyHolderSet`, `AndHolderSet`,
`OrHolderSet`, `NotHolderSet`), nested freely. `or` keeps its values' order,
`and` its first value's, and `any` / `not` the registry's. The readers that use it:

- feature block, fluid and biome predicates;
- structure processors (`protected_blocks`, the rule block sets);
- structures' `biomes` and the monument, mineshaft and jigsaw biome fields;
- checkerboard biome sources;
- loot holder sets (items, enchantments, instruments, biomes);
- enchantments' `supported_items`, `primary_items` and `exclusive_set`;
- biome and structure modifier selectors.

A synced enchantment that writes one of these objects reaches clients as the
resolved id list, so vanilla clients decode it. Mod-defined holder-set types
(`HolderSetType`, `ICustomHolderSet`) are not read yet; they wait on the typed
codec foundation ([#811](https://github.com/arismoko/horizon/issues/811)). The
recipe book's network holder sets carry only the vanilla forms.

Objects bind once during the owning load, before publication. Enchantment
item sets use the complete default-component item snapshot (including mod items)
and ordered tags; exclusive sets use the complete enchantment registry. The
same binding supplies synchronized enchantment JSON, so client and server agree
on membership and order. Loot objects bind in known engine-owned fields, including
biome predicates and instrument options; arbitrary component and mod payload JSON
is not rewritten. Invalid objects refuse the load with the definition and field
in the error, rather than becoming an empty set at roll time. Native table-load
replacements are validated before replacing the usable table snapshot.

`any` and `not` require the owning registry universe. An unavailable universe is
a named error, not an empty registry. Plain strings, lists and tags retain their
previous reader semantics, including the structure-biome and processor-list
readers' lenient handling of non-string list members.
