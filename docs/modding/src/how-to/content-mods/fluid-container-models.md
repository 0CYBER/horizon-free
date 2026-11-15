# Fluid container item models

An item whose model is NeoForge's `neoforge:fluid_container` draws the fluid it holds: its base, the fluid's still
sprite through a mask, tinted as a stack holding that fluid, and a cover over it.

## The model

Point the item's definition (`assets/<mod>/items/<item>.json`) at the type:

```json
{
  "model": {
    "type": "neoforge:fluid_container",
    "fluid": "minecraft:empty",
    "textures": {
      "base": "mymod:item/canister",
      "fluid": "neoforge:item/mask/bucket_fluid",
      "cover": "neoforge:item/mask/bucket_fluid_cover"
    }
  }
}
```

`fluid` is what the container draws while it holds none: `minecraft:empty` draws no fluid layer, any other fluid
draws untinted. `cover_is_mask` (default true) makes the cover a mask that picks parts of `base` to draw again over
the fluid; set it false to draw the cover texture itself.

## Report the fluid a stack holds

Vanilla's water and lava buckets answer by themselves. A mod's own container answers through an `ItemPart` that
implements `contained_fluid`. Take the fluid keys once in `register` and set `event.fluid` from the stack:

```rust,ignore
use mod_api::{ClientHost, ContainedFluidEvent, FluidKey, Hooks, ItemPart, Registrar, RegistrationContext};

#[derive(Clone)]
struct Canister {
    water: FluidKey,
    oil: FluidKey,
}

#[mod_api::horizon_part]
impl ItemPart for Canister {
    fn contained_fluid(&self, _: &dyn ClientHost, event: &mut ContainedFluidEvent<'_>) {
        // Read the stack's own component, written when the canister was filled.
        let filled = event
            .stack
            .components()
            .and_then(|patch| patch.as_nbt().get("mymod:filled"));
        *event.fluid = match filled {
            Some(mod_api::Tag::Byte(1)) => Some(self.water),
            Some(mod_api::Tag::Byte(2)) => Some(self.oil),
            _ => None,
        };
    }
}

struct MyMod;
#[mod_api::horizon_mod]
impl Hooks for MyMod {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let part = Canister {
            water: r.fluid_key("minecraft:water"),
            oil: r.declare_fluid("oil"),
        };
        let canister = r.item_key("mymod:canister");
        r.item_part(canister, part);
    }
}
```

- The answer is a fluid, never a sprite or a colour: the model draws the fluid's own still sprite and item tint, as
  for a water bucket. A mod fluid needs a look (`data/<mod>/horizon/fluid/<name>.json`) to draw in an item; a model
  from `register_fluid_models` changes how a fluid with a look draws, but does not make one drawable alone. A
  registered fluid with no look draws the missing texture through the mask, so the missing file shows.
- Leaving `event.fluid` at `None` draws the stack as holding none: the model's declared `fluid`, untinted, or no
  fluid layer when that is `minecraft:empty`.
- Registration returns keys for registered fluids, and the wasm boundary checks fluid keys. An unregistered key
  draws the same as `None` and produces a log message once per item.
- It is asked on the client each time such a stack is drawn, so answer from the stack alone and keep it cheap. Only
  items outside `minecraft:` are asked: a part on a vanilla item is not.
- It is the client's picture only. What the stack holds for gameplay (filling, emptying, pipes) stays in the stack's
  own data and the `ITEM_FLUIDS` capability.

The same part works in a native and a wasm mod.
