//! The `can_equip` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `IItemExtension.canEquip`: may `stack` go into the equipment
    /// slot `slot` of the living entity `entity`, of type `kind`
    /// (`minecraft:player`)? `slot` is an `EquipmentSlot` name: `head`,
    /// `chest`, `legs`, `feet`, `mainhand` or `offhand`, and `body` or
    /// `saddle` from a dispenser. `can_equip` arrives holding the data answer,
    /// as `LivingEntity.isEquippableInSlot` and `Equippable.canBeEquippedBy`
    /// read the stack's own `equippable` component (a stack's override
    /// replaces the item's): yes when it names that slot and its
    /// `allowed_entities` is absent or lists the entity type (an empty list
    /// allows none); with no component, removed or never there, yes for
    /// `mainhand` only. Set it either way. Asked where vanilla asks: the
    /// armour slots of a player's inventory menu (`ArmorSlot.mayPlace`, which a
    /// shift-click into one reaches too), a dispenser equipping what stands in
    /// front of it (`EquipmentDispenseItemBehavior`, one entity at a time) and
    /// a mob picking an item up off the ground (`Mob.equipItemIfPossible`). It
    /// decides whether the stack fits the slot, not what the slot holds,
    /// whether the entity may wear one at all (`canUseSlot`) or the dispenser's
    /// own rules for the entity. Only the `ItemPart`s attached to `stack`'s
    /// item are asked, on both sides, as the client's menu predicts the slot.
    /// `host` is read-only: read `entity` through it (its position, its
    /// equipment) to answer from its state when asked. The stack is converted
    /// once and lent to each slot and entity a native menu click, dispense or pickup
    /// asks about. Each decision prepares one owned copy per participating wasm guest
    /// instance and lends it by token to every candidate. Mutable access invalidates
    /// the source revision; replacing or removing it changes its identity. Stale
    /// preparation and stale apply preconditions refuse without asking again. Pickup
    /// scope is one mob's attempt. Missing prepared exports refuse at load or part
    /// registration. A stale or missing target refuses wherever a part is asked;
    /// the no-listener data path performs no conversion or entity lookup.
    /// Target type, position, health, maximum health, alive/living status and physical
    /// equipment are retained outside traversal; other Host reads can allocate.
    #[hook(
        side = "both",
        version = 2,
        contract = "listen",
        scope = "item",
        key = "stack",
        prepared = "stack",
        host = "read",
        target = "horizon_client::game::native_hooks::can_equip::NativeHookHost<'_>::native_can_equip",
        cancellable = false,
        neoforge = "IItemExtension.canEquip"
    )]
    fn can_equip();
}
