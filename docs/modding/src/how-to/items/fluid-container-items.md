# Fluid container items

Use `ItemPart::contained_fluid` to tell a `neoforge:fluid_container` item model which fluid your stack holds. The model draws the fluid's still sprite through its fluid mask, tinted for that fluid, beneath the cover.

Attach the Part to your item with `item_part`. Keep a `FluidKey` obtained during registration with `fluid_key` or `declare_fluid`, and answer from the stack's components:

```rust,ignore
#[horizon_part]
impl ItemPart for WaterContainer {
    fn contained_fluid(&self, _: &dyn ClientHost, event: &mut ContainedFluidEvent<'_>) {
        *event.fluid = Some(self.water);
    }
}
```

This example reports water for every stack; read `event.stack` to distinguish empty and filled containers. The callback receives an API-owned `ItemStack` snapshot and a read-only `ClientHost`. It runs on both native and wasm clients.

## Rendering rules

| Answer or item | Result |
| --- | --- |
| `Some(key)` for a registered fluid | Draws that fluid through the mask. |
| `None` (the initial answer) | Draws the model's declared fluid untinted, or omits the fluid layer when it declares `minecraft:empty`. |
| Registered fluid without item appearance data (`horizon/fluid`) | Draws the missing texture through the mask. |
| Key with no registered fluid | Uses the `None` result and logs a warning once per item. |
| Item in `minecraft:` | Uses vanilla's water/lava bucket lookup; Parts attached by key or tag do not receive this hook. |

Only items with subscribed Parts receive the callback. The game checks subscriptions before constructing the stack snapshot; an unresolved stack does not dispatch.

## Callback frequency

Keep the answer fast and dependent only on the stack. Each fluid-container node queried during model resolution asks for the fluid. Composite models can query the same stack multiple times in one resolution, and drawing the stack again can query it again. There is no once-per-frame guarantee.

The hook supplies a rendering answer, not a transactional fluid handler. Use `ITEM_FLUIDS` for server-side fluid transfers.
