//! The `register_range_select_item_model_property` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The numeric item model properties a mod adds (NeoForge's
    /// `RegisterRangeSelectItemModelPropertyEvent`): push each id, `<mod id>:<path>`. Asked once at
    /// start on the client, before resources load. A `minecraft:range_dispatch` naming a registered
    /// property thresholds on the number `item_model_range_property` answers per stack, also after a
    /// resource reload. An id in vanilla's namespace, not of that form, or registered already is
    /// logged and left out; a property nobody registers reads 0.
    #[notes = r#"## Numeric item model properties

A client mod adds a numeric `minecraft:range_dispatch` property (NeoForge's
`RegisterRangeSelectItemModelPropertyEvent`) in two steps. `Hooks::register_range_select_item_model_property`
pushes its id once at start, before resources load; `Hooks::item_model_range_property` answers the number
for each stack while a model picks a branch.

```rust,ignore
fn register_range_select_item_model_property(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterRangeSelectItemModelPropertyEvent<'_>,
) {
    event.properties.push("example:fill".into());
}

fn item_model_range_property(&self, _host: &dyn ClientHost, event: &mut ItemModelRangePropertyEvent<'_>) {
    if event.property == "example:fill" {
        event.value = event.stack.count() as f32 / 64.0;
    }
}
```

Use `"property": "example:fill"` in the item's `minecraft:range_dispatch`: the model's `scale` multiplies the
number, then the highest threshold at or below it wins, else `fallback`, as for vanilla's properties. A
resource reload keeps reading the mod's number. An id in vanilla's namespace, not `<namespace>:<path>`, or
registered twice is logged and left out. A property no mod registers reads 0, decided once at load."#]
    #[hook(
        side = "client",
        contract = "collect",
        out = "properties",
        context = "registration",
        neoforge = "RegisterRangeSelectItemModelPropertyEvent"
    )]
    fn register_range_select_item_model_property();
}
