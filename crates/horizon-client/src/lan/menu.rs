//! Container menus for guests (AbstractContainerMenu.doClick): the player's
//! inventory menu (window 0) and block containers (chests, barrels, hoppers,
//! dispensers, shulker boxes). The click modes are pickup, quick move, swap,
//! clone, throw, quick craft (drag) and pickup all; items thrown out of a
//! menu are returned for the game to drop.
#[path = "menu/backing.rs"]
mod backing;
use super::guest::Guest;
use crate::{
    bundle::{Contents, INSERT, INSERT_FAIL, REMOVE_ONE, toggle_selected},
    containers::BlockEntity,
    enchant::Enchantments,
    items::{ItemStack, Items},
    mobs::equip::equippable_in_slot,
    world::World,
};
use horizon_world::native_load_context::LoopContext;

/// The equipment slots of the inventory menu's armour slots 5 to 8, in menu order.
const ARMOR_SLOTS: [&str; 4] = ["head", "chest", "legs", "feet"];

/// What the menus need to know about items and the slots' rules.
pub trait Rules {
    fn items(&self) -> &Items;
    /// The enchantment definitions: what keeps worn armour on (`PREVENT_ARMOR_CHANGE`).
    fn enchantments(&self) -> &Enchantments;
    fn max_stack(&self, item: &str) -> u32;
    /// `ItemStack.getMaxStackSize`: the stack's own `max_stack_size`
    /// component, or vanilla's `getOrDefault` default of 1 when it is removed
    /// (`!minecraft:max_stack_size`), else `max_stack`'s item size.
    fn max_stack_of(&self, stack: &ItemStack) -> u32 {
        stack
            .own_max_stack_size()
            .unwrap_or_else(|| self.max_stack(&stack.item))
            .max(1)
    }
    /// `Slot.mayPickup`: whether the stack in menu slot `slot` may be taken
    /// out. A block container's slots and the player's own allow it; a mod's
    /// menu can refuse.
    fn may_pickup(&self, _slot: usize, _stack: &ItemStack) -> bool {
        true
    }
    /// `Player.canDropItems`: a client that drops items faster than its
    /// throttle allows stops sending THROW clicks (`LocalPlayer.canDropItems`
    /// is its `dropSpamThrottler`'s answer).
    fn can_drop_items(&self) -> bool {
        true
    }
    /// The mods' `menu_slot_rules`, lent by the game for the click (none keeps the slots' own
    /// answers).
    fn slot_rules(&self) -> Option<&dyn crate::menu::SlotRules> {
        None
    }
}

/// An open block container.
pub struct Menu {
    pub window: i32,
    /// The `minecraft:menu` registry id.
    pub kind: i32,
    /// Canonical name from the menu registry, retained for slot dispatch.
    pub registry_id: Box<str>,
    pub pos: Option<(i32, i32, i32)>,
    /// Where the container's slots live.
    pub slots: Slots,
    /// Slots of the container itself (without the 36 inventory slots).
    pub size: usize,
    pub state: i32,
    /// What the client was last told, for change detection.
    pub sent: Vec<Option<ItemStack>>,
    /// An unfinished drag: (mode, slots).
    pub drag: Option<(u8, Vec<usize>)>,
    /// The data slots the client was last sent (ContainerSetData), a furnace's
    /// progress; None until sent, so any first value goes out.
    pub sent_data: Vec<Option<i16>>,
    /// Container slots the menu does not have: a mod's screen hides them, so
    /// no click or quick move reaches them (`Slot` positions are the screen's).
    pub hidden: Vec<usize>,
    /// A content mod's menu type: its data slot count. Its clicks run on the
    /// vanilla menu engine (`menu::Kind::Mod`), not here.
    pub mod_type: Option<&'static crate::containers::ModMenuType>,
    /// A mod menu type's crafting result as the guest is shown it: the block
    /// entity holds the grid, never what it crafts. None until the game made it.
    pub craft: Option<ModCraft>,
    /// A mod menu type's linked slots (`links`): the containers a native mod
    /// links them to (`native_menu_links`), in order, asked again each tick.
    pub links: Vec<(i32, i32, i32)>,
    /// Whether a part on this mod menu type answers `menu_still_valid`,
    /// resolved when it opened; vanilla menu types never ask one.
    pub asks_still_valid: bool,
}

/// What a mod menu type's crafting grid makes (`ResultContainer`), and the
/// grid it was made from, so the game makes it again only when the grid
/// changed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModCraft {
    pub grid: Vec<Option<ItemStack>>,
    pub result: Option<ItemStack>,
}

/// Dispatch a mod menu type's `stillValid` answer for a player's block
/// menu: `valid` arrives as the game's own answer and the scoped part may change it.
#[inline(always)]
pub fn native_menu_still_valid(
    context: &LoopContext,
    player: u128,
    menu: &str,
    pos: [i32; 3],
    eye: [f64; 3],
    valid: &mut bool,
) {
    crate::native_client_hooks::menu_still_valid(context, player, menu, pos, eye, valid);
}

/// Whether a menu of this type asks its parts `menu_still_valid`: only a mod
/// menu type, and only when a part on it answers. Resolved once, when it opens.
pub fn asks_still_valid(mod_type: Option<&crate::containers::ModMenuType>, menu: &str) -> bool {
    mod_type.is_some() && crate::native_client_hooks::menu_still_valid_listens(menu)
}

impl Menu {
    /// `AbstractContainerMenu.stillValid` of this menu for the guest: its
    /// slots' own answer (`Slots::still_valid`), which the parts on its mod
    /// menu type may change (`menu_still_valid`). Vanilla menu types, menus
    /// with no block behind them (a chest vehicle, a native mod's menu) and
    /// menus whose type no part answers are not asked, nor is one whose block
    /// entities are gone: it closes.
    pub fn still_valid(&self, context: &LoopContext, world: &World, guest: &Guest) -> bool {
        let block = match self.slots {
            Slots::Block(k) | Slots::Double(k, _) => Some(k),
            Slots::Ender => self.pos,
            Slots::Native => None,
        };
        let Some(k) = block.filter(|k| self.asks_still_valid && k.1 != VEHICLE_KEY_Y) else {
            return self.slots.still_valid(world, guest, self.pos);
        };
        if !self.slots.valid(world) {
            return false;
        }
        let mut valid = self.slots.in_reach(guest, self.pos);
        native_menu_still_valid(
            context,
            guest.uuid,
            &self.registry_id,
            [k.0, k.1, k.2],
            guest.eye(),
            &mut valid,
        );
        valid
    }

    /// A mod menu type's linked slots and the containers behind them.
    fn linked(&self) -> Option<(std::ops::Range<usize>, &[(i32, i32, i32)])> {
        let l = self.mod_type?.links?;
        Some((l.slots(), &self.links))
    }

    /// The linked slots (`links`) no container stands behind, which hold
    /// nothing and take nothing: they are left out of clicks as a hidden
    /// slot is.
    pub fn unlinked_slots(&self, world: &World) -> Vec<usize> {
        let Some((range, links)) = self.linked() else {
            return Vec::new();
        };
        let bound = linked_slot_count(world, links);
        range.skip(bound).collect()
    }

    /// The container slot of a mod menu type's crafting result, and what it shows.
    fn craft_result(&self) -> Option<(usize, Option<ItemStack>)> {
        let c = self.mod_type?.crafting?;
        Some((c.result, self.craft.as_ref().and_then(|c| c.result.clone())))
    }
}

/// What a guest did with a native mod's menu (`Slots::Native`), for the game
/// to tell the mod once the hub is back in its hands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeMenuEvent {
    /// A click (`ServerboundContainerClickPacket`): the menu's slot (its own
    /// first, then the player's 36; -999 outside), the button and the mode.
    Click {
        uuid: u128,
        window: i32,
        slot: i32,
        button: i32,
        mode: i32,
    },
    /// The menu closed: by the player, by the server, another opened, or
    /// the player left.
    Closed { uuid: u128, window: i32 },
    /// The storage a mod backed the menu's slots with was written (the game's notice): its
    /// slots are looked at again once the write is done.
    PlacesChanged { uuid: u128, window: i32 },
}

/// The y of the key a chest minecart's or chest boat's items are kept under
/// while a menu shows them.
pub const VEHICLE_KEY_Y: i32 = -100_000;

/// Where the slots of a container menu live.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slots {
    /// One block entity.
    Block((i32, i32, i32)),
    /// A double chest (`CompoundContainer`): the first half's slots, then
    /// the second's.
    Double((i32, i32, i32), (i32, i32, i32)),
    /// The player's own ender chest inventory (`PlayerEnderChestContainer`).
    Ender,
    /// A native mod's menu (`Guest::native_menu`): items to look at, which no
    /// click moves; the clicks go to the mod.
    Native,
}

/// The concrete storage a container slot uses, in the container's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotBacking {
    Block((i32, i32, i32), usize),
    Ender(usize),
    Native(usize),
}

/// A container slot's stack as the menu holds it, so a tick can compare it
/// with the copy the client was last sent without copying it first.
#[derive(Debug)]
pub(super) enum SlotRef<'a> {
    /// The slot of the menu's own storage (a block entity's, the ender
    /// chest's, a mod's menu, the player's inventory): the stack where it
    /// lies, `None` for an empty slot and for one the menu does not have.
    Borrowed(Option<&'a ItemStack>),
    /// A slot no storage holds as a list (`Menu::craft_result`, a mod menu
    /// type's linked containers), read out and copied to be looked at.
    Owned(Option<ItemStack>),
}

impl SlotRef<'_> {
    /// Whether `sent` (the copy the menu keeps of what the client was last
    /// told, none for a slot it never got) already holds this stack. Both
    /// sides are compared by value: storages hand out a fresh `ItemStack`
    /// every read, its `Box<StackData>` included, so the nested contents of
    /// a shulker or a book are compared too.
    pub(super) fn matches_sent(&self, sent: Option<&Option<ItemStack>>) -> bool {
        match self {
            SlotRef::Borrowed(now) => sent.is_some_and(|sent| sent.as_ref() == *now),
            SlotRef::Owned(now) => sent == Some(now),
        }
    }
    /// The slot's stack, copied if the menu lent it.
    pub(super) fn into_stack(self) -> Option<ItemStack> {
        match self {
            SlotRef::Borrowed(now) => now.cloned(),
            SlotRef::Owned(now) => now,
        }
    }
}

impl Slots {
    /// Optional persisted limits in the container's menu order.
    pub fn stack_limits(self, world: &World, size: usize) -> Vec<Option<u32>> {
        match self {
            Self::Block(pos) => world
                .container_stack_limit(pos)
                .map_or_else(Vec::new, |limit| vec![Some(limit); size]),
            Self::Double(a, b) => {
                let (a, b) = (
                    world.container_stack_limit(a),
                    world.container_stack_limit(b),
                );
                if a.is_none() && b.is_none() {
                    Vec::new()
                } else {
                    [vec![a; 27], vec![b; 27]].concat()
                }
            }
            _ => Vec::new(),
        }
    }

    /// Locate a container slot, including the correct half of a double chest.
    pub fn backing(self, world: &World, n: usize) -> Option<SlotBacking> {
        match self {
            Slots::Block(pos) => {
                (n < shown(world, pos)?.len()).then_some(SlotBacking::Block(pos, n))
            }
            Slots::Double(a, b) => {
                let first = shown(world, a)?.len();
                if n < first {
                    Some(SlotBacking::Block(a, n))
                } else {
                    let n = n - first;
                    (n < shown(world, b)?.len()).then_some(SlotBacking::Block(b, n))
                }
            }
            Slots::Ender => Some(SlotBacking::Ender(n)),
            Slots::Native => Some(SlotBacking::Native(n)),
        }
    }

    /// The guest's open mod menu's linked slots and their containers, when
    /// these are its slots.
    fn links_of(self, guest: &Guest) -> Option<(std::ops::Range<usize>, &[(i32, i32, i32)])> {
        let m = guest.menu.as_ref().filter(|m| m.slots == self)?;
        m.linked()
    }
    fn halves(self) -> Vec<(i32, i32, i32)> {
        match self {
            Slots::Block(k) => vec![k],
            Slots::Double(a, b) => vec![a, b],
            Slots::Ender | Slots::Native => Vec::new(),
        }
    }
    /// Whether the block entities behind the slots are still there
    /// (`Container.stillValid`).
    pub fn valid(self, world: &World) -> bool {
        self.halves()
            .iter()
            .all(|k| world.block_entities.contains_key(k))
    }
    /// How many slots the block entities behind a block menu show now, a missing one none;
    /// none for the ender chest's and a mod's chest menu, which no block holds.
    pub fn block_slots(self, world: &World) -> Option<usize> {
        match self {
            Slots::Ender | Slots::Native => None,
            _ => Some(
                self.halves()
                    .into_iter()
                    .map(|k| shown(world, k).map_or(0, <[_]>::len))
                    .sum(),
            ),
        }
    }
    /// `Container.stillValid` for the guest: the block entities are there
    /// and the guest is within reach of each (`stillValidBlockEntity`). The
    /// ender chest's block is `at` (`PlayerEnderChestContainer.activeChest`).
    pub fn still_valid(self, world: &World, guest: &Guest, at: Option<(i32, i32, i32)>) -> bool {
        self.valid(world) && self.in_reach(guest, at)
    }
    /// The reach half of `still_valid`: the guest is within its block
    /// interaction range plus 4 of each block behind the slots.
    fn in_reach(self, guest: &Guest, at: Option<(i32, i32, i32)>) -> bool {
        let blocks = match self {
            Slots::Ender => at.into_iter().collect(),
            Slots::Native => Vec::new(),
            _ => self.halves(),
        };
        // A vehicle's container is kept under a key, not a block: the game
        // checks its reach to the vehicle (`isChestVehicleStillValid`).
        blocks
            .iter()
            .all(|k| k.1 == VEHICLE_KEY_Y || guest.within_block_range([k.0, k.1, k.2], 4.0))
    }
    /// Container slot `n`, borrowed where the menu's storage can lend it: the
    /// ender chest, a mod's menu and the block entities are, a crafting result
    /// and the linked containers read out and copied. An empty slot and one
    /// the menu does not have are both `Borrowed(None)`, as both are `None`
    /// for [`Self::get`].
    pub(super) fn get_ref<'a>(
        self,
        world: &'a World,
        guest: &'a Guest,
        mut n: usize,
    ) -> SlotRef<'a> {
        if self == Slots::Ender {
            return SlotRef::Borrowed(guest.ender_items.get(n).and_then(|s| s.as_ref()));
        }
        if self == Slots::Native {
            return SlotRef::Borrowed(guest.native_menu.get(n).and_then(|s| s.as_ref()));
        }
        if let Some((slot, result)) = guest.menu.as_ref().and_then(Menu::craft_result)
            && slot == n
        {
            return SlotRef::Owned(result);
        }
        if let Some((range, links)) = self.links_of(guest)
            && range.contains(&n)
        {
            return SlotRef::Owned(
                read_links(world, links, range.len())
                    .into_iter()
                    .nth(n - range.start)
                    .flatten(),
            );
        }
        for k in self.halves() {
            let Some(slots) = shown(world, k) else {
                return SlotRef::Borrowed(None);
            };
            if n < slots.len() {
                return SlotRef::Borrowed(slots[n].as_ref());
            }
            n -= slots.len();
        }
        SlotRef::Borrowed(None)
    }
    /// Container slot `n`.
    pub fn get(self, world: &World, guest: &Guest, n: usize) -> Option<ItemStack> {
        self.get_ref(world, guest, n).into_stack()
    }
    /// Every container slot, in menu order.
    pub fn read(self, world: &World, guest: &Guest) -> Option<Vec<Option<ItemStack>>> {
        if self == Slots::Ender {
            return Some(guest.ender_items.clone());
        }
        if self == Slots::Native {
            return Some(guest.native_menu.clone());
        }
        let mut items = Vec::new();
        for k in self.halves() {
            items.extend_from_slice(shown(world, k)?);
        }
        if let Some((slot, result)) = guest.menu.as_ref().and_then(Menu::craft_result)
            && let Some(item) = items.get_mut(slot)
        {
            *item = result;
        }
        if let Some((range, links)) = self.links_of(guest) {
            for (k, stack) in range.zip(read_links(world, links, usize::MAX)) {
                if let Some(item) = items.get_mut(k) {
                    *item = stack;
                }
            }
        }
        Some(items)
    }
    /// Store what `read` gave, changed.
    pub fn write(self, world: &mut World, guest: &mut Guest, items: Vec<Option<ItemStack>>) {
        if self == Slots::Ender {
            guest.ender_items = items;
            return;
        }
        if self == Slots::Native {
            // Only the mod changes what it shows.
            return;
        }
        let mut items = items;
        // A crafting result is the menu's: the block entity never holds it.
        if let Some((slot, _)) = guest.menu.as_ref().and_then(Menu::craft_result)
            && let Some(item) = items.get_mut(slot)
        {
            *item = None;
        }
        // Linked slots are their containers', never the block entity's.
        if let Some((range, links)) = self.links_of(guest) {
            let linked: Vec<_> = range
                .filter_map(|k| items.get_mut(k).map(Option::take))
                .collect();
            let links = links.to_vec();
            write_links(world, &links, linked);
        }
        let mut rest = items.into_iter();
        for k in self.halves() {
            let n = shown(world, k).map_or(0, <[_]>::len);
            if let Some(e) = world.block_entities.get_mut(&k) {
                for slot in e.slots_mut().iter_mut().take(n) {
                    *slot = rest.next().flatten();
                }
            }
        }
    }
}

/// The slots of the block entity at `k` its menu shows: a mod container's
/// count, though items past it stay saved.
fn shown(world: &World, k: (i32, i32, i32)) -> Option<&[Option<ItemStack>]> {
    let e = world.block_entities.get(&k)?;
    let slots = e.slots();
    if !matches!(e, BlockEntity::Mod(_)) {
        return Some(slots);
    }
    let state = world.block(k.0, k.1, k.2).0;
    let count = world
        .block_entities
        .mod_container(state)
        .map_or(0, |c| c.slots);
    Some(&slots[..count.min(slots.len())])
}

/// How many slots the linked containers `links` have between them.
pub fn linked_slot_count(world: &World, links: &[(i32, i32, i32)]) -> usize {
    links
        .iter()
        .map(|&k| shown(world, k).map_or(0, <[_]>::len))
        .sum()
}

/// The stacks of the linked containers `links`, their slots in order, at
/// most `count`. A container no longer there has no slots.
pub fn read_links(
    world: &World,
    links: &[(i32, i32, i32)],
    count: usize,
) -> Vec<Option<ItemStack>> {
    links
        .iter()
        .flat_map(|&k| shown(world, k).unwrap_or_default().iter().cloned())
        .take(count)
        .collect()
}

/// Stores `stacks` in the linked containers `links`, their slots in order
/// (what [`read_links`] gave, changed). The containers whose items changed,
/// for the game's `setChanged`.
pub fn write_links(
    world: &mut World,
    links: &[(i32, i32, i32)],
    stacks: Vec<Option<ItemStack>>,
) -> Vec<(i32, i32, i32)> {
    let mut rest = stacks.into_iter();
    let mut changed = Vec::new();
    for &k in links {
        let n = shown(world, k).map_or(0, <[_]>::len);
        let Some(e) = world.block_entities.get_mut(&k) else {
            continue;
        };
        let mut moved = false;
        for slot in e.slots_mut().iter_mut().take(n) {
            let Some(stack) = rest.next() else {
                break;
            };
            if *slot != stack {
                *slot = stack;
                moved = true;
            }
        }
        if moved {
            changed.push(k);
        }
    }
    changed
}

/// Slots of a menu seen as one list.
pub struct View<'a> {
    pub guest: &'a mut Guest,
    /// The container's slots, or None for the inventory menu.
    pub container: Option<&'a mut [Option<ItemStack>]>,
    /// The container is a shulker box (`ShulkerBoxSlot`).
    pub shulker: bool,
    pub container_limits: Vec<Option<u32>>,
    /// Container slots that do not exist in this menu (see [`Menu::hidden`]).
    pub hidden: &'a [usize],
    /// The context the hooks fired inside a click are given.
    pub loop_context: &'a LoopContext,
    /// The parts' one copy of the container for this click (`menu_slot_rules`).
    pub slot_snapshot: crate::menu::SlotSnapshot,
    /// The sounds the click played at the player (`BundleItem.playInsertSound`
    /// and the others, `Player.playSound`), in order; the hub sends them.
    pub sounds: Vec<&'static str>,
}

impl View<'_> {
    pub fn len(&self) -> usize {
        match &self.container {
            Some(c) => c.len() + 36,
            None => 46,
        }
    }
    pub fn is_empty(&self) -> bool {
        false
    }
    pub fn get(&self, i: usize) -> Option<ItemStack> {
        match &self.container {
            Some(c) if i < c.len() => c[i].clone(),
            Some(c) => self
                .guest
                .inventory
                .get(player_slot(i - c.len()))
                .cloned()
                .flatten(),
            None => self.guest.menu_slot(i),
        }
    }
    pub fn set(&mut self, i: usize, item: Option<ItemStack>) {
        self.slot_snapshot.clear();
        match &mut self.container {
            Some(c) if i < c.len() => c[i] = item,
            Some(c) => {
                let n = c.len();
                self.guest.inventory[player_slot(i - n)] = item;
            }
            None => self.guest.set_menu_slot(i, item),
        }
    }
    /// Map the flattened server slots to the same containers and indices as prediction.
    fn slot_context(&self, i: usize) -> (usize, crate::menu::SlotContext<'_>) {
        let slots = self.container.as_deref().unwrap_or_default();
        let menu = self.guest.menu.as_ref();
        let crafting = menu
            .and_then(|menu| menu.mod_type)
            .and_then(|menu| menu.crafting);
        let mut context = crate::menu::SlotContext {
            player: Some(self.guest.uuid),
            pos: menu.and_then(|menu| menu.pos).map(|(x, y, z)| [x, y, z]),
            container: crate::menu::lan_container(i, self.container.as_deref()),
            slots,
            snapshot: Some(&self.slot_snapshot),
            ..Default::default()
        };
        let mut index = if i < slots.len() {
            i
        } else {
            player_slot(i - slots.len())
        };
        if let Some(crafting) = crafting {
            let range = crafting.grid_slots();
            context.grid = slots.get(range.clone()).unwrap_or_default();
            context.result = slots
                .get(crafting.result..crafting.result + 1)
                .unwrap_or_default();
            context.main_crafting = Some(crafting);
            if i < slots.len() && range.contains(&i) {
                context.container = mod_api::MenuContainer::Grid;
                index = i - range.start;
            } else if i < slots.len() && i == crafting.result {
                context.container = mod_api::MenuContainer::Result;
                index = 0;
            }
        }
        (index, context)
    }
    /// Whether the slot takes part in clicks at all.
    fn enabled(&self, i: usize) -> bool {
        (self.container.is_some() || i >= 5) && !self.hidden.contains(&i)
    }
    fn vanilla_may_place(&self, i: usize, stack: &ItemStack, rules: &dyn Rules) -> bool {
        if self.container.is_none() && (5..=8).contains(&i) {
            // `ArmorSlot.mayPlace`: `LivingEntity.isEquippableInSlot` for the guest, from the
            // stack's `equippable` component. (The game clicks a guest's inventory menu through
            // `crate::menu`, which also asks the parts of the item.)
            let tags = &rules.enchantments().entity_tags;
            let player = "minecraft:player";
            return equippable_in_slot(rules.items(), tags, stack, ARMOR_SLOTS[i - 5], player);
        }
        !(self.shulker
            && self.container.as_ref().is_some_and(|c| i < c.len())
            && !crate::menu::fits_inside_container_items(self.loop_context, stack))
    }
    fn slot_rules(&self, i: usize, stack: &ItemStack, rules: &dyn Rules) -> (bool, u32) {
        let own = self.vanilla_may_place(i, stack, rules);
        let max = if self.container.is_none() && (5..=8).contains(&i) {
            1
        } else {
            u32::MAX
        };
        let item_max = rules.max_stack_of(stack);
        let Some(menu) = self
            .guest
            .menu
            .as_ref()
            .filter(|_| self.container.is_some())
        else {
            return (own, max.min(item_max));
        };
        let (index, context) = self.slot_context(i);
        crate::menu::apply_menu_slot_rules(
            rules.slot_rules(),
            &menu.registry_id,
            index,
            stack,
            context,
            own,
            max,
            item_max,
        )
    }
    fn may_place(&self, i: usize, stack: &ItemStack, rules: &dyn Rules) -> bool {
        self.enabled(i) && self.slot_rules(i, stack, rules).0
    }
    fn slot_max(&self, i: usize, stack: &ItemStack, rules: &dyn Rules) -> u32 {
        let max = self.slot_rules(i, stack, rules).1;
        self.container_limits
            .get(i)
            .copied()
            .flatten()
            .map_or(max, |limit| max.min(limit))
    }
    /// `Slot.mayPickup`: whether menu slot `i` gives its stack up. The
    /// armour slots refuse while a survival player wears a binding curse
    /// (`ArmorSlot.mayPickup`: `PREVENT_ARMOR_CHANGE` and no creative); the
    /// menu's rules answer for their own slots.
    fn may_pickup(&self, i: usize, rules: &dyn Rules) -> bool {
        self.may_pickup_item(i, self.get(i).as_ref(), rules)
    }
    fn may_pickup_item(&self, i: usize, stack: Option<&ItemStack>, rules: &dyn Rules) -> bool {
        if self.container.is_none()
            && (5..=8).contains(&i)
            && !self.guest.creative()
            && stack.is_some_and(|s| rules.enchantments().prevents_armor_change(s))
        {
            return false;
        }
        stack.is_none_or(|s| rules.may_pickup(i, s))
    }
    /// The ranges quick move uses: (container start..end, main, hotbar).
    fn player_ranges(&self) -> (std::ops::Range<usize>, std::ops::Range<usize>) {
        match &self.container {
            Some(c) => (c.len()..c.len() + 27, c.len() + 27..c.len() + 36),
            None => (9..36, 36..45),
        }
    }
}

struct HostedSlotView<'a, 'v> {
    view: &'a mut View<'v>,
    slot: usize,
    rules: &'a dyn Rules,
    staged: Option<Option<ItemStack>>,
}

impl HostedSlotView<'_, '_> {
    fn set(&mut self, value: Option<ItemStack>) {
        if let Some(staged) = &mut self.staged {
            *staged = value;
        } else {
            self.view.set(self.slot, value);
        }
    }
}

impl crate::menu::SlotView for HostedSlotView<'_, '_> {
    fn with_staged(
        &mut self,
        edit: &mut dyn FnMut(&mut dyn crate::menu::SlotView) -> bool,
    ) -> bool {
        let initial = self.item();
        let previous = self.staged.replace(initial);
        let accepted = edit(self);
        let value = self.staged.take().expect("a staged slot");
        self.staged = previous;
        if accepted {
            self.set(value);
        }
        accepted
    }
    fn items(&self) -> &crate::items::Items {
        self.rules.items()
    }

    fn item(&self) -> Option<ItemStack> {
        self.staged
            .clone()
            .unwrap_or_else(|| self.view.get(self.slot))
    }
    fn may_place(&self, stack: &ItemStack) -> bool {
        self.view.may_place(self.slot, stack, self.rules)
    }
    fn may_pickup(&self) -> bool {
        self.view
            .may_pickup_item(self.slot, self.item().as_ref(), self.rules)
    }
    fn allow_modification(&self) -> bool {
        let item = self
            .item()
            .unwrap_or_else(|| ItemStack::new("minecraft:air", 0));
        self.may_pickup() && self.may_place(&item)
    }
    fn safe_insert(&mut self, mut stack: Option<ItemStack>, count: u32) -> Option<ItemStack> {
        let Some(input) = stack.as_mut() else {
            return stack;
        };
        if !self.may_place(input) {
            return stack;
        }
        let current = self.item();
        let have = current.as_ref().map_or(0, |s| s.count);
        let moved = count.min(input.count).min(
            self.view
                .slot_max(self.slot, input, self.rules)
                .saturating_sub(have),
        );
        if moved == 0 {
            return stack;
        }
        match current {
            None => {
                let mut part = input.clone();
                part.count = moved;
                input.count -= moved;
                self.set(Some(part));
            }
            Some(mut current) if current.same_item_same_components(input) => {
                current.count += moved;
                input.count -= moved;
                self.set(Some(current));
            }
            Some(_) => {}
        }
        if input.count == 0 { None } else { stack }
    }
    fn safe_take(&mut self, count: u32, max: u32) -> Option<ItemStack> {
        if !self.may_pickup() {
            return None;
        }
        let mut current = self.item()?;
        if !self.allow_modification() && max < current.count {
            return None;
        }
        let moved = count.min(max).min(current.count);
        if moved == 0 {
            return None;
        }
        let mut taken = current.clone();
        taken.count = moved;
        current.count -= moved;
        self.set((current.count > 0).then_some(current));
        Some(taken)
    }
}

/// Inventory index of the n-th player slot of a container menu (27 main, then hotbar).
fn player_slot(n: usize) -> usize {
    if n < 27 { 9 + n } else { n - 27 }
}

/// Inventory.add-like merge of `stack` into slots `range` (moveItemStackTo).
fn move_to(
    view: &mut View,
    mut stack: ItemStack,
    range: std::ops::Range<usize>,
    reverse: bool,
    rules: &dyn Rules,
) -> Option<ItemStack> {
    let order: Vec<usize> = if reverse {
        range.rev().collect()
    } else {
        range.collect()
    };
    for &i in &order {
        if stack.count == 0 {
            break;
        }
        if !view.enabled(i) {
            continue;
        }
        if let Some(mut existing) = view.get(i)
            && existing.same_item_same_components(&stack)
        {
            if let Some(menu) = view
                .guest
                .menu
                .as_ref()
                .filter(|_| view.container.is_some())
            {
                let (index, context) = view.slot_context(i);
                if !crate::menu::apply_menu_slot_rules(
                    rules.slot_rules(),
                    &menu.registry_id,
                    index,
                    &stack,
                    context,
                    true,
                    u32::MAX,
                    rules.max_stack_of(&stack),
                )
                .0
                {
                    continue;
                }
            }
            let max = view.slot_max(i, &existing, rules);
            if existing.count < max {
                let moved = (max - existing.count).min(stack.count);
                existing.count += moved;
                stack.count -= moved;
                view.set(i, Some(existing));
            }
        }
    }
    for &i in &order {
        if stack.count == 0 {
            break;
        }
        if view.enabled(i) && view.get(i).is_none() && view.may_place(i, &stack, rules) {
            let moved = stack.count.min(view.slot_max(i, &stack, rules));
            view.set(
                i,
                Some(ItemStack {
                    count: moved,
                    ..stack.clone()
                }),
            );
            stack.count -= moved;
        }
    }
    (stack.count > 0).then_some(stack)
}

fn quick_move(view: &mut View, i: usize, rules: &dyn Rules) {
    let Some(stack) = view.get(i) else {
        return;
    };
    if !view.enabled(i) {
        return;
    }
    let (main, hotbar) = view.player_ranges();
    let rest = if let Some(c) = &view.container {
        let n = c.len();
        if i < n {
            // Container to inventory, filling from the hotbar's end.
            move_to(view, stack, n..n + 36, true, rules)
        } else {
            move_to(view, stack, 0..n, false, rules)
        }
    } else if (5..=8).contains(&i) || i < 5 {
        move_to(view, stack, 9..45, false, rules)
    } else {
        // Wear it, otherwise the stack fills the inventory: from the main
        // slots for the hotbar and the off hand, from the hotbar for the
        // main ones. The off hand is in this arm too, so armour worn there
        // is put on like any other (InventoryMenu.quickMoveStack asks for
        // the item's equipment slot before it looks at the clicked index).
        let worn = crate::mobs::equip::equippable(rules.items(), &stack).map(|e| e.slot);
        let armor = ARMOR_SLOTS
            .iter()
            .position(|slot| worn == Some(*slot))
            .filter(|a| view.get(5 + a).is_none());
        if let Some(a) = armor {
            // `moveItemStackTo` asks `ArmorSlot.mayPlace`: a refusal moves nothing, and the
            // stack doesn't fall through to the inventory.
            if view.may_place(5 + a, &stack, rules) {
                view.set(
                    5 + a,
                    Some(ItemStack {
                        count: 1,
                        ..stack.clone()
                    }),
                );
                let mut rest = stack.clone();
                rest.count -= 1;
                (rest.count > 0).then_some(rest)
            } else {
                Some(stack)
            }
        } else if worn == Some("offhand") && view.get(45).is_none() {
            // A shield or a totem goes to the empty off hand before the
            // hotbar (InventoryMenu.quickMoveStack's OFFHAND branch).
            let max = view.slot_max(45, &stack, rules);
            let moved = stack.count.min(max);
            view.set(
                45,
                Some(ItemStack {
                    count: moved,
                    ..stack.clone()
                }),
            );
            let rest = stack.count - moved;
            (rest > 0).then_some(ItemStack {
                count: rest,
                ..stack.clone()
            })
        } else if hotbar.contains(&i) {
            move_to(view, stack, main, false, rules)
        } else if main.contains(&i) {
            move_to(view, stack, hotbar, false, rules)
        } else {
            move_to(view, stack, 9..45, false, rules)
        }
    };
    view.set(i, rest);
}

/// The click modes of ServerboundContainerClickPacket.
pub mod input {
    pub const PICKUP: i32 = 0;
    pub const QUICK_MOVE: i32 = 1;
    pub const SWAP: i32 = 2;
    pub const CLONE: i32 = 3;
    pub const THROW: i32 = 4;
    pub const QUICK_CRAFT: i32 = 5;
    pub const PICKUP_ALL: i32 = 6;
}

/// `AbstractContainerMenu.isContainerClickButton`: a pickup and a quick move
/// are clicks with the left or the right button only.
fn is_container_click_button(button: i32) -> bool {
    button == 0 || button == 1
}

/// Apply one click; returns the stacks thrown out and the slots changed.
pub fn click(
    view: &mut View,
    drag: &mut Option<(u8, Vec<usize>)>,
    slot: i32,
    button: i32,
    mode: i32,
    rules: &dyn Rules,
) -> (Vec<ItemStack>, Vec<usize>) {
    let mut drops = Vec::new();
    let before: Vec<Option<ItemStack>> = (0..view.len()).map(|i| view.get(i)).collect();
    if mode == input::QUICK_CRAFT {
        quick_craft(view, drag, slot, button, rules);
    } else {
        *drag = None;
        match mode {
            // Invalid container click buttons must not move the slot or cursor.
            input::PICKUP | input::QUICK_MOVE if !is_container_click_button(button) => {}
            input::PICKUP | input::QUICK_MOVE if slot == -999 => {
                if mode == input::PICKUP
                    && let Some(mut carried) = view.guest.carried.take()
                {
                    if button == 1 && carried.count > 1 {
                        let mut one = carried.clone();
                        one.count = 1;
                        carried.count -= 1;
                        view.guest.carried = Some(carried);
                        drops.push(one);
                    } else {
                        drops.push(carried);
                    }
                }
            }
            input::PICKUP => pickup(view, slot, button, rules),
            input::QUICK_MOVE => {
                // doClick calls quickMoveStack again while the clicked slot
                // still holds the item it moved, because moveItemStackTo fills
                // one empty slot per call. move_to here fills every empty slot
                // of its range in one pass and leaves the same remainder, so
                // one call is vanilla's loop: a full inventory keeps its stack,
                // and a stack too big for one slot spreads over several.
                if let Ok(i) = usize::try_from(slot)
                    && i < view.len()
                    && view.may_pickup(i, rules)
                {
                    quick_move(view, i, rules);
                }
            }
            input::SWAP => swap(view, slot, button, rules, &mut drops),
            input::CLONE => {
                if view.guest.carried.is_none()
                    && view.guest.creative()
                    && let Ok(i) = usize::try_from(slot)
                    && let Some(stack) = view.get(i)
                {
                    let max = rules.max_stack_of(&stack);
                    view.guest.carried = Some(ItemStack {
                        count: max,
                        ..stack
                    });
                }
            }
            input::THROW => {
                if rules.can_drop_items()
                    && view.guest.carried.is_none()
                    && let Ok(i) = usize::try_from(slot)
                    && view.enabled(i)
                    // `Slot.safeTake`: a slot that withholds its stack throws
                    // nothing, whatever the client asked for.
                    && view.may_pickup(i, rules)
                    && let Some(mut stack) = view.get(i)
                {
                    // doClick's THROW takes one item for button 0 and the
                    // whole stack for any other button.
                    let amount = if button == 0 { 1 } else { stack.count };
                    if amount >= stack.count {
                        drops.push(stack);
                        view.set(i, None);
                    } else {
                        let mut one = stack.clone();
                        one.count = amount;
                        stack.count -= amount;
                        view.set(i, Some(stack));
                        drops.push(one);
                    }
                }
            }
            input::PICKUP_ALL => pickup_all(view, slot, button, rules),
            _ => {}
        }
    }
    let changed = (0..view.len())
        .filter(|&i| view.get(i) != before[i])
        .collect();
    (drops, changed)
}

fn pickup(view: &mut View, slot: i32, button: i32, rules: &dyn Rules) {
    let Ok(i) = usize::try_from(slot) else {
        return;
    };
    if i >= view.len() || !view.enabled(i) {
        return;
    }
    let right = button == 1;
    let mut canceled = false;
    let mut handled = false;
    if crate::native_client_hooks::has_mods() {
        let (container, container_slot) = match &view.container {
            Some(c) if i < c.len() => (crate::menu::MAIN, i),
            Some(c) => (crate::menu::PLAYER, player_slot(i - c.len())),
            None => (
                crate::menu::PLAYER,
                match i {
                    5..=8 => 44 - i,
                    9..=35 => i,
                    36..=44 => i - 36,
                    45 => 40,
                    _ => i,
                },
            ),
        };
        let context = view.loop_context;
        let player = view.guest.uuid;
        let player_entity_id = view.guest.entity_id;
        let mut carried = view.guest.carried.take();
        let mut slot_view = HostedSlotView {
            view,
            slot: i,
            rules,
            staged: None,
        };
        let mut event = crate::menu::StackedOnOther {
            player: Some(player),
            player_entity_id: Some(player_entity_id),
            slot: i,
            container,
            container_slot,
            primary: !right,
            carried: crate::menu::CarriedAccess(&mut carried),
            clicked: &mut slot_view,
            cancellation_result: false,
        };
        crate::menu::native_item_stacked_on_other(context, &mut event, &mut canceled);
        handled = event.cancellation_result;
        view.guest.carried = carried;
    }
    let handled = if canceled {
        handled
    } else {
        bundle_click(view, i, right, rules)
    };
    if handled {
        return;
    }
    let (slot_item, carried) = (view.get(i), view.guest.carried.clone());
    match (slot_item, carried) {
        (None, None) => {}
        (None, Some(mut c)) => {
            let (may_place, max) = view.slot_rules(i, &c, rules);
            let max = view
                .container_limits
                .get(i)
                .copied()
                .flatten()
                .map_or(max, |limit| max.min(limit));
            if view.enabled(i) && may_place {
                let amount = if right { 1 } else { c.count }.min(max);
                view.set(
                    i,
                    Some(ItemStack {
                        count: amount,
                        ..c.clone()
                    }),
                );
                c.count -= amount;
                view.guest.carried = (c.count > 0).then_some(c);
            }
        }
        (Some(mut s), None) => {
            // doClick's PICKUP arm takes from the slot only while
            // `Slot.mayPickup` allows it.
            if !view.may_pickup(i, rules) {
                return;
            }
            let take = if right { s.count.div_ceil(2) } else { s.count };
            let taken = ItemStack {
                count: take,
                ..s.clone()
            };
            s.count -= take;
            view.set(i, (s.count > 0).then_some(s));
            view.guest.carried = Some(taken);
        }
        (Some(mut s), Some(mut c)) => {
            // Merging into, swapping with and topping the hand up from the
            // slot are all takes (`allowModification`: mayPickup and
            // mayPlace).
            if !view.may_pickup(i, rules) {
                return;
            }
            if !view.may_place(i, &c, rules) {
                // A slot that takes nothing gives onto the same carried item,
                // as far as the hand holds (`tryRemove`).
                if s.same_item_same_components(&c) {
                    let max = rules.max_stack_of(&c).saturating_sub(c.count);
                    let mut slot = HostedSlotView {
                        view,
                        slot: i,
                        rules,
                        staged: None,
                    };
                    if let Some(taken) = crate::menu::SlotView::safe_take(&mut slot, s.count, max) {
                        c.count += taken.count;
                        slot.view.guest.carried = Some(c);
                    }
                }
                return;
            }
            if s.same_item_same_components(&c) {
                let max = view.slot_max(i, &c, rules);
                let moved = if right { 1 } else { c.count }.min(max.saturating_sub(s.count));
                s.count += moved;
                c.count -= moved;
                view.set(i, Some(s));
                view.guest.carried = (c.count > 0).then_some(c);
            } else if c.count <= view.slot_max(i, &c, rules) {
                view.set(i, Some(c));
                view.guest.carried = Some(s);
            }
        }
    }
}

/// `tryItemClickBehaviourOverride`: the carried stack's
/// `BundleItem.overrideStackedOnOther`, then the clicked one's
/// `overrideOtherStackedOnMe`. True when a bundle took the click.
fn bundle_click(view: &mut View, i: usize, right: bool, rules: &dyn Rules) -> bool {
    use crate::menu::container_items::{can_be_in_bundle, fits_inside_container_items};
    let items = rules.items();
    let context = view.loop_context;
    let other = view.get(i);
    if let Some(mut carried) = view.guest.carried.clone()
        && let Some(mut contents) = Contents::of(items, &carried)
        && other.is_some() != right
    {
        match other {
            // tryTransfer: as much as fits, through `safeTake`.
            Some(other) => {
                let mut moved = 0;
                if can_be_in_bundle(context, &other) {
                    let amount = other.count.min(contents.room_for(items, &other));
                    let mut slot = HostedSlotView {
                        view,
                        slot: i,
                        rules,
                        staged: None,
                    };
                    if let Some(mut taken) =
                        crate::menu::SlotView::safe_take(&mut slot, other.count, amount)
                    {
                        // `tryInsert` asks `canItemBeInBundle` again: the answer above.
                        moved = contents.try_insert(items, &mut taken, true);
                    }
                }
                // `tryTransfer(...) > 0` ? playInsertSound : playInsertFailSound.
                view.sounds
                    .push(if moved > 0 { INSERT } else { INSERT_FAIL });
            }
            // removeOne into the empty slot (`safeInsert`); the rest goes back.
            None => {
                if let Some(mut stack) = contents.remove_one(items) {
                    if view.may_place(i, &stack, rules) {
                        let amount = stack.count.min(view.slot_max(i, &stack, rules));
                        view.set(
                            i,
                            Some(ItemStack {
                                count: amount,
                                ..stack.clone()
                            }),
                        );
                        stack.count -= amount;
                    }
                    if stack.count > 0 {
                        let fits = fits_inside_container_items(context, &stack);
                        contents.try_insert(items, &mut stack, fits);
                    } else {
                        // All of it went in: playRemoveOneSound.
                        view.sounds.push(REMOVE_ONE);
                    }
                }
            }
        }
        contents.store(&mut carried);
        view.guest.carried = Some(carried);
        return true;
    }
    let Some(mut bundle) = other else {
        return false;
    };
    let Some(mut contents) = Contents::of(items, &bundle) else {
        return false;
    };
    // `Slot.allowModification`: `mayPickup` and `mayPlace` of the stack it
    // holds. A slot that withholds the bundle keeps what is inside it.
    let allowed = view.may_pickup(i, rules) && view.may_place(i, &bundle, rules);
    match (view.guest.carried.take(), right) {
        (Some(mut carried), false) => {
            let moved = if allowed {
                let fits = fits_inside_container_items(context, &carried);
                contents.try_insert(items, &mut carried, fits)
            } else {
                0
            };
            view.sounds
                .push(if moved > 0 { INSERT } else { INSERT_FAIL });
            view.guest.carried = (carried.count > 0).then_some(carried);
        }
        (None, true) => {
            if allowed && let Some(removed) = contents.remove_one(items) {
                view.sounds.push(REMOVE_ONE);
                view.guest.carried = Some(removed);
            }
        }
        (carried, _) => {
            // toggleSelectedItem(-1), then the click goes on.
            view.guest.carried = carried;
            toggle_selected(&mut bundle, -1);
            view.set(i, Some(bundle));
            return false;
        }
    }
    contents.store(&mut bundle);
    view.set(i, Some(bundle));
    true
}

fn swap(view: &mut View, slot: i32, button: i32, rules: &dyn Rules, drops: &mut Vec<ItemStack>) {
    let Ok(i) = usize::try_from(slot) else {
        return;
    };
    if i >= view.len() || !view.enabled(i) {
        return;
    }
    // Hotbar slot `button` (0..9) or the off hand (40). A chest has no
    // off-hand slot, so F swaps with `Guest.offhand` (Inventory slot 40).
    if button == 40 && view.container.is_some() {
        let (clicked, off) = (view.get(i), view.guest.offhand.clone());
        // The stack coming out of the slot is a take like any other, and the
        // one going in must fit.
        if (clicked.is_some() && !view.may_pickup(i, rules))
            || off
                .as_ref()
                .is_some_and(|off| !view.may_place(i, off, rules))
        {
            return;
        }
        if let Some(mut source) = off.clone()
            && source.count > view.slot_max(i, &source, rules)
        {
            let max = view.slot_max(i, &source, rules);
            let mut part = source.clone();
            part.count = max;
            source.count -= max;
            view.set(i, Some(part));
            view.guest.offhand = Some(source);
            if let Some(taken) = clicked {
                add_to_inventory(view, taken, rules, drops);
            }
        } else {
            view.set(i, off);
            view.guest.offhand = clicked;
        }
        return;
    }
    let other = match button {
        0..=8 => match &view.container {
            Some(c) => c.len() + 27 + button as usize,
            None => 36 + button as usize,
        },
        40 => 45,
        _ => return,
    };
    if other == i {
        return;
    }
    let (a, b) = (view.get(i), view.get(other));
    // `doClick`'s SWAP arm asks the clicked slot before anything moves: the
    // stack it holds only comes out while `Slot.mayPickup` allows it, and the
    // hotbar's stack only goes in where the slot takes it.
    if (a.is_some() && !view.may_pickup(i, rules))
        || b.as_ref().is_some_and(|b| !view.may_place(i, b, rules))
    {
        return;
    }
    if let Some(mut source) = b.clone()
        && source.count > view.slot_max(i, &source, rules)
    {
        let max = view.slot_max(i, &source, rules);
        let mut part = source.clone();
        part.count = max;
        source.count -= max;
        view.set(i, Some(part));
        view.set(other, Some(source));
        if let Some(taken) = a {
            add_to_inventory(view, taken, rules, drops);
        }
    } else {
        view.set(i, b);
        view.set(other, a);
    }
}

fn add_to_inventory(
    view: &mut View,
    mut stack: ItemStack,
    rules: &dyn Rules,
    drops: &mut Vec<ItemStack>,
) {
    let mut inventory = std::mem::take(&mut view.guest.inventory);
    inventory.resize(41, None);
    inventory[40] = view.guest.offhand.take();
    let added = crate::menu::inventory_add(
        &mut inventory,
        view.guest.selected,
        view.guest.creative(),
        rules.items(),
        &mut stack,
    );
    view.guest.offhand = inventory[40].take();
    inventory.truncate(36);
    view.guest.inventory = inventory;
    if !added && stack.count > 0 {
        drops.push(stack);
    }
}

fn pickup_all(view: &mut View, slot: i32, button: i32, rules: &dyn Rules) {
    let Ok(i) = usize::try_from(slot) else {
        return;
    };
    if i >= view.len() {
        return;
    }
    let Some(mut carried) = view.guest.carried.clone() else {
        return;
    };
    // doClick returns when the clicked slot holds a stack it may pick up, so
    // a double-click collects from an empty slot or one that refuses.
    if view.get(i).is_some() && view.may_pickup(i, rules) {
        return;
    }
    let max = rules.max_stack_of(&carried);
    // Button 0 walks the slots from the first, button 1 from the last.
    let order: Vec<usize> = if button == 0 {
        (0..view.len()).collect()
    } else {
        (0..view.len()).rev().collect()
    };
    // Partial stacks first, then full ones.
    for full in [false, true] {
        for &i in &order {
            if carried.count >= max {
                break;
            }
            if !view.enabled(i) || (view.container.is_none() && i == 0) {
                continue;
            }
            // Each swept slot must give its stack up too.
            if !view.may_pickup(i, rules) {
                continue;
            }
            let Some(s) = view.get(i) else {
                continue;
            };
            if !s.same_item_same_components(&carried) || (s.count == max) != full {
                continue;
            }
            let mut slot = HostedSlotView {
                view,
                slot: i,
                rules,
                staged: None,
            };
            if let Some(taken) =
                crate::menu::SlotView::safe_take(&mut slot, s.count, max - carried.count)
            {
                carried.count += taken.count;
            }
        }
    }
    view.guest.carried = Some(carried);
}

/// Dragging a stack over slots (QUICK_CRAFT): start, add slots, end.
fn quick_craft(
    view: &mut View,
    drag: &mut Option<(u8, Vec<usize>)>,
    slot: i32,
    button: i32,
    rules: &dyn Rules,
) {
    let (stage, kind) = (button & 3, ((button >> 2) & 3) as u8);
    match stage {
        0 => {
            // `getQuickcraftType` and `isValidQuickcraftType`: a charitable
            // (0) or greedy (1) drag is anyone's, a clone drag (2) needs
            // infinite materials, and 3 is no type at all. A start that is
            // not legal, or that arrives while a drag is open, leaves the
            // fields reset instead of collecting slots.
            let usable = match kind {
                0 | 1 => true,
                2 => view.guest.creative(),
                _ => false,
            };
            *drag = (usable && drag.is_none() && view.guest.carried.is_some())
                .then_some((kind, Vec::new()));
        }
        1 => {
            if let (Some((_, slots)), Ok(i)) = (drag.as_mut(), usize::try_from(slot))
                && i < view.len()
                && !slots.contains(&i)
                && let Some(c) = &view.guest.carried
                && view.enabled(i)
                && view.may_place(i, c, rules)
                && view.get(i).is_none_or(|s| s.same_item_same_components(c))
            {
                slots.push(i);
            }
        }
        _ => {
            let Some((kind, slots)) = drag.take() else {
                return;
            };
            let Some(carried) = view.guest.carried.clone() else {
                return;
            };
            if slots.is_empty() {
                return;
            }
            if slots.len() == 1 {
                // A single slot behaves like a plain click.
                if kind <= 1 {
                    pickup(view, slots[0] as i32, i32::from(kind), rules);
                }
                return;
            }
            let mut remaining = i64::from(carried.count);
            let item_max = rules.max_stack_of(&carried);
            let count = slots.len();
            for i in slots {
                if !view.enabled(i)
                    || !view.may_place(i, &carried, rules)
                    || (kind != 2 && (carried.count as usize) < count)
                    || view.get(i).is_some_and(|s| {
                        !s.same_item_same_components(&carried) || s.count > item_max
                    })
                {
                    continue;
                }
                let existing = view.get(i).map_or(0, |s| s.count);
                let max = item_max.min(view.slot_max(i, &carried, rules));
                let new_count = crate::menu::quick_craft_count(
                    kind,
                    carried.count,
                    count,
                    existing,
                    max,
                    item_max,
                );
                remaining -= i64::from(new_count) - i64::from(existing);
                view.set(
                    i,
                    Some(ItemStack {
                        count: new_count,
                        ..carried.clone()
                    }),
                );
            }
            let mut carried = carried;
            carried.count = remaining.max(0) as u32;
            view.guest.carried = (carried.count > 0).then_some(carried);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::lan::{Joined, PlayerSender};
    use horizon_core::nbt::Tag;

    /// The test enchantments: Curse of Binding, and a data pack's `test:clinging` with its
    /// `prevent_armor_change` effect.
    fn enchantments() -> &'static Enchantments {
        static ENCHANTMENTS: std::sync::LazyLock<Enchantments> =
            std::sync::LazyLock::new(crate::enchant::tests::sample);
        &ENCHANTMENTS
    }
    use horizon_protocol::packets::configuration::ClientInformation;
    use horizon_world::loot::PackOverlay;
    use std::sync::{Arc, atomic::AtomicI32, mpsc};

    struct ItemRules(Items);
    impl Rules for ItemRules {
        fn items(&self) -> &Items {
            &self.0
        }
        fn enchantments(&self) -> &Enchantments {
            enchantments()
        }
        fn max_stack(&self, item: &str) -> u32 {
            self.0.get(item).map_or(64, |i| i.max_stack)
        }
    }

    pub(crate) fn guest() -> Guest {
        let (tx, _rx) = mpsc::channel();
        Guest::new(
            Joined {
                conn: 2,
                uuid: 2,
                name: "Alex".into(),
                properties: Vec::new(),
                info: ClientInformation::new(8),
                brand: "test".into(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:25565".parse().unwrap(),
                expected: false,
            },
            2,
            [0.0, 64.0, 0.0],
            8,
        )
    }

    /// The prepared resources' items, or None (the test is skipped) without them.
    fn rules() -> Option<ItemRules> {
        let root = crate::menu::tests::resource_root();
        if !root.join("items.json").exists() {
            eprintln!("skipping resource-backed menu check: {}", root.display());
            return None;
        }
        let registry = horizon_world::level::data::load_registry(&root).unwrap();
        let facts = crate::blocks::BlockFacts::load(&root, &registry).unwrap();
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        Some(ItemRules(
            Items::load(&root, &registry, &facts, &PackOverlay::NONE, &defaults).unwrap(),
        ))
    }

    fn contents(stack: &Option<ItemStack>) -> Vec<(String, u32)> {
        let inside = stack.as_ref().and_then(|s| s.data());
        inside.map_or(Vec::new(), |d| {
            d.bundle_contents
                .iter()
                .map(|s| (s.item.to_string(), s.count))
                .collect()
        })
    }

    #[test]
    fn hosted_slot_view_respects_limits_and_transfers() {
        use crate::menu::SlotView;
        let Some(rules) = rules() else { return };
        let mut guest = guest();
        let mut chest = vec![Some(ItemStack::new("minecraft:stone", 60))];
        let mut view = View {
            guest: &mut guest,
            container: Some(&mut chest),
            shulker: false,
            container_limits: Vec::new(),
            hidden: &[],
            sounds: Vec::new(),
            slot_snapshot: Default::default(),
            loop_context: &crate::test_resources::loop_context(
                horizon_world::native_load_context::LoopSide::Server,
            ),
        };
        let mut slot = HostedSlotView {
            view: &mut view,
            slot: 0,
            rules: &rules,
            staged: None,
        };
        let rest = slot
            .safe_insert(Some(ItemStack::new("minecraft:stone", 10)), 10)
            .unwrap();
        assert_eq!(rest.count, 6);
        assert_eq!(slot.item().unwrap().count, 64);
        assert_eq!(slot.safe_take(8, 3).unwrap().count, 3);
        assert_eq!(slot.item().unwrap().count, 61);
    }

    #[test]
    fn invalid_pickup_button_leaves_chest_and_carried_unchanged() {
        let Some(rules) = rules() else { return };
        let mut g = guest();
        let mut chest = vec![Some(ItemStack::new("minecraft:stone", 10))];
        g.carried = Some(ItemStack::new("minecraft:bundle", 1));
        let mut view = View {
            guest: &mut g,
            container: Some(&mut chest),
            shulker: false,
            container_limits: Vec::new(),
            hidden: &[],
            sounds: Vec::new(),
            slot_snapshot: Default::default(),
            loop_context: &crate::test_resources::loop_context(
                horizon_world::native_load_context::LoopSide::Server,
            ),
        };
        let mut drag = None;
        let (drops, changed) = click(&mut view, &mut drag, 0, 2, input::PICKUP, &rules);
        assert!(drops.is_empty() && changed.is_empty());
        assert_eq!(view.get(0).unwrap().count, 10);
        assert_eq!(
            view.guest.carried.as_ref().unwrap().item.as_ref(),
            "minecraft:bundle"
        );
        let mut creative_drag = Some((2, vec![0]));
        let (drops, changed) = click(
            &mut view,
            &mut creative_drag,
            0,
            10,
            input::QUICK_CRAFT,
            &rules,
        );
        assert!(drops.is_empty() && changed.is_empty());
        assert_eq!(view.get(0).unwrap().count, 10);
    }

    /// A double-click's collect walks the slots in the direction the button
    /// picks: `doClick`'s PICKUP_ALL arm starts at slot 0 with a step of +1 for
    /// button 0, and at the last slot with -1 for button 1, which is the
    /// button the screen sends for a double-click (`mouseReleased`).
    #[test]
    fn pickup_all_walks_from_the_buttons_side() {
        let rules = SwapRules;
        let mut g = guest();
        let mut chest = vec![None; 5];
        chest[0] = Some(ItemStack::new("minecraft:stone", 5));
        chest[1] = Some(ItemStack::new("minecraft:stone", 5));
        let mut drag = None;
        let mut view = View {
            guest: &mut g,
            container: Some(&mut chest),
            shulker: false,
            container_limits: Vec::new(),
            hidden: &[],
            sounds: Vec::new(),
            slot_snapshot: Default::default(),
            loop_context: &crate::test_resources::loop_context(
                horizon_world::native_load_context::LoopSide::Server,
            ),
        };
        // `doClick` only starts the collect when the carried stack is not
        // empty and the clicked slot has no item or may not be taken
        // (`!carried.isEmpty() && (!slot.hasItem() || !slot.mayPickup(player))`),
        // so the click lands on an empty slot and the sweep picks the side.
        // One stone short of a full stack: the right double-click takes it
        // from the last matching slot.
        view.guest.carried = Some(ItemStack::new("minecraft:stone", 63));
        click(&mut view, &mut drag, 2, 1, input::PICKUP_ALL, &rules);
        assert_eq!(view.get(0).map(|s| s.count), Some(5));
        assert_eq!(view.get(1).map(|s| s.count), Some(4));
        assert_eq!(view.guest.carried.as_ref().map(|s| s.count), Some(64));
        // The left one takes it from the first.
        view.guest.carried = Some(ItemStack::new("minecraft:stone", 63));
        click(&mut view, &mut drag, 2, 0, input::PICKUP_ALL, &rules);
        assert_eq!(view.get(0).map(|s| s.count), Some(4));
        assert_eq!(view.get(1).map(|s| s.count), Some(4));
        assert_eq!(view.guest.carried.as_ref().map(|s| s.count), Some(64));
    }

    /// Stone with its own `max_stack_size`, or with the component removed
    /// (`!minecraft:max_stack_size`, which vanilla reads as `getOrDefault`'s
    /// default of one).
    fn stone(size: Option<i32>, count: u32) -> ItemStack {
        let mut stack = ItemStack::new("minecraft:stone", count);
        let data = stack.data_mut();
        match size {
            Some(n) => data.set_other(
                "minecraft:max_stack_size",
                Some(horizon_core::nbt::Tag::Int(n)),
            ),
            None => data.set_other(
                "!minecraft:max_stack_size",
                Some(horizon_core::nbt::Tag::Compound(Default::default())),
            ),
        };
        stack.tidy();
        stack
    }

    /// `Slot.getMaxStackSize(stack)` in a guest's window is
    /// `min(container, ItemStack.getMaxStackSize)`: a stack's own
    /// `max_stack_size` caps the merge, and a removed one leaves one per slot.
    #[test]
    fn guest_slot_merges_stop_at_the_stacks_own_size() {
        let Some(rules) = rules() else {
            return;
        };
        let mut g = guest();
        let mut chest = vec![Some(stone(Some(4), 2)), Some(stone(None, 1))];
        let mut drag = None;
        let mut view = View {
            guest: &mut g,
            container: Some(&mut chest),
            shulker: false,
            container_limits: Vec::new(),
            hidden: &[],
            sounds: Vec::new(),
            slot_snapshot: Default::default(),
            loop_context: &crate::test_resources::loop_context(
                horizon_world::native_load_context::LoopSide::Server,
            ),
        };
        // The slot takes four, two are there: two of the five in hand go in.
        view.guest.carried = Some(stone(Some(4), 5));
        click(&mut view, &mut drag, 0, 0, input::PICKUP, &rules);
        assert_eq!(view.get(0).map(|s| s.count), Some(4));
        assert_eq!(view.guest.carried.as_ref().map(|s| s.count), Some(3));
        // A removed component holds one, so nothing merges into it.
        view.guest.carried = Some(stone(None, 1));
        click(&mut view, &mut drag, 1, 0, input::PICKUP, &rules);
        assert_eq!(view.get(1).map(|s| s.count), Some(1));
        assert_eq!(view.guest.carried.as_ref().map(|s| s.count), Some(1));
    }

    /// A guest's chest clicks run `BundleItem`'s overrides: a carried
    /// bundle takes a slot's stack and drops its first one into an empty
    /// slot; a stack clicked onto a bundle goes in, as far as it fits.
    #[test]
    fn guest_chest_clicks_fill_and_empty_bundles() {
        let Some(rules) = rules() else {
            return;
        };
        let mut g = guest();
        let mut chest = vec![
            Some(ItemStack::new("minecraft:stone", 48)),
            None,
            Some(ItemStack::new("minecraft:bundle", 1)),
        ];
        let mut drag = None;
        let mut view = View {
            guest: &mut g,
            container: Some(&mut chest),
            shulker: false,
            container_limits: Vec::new(),
            hidden: &[],
            sounds: Vec::new(),
            slot_snapshot: Default::default(),
            loop_context: &crate::test_resources::loop_context(
                horizon_world::native_load_context::LoopSide::Server,
            ),
        };
        view.guest.carried = Some(ItemStack::new("minecraft:bundle", 1));
        // Left click with the bundle: the stone goes in.
        click(&mut view, &mut drag, 0, 0, input::PICKUP, &rules);
        assert_eq!(view.get(0), None);
        assert_eq!(
            contents(&view.guest.carried),
            [("minecraft:stone".into(), 48)]
        );
        // Right click on an empty slot: the stone comes back out.
        click(&mut view, &mut drag, 1, 1, input::PICKUP, &rules);
        assert_eq!(view.get(1).map(|s| s.count), Some(48));
        assert_eq!(contents(&view.guest.carried), []);
        // Ender pearls (1/16 each) onto the bundle in the chest: 16 fit.
        view.guest.carried = Some(ItemStack::new("minecraft:ender_pearl", 16));
        click(&mut view, &mut drag, 2, 0, input::PICKUP, &rules);
        assert_eq!(view.guest.carried, None);
        let bundle = view.get(2);
        assert_eq!(contents(&bundle), [("minecraft:ender_pearl".into(), 16)]);
        // Full: more stone stays in hand.
        view.guest.carried = Some(ItemStack::new("minecraft:stone", 5));
        click(&mut view, &mut drag, 2, 0, input::PICKUP, &rules);
        assert_eq!(view.guest.carried.as_ref().map(|s| s.count), Some(5));
        // Right click on it with an empty hand takes the pearls out.
        view.guest.carried = None;
        click(&mut view, &mut drag, 2, 1, input::PICKUP, &rules);
        assert_eq!(
            view.guest.carried.as_ref().map(|s| (&*s.item, s.count)),
            Some(("minecraft:ender_pearl", 16))
        );
        assert_eq!(contents(&view.get(2)), []);
    }

    /// `Slot.allowModification`: `BundleItem.overrideOtherStackedOnMe` asks
    /// for `mayPickup` and `mayPlace` of the bundle it holds before it
    /// inserts or removes anything, so a slot whose rules refuse the pickup
    /// keeps its bundle's contents.
    #[test]
    fn guest_bundle_keeps_its_contents_in_a_refusing_slot() {
        /// The prepared catalog, with a menu that refuses one slot.
        struct Refusing {
            items: Items,
        }
        impl Rules for Refusing {
            fn items(&self) -> &Items {
                &self.items
            }
            fn enchantments(&self) -> &Enchantments {
                enchantments()
            }
            fn max_stack(&self, item: &str) -> u32 {
                self.items.get(item).map_or(64, |i| i.max_stack)
            }
            fn may_pickup(&self, slot: usize, _: &ItemStack) -> bool {
                slot != 1
            }
        }
        let Some(ItemRules(items)) = rules() else {
            return;
        };
        let rules = Refusing { items };
        let mut g = guest();
        let mut bundle = ItemStack::new("minecraft:bundle", 1);
        bundle.data_mut().bundle_contents = vec![ItemStack::new("minecraft:apple", 16)];
        let mut chest = vec![Some(ItemStack::new("minecraft:bundle", 1)), Some(bundle)];
        let mut drag = None;
        let mut view = View {
            guest: &mut g,
            container: Some(&mut chest),
            shulker: false,
            container_limits: Vec::new(),
            hidden: &[],
            sounds: Vec::new(),
            slot_snapshot: Default::default(),
            loop_context: crate::test_resources::server_loop_context(),
        };
        // Left click with stone in hand over the refused bundle: nothing goes in.
        view.guest.carried = Some(ItemStack::new("minecraft:stone", 5));
        let (_, changed) = click(&mut view, &mut drag, 1, 0, input::PICKUP, &rules);
        assert!(changed.is_empty());
        assert_eq!(view.guest.carried.as_ref().map(|s| s.count), Some(5));
        assert_eq!(contents(&view.get(1)), [("minecraft:apple".into(), 16)]);
        // A right click with an empty hand takes nothing out of it either.
        view.guest.carried = None;
        let (_, changed) = click(&mut view, &mut drag, 1, 1, input::PICKUP, &rules);
        assert!(changed.is_empty());
        assert!(view.guest.carried.is_none());
        assert_eq!(contents(&view.get(1)), [("minecraft:apple".into(), 16)]);
        // The empty bundle in the slot next to it does take the stone.
        view.guest.carried = Some(ItemStack::new("minecraft:stone", 5));
        click(&mut view, &mut drag, 0, 0, input::PICKUP, &rules);
        assert_eq!(view.guest.carried, None);
        assert_eq!(contents(&view.get(0)), [("minecraft:stone".into(), 5)]);
    }

    #[test]
    fn carried_bundle_uses_safe_take_for_refused_and_allowed_slots() {
        let Some(ItemRules(items)) = rules() else {
            return;
        };
        let mut guest = guest();
        let mut chest = vec![Some(ItemStack::new("minecraft:stone", 8))];
        let mut view = View {
            guest: &mut guest,
            container: Some(&mut chest),
            shulker: false,
            container_limits: Vec::new(),
            hidden: &[],
            sounds: Vec::new(),
            slot_snapshot: Default::default(),
            loop_context: crate::test_resources::server_loop_context(),
        };
        let mut bundle = ItemStack::new("minecraft:bundle", 1);
        bundle.data_mut().bundle_contents = vec![ItemStack::new("minecraft:stone", 60)];
        view.guest.carried = Some(bundle);
        let refused = StrictRules {
            items: items.clone(),
            refuse: &[0],
            drops: true,
        };
        bundle_click(&mut view, 0, false, &refused);
        assert_eq!(view.get(0).unwrap().count, 8);
        assert_eq!(
            contents(&view.guest.carried),
            [("minecraft:stone".into(), 60)]
        );
        let allowed = ItemRules(items);
        bundle_click(&mut view, 0, false, &allowed);
        assert_eq!(view.get(0).unwrap().count, 4);
        assert_eq!(
            contents(&view.guest.carried),
            [("minecraft:stone".into(), 64)]
        );
    }

    /// A mod menu type's crafting result is the menu's: the guest is shown
    /// what the game made, never what the block entity holds there, and a
    /// write leaves the block entity's slot empty.
    #[test]
    fn a_mod_crafting_result_is_shown_from_the_menu() {
        use crate::containers::{ModBlockEntityFields, ModContainer, ModCrafting, ModMenuType};
        let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
        let container = ModContainer {
            slots: 5,
            ..Default::default()
        };
        world
            .block_entities
            .set_mod_containers([(0, Arc::new(container))].into_iter().collect());
        let k = (0, 64, 0);
        let mut items = vec![None; 5];
        items[1] = Some(ItemStack::new("minecraft:oak_log", 1));
        world.block_entities.insert(
            k,
            BlockEntity::Mod(Box::new(crate::containers::ModBlockEntity::new(
                "test:station".into(),
                ModBlockEntityFields {
                    items,
                    ..Default::default()
                },
            ))),
        );
        let t: &'static ModMenuType = Box::leak(Box::new(ModMenuType {
            id: "test:station",
            slots: 5,
            data_slots: 0,
            results: vec![],
            links: None,
            crafting: Some(ModCrafting {
                grid: 1,
                size: 2,
                result: 0,
            }),
        }));
        let planks = Some(ItemStack::new("minecraft:oak_planks", 4));
        let mut g = guest();
        let slots = Slots::Block(k);
        g.menu = Some(Menu {
            window: 1,
            kind: 0,
            pos: Some(k),
            slots,
            size: 5,
            state: 0,
            sent: Vec::new(),
            drag: None,
            sent_data: Vec::new(),
            hidden: Vec::new(),
            mod_type: Some(t),
            registry_id: "test:station".into(),
            craft: None,
            links: Vec::new(),
            asks_still_valid: false,
        });
        // Nothing made yet: nothing shown.
        assert_eq!(slots.get(&world, &g, 0), None);
        if let Some(m) = g.menu.as_mut() {
            m.craft = Some(ModCraft {
                grid: Vec::new(),
                result: planks.clone(),
            });
        }
        assert_eq!(slots.get(&world, &g, 0), planks);
        let mut read = slots.read(&world, &g).unwrap();
        assert_eq!((&read[0], read[1].is_some()), (&planks, true));
        {
            use mod_api::MenuContainer::{Grid, Main, Player, Result};
            let mut view = View {
                guest: &mut g,
                container: Some(&mut read),
                shulker: false,
                container_limits: Vec::new(),
                hidden: &[],
                sounds: Vec::new(),
                slot_snapshot: Default::default(),
                loop_context: crate::test_resources::server_loop_context(),
            };
            let (index, context) = view.slot_context(2);
            assert_eq!((index, context.container), (1, Grid));
            assert_eq!(
                context.slot(Grid, 0).unwrap().item.as_ref(),
                "minecraft:oak_log"
            );
            assert_eq!(context.slot(Result, 0).unwrap().count, 4);
            assert!(context.slot(Main, 1).is_none());
            assert!(context.slot(Main, 0).is_none());
            let (index, context) = view.slot_context(0);
            assert_eq!((index, context.container), (0, Result));
            let (index, context) = view.slot_context(5);
            assert_eq!((index, context.container), (9, Player));
            let builds = std::cell::Cell::new(0);
            let build = || {
                builds.set(builds.get() + 1);
                mod_api::MenuSlotContext::new(None, None, Vec::new())
            };
            view.slot_snapshot.get_or_init(build);
            view.slot_snapshot.get_or_init(build);
            assert_eq!(builds.get(), 1);
            view.set(1, Some(ItemStack::new("minecraft:birch_log", 3)));
            let (_, context) = view.slot_context(1);
            assert_eq!(
                context.slot(Grid, 0).unwrap().item.as_ref(),
                "minecraft:birch_log"
            );
            assert_eq!(context.slot(Grid, 0).unwrap().count, 3);
            context.snapshot.unwrap().get_or_init(build);
            assert_eq!(builds.get(), 2, "a mutation starts a fresh snapshot");
        }
        slots.write(&mut world, &mut g, read);
        let Some(BlockEntity::Mod(m)) = world.block_entities.get(&k) else {
            panic!("not a mod's");
        };
        assert_eq!(m.items[0], None);
        assert!(m.items[1].is_some());
    }

    /// A mod menu type's linked slots show the containers a mod links them
    /// to, in order: reads come from those containers, writes go back to
    /// them and never into the block entity, and slots past them are left
    /// out of clicks.
    #[test]
    fn mod_menu_linked_slots_are_their_containers() {
        use crate::containers::{ModBlockEntityFields, ModContainer, ModLinks, ModMenuType};
        let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
        let container = ModContainer {
            slots: 6,
            ..Default::default()
        };
        world
            .block_entities
            .set_mod_containers([(0, Arc::new(container))].into_iter().collect());
        let k = (0, 64, 0);
        let mut own = vec![None; 6];
        own[0] = Some(ItemStack::new("minecraft:oak_log", 1));
        world.block_entities.insert(
            k,
            BlockEntity::Mod(Box::new(crate::containers::ModBlockEntity::new(
                "test:station".into(),
                ModBlockEntityFields {
                    items: own,
                    ..Default::default()
                },
            ))),
        );
        let chest = (1, 64, 0);
        let stick = Some(ItemStack::new("minecraft:stick", 8));
        let stone = Some(ItemStack::new("minecraft:stone", 3));
        world.block_entities.insert(
            chest,
            BlockEntity::Chest(vec![stick.clone(), None, stone.clone()]),
        );
        let t: &'static ModMenuType = Box::leak(Box::new(ModMenuType {
            id: "test:station",
            slots: 6,
            data_slots: 0,
            results: vec![],
            crafting: None,
            links: Some(ModLinks { from: 2, count: 4 }),
        }));
        let mut g = guest();
        let slots = Slots::Block(k);
        g.menu = Some(Menu {
            window: 1,
            kind: 0,
            registry_id: "test:station".into(),
            pos: Some(k),
            slots,
            size: 6,
            state: 0,
            sent: Vec::new(),
            drag: None,
            sent_data: Vec::new(),
            hidden: Vec::new(),
            mod_type: Some(t),
            craft: None,
            links: Vec::new(),
            asks_still_valid: false,
        });
        // Nothing linked yet: the linked slots are empty and all left out.
        let read = slots.read(&world, &g).unwrap();
        assert_eq!(read[2..], [None, None, None, None]);
        let m = g.menu.as_ref().unwrap();
        assert_eq!(m.unlinked_slots(&world), [2, 3, 4, 5]);
        // Linked to the chest: its three slots, then one slot past them.
        g.menu.as_mut().unwrap().links = vec![chest];
        assert_eq!(slots.get(&world, &g, 2), stick);
        assert_eq!(slots.get(&world, &g, 4), stone);
        assert_eq!(slots.get(&world, &g, 5), None);
        assert_eq!(g.menu.as_ref().unwrap().unlinked_slots(&world), [5]);
        let mut read = slots.read(&world, &g).unwrap();
        assert_eq!(read[0], Some(ItemStack::new("minecraft:oak_log", 1)));
        assert_eq!(read[2..], [stick.clone(), None, stone.clone(), None]);
        // Move the stone to the chest's middle slot and the stick out.
        read[3] = read[4].take();
        read[2] = None;
        slots.write(&mut world, &mut g, read);
        let Some(BlockEntity::Chest(items)) = world.block_entities.get(&chest) else {
            panic!("not a chest");
        };
        assert_eq!(items[..], [None, stone.clone(), None]);
        let Some(BlockEntity::Mod(m)) = world.block_entities.get(&k) else {
            panic!("not a mod's");
        };
        assert_eq!(m.items[2..], [None, None, None, None]);
        assert!(m.items[0].is_some());
        // A link to a container that is gone shows nothing and takes nothing.
        world.block_entities.remove(&chest);
        assert_eq!(slots.get(&world, &g, 3), None);
        assert_eq!(
            g.menu.as_ref().unwrap().unlinked_slots(&world),
            [2, 3, 4, 5]
        );
        assert_eq!(write_links(&mut world, &[chest], vec![stick]), []);
    }

    /// A mod container saved with more items than its menu has slots: the
    /// menu reads and writes only its count, the rest stay saved.
    #[test]
    fn mod_menu_shows_only_its_container_slots() {
        use crate::containers::{ModBlockEntity, ModBlockEntityFields, ModContainer};
        let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
        let container = ModContainer {
            slots: 5,
            ..Default::default()
        };
        world
            .block_entities
            .set_mod_containers([(0, Arc::new(container))].into_iter().collect());
        let mut items = vec![None; 9];
        items[1] = Some(ItemStack::new("minecraft:stone", 3));
        items[7] = Some(ItemStack::new("minecraft:iron_ore", 2));
        let k = (0, 64, 0);
        world.block_entities.insert(
            k,
            BlockEntity::Mod(Box::new(ModBlockEntity::new(
                "gtmod:machine".into(),
                ModBlockEntityFields {
                    items,
                    ..Default::default()
                },
            ))),
        );
        let mut g = guest();
        let slots = Slots::Block(k);
        let mut read = slots.read(&world, &g).unwrap();
        assert_eq!(read.len(), 5);
        assert_eq!(slots.get(&world, &g, 7), None);
        // A shift-click from the hotbar (menu slot 5 + 27) lands in the
        // container's first empty slot, not past its count.
        if let Some(rules) = rules() {
            g.inventory[0] = Some(ItemStack::new("minecraft:dirt", 1));
            let mut drag = None;
            let mut view = View {
                guest: &mut g,
                container: Some(&mut read),
                shulker: false,
                container_limits: Vec::new(),
                hidden: &[],
                sounds: Vec::new(),
                slot_snapshot: Default::default(),
                loop_context: &crate::test_resources::loop_context(
                    horizon_world::native_load_context::LoopSide::Server,
                ),
            };
            click(&mut view, &mut drag, 5 + 27, 0, input::QUICK_MOVE, &rules);
            assert_eq!(view.get(0).map(|s| s.item), Some("minecraft:dirt".into()));
        } else {
            read[0] = Some(ItemStack::new("minecraft:dirt", 1));
        }
        slots.write(&mut world, &mut g, read);
        let Some(BlockEntity::Mod(m)) = world.block_entities.get(&k) else {
            panic!("not a mod's");
        };
        assert_eq!(m.items.len(), 9);
        assert_eq!(
            m.items[0].as_ref().map(|s| &*s.item),
            Some("minecraft:dirt")
        );
        assert_eq!(m.items[1].as_ref().map(|s| s.count), Some(3));
        assert_eq!(m.items[7].as_ref().map(|s| s.count), Some(2));
    }

    /// A slot a mod's screen hides is not in the menu: a shift-click from the
    /// inventory skips it, and a shift-click on it moves nothing.
    #[test]
    fn a_hidden_slot_takes_no_quick_move() {
        let Some(rules) = rules() else {
            return;
        };
        let mut g = guest();
        let mut container = vec![None; 3];
        container[2] = Some(ItemStack::new("minecraft:stone", 1));
        g.inventory[0] = Some(ItemStack::new("minecraft:dirt", 1));
        let hidden = [0, 2];
        let mut drag = None;
        let mut view = View {
            guest: &mut g,
            container: Some(&mut container),
            shulker: false,
            container_limits: Vec::new(),
            hidden: &hidden,
            sounds: Vec::new(),
            slot_snapshot: Default::default(),
            loop_context: &crate::test_resources::loop_context(
                horizon_world::native_load_context::LoopSide::Server,
            ),
        };
        // The hotbar's first slot (menu slot 3 + 27) lands in the one slot shown.
        click(&mut view, &mut drag, 3 + 27, 0, input::QUICK_MOVE, &rules);
        assert!(view.get(0).is_none());
        assert_eq!(view.get(1).map(|s| s.item), Some("minecraft:dirt".into()));
        // Nothing leaves a hidden slot either.
        click(&mut view, &mut drag, 2, 0, input::QUICK_MOVE, &rules);
        assert_eq!(view.get(2).map(|s| s.item), Some("minecraft:stone".into()));
    }

    #[test]
    fn quick_craft_uses_original_stack_and_clamps_existing_count() {
        let Some(rules) = rules() else { return };
        let mut guest = guest();
        let mut chest = vec![None; 2];
        let mut view = View {
            guest: &mut guest,
            container: Some(&mut chest),
            shulker: false,
            container_limits: Vec::new(),
            hidden: &[],
            sounds: Vec::new(),
            slot_snapshot: Default::default(),
            loop_context: crate::test_resources::server_loop_context(),
        };
        view.guest.carried = Some(ItemStack::new("minecraft:stone", 8));
        let mut drag = Some((0, vec![0, 1]));
        quick_craft(&mut view, &mut drag, -999, 2, &rules);
        assert_eq!(
            (view.get(0).unwrap().count, view.get(1).unwrap().count),
            (4, 4)
        );
        assert!(view.guest.carried.is_none());
        assert_eq!(crate::menu::quick_craft_count(0, 8, 2, 8, 3, 64), 3);
    }

    #[test]
    fn guest_drag_matches_menu_for_vanilla_stack_limits() {
        let Some(rules) = rules() else { return };
        crate::menu::tests::with_menu(crate::menu::Kind::Chest(1), |env, menu| {
            for (id, existing, carried, expected) in [
                ("minecraft:stone", 63, 8, (64, 4, 3)),
                ("minecraft:ender_pearl", 15, 8, (16, 4, 3)),
                ("minecraft:diamond_sword", 0, 2, (1, 1, 0)),
            ] {
                let mut guest = guest();
                guest.carried = Some(ItemStack::new(id, carried));
                let mut chest = vec![None; 9];
                if existing > 0 {
                    chest[0] = Some(ItemStack::new(id, existing));
                }
                let mut view = View {
                    guest: &mut guest,
                    container: Some(&mut chest),
                    shulker: false,
                    container_limits: Vec::new(),
                    hidden: &[],
                    sounds: Vec::new(),
                    slot_snapshot: Default::default(),
                    loop_context: crate::test_resources::server_loop_context(),
                };
                menu.load_slot(0, view.get(0));
                menu.load_slot(1, None);
                menu.carried = Some(ItemStack::new(id, carried));
                let mut drag = None;
                for (slot, button) in [(-999, 0), (0, 1), (1, 1), (-999, 2)] {
                    click(
                        &mut view,
                        &mut drag,
                        slot,
                        button,
                        input::QUICK_CRAFT,
                        &rules,
                    );
                    menu.click(env, slot, button, crate::menu::Input::QuickCraft)
                        .unwrap();
                }
                let counts = (
                    view.get(0).map_or(0, |s| s.count),
                    view.get(1).map_or(0, |s| s.count),
                    view.guest.carried.as_ref().map_or(0, |s| s.count),
                );
                let predicted = (
                    menu.slot(0).map_or(0, |s| s.count),
                    menu.slot(1).map_or(0, |s| s.count),
                    menu.carried.as_ref().map_or(0, |s| s.count),
                );
                assert_eq!(counts, expected, "{id}");
                assert_eq!(predicted, expected, "{id}");
                assert_eq!(counts.0 + counts.1 + counts.2, existing + carried);
            }
        });
    }

    /// `AbstractContainerMenu.isValidQuickcraftType`: quick-craft type 2 (a
    /// clone drag) is for players with infinite materials only, and type 3 is
    /// no type at all. A survival guest that sends either moves nothing; a
    /// creative guest's clone drag fills the slots and consumes its cursor.
    #[test]
    fn a_clone_drag_needs_infinite_materials() {
        let rules = SwapRules;
        let mut g = guest();
        let mut chest = vec![None; 3];
        let mut drag = None;
        let mut view = View {
            guest: &mut g,
            container: Some(&mut chest),
            shulker: false,
            container_limits: Vec::new(),
            hidden: &[],
            sounds: Vec::new(),
            slot_snapshot: Default::default(),
            loop_context: &crate::test_resources::loop_context(
                horizon_world::native_load_context::LoopSide::Server,
            ),
        };
        view.guest.carried = Some(ItemStack::new("minecraft:stone", 64));
        // Start, two slots, end, for the two types a survival guest may not
        // use: 3 is no type, 2 is a clone drag.
        for kind in [3, 2] {
            click(
                &mut view,
                &mut drag,
                0,
                kind << 2,
                input::QUICK_CRAFT,
                &rules,
            );
            click(
                &mut view,
                &mut drag,
                0,
                kind << 2 | 1,
                input::QUICK_CRAFT,
                &rules,
            );
            click(
                &mut view,
                &mut drag,
                1,
                kind << 2 | 1,
                input::QUICK_CRAFT,
                &rules,
            );
            click(
                &mut view,
                &mut drag,
                -999,
                kind << 2 | 2,
                input::QUICK_CRAFT,
                &rules,
            );
            assert_eq!(view.get(0), None, "type {kind}");
            assert_eq!(view.get(1), None, "type {kind}");
            assert_eq!(view.guest.carried.as_ref().map(|s| s.count), Some(64));
        }
        // A charitable drag over the same two slots still shares the stack.
        click(&mut view, &mut drag, 0, 0, input::QUICK_CRAFT, &rules);
        click(&mut view, &mut drag, 0, 1, input::QUICK_CRAFT, &rules);
        click(&mut view, &mut drag, 1, 1, input::QUICK_CRAFT, &rules);
        click(&mut view, &mut drag, -999, 2, input::QUICK_CRAFT, &rules);
        assert_eq!(view.get(0).map(|s| s.count), Some(32));
        assert_eq!(view.get(1).map(|s| s.count), Some(32));
        assert_eq!(view.guest.carried, None);
        // With infinite materials a clone drag goes through: both slots fill
        // and vanilla subtracts the placed counts from the cursor.
        view.guest.game_mode = crate::lan::guest::CREATIVE;
        view.guest.carried = Some(ItemStack::new("minecraft:stone", 64));
        click(&mut view, &mut drag, 0, 2 << 2, input::QUICK_CRAFT, &rules);
        click(
            &mut view,
            &mut drag,
            0,
            2 << 2 | 1,
            input::QUICK_CRAFT,
            &rules,
        );
        click(
            &mut view,
            &mut drag,
            2,
            2 << 2 | 1,
            input::QUICK_CRAFT,
            &rules,
        );
        click(
            &mut view,
            &mut drag,
            -999,
            2 << 2 | 2,
            input::QUICK_CRAFT,
            &rules,
        );
        assert_eq!(view.get(0).map(|s| s.count), Some(64));
        assert_eq!(view.get(2).map(|s| s.count), Some(64));
        assert_eq!(view.guest.carried, None);
    }

    struct SwapRules;
    impl Rules for SwapRules {
        fn items(&self) -> &Items {
            unreachable!("swap does not read the item catalog");
        }
        fn enchantments(&self) -> &Enchantments {
            enchantments()
        }
        fn max_stack(&self, _: &str) -> u32 {
            64
        }
    }

    #[test]
    fn oversized_hotbar_swap_respects_slot_limit_and_returns_displaced_items() {
        let Some(rules) = rules() else { return };
        let mut guest = guest();
        let mut dirt = ItemStack::new("minecraft:dirt", 8);
        dirt.data_mut().set_other(
            "minecraft:max_stack_size",
            Some(horizon_core::nbt::Tag::Int(16)),
        );
        let mut partial = dirt.clone();
        partial.count = 15;
        guest.inventory[1] = Some(partial);
        guest.selected = 1;
        guest.inventory[0] = Some(ItemStack::new("minecraft:stone", 80));
        let mut chest = vec![Some(dirt)];
        let mut view = View {
            guest: &mut guest,
            container: Some(&mut chest),
            shulker: false,
            container_limits: Vec::new(),
            hidden: &[],
            sounds: Vec::new(),
            slot_snapshot: Default::default(),
            loop_context: crate::test_resources::server_loop_context(),
        };
        let (drops, _) = click(&mut view, &mut None, 0, 0, input::SWAP, &rules);
        assert!(drops.is_empty());
        assert_eq!(view.get(0).unwrap().count, 64);
        assert_eq!(view.guest.inventory[0].as_ref().unwrap().count, 16);
        assert_eq!(view.guest.inventory[1].as_ref().unwrap().count, 16);
        assert_eq!(
            view.guest
                .inventory
                .iter()
                .flatten()
                .filter(|s| &*s.item == "minecraft:dirt")
                .map(|s| s.count)
                .sum::<u32>(),
            23
        );
    }

    /// `doClick`'s SWAP and THROW arms take the clicked slot's stack through
    /// `Slot.mayPickup` (`SWAP` puts it straight into the `Inventory`, and
    /// `THROW` goes through `safeTake`), so a slot that withholds its stack
    /// gives a number key, the off hand's F and a drop click nothing at all.
    #[test]
    fn guest_swap_and_throw_need_the_slots_consent() {
        let rules = StrictRules::new(&[0]);
        let mut g = guest();
        g.offhand = Some(ItemStack::new("minecraft:shield", 1));
        let mut chest = vec![
            Some(ItemStack::new("minecraft:stone", 5)),
            Some(ItemStack::new("minecraft:dirt", 2)),
            Some(ItemStack::new("minecraft:apple", 3)),
        ];
        let mut drag = None;
        let mut view = View {
            guest: &mut g,
            container: Some(&mut chest),
            shulker: false,
            container_limits: Vec::new(),
            hidden: &[],
            sounds: Vec::new(),
            slot_snapshot: Default::default(),
            loop_context: &crate::test_resources::loop_context(
                horizon_world::native_load_context::LoopSide::Server,
            ),
        };
        // Key 1 over the refused slot moves neither stack.
        click(&mut view, &mut drag, 0, 0, input::SWAP, &rules);
        assert_eq!(view.get(0).map(|s| s.count), Some(5));
        assert!(view.guest.inventory[0].is_none());
        // F over it leaves the off hand alone too.
        click(&mut view, &mut drag, 0, 40, input::SWAP, &rules);
        assert_eq!(view.get(0).map(|s| s.count), Some(5));
        assert_eq!(
            view.guest.offhand.as_ref().map(|s| &*s.item),
            Some("minecraft:shield")
        );
        // A drop click throws nothing out of it.
        let (drops, _) = click(&mut view, &mut drag, 0, 1, input::THROW, &rules);
        assert!(drops.is_empty());
        assert_eq!(view.get(0).map(|s| s.count), Some(5));
        // The slots next to it do all three.
        click(&mut view, &mut drag, 1, 1, input::SWAP, &rules);
        assert!(view.get(1).is_none());
        assert_eq!(view.guest.inventory[1].as_ref().map(|s| s.count), Some(2));
        click(&mut view, &mut drag, 2, 40, input::SWAP, &rules);
        assert_eq!(
            view.get(2).as_ref().map(|s| &*s.item),
            Some("minecraft:shield")
        );
        assert_eq!(
            view.guest.offhand.as_ref().map(|s| (&*s.item, s.count)),
            Some(("minecraft:apple", 3))
        );
        // The first hotbar slot below the three chest slots (menu slot 30)
        // throws its stack.
        view.guest.inventory[0] = Some(ItemStack::new("minecraft:stick", 4));
        let (drops, _) = click(&mut view, &mut drag, 30, 1, input::THROW, &rules);
        assert_eq!(drops.iter().fold(0, |n, s| n + s.count), 4);
        assert!(view.guest.inventory[0].is_none());
    }

    /// F (SWAP button 40) over a chest or the inventory below it swaps with
    /// the off hand, even though the menu has no off-hand slot.
    #[test]
    fn guest_chest_f_swaps_with_the_off_hand() {
        let rules = SwapRules;
        let mut g = guest();
        g.offhand = Some(ItemStack::new("minecraft:shield", 1));
        let mut chest = vec![Some(ItemStack::new("minecraft:diamond", 1)), None];
        let mut drag = None;
        let mut view = View {
            guest: &mut g,
            container: Some(&mut chest),
            shulker: false,
            container_limits: Vec::new(),
            hidden: &[],
            sounds: Vec::new(),
            slot_snapshot: Default::default(),
            loop_context: &crate::test_resources::loop_context(
                horizon_world::native_load_context::LoopSide::Server,
            ),
        };
        click(&mut view, &mut drag, 0, 40, input::SWAP, &rules);
        assert_eq!(
            view.get(0).as_ref().map(|s| &*s.item),
            Some("minecraft:shield")
        );
        assert_eq!(
            view.guest.offhand.as_ref().map(|s| &*s.item),
            Some("minecraft:diamond")
        );
        // Empty chest slot: the off hand goes in.
        click(&mut view, &mut drag, 1, 40, input::SWAP, &rules);
        assert_eq!(
            view.get(1).as_ref().map(|s| &*s.item),
            Some("minecraft:diamond")
        );
        assert!(view.guest.offhand.is_none());
        // F over the first hotbar slot (menu slot 2 + 27).
        view.guest.offhand = Some(ItemStack::new("minecraft:oak_boat", 1));
        view.guest.inventory[0] = Some(ItemStack::new("minecraft:apple", 1));
        click(&mut view, &mut drag, 2 + 27, 40, input::SWAP, &rules);
        assert_eq!(
            view.guest.inventory[0].as_ref().map(|s| &*s.item),
            Some("minecraft:oak_boat")
        );
        assert_eq!(
            view.guest.offhand.as_ref().map(|s| &*s.item),
            Some("minecraft:apple")
        );
    }

    #[test]
    fn guest_shulker_box_menu_refuses_shulker_boxes() {
        let Some(rules) = rules() else {
            return;
        };
        for shulker in [true, false] {
            let mut g = guest();
            let mut container = vec![None; 27];
            let mut drag = None;
            let mut view = View {
                guest: &mut g,
                container: Some(&mut container),
                shulker,
                container_limits: Vec::new(),
                hidden: &[],
                sounds: Vec::new(),
                slot_snapshot: Default::default(),
                loop_context: &crate::test_resources::loop_context(
                    horizon_world::native_load_context::LoopSide::Server,
                ),
            };
            // Carried onto an empty slot, shift-clicked from the hotbar
            // (menu slot 27 + 27) and swapped in with the number key 1.
            view.guest.carried = Some(ItemStack::new("minecraft:red_shulker_box", 1));
            click(&mut view, &mut drag, 0, 0, input::PICKUP, &rules);
            assert_eq!(view.get(0).is_none(), shulker);
            view.guest.carried = None;
            view.guest.inventory[0] = Some(ItemStack::new("minecraft:shulker_box", 1));
            click(&mut view, &mut drag, 54, 0, input::QUICK_MOVE, &rules);
            assert_eq!(view.guest.inventory[0].is_some(), shulker);
            view.guest.inventory[0] = Some(ItemStack::new("minecraft:blue_shulker_box", 1));
            click(&mut view, &mut drag, 5, 0, input::SWAP, &rules);
            assert_eq!(view.get(5).is_none(), shulker);
            // Anything else goes in.
            view.guest.carried = Some(ItemStack::new("minecraft:diamond", 1));
            click(&mut view, &mut drag, 9, 0, input::PICKUP, &rules);
            assert!(view.get(9).is_some());
        }
    }

    /// `item` with an `equippable` component for `slot`, which an empty catalog then wears.
    fn worn(item: &str, slot: &str) -> ItemStack {
        let mut stack = ItemStack::new(item, 1);
        let component = Tag::compound([("slot", Tag::String(slot.into()))]);
        stack
            .data_mut()
            .set_other("minecraft:equippable", Some(component));
        stack
    }

    /// A shift-click in the inventory menu puts a shield in the empty off-hand
    /// slot: `InventoryMenu.quickMoveStack` asks `getEquipmentSlotForItem` for
    /// `EquipmentSlot.OFFHAND` and moves it to slot 45 before it tries the
    /// hotbar. A full off hand, or an item with no `equippable` slot, still
    /// goes to the hotbar.
    #[test]
    fn inventory_quick_move_equips_an_off_hand_item() {
        let rules = ItemRules(Items::for_tests(Default::default()));
        let mut g = guest();
        let mut drag = None;
        let mut view = View {
            guest: &mut g,
            container: None,
            shulker: false,
            container_limits: Vec::new(),
            hidden: &[],
            sounds: Vec::new(),
            slot_snapshot: Default::default(),
            loop_context: &crate::test_resources::loop_context(
                horizon_world::native_load_context::LoopSide::Server,
            ),
        };
        // Menu slot 9 is the first main-inventory slot, 45 the off hand.
        view.guest.inventory[9] = Some(worn("minecraft:shield", "offhand"));
        click(&mut view, &mut drag, 9, 0, input::QUICK_MOVE, &rules);
        assert_eq!(
            view.get(45).as_ref().map(|s| &*s.item),
            Some("minecraft:shield")
        );
        assert!(view.get(9).is_none());
        assert!(view.get(36).is_none());
        // The off hand is taken: the next shield goes to the hotbar instead.
        view.guest.inventory[9] = Some(worn("minecraft:shield", "offhand"));
        click(&mut view, &mut drag, 9, 0, input::QUICK_MOVE, &rules);
        assert_eq!(
            view.get(36).as_ref().map(|s| &*s.item),
            Some("minecraft:shield")
        );
        assert!(view.get(9).is_none());
        // An item that equips nowhere is untouched by the off-hand branch.
        view.guest.inventory[10] = Some(ItemStack::new("minecraft:diamond", 3));
        click(&mut view, &mut drag, 10, 0, input::QUICK_MOVE, &rules);
        assert_eq!(view.get(37).as_ref().map(|s| s.count), Some(3));
        assert_eq!(
            view.get(45).as_ref().map(|s| &*s.item),
            Some("minecraft:shield")
        );
    }

    /// Rules that refuse a pickup in some menu slots and can stop drops.
    struct StrictRules {
        items: Items,
        refuse: &'static [usize],
        drops: bool,
    }
    impl StrictRules {
        fn new(refuse: &'static [usize]) -> Self {
            Self {
                items: Items::for_tests(Default::default()),
                refuse,
                drops: true,
            }
        }
    }
    impl Rules for StrictRules {
        fn items(&self) -> &Items {
            &self.items
        }
        fn enchantments(&self) -> &Enchantments {
            enchantments()
        }
        fn max_stack(&self, _: &str) -> u32 {
            64
        }
        fn may_pickup(&self, slot: usize, _: &ItemStack) -> bool {
            !self.refuse.contains(&slot)
        }
        fn can_drop_items(&self) -> bool {
            self.drops
        }
    }

    /// `Slot.mayPickup`: a pickup and a quick move leave a slot's stack alone
    /// when the menu's rules refuse it there, while the slot next to it still
    /// gives its own stack up.
    #[test]
    fn guest_pickup_and_quick_move_need_the_slots_consent() {
        let rules = StrictRules::new(&[0]);
        let mut g = guest();
        let mut chest = vec![
            Some(ItemStack::new("minecraft:stone", 5)),
            Some(ItemStack::new("minecraft:dirt", 2)),
        ];
        let mut drag = None;
        let mut view = View {
            guest: &mut g,
            container: Some(&mut chest),
            shulker: false,
            container_limits: Vec::new(),
            hidden: &[],
            sounds: Vec::new(),
            slot_snapshot: Default::default(),
            loop_context: &crate::test_resources::loop_context(
                horizon_world::native_load_context::LoopSide::Server,
            ),
        };
        // The refused slot takes no click and gives nothing to the cursor.
        click(&mut view, &mut drag, 0, 0, input::PICKUP, &rules);
        assert_eq!(view.get(0).map(|s| s.count), Some(5));
        assert_eq!(view.guest.carried, None);
        // A quick move out of it takes nothing with it either.
        click(&mut view, &mut drag, 0, 0, input::QUICK_MOVE, &rules);
        assert_eq!(view.get(0).map(|s| s.count), Some(5));
        assert!(view.get(2).is_none());
        // The slot next to it takes both clicks.
        click(&mut view, &mut drag, 1, 0, input::PICKUP, &rules);
        assert!(view.get(1).is_none());
        assert_eq!(
            view.guest.carried.as_ref().map(|s| (&*s.item, s.count)),
            Some(("minecraft:dirt", 2))
        );
        let back = view.guest.carried.take();
        view.set(1, back);
        click(&mut view, &mut drag, 1, 0, input::QUICK_MOVE, &rules);
        // A container's quick move fills the inventory from its end.
        assert_eq!(
            view.get(37).as_ref().map(|s| &*s.item),
            Some("minecraft:dirt")
        );
    }

    /// `doClick`'s PICKUP_ALL arm returns when the clicked slot holds a stack
    /// it may pick up, so a double-click collects from an empty slot, or from
    /// one that refuses, only.
    #[test]
    fn guest_pickup_all_starts_from_an_empty_or_refusing_slot() {
        let rules = StrictRules::new(&[]);
        let mut g = guest();
        let mut chest = vec![Some(ItemStack::new("minecraft:stone", 5)), None];
        let mut drag = None;
        let mut view = View {
            guest: &mut g,
            container: Some(&mut chest),
            shulker: false,
            container_limits: Vec::new(),
            hidden: &[],
            sounds: Vec::new(),
            slot_snapshot: Default::default(),
            loop_context: &crate::test_resources::loop_context(
                horizon_world::native_load_context::LoopSide::Server,
            ),
        };
        // One stone short of a full stack, over a slot that holds stone.
        view.guest.carried = Some(ItemStack::new("minecraft:stone", 63));
        click(&mut view, &mut drag, 0, 1, input::PICKUP_ALL, &rules);
        assert_eq!(view.get(0).map(|s| s.count), Some(5));
        assert_eq!(view.guest.carried.as_ref().map(|s| s.count), Some(63));
        // The empty slot next to it does collect that one stone.
        click(&mut view, &mut drag, 1, 1, input::PICKUP_ALL, &rules);
        assert_eq!(view.get(0).map(|s| s.count), Some(4));
        assert_eq!(view.guest.carried.as_ref().map(|s| s.count), Some(64));
        // A refusing slot is where a collect may start: the stone comes from
        // the slot that allows it, the refused one is left alone.
        let refusing = StrictRules::new(&[0]);
        view.set(0, Some(ItemStack::new("minecraft:stone", 5)));
        view.set(1, Some(ItemStack::new("minecraft:stone", 5)));
        view.guest.carried = Some(ItemStack::new("minecraft:stone", 63));
        click(&mut view, &mut drag, 0, 1, input::PICKUP_ALL, &refusing);
        assert_eq!(view.get(0).map(|s| s.count), Some(5));
        assert_eq!(view.get(1).map(|s| s.count), Some(4));
        assert_eq!(view.guest.carried.as_ref().map(|s| s.count), Some(64));
    }

    /// `Player.canDropItems`: a client that has dropped too fast sends THROW
    /// clicks the menu does not act on.
    #[test]
    fn guest_throw_needs_can_drop_items() {
        let mut rules = StrictRules::new(&[]);
        rules.drops = false;
        let mut g = guest();
        let mut chest = vec![Some(ItemStack::new("minecraft:stone", 5))];
        let mut drag = None;
        let mut view = View {
            guest: &mut g,
            container: Some(&mut chest),
            shulker: false,
            container_limits: Vec::new(),
            hidden: &[],
            sounds: Vec::new(),
            slot_snapshot: Default::default(),
            loop_context: &crate::test_resources::loop_context(
                horizon_world::native_load_context::LoopSide::Server,
            ),
        };
        let (drops, _) = click(&mut view, &mut drag, 0, 0, input::THROW, &rules);
        assert!(drops.is_empty());
        assert_eq!(view.get(0).map(|s| s.count), Some(5));
        rules.drops = true;
        let (drops, _) = click(&mut view, &mut drag, 0, 0, input::THROW, &rules);
        assert_eq!(drops.iter().fold(0, |n, s| n + s.count), 1);
        assert_eq!(view.get(0).map(|s| s.count), Some(4));
    }

    /// `ArmorSlot.mayPickup`: a survival guest cannot take armour with `prevent_armor_change` off,
    /// Curse of Binding's or a data pack enchantment's under another id; a creative one can, by a
    /// click and by a shift-click. Armour with another enchantment comes off.
    #[test]
    fn guest_survival_cannot_remove_cursed_armour() {
        let rules = StrictRules::new(&[]);
        for (enchantment, game_mode, taken) in [
            (
                "minecraft:binding_curse",
                crate::lan::guest::SURVIVAL,
                false,
            ),
            ("test:clinging", crate::lan::guest::SURVIVAL, false),
            ("minecraft:unbreaking", crate::lan::guest::SURVIVAL, true),
            ("minecraft:binding_curse", crate::lan::guest::CREATIVE, true),
            ("test:clinging", crate::lan::guest::CREATIVE, true),
        ] {
            for mode in [input::PICKUP, input::QUICK_MOVE] {
                let mut g = guest();
                let mut cursed = ItemStack::new("minecraft:diamond_chestplate", 1);
                cursed.set_crafting_enchantments(vec![(enchantment.into(), 1)]);
                g.armor[1] = Some(cursed);
                g.game_mode = game_mode;
                let mut drag = None;
                let mut view = View {
                    guest: &mut g,
                    container: None,
                    shulker: false,
                    container_limits: Vec::new(),
                    hidden: &[],
                    sounds: Vec::new(),
                    slot_snapshot: Default::default(),
                    loop_context: &crate::test_resources::loop_context(
                        horizon_world::native_load_context::LoopSide::Server,
                    ),
                };
                // Menu slot 6 is the chest slot.
                click(&mut view, &mut drag, 6, 0, mode, &rules);
                assert_eq!(view.get(6).is_some(), !taken, "{enchantment}");
                let moved = view.guest.carried.is_some() || view.get(9).is_some();
                assert_eq!(moved, taken, "{enchantment}");
            }
        }
    }

    /// The same `ArmorSlot.mayPickup` answers a number key and a drop click:
    /// `doClick`'s SWAP arm moves the slot's stack into the `Inventory`
    /// itself and THROW calls `safeTake`, so both ask the slot first.
    #[test]
    fn guest_swap_and_throw_keep_cursed_armour_on() {
        let rules = StrictRules::new(&[]);
        for (game_mode, moved) in [
            (crate::lan::guest::SURVIVAL, false),
            (crate::lan::guest::CREATIVE, true),
        ] {
            for (mode, button) in [(input::THROW, 1), (input::SWAP, 0)] {
                let mut g = guest();
                let mut cursed = ItemStack::new("minecraft:diamond_chestplate", 1);
                cursed.set_crafting_enchantments(vec![("minecraft:binding_curse".into(), 1)]);
                g.armor[1] = Some(cursed);
                g.game_mode = game_mode;
                let mut drag = None;
                let mut view = View {
                    guest: &mut g,
                    container: None,
                    shulker: false,
                    container_limits: Vec::new(),
                    hidden: &[],
                    sounds: Vec::new(),
                    slot_snapshot: Default::default(),
                    loop_context: &crate::test_resources::loop_context(
                        horizon_world::native_load_context::LoopSide::Server,
                    ),
                };
                // Menu slot 6 is the chest slot. A drop click throws the
                // stack out; number key 1 puts it in the first hotbar slot
                // (menu slot 36).
                let (drops, _) = click(&mut view, &mut drag, 6, button, mode, &rules);
                assert_eq!(view.get(6).is_some(), !moved, "mode {mode}");
                if mode == input::THROW {
                    assert_eq!(drops.is_empty(), !moved, "mode {mode} drops");
                } else {
                    assert_eq!(view.guest.inventory[0].is_some(), moved, "mode {mode}");
                }
            }
        }
    }

    /// Where `stack` ends up in a guest's inventory menu: the armour slots (5 head .. 8 feet) a
    /// click from the cursor puts it in, each tried on its own, and every slot holding anything
    /// after a shift-click of it from the first hotbar slot (36), `usize::MAX` for the cursor.
    fn worn_by_guest(rules: &dyn Rules, stack: &ItemStack) -> (Vec<usize>, Vec<usize>) {
        let context = crate::test_resources::loop_context(
            horizon_world::native_load_context::LoopSide::Server,
        );
        let mut clicked = Vec::new();
        for slot in 5..=8_usize {
            let mut g = guest();
            let mut view = View {
                guest: &mut g,
                container: None,
                shulker: false,
                container_limits: Vec::new(),
                hidden: &[],
                sounds: Vec::new(),
                slot_snapshot: Default::default(),
                loop_context: &context,
            };
            view.guest.carried = Some(stack.clone());
            click(&mut view, &mut None, slot as i32, 0, input::PICKUP, rules);
            if view.get(slot).as_ref() == Some(stack) {
                clicked.push(slot);
            }
        }
        let mut g = guest();
        let mut view = View {
            guest: &mut g,
            container: None,
            shulker: false,
            container_limits: Vec::new(),
            hidden: &[],
            sounds: Vec::new(),
            slot_snapshot: Default::default(),
            loop_context: &context,
        };
        view.set(36, Some(stack.clone()));
        click(&mut view, &mut None, 36, 0, input::QUICK_MOVE, rules);
        let mut held: Vec<usize> = (0..46).filter(|s| view.get(*s).is_some()).collect();
        if view.guest.carried.is_some() {
            held.push(usize::MAX);
        }
        (clicked, held)
    }

    /// `ArmorSlot.mayPlace` for a guest reads the stack's own `equippable` (arismoko/horizon#2669): an iron
    /// helmet goes on the head; with its component removed it is no armour, and a shift-click
    /// fills the inventory; an override for the feet puts it there; an `allowed_entities` without
    /// the player, or an explicitly empty one, keeps it off, and a shift-click then moves nothing
    /// at all (`moveItemStackTo` refused). Data only: no part answers.
    #[test]
    fn guest_armour_slots_read_the_stacks_own_equippable() {
        let Some(rules) = rules() else {
            return;
        };
        let helmet = |component: Option<Vec<(&str, Tag)>>| {
            let mut helmet = ItemStack::new("minecraft:iron_helmet", 1);
            let (id, value) = match component {
                Some(fields) => ("minecraft:equippable", Tag::compound(fields)),
                None => ("!minecraft:equippable", Tag::Compound(Default::default())),
            };
            helmet.data_mut().set_other(id, Some(value));
            helmet
        };
        let slot = |name: &str| ("slot", Tag::String(name.into()));
        let allowed = |list: Vec<Tag>| ("allowed_entities", Tag::List(list));
        let plain = ItemStack::new("minecraft:iron_helmet", 1);
        assert_eq!(worn_by_guest(&rules, &plain), (vec![5], vec![5]));
        assert_eq!(worn_by_guest(&rules, &helmet(None)), (vec![], vec![9]));
        let feet = helmet(Some(vec![slot("feet")]));
        assert_eq!(worn_by_guest(&rules, &feet), (vec![8], vec![8]));
        let zombie = vec![Tag::String("zombie".into())];
        let zombie = helmet(Some(vec![slot("head"), allowed(zombie)]));
        assert_eq!(worn_by_guest(&rules, &zombie), (vec![], vec![36]));
        let nobody = helmet(Some(vec![slot("head"), allowed(Vec::new())]));
        assert_eq!(worn_by_guest(&rules, &nobody), (vec![], vec![36]));
        let player = vec![Tag::String("player".into())];
        let player = helmet(Some(vec![slot("head"), allowed(player)]));
        assert_eq!(worn_by_guest(&rules, &player), (vec![5], vec![5]));
    }

    /// A shift-click from the off-hand slot wears the armour it holds:
    /// `InventoryMenu.quickMoveStack` asks for the item's equipment slot
    /// before it looks at the clicked index, so the helmet goes to the empty
    /// helmet slot. With that slot taken, and for anything else in the off
    /// hand, the stack fills the main inventory before the hotbar.
    #[test]
    fn inventory_quick_move_wears_armor_from_the_off_hand() {
        let rules = ItemRules(Items::for_tests(Default::default()));
        let mut g = guest();
        let mut drag = None;
        let mut view = View {
            guest: &mut g,
            container: None,
            shulker: false,
            container_limits: Vec::new(),
            hidden: &[],
            sounds: Vec::new(),
            slot_snapshot: Default::default(),
            loop_context: &crate::test_resources::loop_context(
                horizon_world::native_load_context::LoopSide::Server,
            ),
        };
        // Menu slot 45 is the off hand; 5 to 8 are the armour slots, head first.
        view.guest.offhand = Some(worn("minecraft:iron_helmet", "head"));
        click(&mut view, &mut drag, 45, 0, input::QUICK_MOVE, &rules);
        assert_eq!(
            view.get(5).as_ref().map(|s| &*s.item),
            Some("minecraft:iron_helmet")
        );
        assert!(view.get(45).is_none());
        assert!(view.get(9).is_none());
        // The helmet slot is taken: the next helmet goes to the inventory.
        view.guest.offhand = Some(worn("minecraft:iron_helmet", "head"));
        click(&mut view, &mut drag, 45, 0, input::QUICK_MOVE, &rules);
        assert_eq!(
            view.get(9).as_ref().map(|s| &*s.item),
            Some("minecraft:iron_helmet")
        );
        assert!(view.get(45).is_none());
        // Anything else fills the main inventory before the hotbar.
        view.guest.offhand = Some(ItemStack::new("minecraft:diamond", 3));
        click(&mut view, &mut drag, 45, 0, input::QUICK_MOVE, &rules);
        assert_eq!(view.get(10).as_ref().map(|s| s.count), Some(3));
        assert!(view.get(36).is_none());
        assert!(view.get(45).is_none());
    }

    /// `doClick` repeats `quickMoveStack` while the clicked slot keeps the
    /// item, because `moveItemStackTo` fills one empty slot per call. This
    /// engine's `move_to` fills every empty slot of its range in one pass, so a
    /// single quick move must spread a stack over several slots with the same
    /// remainder vanilla's loop leaves, and a full inventory must leave the
    /// stack where it is.
    #[test]
    fn container_override_caps_guest_pickup_and_shift_click() {
        let rules = ItemRules(Items::for_tests(Default::default()));
        let mut g = guest();
        let mut chest = vec![None; 3];
        let context = crate::test_resources::loop_context(
            horizon_world::native_load_context::LoopSide::Server,
        );
        let mut view = View {
            guest: &mut g,
            container: Some(&mut chest),
            shulker: false,
            container_limits: vec![Some(10); 3],
            hidden: &[],
            sounds: Vec::new(),
            slot_snapshot: Default::default(),
            loop_context: &context,
        };
        view.guest.carried = Some(ItemStack::new("minecraft:stick", 20));
        let mut drag = None;
        click(&mut view, &mut drag, 0, 0, input::PICKUP, &rules);
        assert_eq!(view.get(0).unwrap().count, 10);
        assert_eq!(view.guest.carried.as_ref().unwrap().count, 10);
        view.guest.carried = None;
        view.set(3, Some(ItemStack::new("minecraft:stick", 20)));
        click(&mut view, &mut drag, 3, 0, input::QUICK_MOVE, &rules);
        assert_eq!(view.get(1).unwrap().count, 10);
        assert_eq!(view.get(2).unwrap().count, 10);
        assert!(view.get(3).is_none());
    }

    #[test]
    fn one_quick_move_spreads_a_stack_like_vanillas_repeat() {
        struct SmallRules;
        impl Rules for SmallRules {
            fn items(&self) -> &Items {
                unreachable!("a quick move reads no item catalog");
            }
            fn enchantments(&self) -> &Enchantments {
                enchantments()
            }
            fn max_stack(&self, item: &str) -> u32 {
                if item == "minecraft:snowball" { 16 } else { 64 }
            }
        }
        let rules = SmallRules;
        let mut g = guest();
        let mut chest = vec![Some(ItemStack::new("minecraft:snowball", 40))];
        let mut drag = None;
        let mut view = View {
            guest: &mut g,
            container: Some(&mut chest),
            shulker: false,
            container_limits: Vec::new(),
            hidden: &[],
            sounds: Vec::new(),
            slot_snapshot: Default::default(),
            loop_context: &crate::test_resources::loop_context(
                horizon_world::native_load_context::LoopSide::Server,
            ),
        };
        // A container's quick move fills the inventory from its end: 16
        // snowballs in the last hotbar slot (menu slot 36), 16 in the one
        // before it, 8 in the third, and nothing left in the chest.
        click(&mut view, &mut drag, 0, 0, input::QUICK_MOVE, &rules);
        assert!(view.get(0).is_none());
        assert_eq!(view.get(36).map(|s| s.count), Some(16));
        assert_eq!(view.get(35).map(|s| s.count), Some(16));
        assert_eq!(view.get(34).map(|s| s.count), Some(8));
        // Two slots' worth of room only: the loop vanilla runs would stop
        // after the second call, so 8 stay in the chest.
        for i in 0..36 {
            view.guest.inventory[i] = Some(ItemStack::new("minecraft:dirt", 1));
        }
        view.set(0, Some(ItemStack::new("minecraft:snowball", 40)));
        // The fill starts at the hotbar's end: menu slots 36 and 35.
        view.guest.inventory[8] = None;
        view.guest.inventory[7] = None;
        click(&mut view, &mut drag, 0, 0, input::QUICK_MOVE, &rules);
        assert_eq!(view.get(0).map(|s| s.count), Some(8));
        assert_eq!(view.get(36).map(|s| s.count), Some(16));
        assert_eq!(view.get(35).map(|s| s.count), Some(16));
        // A full inventory moves nothing at all.
        for i in 0..36 {
            view.guest.inventory[i] = Some(ItemStack::new("minecraft:dirt", 1));
        }
        view.set(0, Some(ItemStack::new("minecraft:snowball", 5)));
        let (drops, changed) = click(&mut view, &mut drag, 0, 0, input::QUICK_MOVE, &rules);
        assert_eq!(view.get(0).map(|s| s.count), Some(5));
        assert!(drops.is_empty() && changed.is_empty());
    }

    /// `AbstractContainerMenu.isContainerClickButton`: a pickup and a quick
    /// move exist for buttons 0 and 1 only, so a modified client's other
    /// buttons take nothing out of a slot, move nothing to the hotbar and
    /// drop nothing from the cursor. The other modes are not buttons' business.
    #[test]
    fn guest_clicks_with_a_button_other_than_0_or_1_move_nothing() {
        let rules = ItemRules(Items::for_tests(Default::default()));
        let mut g = guest();
        let mut drag = None;
        let mut view = View {
            guest: &mut g,
            container: None,
            shulker: false,
            container_limits: Vec::new(),
            hidden: &[],
            sounds: Vec::new(),
            slot_snapshot: Default::default(),
            loop_context: &crate::test_resources::loop_context(
                horizon_world::native_load_context::LoopSide::Server,
            ),
        };
        // Menu slot 9 stands for inventory index 9, 36 for the hotbar's first.
        view.guest.inventory[9] = Some(ItemStack::new("minecraft:diamond", 3));
        click(&mut view, &mut drag, 9, 2, input::PICKUP, &rules);
        assert_eq!(view.guest.carried, None);
        assert_eq!(view.get(9).map(|s| s.count), Some(3));
        click(&mut view, &mut drag, 9, 2, input::QUICK_MOVE, &rules);
        assert_eq!(view.get(9).map(|s| s.count), Some(3));
        assert!(view.get(36).is_none());
        // A click outside the window drops nothing either.
        view.guest.carried = Some(ItemStack::new("minecraft:dirt", 5));
        let (drops, changed) = click(&mut view, &mut drag, -999, 2, input::PICKUP, &rules);
        assert!(drops.is_empty() && changed.is_empty());
        assert_eq!(view.guest.carried.as_ref().map(|s| s.count), Some(5));
        // Button 1 still takes half of the stack.
        view.guest.carried = None;
        click(&mut view, &mut drag, 9, 1, input::PICKUP, &rules);
        assert_eq!(view.guest.carried.as_ref().map(|s| s.count), Some(2));
        assert_eq!(view.get(9).map(|s| s.count), Some(1));
        // THROW reads its button as "one or the whole stack", not as a
        // primary or secondary action.
        view.guest.carried = None;
        view.guest.inventory[9] = Some(ItemStack::new("minecraft:diamond", 4));
        let (drops, _) = click(&mut view, &mut drag, 9, 2, input::THROW, &rules);
        assert_eq!(drops.iter().fold(0, |n, s| n + s.count), 4);
        assert!(view.get(9).is_none());
    }
}
