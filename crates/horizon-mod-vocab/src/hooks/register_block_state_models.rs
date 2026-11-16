//! The `register_block_state_models` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The block-state definition types a mod adds (NeoForge's `RegisterBlockStateModels`,
    /// its `registerDefinition`): push each type id, `<mod id>:<path>`. Asked once at start on
    /// the client, before resources load. A blockstate file whose `neoforge:definition_type` is
    /// a registered id has each state's model picked by `instantiate_block_state_definition`, on
    /// every load and reload. An id in vanilla's namespace, not of that form, or registered
    /// already is logged and left out.
    #[notes = r#"## Block-state definition types

A blockstate file (`assets/<namespace>/blockstates/<block>.json`) picks each state's model with vanilla's
`variants` or `multipart`. A mod adds a definition type of its own (NeoForge's `RegisterBlockStateModels`)
in two steps, as a model loader: `Hooks::register_block_state_models` pushes the type id once at start,
before resources load, and `Hooks::instantiate_block_state_definition` answers each file that names it in
`neoforge:definition_type`, decoding the file and picking the model of every state, on every load and
reload.

```rust,ignore
fn register_block_state_models(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterBlockStateModelsEvent<'_>,
) {
    event.definitions.push("example:by_power".into());
}

fn instantiate_block_state_definition(
    &self,
    _context: &RegistrationContext,
    event: &mut InstantiateBlockStateDefinitionEvent<'_>,
) {
    if event.definition_type != "example:by_power" {
        return;
    }
    // {"neoforge:definition_type": "example:by_power", "on": "example:block/lamp_on", "off": "example:block/lamp_off"}
    let (Some(Tag::String(on)), Some(Tag::String(off))) = (event.config.get("on"), event.config.get("off")) else {
        event.models.push(BlockStateModelPick::Refuse { reason: "needs \"on\" and \"off\" models".into() });
        return;
    };
    for (state, properties) in event.states.iter().enumerate() {
        let lit = properties.get("powered").is_some_and(|value| value == "true");
        event.models.push(BlockStateModelPick::Model {
            state: state as u32,
            model: (if lit { on } else { off }).to_string_lossy(),
            turns: [0; 3],
            uvlock: false,
        });
    }
}
```

`event.config` is the file's whole JSON object converted to a `Tag`, `event.block` the block it belongs to,
and `event.states` the block's states in registry order, each as its property names and values. A pick
names a state by its index there. The file is refused whole, with its name and a reason logged, when the
type is not registered, a mod pushes `Refuse`, a pick names a state the block lacks, names one state twice
or a model id that doesn't read, or no state is picked. A file several packs provide binds each pack's
layer on its own, as vanilla does: a refused layer leaves the states to the layers below it."#]
    #[hook(
        side = "client",
        contract = "collect",
        out = "definitions",
        context = "registration",
        neoforge = "RegisterBlockStateModels"
    )]
    fn register_block_state_models();
}
