# Parts

Use a part to attach behavior to a specific registry type rather than listen to every event of that
kind. You attach parts once, at startup, in `Hooks::register`. For example, this `BlockPart` gives
`minecraft:stone` a comparator output of 15:

```rust,ignore
use mod_api::{
    AnalogOutputEvent, BlockPart, HasAnalogOutputEvent, Hooks, LevelHost, Registrar,
    RegistrationContext,
};

#[derive(Default)]
struct MyMod;
struct Gauge;

#[mod_api::horizon_part]
impl BlockPart for Gauge {
    fn has_analog_output(&self, _: &dyn LevelHost, event: &mut HasAnalogOutputEvent) {
        event.output = true;
    }
    fn analog_output(&self, _: &dyn LevelHost, event: &mut AnalogOutputEvent) {
        event.output = 15;
    }
}

#[mod_api::horizon_mod]
impl Hooks for MyMod {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let stone = r.block_key("minecraft:stone");
        r.block_part(stone, Gauge);
    }
}
```

## Part types

| Trait | Target |
|---|---|
| `BlockPart` | A block type, your own or vanilla's |
| `BlockEntityPart` | A block entity type, with block behavior through `BlockPart` |
| `ItemPart` | An item type, your own or vanilla's |
| `MenuPart` | A mod menu type |
| `FluidPart` | A mod world fluid type |
| `EntityPart` | An entity type, your own or vanilla's; registered through `Registrar::entity_part` on native builds |

- Annotate each part trait implementation with `#[mod_api::horizon_part]`. For scoped hooks, the
  attribute subscribes the part only to the methods you implement.
- `BlockEntityPart` extends `BlockPart`: provide a separate `BlockPart` implementation and annotate
  both implementations.
- Parts implement `Send + Sync + 'static`.
- In `has_analog_output`, answer from the block state alone. In `analog_output`, do not change the
  world. The game clamps the comparator signal to the range 0 to 15.
