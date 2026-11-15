//! The `register_item_tint_sources` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The item tint sources a mod adds (NeoForge's `RegisterColorHandlersEvent.ItemTintSources`):
    /// push each id, `<mod id>:<path>`. Asked once at start on the client, before resources load.
    /// An entry in a `minecraft:model`'s `tints` whose `type` is a registered id colours its
    /// `tintindex` with what `item_tint_source` answers per stack, also after a resource reload. An id
    /// in vanilla's namespace, not of that form, or registered already is logged and left out; a
    /// tint source nobody registers draws white.
    #[notes = r#"## Item tint sources

An item tint source (NeoForge's `RegisterColorHandlersEvent.ItemTintSources`) colours a greyscale item per
stack, in two steps as well: `Hooks::register_item_tint_sources` pushes its id once at start, before resources
load, and `Hooks::item_tint_source` answers the ARGB colour for each stack, in hand, in a screen or on the ground.

```rust,ignore
fn register_item_tint_sources(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterItemTintSourcesEvent<'_>,
) {
    event.sources.push("example:charge".into());
}

fn item_tint_source(&self, _host: &dyn ClientHost, event: &mut ItemTintSourceEvent<'_>) {
    if event.source == "example:charge" {
        let rgb = match event.config.get("default") {
            Some(Tag::Int(value)) => *value as u32,
            _ => 0xFFFFFF,
        };
        let rgb = if event.stack.count() > 1 { 0x55FF55 } else { rgb };
        event.color = 0xFF000000 | rgb;
    }
}
```

Use it as an entry of a `minecraft:model`'s `tints`, `{"type": "example:charge", "default": 65280}`: the entry's
place in the list is the `tintindex` it colours, and its object reaches the hook as `event.config`. A resource
reload keeps reading the mod's colour. Ids are checked as for numeric properties. A source no mod registers draws
white, decided once at load, and so does an entry no mod answers; an item with no mod source never reaches the hook."#]
    #[hook(
        side = "client",
        contract = "collect",
        out = "sources",
        context = "registration",
        neoforge = "RegisterColorHandlersEvent.ItemTintSources"
    )]
    fn register_item_tint_sources();
}
