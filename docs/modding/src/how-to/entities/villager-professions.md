# Villager professions and trades

Declare custom professions once per mod load through
`Hooks::register_villager_professions`. The callback receives a
`RegistrationContext` and a `RegisterVillagerProfessionsEvent`; call
`event.professions.register(definition)` on its `VillagerProfessions` registrar.
The engine records the declaring mod as owner. World mutation is unavailable
at registration, and datapack reload resolves the declarations again without
rerunning the callback. Native and wasm mods use the same declaration API.

## Definitions and resources

`ProfessionDefinition` contains:

- A `ProfessionKey`, wrapping a qualified `Identifier` such as `example:engineer`.
- `workstations`: `ProfessionWorkstation` selectors, each with a `PoiKey`, block
  identifier and required block-state properties. Omitted properties match all
  values.
- `held_poi` and `acquirable_poi`: `PoiKey` lists for retaining and acquiring a job.
- `requested_items`, `secondary_blocks` and an optional `work_sound`, using
  resource identifiers. Requested items do not grant farmer AI.
- `trades`: explicit level-to-`TradeSetKey` pairs for levels 1–5. A missing level
  adds no offers; an empty map is legal.

`PoiKey` and `TradeSetKey` also wrap `Identifier`. A trade-set key is independent
of the profession key: no namespace or level suffix is inferred. Provide trade
sets and offers through the `trade_set` and `villager_trade` datapack
registries; trade JSON alone does not register a profession.

## Registration refusals

References resolve against the complete candidate tables before publication.
`ProfessionRegistrationError` describes refusals:

| Variant | Reason |
|---|---|
| `DuplicateProfession` | Multiple declarations use one profession key; owners are reported in sorted order. |
| `ReservedProfession` | A profession key uses the reserved `minecraft:` namespace. |
| `OverlappingPoi` | Two professions acquire jobs from the same custom POI. |
| `UnacquirablePoi` | A declared custom workstation has no acquiring profession. |
| `InvalidWorkstation` | A workstation selector is invalid or conflicts with another POI. |
| `UnknownPoi`, `UnknownTradeSet`, `UnknownItem` | A referenced POI, trade set or item is missing. |
| `UnknownBlock`, `UnknownSound` | A secondary block or work sound is missing. |
| `InvalidTradeLevel` | A level is outside 1–5 or appears more than once. |

Custom workstation POI identities must also stay outside `minecraft:`. Missing
workstation blocks and selectors matching no valid states refuse preparation.
A failed candidate publishes nothing; a failed reload keeps the active tables.

## Identity, ordering and multiplayer

Profession and custom POI table order depends only on registered qualified IDs,
not mod load order or declaring owner. Vanilla's 15 professions retain the first
15 profession slots; custom professions follow in sorted order. Numeric slots
are runtime lookups, not save identities. Saves retain qualified profession and
POI names. A missing profession remains unresolved without discarding its name
or existing offers, so restoring the mod can resolve it again.

Clients build the same deterministic identity table from their installed mods and
use it for villager rendering. Install the profession mod on both the server and
its clients, with the mod declared for both sides. The mod-list handshake refuses
missing or mismatched both-side mods; client-only mods do not take part in that
comparison. Missing professions are not replaced with vanilla
identities.

## Reading a villager on the client

A server mod narrows an entity with `host.merchant(entity)`; on a client that
answers `None`. A client mod narrows an entity the server shows the window with
`host.client_merchant(entity)` instead, which gives a `MerchantRef` for a
villager or a wandering trader. Its handle reads what the server syncs of the
villager's `VillagerData`, the entity data vanilla's client renders it from:

```rust,ignore
let entity = host.entity(id)?;
let villager = host.client_merchant(entity)?;
let profession = villager.profession(host); // Some("minecraft:librarian")
let level = villager.level(host); // Some(3), at least 1
let kind = villager.villager_type(host); // Some("minecraft:desert")
```

Each read is the value the server last sent, so it follows a villager that takes
or loses a job or levels up. A wandering trader has no villager data: its three
reads are `None`. A profession is the id the client's table names for the slot the
server sent, so a mod profession reads as its own id when the mod is installed on
both sides, and as `minecraft:none` while the server's villager has an unresolved
profession. The other `MerchantRef` reads (`offers`, `xp`, `trading_player`) are
the server's and answer `None` on a client. The offers of a trade the local
player has open come from the open menu,
`menu.merchant_offers(host)` ([Menu slots](../../concepts/capabilities/menu-slots.md#reading-and-clicking)).
