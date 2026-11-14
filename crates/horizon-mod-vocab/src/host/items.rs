//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The host's methods both sides share of the `items` area, implemented by the game and reached through [`Host`].
#[horizon_mod_api_macros::host_area]
pub trait ItemsHost {
    /// Side: both.
    /// The most of the item `item` one slot holds (`Item.getDefaultMaxStackSize`),
    /// or `None` for an unknown item: for a mod's container that fills or
    /// merges stacks itself.
    #[access(Metadata)]
    fn max_stack_size(&self, item: ItemKey) -> Option<u32>;
    /// Side: both.
    /// Default durability of the resolved item, or none if its metadata is unavailable.
    /// Stack-specific component overrides are checked at the game's write boundary.
    #[access(Metadata)]
    fn max_damage(&self, item: ItemKey) -> Option<u32>;
    /// Side: both.
    /// Resolve an item once at load, including vanilla items, to its process-local identity.
    /// None if the completed registry does not contain the identifier.
    #[access(Metadata)]
    fn item_key(&self, id: &Identifier) -> Option<ItemKey>;
    /// Side: both.
    /// Item `item`'s default value of component `component`, in its prototype
    /// (`Item.components().get`): `minecraft:max_damage` on an iron sword is `250`, data
    /// packs' and native mods' changes included. Read from the item report's JSON, so a
    /// number is an `Int`, `Long` or `Double` and a boolean a `Byte`. None where the item
    /// hasn't the component, and for an unknown item.
    #[access(Metadata)]
    fn item_default_component(&self, item: ItemKey, component: &str) -> Option<Tag>;
    /// Side: both.
    /// Item `item`'s cooking-fuel burn time in ticks (`FuelValues.burnDuration` of its `cooking_fuel`
    /// component, the data pack's context int provider resolved), None where it is no fuel. API
    /// @API:item_components@.
    #[access(Metadata)]
    fn item_burn_time(&self, item: ItemKey) -> Option<i32>;
    /// Side: both.
    /// Item `item`'s chance to raise a composter's level (its `compostable` component's layers provider
    /// resolved), None where it is not compostable. API @API:item_components@.
    #[access(Metadata)]
    fn item_compost_chance(&self, item: ItemKey) -> Option<f32>;
    /// Side: both.
    /// Whether enchantments `a` and `b` may share an item (`Enchantment.areCompatible`), by the
    /// world's enchantment definitions. API @API:item_components@.
    #[access(WorldOnly)]
    fn enchantments_compatible(&self, a: &str, b: &str) -> bool;
    /// Side: both.
    /// `EntityType.is(TagKey)`: whether `entity_type` (`minecraft:player`) is in the entity
    /// type tag `tag` (`minecraft:cannot_be_pushed_onto_boats`).
    #[access(Metadata)]
    fn entity_type_in_tag(&self, entity_type: &str, tag: &str) -> bool;
    /// Side: both.
    /// The stack an SNBT item compound stands for, read as `ItemStack.CODEC` reads a saved
    /// item: `{id:"minecraft:diamond_sword",count:1,components:{"minecraft:damage":5}}`
    /// (`count` 1 when missing, `stone` is `minecraft:stone`). This API refuses with None:
    /// text that is no SNBT compound or has anything but whitespace after it, a `count`
    /// outside 1 to 99 (the codec's range; it is not clamped to 1), air, and an item this
    /// world does not have. It also refuses text over 2 MiB and nesting past 512 levels. Those
    /// two are this API's own safety bound, the one place it differs from vanilla: vanilla's
    /// SNBT parser has no size or depth limit (very deep text ends in a stack overflow), and
    /// 2 MiB and 512 are binary NBT's limits, taken here so text a mod supplies cannot
    /// overflow the parser.
    #[access(WorldOnly)]
    fn item_from_snbt(&self, snbt: &str) -> Option<ItemStack>;
}
/// The server host's methods of the `items` area, implemented by the game and reached through [`ServerHost`].
#[horizon_mod_api_macros::host_area]
pub trait ItemsServerHost {
    /// Side: server.
    /// Drop one item as DefaultDispenseItemBehavior does. For a dispenser hook:
    /// return this remainder in its answer with `CLICK`; the caller plays the
    /// answer's event and smoke once. A client, another dimension or an unloaded
    /// chunk keeps the input stack unchanged.
    #[access(WorldOnly)]
    fn dispense_default(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Option<ItemStack>, StackWriteError>;
    /// Side: server.
    /// Holds `stack` outside the world, in the open transaction `tx` (`ItemAccess.forStack`):
    /// the [`ItemPlace::Scratch`] to run item capabilities against. It lives until the
    /// outermost transaction closes, and an abort of the one that made it drops it. None
    /// where `tx` is not open.
    #[access(WorldOnly)]
    fn scratch_stack(
        &mut self,
        tx: &Transaction,
        stack: ItemStack,
    ) -> Result<Option<ItemPlace>, StackWriteError>;
    /// Side: server.
    /// Takes the stack out of scratch place `place`, in `tx`, ending the place: none where it
    /// is empty, no longer held, or not a scratch place.
    #[access(WorldOnly)]
    fn scratch_take(&mut self, tx: &Transaction, place: &ItemPlace) -> Option<ItemStack>;
}
/// The client host's methods of the `items` area, implemented by the game and reached through [`crate::ClientHost`].
#[horizon_mod_api_macros::host_area]
pub trait ItemsClientHost {
    /// Side: client.
    /// Creative mode only: gives the player `count` of `item`
    /// (`minecraft:diamond`) as taking it from the creative inventory does
    /// (`ServerboundSetCreativeModeSlot`): the selected slot first, then the
    /// hotbar, then the inventory. The count is cut to one stack.
    #[access(WorldOnly)]
    fn give(&mut self, item: ItemKey, count: u32) -> Result<(), String>;
    /// Side: client.
    /// `ItemStack.getHoverName()` as plain text: the custom name, else a written book's non-blank
    /// title, else the item's name in the client's language (potion contents, a head's owner and
    /// the `item_name` component as vanilla). No rarity colour or italics. An invalid stack reads
    /// an empty string.
    #[access(ClientSetup)]
    fn hover_name(&self, stack: &ItemStack) -> String;
    /// Side: client.
    /// Registered enchantments, in registry order.
    #[access(ClientSetup)]
    fn enchantments(&self) -> Vec<EnchantmentInfo>;
    /// Side: client.
    /// `Enchantment.getMinCost(level)`: the enchanting power needed, or None for an unknown id.
    #[access(ClientSetup)]
    fn enchantment_min_cost(&self, id: &str, level: u32) -> Option<i32>;
    /// Side: client.
    /// Whether enchantment `id` can go on `item` (`Enchantment.isSupportedItem`).
    #[access(ClientSetup)]
    fn enchantment_supports(&self, id: &str, item: ItemKey) -> bool;
}
/// The handles' reads of the `items` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait ItemsBackend {
    /// Side: both. Effective damageability after defaults and the stack patch.
    #[access(Metadata)]
    fn item_resource_damageable(&self, stack: &ItemStack) -> bool;
    /// Side: both. Effective glint, including overrides and item-specific foil.
    #[access(Metadata)]
    fn item_resource_foil(&self, stack: &ItemStack) -> bool;
    /// Side: both. Effective repair ingredient membership, including item tags.
    #[access(Metadata)]
    fn item_resource_repairable(&self, stack: &ItemStack, material: ItemKey) -> bool;

    /// Side: both. ItemResource's effective use duration, including its Part.
    #[access(WorldOnly)]
    fn item_resource_use_duration(&self, stack: &ItemStack) -> Option<i32>;
    /// Side: both. The mod that made the stack, including its Part.
    #[access(WorldOnly)]
    fn item_resource_creator_mod_id(&self, stack: &ItemStack) -> Option<String>;
    /// Side: both. The effective vanilla bar, before client drawing hooks.
    #[access(WorldOnly)]
    fn item_resource_bar(&self, stack: &ItemStack) -> Option<ItemBar>;
    /// Side: both. Effective stack range for the supplied actor.
    #[access(WorldOnly)]
    fn item_resource_entity_attack_range(
        &self,
        stack: &ItemStack,
        attacker: LivingRef,
    ) -> Option<ItemAttackRange>;
    /// Side: both. Effective stack melee source for the supplied actor.
    #[access(WorldOnly)]
    fn item_resource_damage_source(
        &self,
        stack: &ItemStack,
        attacker: LivingRef,
    ) -> Option<ItemDamageSource>;
    /// Side: both. Item immunity to the supplied damage source.
    #[access(WorldOnly)]
    fn item_resource_can_be_hurt_by(
        &self,
        stack: &ItemStack,
        source: &ItemDamageSource,
    ) -> Option<bool>;
    /// Side: both. Attack bonus preview without consuming world RNG or equipping the stack.
    #[access(WorldOnly)]
    fn item_resource_attack_damage_bonus(
        &self,
        stack: &ItemStack,
        attacker: LivingRef,
        target: EntityRef,
        base_damage: f32,
    ) -> Option<f32>;
    /// Side: client. The item's standard tooltip lines before mod callbacks or drawing.
    #[access(WorldOnly)]
    fn item_resource_tooltip(
        &self,
        stack: &ItemStack,
        advanced: bool,
        creative: bool,
    ) -> Option<Vec<TooltipElement>>;
    /// Side: server.
    /// How many container slots `at`'s side reaches (none: every slot), by its
    /// `WorldlyContainer` rules: a furnace's top reaches its input only. A composter's up,
    /// down and unsided ports each expose one state-backed slot. None without a supported
    /// container/port, in a level not in place, and on a client's copy of the world.
    #[access(WorldOnly)]
    fn container_size(&self, at: BlockSide) -> Option<usize>;
    /// Side: server.
    /// The stack in slot `slot` of those `at`'s side reaches (`container_size`).
    #[access(WorldOnly)]
    fn container_stack(&self, at: BlockSide, slot: usize) -> Option<ItemStack>;
    /// Side: server.
    /// How many of `stack` slot `slot` of those `at`'s side reaches holds when empty, as
    /// NeoForge's `isValid` and `getCapacityAsLong` answer: none where it may not go in (a
    /// furnace's output, `canPlaceItem`), else the slot's limit for it.
    #[access(WorldOnly)]
    fn container_capacity(&self, at: BlockSide, slot: usize, stack: &ItemStack) -> u32;
    /// Side: server.
    /// How much of `resource` the slots `at`'s side reaches hold in all: [`STORAGE`]'s
    /// amount for a container or composter port, in one pass. None where the total does
    /// not fit in 64 bits; 0 where no supported container or port exists.
    ///
    /// [`STORAGE`]: crate::STORAGE
    #[access(WorldOnly)]
    fn container_kind_amount(&self, at: BlockSide, resource: &ItemResource) -> Option<u64>;
    /// Side: server.
    /// How much of `resource` an insert through `at`'s side would take now: the room left in
    /// the slots holding it and in the empty slots it may go in, in one pass. None where the
    /// total does not fit in 64 bits.
    #[access(WorldOnly)]
    fn container_kind_space(&self, at: BlockSide, resource: &ItemResource) -> Option<u64>;
    /// Side: server.
    /// What the slots `at`'s side reaches hold, from its `from`th to before its
    /// `from + limit`th (fewer at the end), empty ones included, never more than
    /// [`MAX_STORAGE_PAGE`](crate::MAX_STORAGE_PAGE) slots whatever `limit` asks. None where no
    /// supported container or composter port exists.
    #[access(WorldOnly)]
    fn container_listing(
        &self,
        at: BlockSide,
        from: usize,
        limit: usize,
    ) -> Option<Vec<Option<ItemStack>>>;
    /// Side: both.
    /// Whether an enchantment on `stack` has effect component `effect`, by the world's
    /// enchantment definitions (`EnchantmentHelper.has`), behind
    /// `ItemResource::has_enchantment_effect`, through the opaque backend. API
    /// @API:item_components@.
    #[access(WorldOnly)]
    fn item_has_enchantment_effect(&self, stack: &ItemStack, effect: &str) -> bool;
    /// Side: both.
    /// The slot `stack` is equipped to (`Equippable.slot`) after its component patch, behind
    /// `ItemResource::equippable_slot`, through the opaque backend. API @API:item_components@.
    #[access(WorldOnly)]
    fn item_equippable_slot(&self, stack: &ItemStack) -> Option<String>;
    /// Side: both.
    /// `Capabilities.Item.ITEM`, read: the slots of one item of `stack`,
    /// empty ones included, kept in its `minecraft:container`. None for an
    /// item without slots, and for an empty stack.
    #[access(WorldOnly)]
    fn item_container_slots(&self, stack: &ItemStack) -> Option<Vec<Option<ItemStack>>>;
    /// Side: both.
    /// `ResourceHandler.insert`: as much of `item` as fits into one item of
    /// `stack`, topping up the slots that hold the same item and components
    /// first, then the first empty ones (`SimpleContainer.addItem`'s order),
    /// no slot past its limit (99, or the item's own when less). A shulker
    /// box never goes in. The count that went in, and the single item it
    /// becomes (the item itself when none did). The stack given is not
    /// changed: shrink it by one and hand out the result, or leave the result
    /// unused to simulate. None where the item has no slots.
    #[access(WorldOnly)]
    fn item_container_insert(
        &self,
        stack: &ItemStack,
        item: &ItemStack,
    ) -> Option<(u32, ItemStack)>;
    /// Side: both.
    /// `ResourceHandler.extract`: up to `max` out of slot `slot` of one item
    /// of `stack`. What came out (none when the slot is empty or past the
    /// item's slots), and the single item it becomes. The stack given is not
    /// changed, as with `native_item_container_insert`. None where the item
    /// has no slots.
    #[access(WorldOnly)]
    fn item_container_extract(
        &self,
        stack: &ItemStack,
        slot: usize,
        max: u32,
    ) -> Option<(Option<ItemStack>, ItemStack)>;
    /// Side: both.
    /// `ResourceHandler.insert(index, …)`: as `native_item_container_insert`,
    /// into slot `slot` alone: topped up when it holds the same item and
    /// components, filled when empty. Nothing goes into a slot past the
    /// item's slots or holding another item.
    #[access(WorldOnly)]
    fn item_container_insert_slot(
        &self,
        stack: &ItemStack,
        slot: usize,
        item: &ItemStack,
    ) -> Option<(u32, ItemStack)>;
    /// Side: both.
    /// `ResourceHandler.isValid` and `getCapacityAsLong`: how many of `item`
    /// slot `slot` of one item of `stack` holds when empty. None of a shulker
    /// box, nor into a slot past the item's slots.
    #[access(WorldOnly)]
    fn item_container_capacity(&self, stack: &ItemStack, slot: usize, item: &ItemStack) -> u32;
    /// Side: both.
    /// The stacks in one item of `stack`'s `minecraft:bundle_contents`, front first (the
    /// stack the bundle gives back next is slot 0). None for an item without the component.
    #[access(WorldOnly)]
    fn item_bundle_slots(&self, stack: &ItemStack) -> Option<Vec<Option<ItemStack>>>;
    /// Side: both.
    /// `ResourceHandler.isValid` and `getCapacityAsLong` for a bundle: how many of `item` one
    /// item of `stack` still takes, by `BundleContents.getMaxAmountToAdd` (weight left, nested
    /// bundles' weight); 0 for an item the bundle refuses, or without the component.
    #[access(WorldOnly)]
    fn item_bundle_capacity(&self, stack: &ItemStack, slot: usize, item: &ItemStack) -> u32;
    /// Side: both.
    /// `ResourceHandler.insert` for a bundle: as much of `item` as the bundle takes goes in at
    /// the front, as the bundle screen inserts. Gives how many went in and the changed bundle;
    /// None without the component.
    #[access(WorldOnly)]
    fn item_bundle_insert_slot(
        &self,
        stack: &ItemStack,
        slot: usize,
        item: &ItemStack,
    ) -> Option<(u32, ItemStack)>;
    /// Side: both.
    /// `ResourceHandler.extract` for a bundle: up to `max` of the front stack (slot 0 only), as
    /// the bundle screen takes out. Gives the taken stack, if any, and the changed bundle; None
    /// without the component.
    #[access(WorldOnly)]
    fn item_bundle_extract(
        &self,
        stack: &ItemStack,
        slot: usize,
        max: u32,
    ) -> Option<(Option<ItemStack>, ItemStack)>;
    /// Side: server.
    /// How many slots the inventory of entity `entity` in the level in place has, as
    /// `entity_slots` reaches them: none for an entity without one, and on a client.
    #[access(WorldOnly)]
    fn entity_container_size(&self, entity: u32) -> Option<usize>;
    /// Side: server.
    /// How many slots the container entity `entity` of the level in place shows automation
    /// (`EntitySelector.CONTAINER_ENTITY_SELECTOR`: chest and hopper minecarts, chest boats; a
    /// chested mount is not one): none for any other entity, and on a client.
    #[access(WorldOnly)]
    fn entity_automation_size(&self, entity: u32) -> Option<usize>;
    /// Side: server.
    /// How many equipment slots living entity `entity`'s [`ENTITY_ITEMS`](crate::ENTITY_ITEMS)
    /// answers (NeoForge's `LivingEntityEquipmentWrapper`): a mob's or an armor stand's
    /// seven (main hand, off hand, feet, legs, chest, head and body, by `EquipmentSlot`'s
    /// ordinal; the saddle is not automation's to move), none for any other entity, and on a
    /// client.
    #[access(WorldOnly)]
    fn entity_equipment_size(&self, entity: u32) -> Option<usize>;
}
/// The handles' writes of the `items` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait ItemsBackendMut {
    /// Side: server. Set a persisted container limit in 1..=99 under the transaction.
    #[access(WorldOnly)]
    fn container_set_stack_limit(&mut self, at: BlockSide, limit: u32, tx: &Transaction) -> bool;

    /// Side: server. Roll an enchantment result with the named level's random source.
    #[access(WorldOnly)]
    fn item_resource_enchant_randomly(
        &mut self,
        stack: &ItemStack,
        level: LevelRef,
        power: i32,
        pool: &Identifier,
    ) -> Result<Option<ItemStack>, StackWriteError>;
    /// Side: server.
    /// Puts up to `amount` of `resource` into slot `slot` of those `at`'s side reaches, as a hopper
    /// would (`canPlaceItem`, the slot's limit, the same item and components), in transaction
    /// `tx`; what didn't fit.
    #[access(WorldOnly)]
    fn container_insert(
        &mut self,
        at: BlockSide,
        slot: usize,
        resource: &ItemResource,
        amount: u32,
        tx: &Transaction,
    ) -> Result<Option<ItemStack>, StackWriteError>;
    /// Side: server.
    /// Takes up to `max` out of slot `slot` of those `at`'s side reaches, in transaction `tx`;
    /// through a side, only what that side may take (`canTakeItemThroughFace`). What came out.
    #[access(WorldOnly)]
    fn container_extract(
        &mut self,
        at: BlockSide,
        slot: usize,
        max: u32,
        tx: &Transaction,
    ) -> Option<ItemStack>;
    /// Side: server.
    /// Puts up to `amount` of `resource` into the slots `at`'s side reaches, as hoppers fill
    /// them, the slots holding it first and then the empty ones, in transaction `tx`, in one
    /// pass: how much went in.
    #[access(WorldOnly)]
    fn container_insert_kind(
        &mut self,
        at: BlockSide,
        resource: &ItemResource,
        amount: u64,
        tx: &Transaction,
    ) -> u64;
    /// Side: server.
    /// Takes up to `amount` of `resource` out of the slots `at`'s side reaches and may take
    /// from, in transaction `tx`, in one pass: how much came out.
    #[access(WorldOnly)]
    fn container_extract_kind(
        &mut self,
        at: BlockSide,
        resource: &ItemResource,
        amount: u64,
        tx: &Transaction,
    ) -> u64;
    /// Side: server.
    /// The stack at a player's or an item entity's place; none for an empty one, a player who
    /// isn't here or is dead, an item entity not in the level in place, any other place
    /// (`ItemPlace::stack` reads those), and on a client.
    #[access(WorldOnly)]
    fn game_stack(&mut self, place: &ItemPlace) -> Option<ItemStack>;
    /// Side: server.
    /// `ItemAccess.insert` at a player's place, an item entity's or an entity's slot: up to
    /// `amount` of `resource`, into the place where it's empty or holds the same item and
    /// components, up to the item's stack size. The rest goes where vanilla puts an item split
    /// off it: a player's into their inventory, else dropped as `Player.drop(stack, false)` at
    /// the root commit (a creative player's hand takes anything, and gives them one if they have
    /// none, as `ItemUtils.createFilledResult`); an item entity's dropped beside it at the root
    /// commit; an entity slot's into the entity's other slots (`forHandlerIndex`). How much
    /// went: 0 where `native_game_stack` reads nothing for want of the place. In transaction
    /// `tx`, which keeps each place as it was. The player's screen follows with the tick's sync.
    #[access(WorldOnly)]
    fn game_insert(
        &mut self,
        place: &ItemPlace,
        resource: &ItemResource,
        amount: u32,
        tx: &Transaction,
    ) -> Result<u32, StackWriteError>;
    /// Side: server.
    /// `ItemAccess.extract` at those places: up to `amount` of `resource`, how many came out. An item entity emptied stays, empty, until the root commit
    /// removes it. A creative player's hand gives without losing any (`forInfiniteMaterials`).
    #[access(WorldOnly)]
    fn game_extract(
        &mut self,
        place: &ItemPlace,
        resource: &ItemResource,
        amount: u32,
        tx: &Transaction,
    ) -> Result<u32, StackWriteError>;
    /// Side: server.
    /// Drops `stack` at the middle of the block at `pos`, as a block entity's handler gives up
    /// what it can't put back.
    #[access(WorldOnly)]
    fn drop_at(&mut self, pos: [i32; 3], stack: ItemStack) -> Result<(), StackWriteError>;
}
