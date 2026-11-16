//! The `item_model_property` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Client-side evaluation of a mod's namespaced conditional item model property.
    /// Set value only for a property your mod owns; unhandled properties stay false.
    /// The owned stack snapshot is the one being resolved, including nested bundle items.
    #[notes = r#"## Conditional item model properties

A client mod supplies its own namespaced `minecraft:condition` property by implementing
`Hooks::item_model_property`. This hook is the registration: match your property name, answer
through `value`, and leave other names untouched. The resolver calls it when choosing a branch,
not when loading resources. It defaults to false without a provider and skips dispatch entirely
once no loaded mod overrides it. Vanilla properties do not enter the hook.

```rust,ignore
fn item_model_property(
    &self,
    _host: &dyn ClientHost,
    event: &mut ItemModelPropertyEvent<'_>,
) {
    if event.property == "example:charged" {
        event.value = event.stack.components().and_then(|patch| patch.as_nbt().get("example:charged"))
            .is_some_and(|value| matches!(value, mod_api::Tag::Byte(1)));
    }
}
```

`ClientHost`, `ItemModelPropertyEvent`, and `ItemStack` are exported by `mod_api`. The stack is an owned `ItemStack` snapshot,
including for nested bundle items: `item()`, `count()` and explicit `components()` (not merged
defaults). `bundle_selected` carries the runtime selection separately; -1 means none. Listeners run in mod order; later answers win.
Use `"property": "example:charged"` with both `on_true` and `on_false` in the item's
`minecraft:condition` node. A `minecraft:special` item model of a mod's own type is drawn by the renderer the
mod registers in `register_special_model_renderer`; one of a type no mod registered is an error naming it, and
its item draws the missing model."#]
    /// `bundle_selected` is the runtime bundle selection index; -1 means none.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "RegisterConditionalItemModelPropertyEvent"
    )]
    fn item_model_property();
}
