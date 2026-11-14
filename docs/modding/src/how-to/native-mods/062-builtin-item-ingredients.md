<!-- chapter: Built-in item ingredients -->

# Built-in item ingredients

Item ingredients accept item ids, lists and `#item_tag` references. Ingredient objects
read `neoforge:ingredient_type` first, then `type`. The built-in NeoForge kinds are
`compound` (any child), `intersection` (every child), `difference` (base without
subtracted), `components`, `block_tag` and `custom_display`. These built-ins never
call a mod's `ingredient` hook. Holder-set objects are a separate format and are
not accepted here. These object forms are read from recipe resources. The
`Host::recipe_ingredient` convenience reader accepts flat item inputs.

```json
{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:damage":5},"strict":false}
```

`components.items` accepts an id, list or `#item_tag`. Every component in the patch
must equal the concrete stack's effective value, including item defaults. A `!`
prefix requires absence: `{"!minecraft:enchantments":{}}`. `strict` defaults to
false; when true, every effective component must also be named in the patch.
For example, a diamond sword's default `minecraft:max_damage` counts. Count does
not participate; empty stacks never match.

```json
{"type":"neoforge:block_tag","tag":"minecraft:logs"}
{"type":"neoforge:custom_display","base":"minecraft:apple","display":{"type":"minecraft:item","item":"minecraft:diamond"}}
```

`block_tag.tag` names a block tag without `#`, matching its blocks' items.
`custom_display` matches its base and uses the slot display only for presentation.
As with packet slot displays, potion, dye and trim demo displays show their source
stacks. Matching, placement and network ingredient membership retain the base's
item list.

Plain item lists, item tags and block tags are simple. Components and mod kinds
are not simple. Compound, intersection and difference are simple only when every
child is simple; custom display follows its base. Shapeless recipes use item lists
when all ingredients are simple and otherwise test each concrete stack. Shaped
recipes always test the concrete stack. Recipe-book craftability, placement,
recipe property sets and network ingredients continue to use item lists.

## Component decoder support

Every component id must be registered, including removal keys. A payload is
rejected when its available decoder rejects it, and the whole engine-owned
recipe is omitted. Mod-owned recipe types retain their own validation.

Registered components without a shared decoder **load**. Raw `other` values
compare by exact stored NBT equality. Modelled values, including potion, bundle,
container and trim contents, compare against their complete retained fields.
All preparation happens at load; candidate comparisons borrow values without
allocating or converting them to JSON. Removal keys require registration only.
Native registrations retain their persistent NBT validation contract.

The shared decoder currently covers 64 registered built-ins:
`cat/sound_variant`, `cat/variant`, `wolf/sound_variant`, `wolf/variant`, `container_loot`, `tooltip_display`, `tropical_fish/pattern_color`, `tropical_fish/base_color`, `cushion/color`, `recipes`, `mob_visibility`, `custom_model_data`, `wolf/collar`, `cat/collar`, `shulker/color`, `sheep/color`, `dye`, `block_state`, `use_cooldown`, `cooking_fuel`, `brewing_fuel`, `compostable`, `fox/variant`, `weapon`, `food`, `damage`, `repair_cost`, `max_damage`, `max_stack_size`, `map_id`, `rarity`, `enchantments`, `stored_enchantments`, `enchantment_glint_override`, `custom_name`, `item_name`, `item_model`, `lodestone_tracker`, `damage_resistant`, `use_effects`, `block_entity_data`, `enchantable`, `ominous_bottle_amplifier`, `potion_duration_scale`, `minimum_attack_charge`, `dyed_color`, `attack_animation`, `attack_range`, `rabbit/variant`, `horse/variant`, `llama/variant`, `mooshroom/variant`, `axolotl/variant`, `salmon/size`, `parrot/variant`, `tropical_fish/pattern`, `villager_food`, `unbreakable`, `base_color`, `note_block_sound`, `tooltip_style`, `waxed`, `intangible_projectile`, `glider`,
in the `minecraft` namespace.
`enchantable` requires a positive integer `value`; extra record fields are ignored
and equivalent numeric NBT widths match, including item defaults. Map ids accept
signed numbers, including negative ids, using Java's integer conversion.
The amplifier accepts numeric forms converted to an integer in 0..=4;
out-of-range payloads are rejected, never clamped.
Potion duration scales decode numeric payloads to floats; zero and finite positive
values are accepted, but negative-zero floating tags, negatives, infinities, NaN,
and nonnumeric values are rejected. JSON's integral `-0.0` becomes zero before
decoding. Matching compares decoded float values, including item defaults,
without copying candidate payloads.
`minimum_attack_charge` converts numbers to 32-bit floats before checking the
inclusive range 0 to 1; decoded negative zero, NaN and infinities are rejected.
JSON-to-NBT conversion first canonicalizes literal `-0.0` to integer zero.
Equivalent numeric representations match live values and item defaults.
`item_model` accepts valid resource identifiers without requiring an existing model;
bare names and an empty namespace normalize to `minecraft`.
`enchantment_glint_override` accepts booleans and all numeric NBT widths:
zero (including negative zero) is false; nonzero values, fractions and NaN are
true. Displays retain canonical byte 0 or 1.
`dyed_color` accepts packed integer colors or three numeric RGB channels,
matching the decoded values without copying candidate payloads.
The other 55 persistent registered built-ins use stored-value matching while their shared
decoders are added incrementally in **#3875**. Their absence is not a load refusal.
The transient types `creative_slot_lock`, `additional_trade_cost` and
`map_post_processing` are refused as stored component values, not matched as opaque NBT.
They may still be named by `tooltip_display.hidden_components`.

`dye` accepts exactly the 16 lowercase vanilla color names (including `light_blue`
and `light_gray`). Unknown names, differently cased names and non-string values
reject the recipe. Numeric dye ordinals belong to compressed dynamic operations,
not ordinary persistent NBT; compressing an NBT file does not change this rule.

`mooshroom/variant` accepts exactly `red` and `brown`; `axolotl/variant` accepts
`lucy`, `wild`, `gold`, `cyan`, and `blue`; `salmon/size` accepts `small`,
`medium`, and `large`. Numeric network ids, case variants and namespaced values
are refused. Mob defaults are not item-component defaults.

Component ingredient displays contain the patch, including removals. Difference
lists retain the base list and remove only items whose default stack passes the
subtracted predicate. Non-simple cooking ingredients are tested outside the
simple item index, in original recipe priority order.

A non-simple ingredient may have an empty default-stack display list and still
load. It retains its full candidate set for concrete matching and cooking.
For example, a diamond sword minus swords with damage 0 has no default display,
but a damage-5 sword satisfies it. Empty display lists do not erase candidates.

The decoded records also include `food`, `weapon`, `fox/variant`, `compostable`,
`brewing_fuel`, `cooking_fuel`, `use_cooldown` and `block_state`. Fuel records
require both fields without defaults or bounds. Cooldown requires positive finite
seconds; its optional group is a valid identifier. Compostable requires layers.
Block state is a string-to-string property map. Optional JSON record fields treat
null as absent; required fields still refuse null. Weapon seconds refuse negative
zero before JSON-to-NBT conversion.

Additional decoded values include sheep and shulker colors, cat and wolf collar
colors (the same strict dye palette), ordered custom model data lists, mob
visibility holder sets against active entity data, and ordered recipe keys.
Custom model data lists default to empty, including JSON null optional fields.
Recipe keys need no registry lookup; order and duplicates are retained.

The pinned 26.3 registry has 122 component types: 119 persistent and 3 transient.
mod-registered cat/wolf variants are not matched yet.

`attack_animation` accepts a record with optional `type` (`none`, `whack`,
`stab`; default `whack`) and nonnegative integer `duration` (default 6).
Optional JSON null fields use their defaults; the whole payload must be a map.
JSON booleans become numeric byte flags before decoding, including durations.
Very large JSON doubles saturate to an integer where vanilla JSON operations
wrap; this existing conversion difference is not tightened here.
`damage_resistant` requires `types`: a damage type id, ordered list of ids
(including duplicates and the empty list), or a named `#tag` from the active
frozen damage registry. Missing or null `types` is refused. Named sets compare
by tag identity, never equal to direct sets with the same contents.
`use_effects` has optional `can_sprint` (false), `interact_vibrations` (true),
and `speed_multiplier` (0.2, range 0 to 1). `attack_range` has optional
`min_reach` (0), `max_reach` (3), `min_creative_reach` (0),
`max_creative_reach` (5), `hitbox_margin` (0.3), and `mob_factor` (1).
The four reach fields range from 0 to 64, margin from 0 to 1, and mob factor
from 0 to 2, inclusive. No minimum/maximum ordering check is imposed.
These are codec defaults, not effective reach when the component is absent:
that uses entity attributes. Optional null fields use defaults. Decoded negative
zero, NaN and infinities are refused for bounded floats. JSON numbers are also
accepted for boolean fields by the common NBT boundary.
The scalar enum decoders accept exact persistent names only: rabbit uses
`brown`, `white`, `black`, `white_splotched`, `gold`, `salt`, `evil`;
horse uses `white`, `creamy`, `chestnut`, `brown`, `black`, `gray`, `dark_brown`;
llama uses `creamy`, `white`, `brown`, `gray`.
Numeric ordinals, namespaced names, case variants and null are refused.

The `fox/variant` persistent codec accepts only the exact strings `"red"` and
`"snow"`; `parrot/variant` accepts `"red_blue"`, `"blue"`, `"green"`,
`"yellow_blue"`, and `"gray"`. Both reject numeric network ids, case variants
and namespaced values. Parrot spawn eggs have no prototype `parrot/variant`;
the mob's default variant does not supply an item component.

The `tropical_fish/pattern` codec accepts only `"kob"`,
`"sunstreak"`, `"snooper"`, `"dasher"`, `"brinely"`, `"spotty"`, `"flopper"`,
`"stripey"`, `"glitter"`, `"blockfish"`, `"betty"`, and `"clayfish"`. It rejects
numeric network ids, case variants and namespaced values. Tropical fish buckets
have no pattern component by default; the mob's default pattern is not an item default.

`villager_food` requires a compound with positive integer `nutrition`: numeric
NBT uses Java's 32-bit integer conversion before the positive check, and unknown
fields do not participate in equality. Its recipe and item-default values are
prepared at load; live candidates are borrowed without allocation.

`unbreakable` requires an object, ignores its fields and normalizes to `{}`;
scalars and lists are rejected. Matching requires effective presence, including
item defaults, while an explicit removal suppresses that default.

`base_color` accepts exactly the 16 lowercase dye names (for example `red` and
`light_blue`), not numeric dye ids or uppercase names. It compares the shield's
stored base color; a banner's item color does not imply this component, and an
undyed shield has no base color.

`note_block_sound` accepts an identifier string, not a sound-registry entry.
Omitted or empty namespaces become `minecraft`; empty paths are allowed, so
`""`, `":"`, and `"minecraft:"` decode to the same identifier. Namespace and
path characters follow vanilla's lowercase ASCII identifier rules (including
the forbidden namespace `..`); malformed UTF-16 strings are rejected.

Component ingredient displays contain the patch, including removals. Difference
lists retain the base list and remove only items whose default stack passes the
subtracted predicate. Non-simple cooking ingredients are tested outside the
simple item index, in original recipe priority order.

A non-simple ingredient may have an empty default-stack display list and still
load. It retains its full candidate set for concrete matching and cooking.
For example, a diamond sword minus swords with damage 0 has no default display,
but a damage-5 sword satisfies it. Empty display lists do not erase candidates.

`tooltip_style` accepts identifier strings without looking up a resource; omitted
or empty namespaces become `minecraft`, and empty paths are valid.

`waxed` requires an object (not a scalar or list); its fields are ignored and it
decodes to an empty object. Presence still matters: an absent or explicitly removed
`waxed` does not equal a present empty object.

`intangible_projectile` requires an object, ignores its fields, and normalizes to
`{}`; scalars and lists are invalid. Presence still matters, and
`!minecraft:intangible_projectile` requires absence, including item defaults.

`minecraft:glider` requires an object (`{}`), not a scalar or list. Its unit codec
ignores all object fields, so nonempty objects normalize to `{}` and compare equal.
The component must still be present, including through an item default; an explicit
removal overrides that default. `!minecraft:glider` requires absence instead.
