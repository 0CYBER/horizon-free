# Registered data load conditions

Implement `LoadCondition` as a handler that reads the condition's JSON, then
register it from
`Hooks::register_load_conditions` using its `RegistrationContext` and
`event.conditions.register(Identifier::parse("testmod:flag").unwrap(), Flag)`.
Registration happens once per mod load, with one handler per id and a read-only
`RegistrationContext`. The name must belong to the calling mod's namespace. Duplicate names refuse registration and name both owners;
a registration error refuses admission even if you ignore it.

```rust,ignore
struct Flag;
impl mod_api::LoadCondition for Flag {
    fn test(&self, args: &mod_api::serde_json::Value)
        -> Result<bool, mod_api::ConditionError>
    {
        args["value"].as_bool().ok_or_else(|| mod_api::ConditionError {
            message: "value must be a boolean".into(),
        })
    }
}
```

A recipe can use the registered name alongside the built-in conditions:

```json
{
  "type": "minecraft:crafting_shapeless",
  "ingredients": ["minecraft:flint"],
  "result": {"id": "minecraft:stick"},
  "neoforge:conditions": [{"type": "testmod:flag", "value": true}]
}
```

The handler receives a borrowed raw JSON value: the condition's object without
`type`. It reads and validates its own arguments, retaining JSON booleans,
numbers, strings, lists and objects. Invalid mod input must return
`ConditionError`; handlers must not panic while reading it. `ConditionError`
has explicit Wire encoding. Native and WebAssembly registrants cross the same generated route.
Handlers run in the server's candidate data reload, before the entries become
visible. Results are retained for each entry and condition payload in that
candidate, so repeated readers do not call the handler again. A new reload
reevaluates the entry. Handlers must not mutate live data. A native handler
panic becomes a named reload error ("handler panicked"), keeping the previous
usable data. A wasm trap follows the runtime's normal failure policy.

`true` loads the entry; `false` omits it, without exposing a lower pack's
version. A callback error refuses the reload and names the file. The previous
usable recipe and resource generation stays installed.

A namespace is owned only when a loaded mod registered at least one condition
in it. A loaded mod ID alone does not own a condition namespace. An unknown
name in an owned namespace refuses reload, naming the file, condition and
registering mod. This refusal also applies at every depth inside `not`, `and` and `or`,
even beside an already false or true branch.

An unknown name in any other namespace fails closed:
the file is absent if the unknown id appears anywhere inside `not`, `and` or
`or`, at any depth. Evaluation visits every branch, so `or(true, othermod:unknown)`
also leaves the file absent. One ERROR-level log entry per file names that file
and all its distinct unknown condition ids; repeated occurrences do not produce
extra entries within one candidate reload. The vanilla view used outside a
candidate reload has no per-file store, so it logs once per evaluation.

The public [NeoForge data load condition documentation](https://docs.neoforged.net/docs/resources/server/conditions/)
describes `ICondition` and the top-level `neoforge:conditions` array. Horizon's
owned-namespace refusal is the Horizon-specific addition described above.
Foreign unknown ids leave the file absent and produce an error-level diagnostic;
they do not themselves refuse the whole reload.
