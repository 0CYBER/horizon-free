//! The `register_item_models` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The item model node types a mod adds (NeoForge's `RegisterItemModelsEvent`): push each
    /// type id, `<mod id>:<path>`. Asked once at start on the client, before resources load. An
    /// item definition's node whose `type` is a registered id draws, per stack, the named models
    /// `item_model_update` answers. An id in vanilla's namespace, not of that form, or registered
    /// already is logged and left out.
    #[notes = r#"## Item model types

An item definition (`assets/<namespace>/items/<path>.json`) is a tree of model nodes: vanilla's `minecraft:model`,
`condition`, `select`, `range_dispatch`, `composite`, `special` and the rest. A mod adds a node type of its own
(NeoForge's `RegisterItemModelsEvent`) in two steps, as an item tint source: `Hooks::register_item_models`
pushes the type id once at start, before resources load, and `Hooks::item_model_update` answers each time an item
whose definition holds that node is drawn.

The node lists the models it can draw in a `models` object; they are ordinary nodes, decoded and baked by the
game. Per stack, the mod pushes the names of the ones to draw, in order:

```rust,ignore
fn register_item_models(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterItemModelsEvent<'_>,
) {
    event.models.push("example:charge".into());
}

fn item_model_update(&self, _host: &dyn ClientHost, event: &mut ItemModelUpdateEvent<'_>) {
    if event.kind == "example:charge" {
        let name = if event.stack.count() > 1 { "full" } else { "empty" };
        event.models.push(name.into());
    }
}
```

```json
{"model": {"type": "example:charge",
  "models": {"empty": {"type": "minecraft:model", "model": "example:item/cell_empty"},
             "full": {"type": "minecraft:model", "model": "example:item/cell_full"}}}}
```

The node's whole object reaches the hook as `event.config`, converted once at load, and its `transformation`
applies to every model it draws. A name the node's `models` doesn't have draws nothing, and so does a node no
mod answers. A node type no mod registered is an error naming it: the item definition holding it isn't loaded,
and the item draws the missing model."#]
    #[hook(
        side = "client",
        contract = "collect",
        out = "models",
        context = "registration",
        neoforge = "RegisterItemModelsEvent"
    )]
    fn register_item_models();
}
