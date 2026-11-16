# Owned item values

`ItemStack` is the mod API's own nonempty value, not an engine inventory slot or a borrowed
rendering view. It contains an `ItemKey`, a nonzero count, an optional numeric damage override
and a lazily allocated component patch. Plain stacks allocate no component storage. An empty
slot or a consumed stack is `None`; it is never a zero-count `ItemStack`.

Every loaded item, vanilla or modded, has an `ItemKey`. Resolve identifiers through
`Host::item_key` once when loading data and retain the key. Forward references requested during
registration keep their slots when the load completes; unresolved references refuse the load.
Keys are process-local identities, not persistent names. Tags likewise have retained `TagKey`
identities, while their item and fluid membership is read through `Host::tags` and changes on
reload. Unknown tags are empty; unavailable authoritative mappings return `None`.

## Stored values and effective components

Count and component edits do not consult the game. `new` refuses zero; `set_count` and `grow`
refuse invalid or overflowing counts without changing the original value. Consuming `split`
and `shrink` return optional portions so consuming the last item needs no empty sentinel.

The damage representation has three states:

| Stored state | Effective meaning |
| --- | --- |
| No numeric override and no removal | Inherit the item's prototype, including its default damage of zero when present. A new plain sword needs no `Components` allocation. |
| `Some(0)` | An explicitly present zero-damage component, even on an item without a damage default. `set_damage(0)` creates this state without allocating a patch. |
| No numeric override and `!minecraft:damage` in the patch | The component is explicitly removed, including an inherited default. `clear_damage` creates this state. |

`damage()` returns the stored number or zero. `damage_component()` distinguishes an explicit
zero from no numeric override. `set_damage(u32)` is infallible; the game validates its effective
maximum when accepting a write. Adding only a maximum-damage override to an item does not
invent a damage component: explicitly adding `Some(0)` is what makes that component present.

All numeric damage lives in the optional field. Raw damage entries normalize out of the
component patch; malformed numbers, conflicting numeric values, or a simultaneous value and
removal refuse the entire replacement. Generic component setters and raw `set_components`
therefore return `Result`. A refusal leaves the stack unchanged. Typed damage setters do not
need an error path.

`apply_components(&patch)` merges over the existing patch rather than replacing it.
Incoming values clear prior removals and incoming `!id` removals clear prior values;
a removal wins if the same patch includes both, except that simultaneous damage and
damage removal are refused. Damage values replace previous numeric overrides and are
normalized just like `set_components`; malformed damage leaves the entire stack unchanged.
`copy_component_from(&other, id)` copies only that component's own value or removal,
including explicit zero damage. If `other` has no override, the target inherits too:
no effective prototype value is materialized. `get` followed by `set` covers `update`.
These owned-value operations require no host. To persist them on a whole stack, use
server-side `ItemPlace::edit` inside a transaction; it preserves item/count and rolls
back with its parent transaction instead of splitting one item off.

`ItemResource::with_component` is also fallible for raw component values. Its default-value
elision and `without_component` preserve the normal prototype-plus-patch semantics. The latter
records real NBT removal entries for default components; it is not an unknown-value fallback.

## Game boundaries

A consume or death-protection override whose opaque native effect payload cannot be decoded
refuses the whole stack write with `StackWriteError::InvalidEffectPayload`, before any
resident state changes.
The items host area owns metadata reads such as `max_stack_size(ItemKey)` and
`max_damage(ItemKey)`. A stack does not take a host just to read a maximum, and an unavailable
maximum never turns into a fabricated limit of 64.

The game validates effective count and durability limits, including component overrides, when
accepting a resident write. An effective `max_stack_size` must be in `1..=99`, and the
presence of `max_damage` forbids a stack limit above one, even when the current count is one.
Removing `max_stack_size` uses the absent limit of one. Invalid values produce typed errors,
not clamped values. A transfer request is different: `ItemResource` plus a separate amount can request or
return more than one resident stack's capacity. The resident writes it produces must
pass validation.

Game inventory, menus, trades and models keep their engine-owned representations. Native
hooks and host methods exchange owned API snapshots. The game converts only after a listener
guard, and stages every mutable result before committing any captured output. Invalid hook
edits retain those captured engine values. Consuming mutable events such as `bonemeal` and
`item_after_vertical_collision` lend `Option<ItemStack>` so a listener can consume the value by
setting `None`.

The `item_stacked_on_other` bridge commits its clicked slot, cursor and cancellation flags as
one unit, including the menu's slot-operation queues and map-id reservations. The block-entity
tick, menu-button and menu-quick-move bridges validate their captured snapshot outputs together
before applying them. These are output guarantees, not whole-world transactions: independent
`ServerHost` storage writes, commands, messages and other effects are not rolled back on a
refused output. Carrier-form callbacks such as `bonemeal` and `item_after_vertical_collision`
also retain their captured stack outputs on refusal but do not journal independent host writes.

Item-model callbacks receive the owned stack and a separate `bundle_selected` number (`-1`
means none). Runtime selection is not a persistent component. Drawing receives the API-owned
`ItemDisplayContext`, not the renderer's enum.

## The NBT escape

`ItemStack::from_nbt(host, compound)` and `to_nbt(host)` are the NBT escape. They live on the
plain API value, not as a second engine implementation in the items host area. They use only
typed host key lookup and canonical id metadata, so native and WebAssembly mods execute the
same parsing and serialization code. Persistent NBT contains identifiers, never process-local
key slots. Unknown ids, malformed values and zero counts fail decoding; unavailable id
metadata fails encoding.

The escape preserves explicit zero damage, default-component removals and opaque component
NBT. It does not apply resident maxima or materialize prototype defaults. Those remain the
responsibility of the metadata service and game write boundary respectively.
