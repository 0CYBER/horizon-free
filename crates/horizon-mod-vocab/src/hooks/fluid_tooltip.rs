//! The `fluid_tooltip` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `FluidTooltipEvent`: the tooltip of `stack`, a fluid the game draws a slot of (the recipe
    /// viewer's fluid wells), once its name, its amount in millibuckets and, with F3+H, its id
    /// are in `lines`, each with a colour as `rgba` packs it (`u32::MAX` keeps the text's own).
    /// Add, change, reorder or remove lines, and an empty list draws nothing. It fires before
    /// the tooltip is laid out or drawn, with no item and no `item_tooltip` involved: a tank's
    /// or a fluid slot's tooltip has neither. Asked each frame the tooltip shows, only while a
    /// mod listens. Client-side.
    #[notes = r#"## Fluid tooltips

- **`native_fluid_tooltip(stack, lines)`** is `FluidTooltipEvent`: the tooltip of a fluid, with no item behind it. `lines` starts as the game's own (`horizon_client::fluid_tooltip::lines`): the fluid's name (`fluid_type.<mod>.<name>`, vanilla's by its block), its amount as `horizon.fluid.amount` ("%s mB") and, with F3+H, its id. Each line is a `Text` (translation-ready) with a colour as `rgba` packs it. Add, change, reorder or remove lines; an empty list draws nothing. The game asks it where it draws a fluid slot itself (the recipe viewer's wells under the cursor), before `render_tooltip_pre` lays the tooltip out. Nothing item-shaped fires for it: `item_tooltip` and `render_tooltip_gather_components` belong to stacks of items."#]
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "FluidTooltipEvent",
        cancellable = false
    )]
    fn fluid_tooltip(stack: &FluidStack, lines: &mut Vec<(Text, u32)>);
}
