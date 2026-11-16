# Item abilities

Ability names are plain, case-sensitive strings with no namespace. They are not registry identifiers.

An item's Part can change the answer to an ability question. The hook receives a read-only Host. Keep answers pure functions of the stack and ability so the client and server agree.

```rust,ignore
#[horizon_part]
impl ItemPart for CarvingTool {
    fn can_perform_action(&self, _: &dyn Host, event: &mut CanPerformActionEvent<'_>) {
        if event.ability == "shears_carve" {
            event.can_perform = true;
        }
    }
}
```

Attach this Part to your item with the registrar's `item_part` method. This grants pumpkin carving only: `shears_harvest` is a separate ability for mob and dispenser shearing. `shield_block` permits blocking and uses the stack's blocking component, or the default component when absent. Leave unknown abilities unchanged.

The game asks these abilities at the following action sites:

| Ability | Consumers |
| --- | --- |
| `shears_carve` | Pumpkin carving. |
| `shears_harvest` | Shearing sheep, mooshrooms, snow golems and bogged; dispenser shearing; harvesting bee nest honeycombs. |
| `shield_block` | Raising a blocking item and blocking attacks. |

`event.can_perform` starts with vanilla's answer. Set it to `true` to grant an ability or `false` to deny it. The answer controls permission, not the action's world effects or item wear.
