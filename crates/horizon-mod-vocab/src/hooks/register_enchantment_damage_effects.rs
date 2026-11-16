pub trait Hooks {
    /// Register namespaced enchantment damage value effects once before content loads.
    /// Custom nodes in `minecraft:damage`, including nested `minecraft:all_of`, resolve to
    /// these callbacks at load. Unknown identifiers reject the enchantment with a diagnostic.
    /// Callbacks receive a read-only ServerHost, actual attacker and victim handles and the
    /// pre-hit weapon snapshot. Vanilla nodes and requirements retain their traversal and RNG
    /// order. This is vanilla EnchantmentHelper.modifyDamage, not an incoming-damage event.
    #[notes = r#"## Custom enchantment damage values

`register_enchantment_damage_effects` registers an `EnchantmentDamageEffect` under a non-vanilla
`Identifier` before content loads. Use that identifier as a value node's `type` in an enchantment's
`minecraft:damage` component. The callback receives a read-only `ServerHost`, actual attacker and
victim handles (`LivingRef` and `EntityRef`), a pre-hit weapon snapshot, enchantment level and the node's configuration as a borrowed NBT `Tag`, converted once at load.
Return the transformed damage value. NaN retains the input; other outputs (including infinities)
are clamped to 0..=1024 before subsequent nodes, bounding per-unit built-in work. Registration rejects duplicate and vanilla identifiers.

The engine resolves identifiers during content loading, including resource/data pack reloads.
Unknown identifiers reject the containing enchantment with a diagnostic naming both identifiers.
Built-in nodes, requirements and nested `minecraft:all_of` keep their original order and random
consumption. The callback runs at `EnchantmentHelper.modifyDamage` during ordinary damage computation,
before incoming-damage hooks and armor reductions. It is an effect registrar, not a generic hit hook.
Callbacks are indexed directly by resolved slots; hits never scan installed mods. Hits without custom
nodes retain the vanilla evaluator without constructing API weapon snapshots."#]
    #[hook(
        side = "server",
        contract = "collect",
        out = "effects",
        context = "registration",
        neoforge = "RegisterEvent"
    )]
    fn register_enchantment_damage_effects();
}
