//! The `register_select_item_model_property` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The categorical item model properties a mod adds (NeoForge's
    /// `RegisterSelectItemModelPropertyEvent`): push each id, `<mod id>:<path>`. Asked once at start
    /// on the client, before resources load. A `minecraft:select` naming a registered property picks
    /// the case whose `when` string equals what `item_model_select_property` answers per stack, also
    /// after a resource reload. An id in vanilla's namespace, not of that form, or registered already
    /// is logged and left out; a select on a property nobody registers draws its fallback.
    #[hook(
        side = "client",
        contract = "collect",
        out = "properties",
        context = "registration",
        neoforge = "RegisterSelectItemModelPropertyEvent"
    )]
    #[notes = r##"## Categorical item model properties

A categorical `minecraft:select` property (NeoForge's `RegisterSelectItemModelPropertyEvent`) works the same
way: `Hooks::register_select_item_model_property` pushes its id once at start, and
`Hooks::item_model_select_property` answers a string for each stack.

```rust,ignore
fn register_select_item_model_property(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterSelectItemModelPropertyEvent<'_>,
) {
    event.properties.push("example:mode".into());
}

fn item_model_select_property(&self, _host: &dyn ClientHost, event: &mut ItemModelSelectPropertyEvent<'_>) {
    if event.property == "example:mode" {
        *event.value = Some("dig".into());
    }
}
```

Each `when` value in the item's `minecraft:select` on `example:mode` is a string (or a list of them); the case
whose value equals the answer is drawn, else `fallback`, also when no mod answers. A resource reload keeps
reading the mod's value. Ids are checked as for numeric properties. A select on a property no mod registers
draws its fallback, decided once at load."##]
    fn register_select_item_model_property();
}
