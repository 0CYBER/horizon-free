# Port ledger

`tools/port_ledger.py` lists every vanilla method next to the Horizon functions that port it, both ways, and
points at the places where a port exists but isn't wired like vanilla. It answers the two questions every
parity fix starts with, without searching:

- where is vanilla's `Class.method` in Horizon (`where`), and
- which vanilla method does this Rust function port (`what`).

`vanilla_map.py` maps classes to files for version ports; the ledger works one level down, per method and per
Rust function with its line span.

## Use

```sh
python3 tools/port_ledger.py build                                   # about 5 s; again after pulling
python3 tools/port_ledger.py where KineticWeapon.damageEntities       # its ports, callees, callers, gaps
python3 tools/port_ledger.py where ShulkerBulletRenderer              # every method of a class, with status
python3 tools/port_ledger.py what crates/horizon-client/src/game/spear.rs:150
python3 tools/port_ledger.py class IronGolem                          # method by method: where, or missing
python3 tools/port_ledger.py coverage --package world/entity --depth 3
python3 tools/port_ledger.py gaps --kind seam                         # see the kinds below
python3 tools/port_ledger.py report                                   # .local/port-ledger/report.md
```

`--json` on any query gives machine output for agents. `build` reads the lab's decompiled sources
(`.local/minecraft/<version>/sources/{client,server}`, from `python3 tools/lab.py decompile`); `--sources DIR`
takes any directory of decompiled Java instead.

## How a method is matched

1. **Ported:** a Rust function whose doc comment opens with the method, the house style
   (`` /// `KineticWeapon.damageEntities`: one tick of the spear in use. ``). `Class.method`, `Class::method` and
   `Class#method` all count, as in `vanilla_map.py`; `Outer.Inner.method` names a nested class. A method a
   class inherits resolves to the class that declares it.
2. **Named:** a function with the method's own name (`tickRain` → `tick_rain`) in a file that cites the class.
3. **Mentioned:** the method appears in some other comment of a function (often a caller explaining a step).
4. **Inferred:** nothing names it, but a function shares what is hard to share by chance: rare numbers
   (`0.458`, `72000`) and game ids (`SoundEvents.ENTITY_SHULKER_BULLET_HIT` against
   `"entity.shulker_bullet.hit"`, `Blocks.DETECTOR_RAIL` against `"minecraft:detector_rail"`).
5. **Missing:** none of these.

Citing the vanilla method in the first line of a function's doc is what makes it findable, by people, agents
and this tool alike. Test code (`#[test]`, `mod tests`, fixtures, probes, examples) never counts as a port.

## Gaps

Both call graphs come from the sources: Java calls resolve through `this`, static class names, and the declared
types of fields, parameters and locals (`this.player.itemUsed` is `LocalPlayer.itemUsed`); a call also reaches
the overrides in subclasses. Rust calls resolve by name.

| Kind | Means | Example it found |
|---|---|---|
| `seam` | A vanilla method ported on two paths (the in-frame game, the server-client side in `game/remote*.rs`, the server in `game/lan*.rs` and `src/lan/`), where one path reaches the port of a callee and the other doesn't. Single player runs on the server thread, so this is where most player-visible bugs sit. | `remote_use` ported `Minecraft.startUseItem` without `LocalPlayer.itemUsed`: the held block never dipped after a placement on a server |
| `client` | A vanilla method with an effect only the client's copy of the world shows (`addParticle`, `playLocalSound`, `itemUsed`, an `isClientSide()` branch), whose port nothing in `game/remote*.rs` reaches. | `AbstractHorse.handleEating`'s happy-villager particle, drawn only on the server's level |
| `class` | A whole class nothing ports while most of its package is ported. Renderers, sound instances and goals are reached by registration, so no call graph shows them. | `ShulkerBulletRenderer`, `ElytraOnPlayerSoundInstance` |
| `callee` | A ported method whose ported callee its port never reaches. Noisier: a port may get the value as an argument instead. | `Projectile.shootFromRotation` without `Entity.getKnownMovement`: throws added a stale server velocity |
| `stale` | A citation whose class or method the pinned version doesn't have (renamed, removed, or a typo). | |
| `unported` | (`gaps --kind unported`) Vanilla methods with no port, ranked by how many vanilla methods call them. | |

A gap is a lead, not a verdict: read the vanilla method and the Rust function it names before changing
anything.
