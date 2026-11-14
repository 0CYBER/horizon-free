//! The `menu_slot_rules` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0131

pub trait Hooks {
    /// Slot placement and stack limits for registered vanilla and mod menus;
    /// mounts use `horizon:mount_inventory`, while player inventory and the
    /// creative picker have no registered type. `slot` is the backing index,
    /// including player inventory slots. The slot's own answers arrive first;
    /// the maximum is bounded to 1–99 and the item limit. Refusal also prevents
    /// quick-move top-ups. Runs on server and client prediction with the read
    /// `Host`: it is a query, so it can neither write nor cancel. A click asks
    /// once for each slot and stack it consults while it plans, and never again
    /// when it validates or applies. Answer from the stack and the host alone,
    /// and fast. On a LAN host's own click the LAN host is not reachable
    /// through the `Host` (it answers as if no LAN were open).
    /// `container` is the container `slot` indexes ([`MenuContainer`]): a mod
    /// crafting grid's slots are `Grid`'s and its result `Result`'s slot 0.
    /// `context` says who and where the ask is about ([`MenuSlotContext`]):
    /// the player whose menu it is, the menu's block entity and a read-only
    /// copy of the menu's own containers' slots, keyed by the same container
    /// (`context.slot(container, index)`), built once per click plan and
    /// filled the same on the server and on the client's prediction. Parts that read `context.setting(..)` or `side()` have no
    /// counterpart on the Host. Named injections request `hook_version = 4`.
    #[hook(
        side = "both",
        contract = "listen",
        scope = "menu",
        key = "menu",
        host = "read",
        cancellable = false,
        version = 4,
        neoforge = "none (vanilla Slot.mayPlace and mayPickup; NeoForge adds no event or extension for it)"
    )]
    fn menu_slot_rules(
        menu: &str,
        slot: usize,
        container: MenuContainer,
        stack: &ItemStack,
        context: &MenuSlotContext,
        may_place: &mut bool,
        max_stack: &mut u32,
    );
}
