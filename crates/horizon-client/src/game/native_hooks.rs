//! Hook points for native mods (docs/native-mods.md, "Mods on the hook points"): empty
//! functions the game calls at fixed places, for horizon-mixins bundles to
//! hook with HEAD. Hooks that answer take an out parameter (`claimed`,
//! `zoom`…) so several mods can hook the same point. A native mod keeps its
//! own state in a module of its bundle; the helpers below read and draw what
//! Horizon's own HUD extras do, and keep a mod's settings where the
//! WebAssembly mod of the same id kept them.
//!
//! API 41 adds bundles' data files: `horizon_client::native_data!` compiles a file of a
//! bundle's `data/` folder in, and `native_built_data` is the folder its post-build step wrote.
//!
//! API 40 adds mod messages between a mod's server and client halves
//! (`native_net_from_client`, `native_net_from_server`, and
//! `native_net_send_to_player`, `native_net_send_to_server` in
//! `native_net.rs`), and hearing players and mobs: `native_player_died`,
//! `native_mob_spawned`, `native_mob_killed`, `native_item_dropped`,
//! `native_item_picked_up`.
//!
//! API 39 adds item behaviour: a held item's hit on a mob
//! (`native_hurt_enemy`), a carried stack's tick (`native_inventory_tick`) and
//! an equipment slot taking a different item (`native_on_equip`).
//!
//! API 38 adds redstone from a mod's block (`native_block_signal`,
//! `native_is_signal_source`, and the helper `native_update_signal_neighbors`).
//!
//! API 37 adds mobs for server mods (`native_spawn_mob`, `native_mob`,
//! `native_mobs`, `native_teleport_mob`, `native_push_mob`,
//! `native_set_mob_health`, `native_kill_mob`, `native_remove_mob`... in
//! `native_mobs.rs`).
//!
//! API 36 adds recipes of a mod's own types (`native_recipes_of_type`,
//! `native_recipe_ingredient`, `native_recipe_result`).
//!
//! API 35 adds the commands a mod owns on the client (`native_commands`), so
//! that the chat box colours, completes and lists them like the game's own.
//!
//! API 34 adds scheduled block ticks a mod can handle (`native_tick_block`).
//!
//! API 33 adds random ticks a mod can handle (`native_random_tick`), and
//! `native_set_block_property` on the level, which takes `Level.setBlock`'s
//! `flags` as `Game::native_set_block_property_with` does.
//!
//! API 32 adds shape updates a mod can answer (`native_update_shape`), and
//! `native_schedule_block_tick` and `native_schedule_fluid_tick` on the
//! level for the ticks a block's shape update schedules.
//!
//! API 31 adds a player's server tick (`native_player_tick`).
//!
//! API 30 adds left-clicking a block (`native_left_click_block`).
//!
//! API 29 adds right-clicking an entity (`native_interact_entity`).
//!
//! API 28 adds block and item tag reads for server tick hooks (`native_block_in_tag`,
//! `native_item_in_tag`).
//!
//! API 27 adds the player-health HUD rows (`native_health_hud`).
//!
//! API 26 adds the game's shared config folder (`native_config_dir`).
//!
//! API 25 adds server sound playback (`native_play_sound`).
//!
//! API 24 adds a server block setter (`native_set_block`).
//!
//! API 23 adds per-level server tick hooks (`native_level_tick_pre`,
//! `native_level_tick_post`).
//!
//! API 22 adds what a mod needs to keep the world from changing blocks
//! it guards: fluid and fire spreading (`native_block_spread`), pistons
//! (`native_piston_move`), explosions (`native_explosion_detonate`),
//! buckets (`native_fill_bucket`) and a player's attack on an entity
//! (`native_attack_entity`).
//!
//! API 21 adds right clicking a block (`native_right_click_block`), and in
//! `native_server.rs` the acting player (`native_acting_player`), a player's
//! health, the action bar (`native_action_bar`) and commands run as the
//! server (`native_run_command`).
//!
//! API 20 adds buttons, tooltips and item icons on mod screens
//! (`native_screen_click`, `native_screen_tooltip`, `native_menu_button`,
//! and `native_item`, `native_screen_title`, `native_data`, `native_block`,
//! `native_block_property`, `native_insert_item`, `native_cursor`).
//!
//! API 19 adds mod-drawn screens for mod blocks (`native_screen`,
//! `native_screen_draw`, and the `native_sprite` helpers), with the mod's own
//! GUI textures.
//!
//! API 18 adds what a machine's tick reaches: a neighbouring mod block entity
//! (`native_with_mod_block_entity`) and its own block's state
//! (`native_set_block_property`).
//!
//! API 17 adds menus for mod containers (the `menu` of a `ModContainer`: vanilla
//! slot grids and furnaces) and has `native_block_entity_tick` take the whole
//! `ModBlockEntity`, with the menu's data slots.
//!
//! API 16 adds item slots for those block entities
//! (`native_block_entity_container`), which hoppers, comparators and
//! breaking the block use as they use vanilla's containers.
//!
//! API 15 adds block entities for a mod's blocks (`native_block_entity`,
//! `native_block_entity_ticks`, `native_block_entity_tick`): made when the
//! block is placed, ticked on the server and saved with the chunk.
//!
//! API 14 adds breaking and placing a block (`native_destroy_block`,
//! `native_place_block`).
//!
//! API 13 adds using an item (`native_use_item_on`, `native_use_item`).
//!
//! API 12 adds item tooltips (`native_tooltip`).
//!
//! API 11 adds server particle emission (`native_send_particles`).
//!
//! API 10 adds server menus (`native_menu_clicked`, `native_menu_closed`,
//! and `native_open_menu`, `native_exchange_items`... in `native_server.rs`).
//!
//! API 9 adds the acting player's plain items (`native_held`,
//! `native_count_items`, `native_take_items`, `native_give_items`... in
//! `native_server.rs`).
//!
//! API 8 adds server commands and players (`native_register_commands`,
//! `native_perform_command`, and the helpers in `native_server.rs`).
//!
//! API 7 adds block-change observations (`native_block_changed`).
//!
//! API 6 adds neighbour-change observations (`native_neighbor_changed`).
//!
//! API 5 adds read-only server block queries (`native_server_block`).
//!
//! API 4 adds authoritative server tick hooks (`native_server_tick`).
//!
//! API 3 adds the world lifecycle hook (`native_world_lifecycle`).
//!
//! API 2 adds the item panel beside container screens and its recipe pages
//! (`native_panel_items`, `native_panel_recipes`, `native_panel_event`),
//! and the game's items and recipes (`native_recipes`,
//! `native_known_recipes`, `native_place_recipe`, `native_give`): what a
//! recipe viewer needs.
//!
//! Names, parameters and call sites are kept from release to release, and
//! hook points are only ever added: each addition raises [`NATIVE_HOOKS_API`].
//! A bundle that hooks only these names the API it needs (`hooks`), and the
//! launcher builds it on any Horizon whose API is at least that one. A change
//! that would break such a bundle is not made.
//! Bodies have no `cfg` and no macros: horizon-mixins refuses a target with
//! either (`tests::hook_points_stay_hookable`).
use super::native_lifecycle::{NativeLifecycleContext, ServerTickPhase, WorldPhase};
use super::{
    Game, GameLevel, LevelDimension, Screen,
    execution_role::ExecutionRole,
    native_panel_recipes::RecipePage,
    native_server::{NativeCommand, NativeServerPlayer},
};
use horizon_client::{
    atlas_sources::qualified,
    block_entities::Pos,
    chat::ChatSource,
    containers::{BlockEntity, MOD_MENUS, ModBlockEntity, ModContainer},
    crafting::{CustomRecipe, Ingredient, recipe_result},
    extras::Corner,
    extras_hud::Corners,
    fast_hash::FxHashMap,
    hud::{HudBatch, rgba},
    items::ItemStack,
    keybinds,
    mod_list::{self, Installed, NativePage},
    particles::{ParticleOptions, ParticleType},
    text::Text,
};
use horizon_core::{
    geometry::Direction,
    position::BlockPos,
    ticks::{ScheduleResult, TickPriority},
};
use horizon_world::{
    fluid::FluidKind,
    level::{BlockId, LevelAccess, Ticks},
    nbt::Compound,
    registry::{BlockRegistry, BlockStateId},
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use winit::{event::MouseButton, keyboard::KeyCode};

/// `Level.setBlock`'s flags, for the `native_set_block_property` helpers:
/// `flags::UPDATE_CLIENTS` as a vanilla crop grows, `flags::UPDATE_ALL` as a
/// furnace sets its `lit`. Horizon sends every write to players, so
/// `UPDATE_CLIENTS`, `UPDATE_INVISIBLE` and `UPDATE_IMMEDIATE` change nothing
/// yet.
pub(super) use horizon_world::level::flags;

/// The hook points' API version. The launcher reads this line from the
/// source it builds; keep its form.
#[allow(dead_code)]
pub(crate) const NATIVE_HOOKS_API: u32 = 41;

impl GameLevel<'_> {
    /// An authoritative server committed a changed state through `Level.setBlock`.
    /// The world contains `new`; block side effects and neighbour updates follow.
    #[inline(always)]
    pub(super) fn native_block_changed(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        old: BlockStateId,
        new: BlockStateId,
    ) {
        let _ = (dimension, pos, old, new);
    }

    /// An authoritative server is about to deliver `neighborChanged`.
    /// State and block IDs are valid in this level's registries for this call.
    #[inline(always)]
    pub(super) fn native_neighbor_changed(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        changed: BlockId,
        moved: bool,
    ) {
        let _ = (dimension, pos, state, changed, moved);
    }

    /// On an authoritative server, a fluid flows or fire spreads from `from`
    /// into `to` (`kind` is `fluid` or `fire`): `FlowingFluid.spreadTo`, and
    /// `FireBlock.tick` setting fire to `to` or burning it (`checkBurnOut`).
    /// Set `canceled` to leave the block at `to` as it is. Fluids ask on
    /// every flow: answer fast. API 22.
    #[inline(always)]
    pub(super) fn native_block_spread(
        &self,
        dimension: LevelDimension,
        kind: &str,
        from: BlockPos,
        to: BlockPos,
        canceled: &mut bool,
    ) {
        let _ = (dimension, kind, from, to, canceled);
    }

    /// On an authoritative server, the piston at `piston` is about to push
    /// the blocks at `push` one step `direction` (`[0, 1, 0]` is up; pulling
    /// when it retracts) and destroy those at `destroy`:
    /// `PistonBaseBlock.moveBlocks`, once its structure resolved. Set
    /// `canceled` and nothing moves, as when the structure can't; a sticky
    /// piston then retracts without its block, and clients are sent the
    /// blocks rather than the retraction to replay. API 22.
    #[inline(always)]
    pub(super) fn native_piston_move(
        &self,
        dimension: LevelDimension,
        piston: BlockPos,
        direction: [i32; 3],
        push: &[BlockPos],
        destroy: &[BlockPos],
        canceled: &mut bool,
    ) {
        let _ = (dimension, piston, direction, push, destroy, canceled);
    }
    /// The redstone signal the block `state` at `pos` gives on its `face`, the
    /// side that touches the block reading it: `BlockState.getDirectSignal`
    /// when `direct` (what passes through a block it touches, as a lever's
    /// does), else `getSignal`. `signal` holds vanilla's answer: change it when
    /// the block is yours (0 to 15; anything else is clamped). Asked very often,
    /// from inside the level's update, so answer from your own state: check the
    /// block first, and don't read or change the world here. When the answer
    /// changes, and once the block is placed, call
    /// `native_update_signal_neighbors`. Authoritative servers only. API 38.
    #[inline(always)]
    pub(super) fn native_block_signal(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        face: Direction,
        direct: bool,
        signal: &mut i32,
    ) {
        let _ = (dimension, pos, state, face, direct, signal);
    }

    /// `BlockState.isSignalSource`: set `source` for a block of yours that
    /// gives a signal, so redstone wire beside it connects to it. Authoritative
    /// servers only. API 38.
    #[inline(always)]
    pub(super) fn native_is_signal_source(&self, state: BlockStateId, source: &mut bool) {
        let _ = (state, source);
    }

    /// An authoritative server's `ServerLevel.tickChunk` is about to
    /// random-tick the block `state` at `pos` (`BlockState.randomTick`):
    /// only blocks that tick randomly, a mod's as its vanilla template block
    /// does. Bone meal's random ticks (`StemBlock.performBonemeal`) don't
    /// fire it. Change the block with `native_set_block_property` and set
    /// `handled` to skip the block's own random tick (a mod crop growing its
    /// own way); a block changed without `handled` takes its own random tick
    /// as it is now, if it still ticks randomly. Lava's fluid random tick
    /// runs either way.
    #[inline(always)]
    pub(super) fn native_random_tick(
        &mut self,
        ticks: &mut Ticks,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        handled: &mut bool,
    ) {
        let _ = (ticks, dimension, pos, state, handled);
    }

    /// An authoritative server is about to compute the state the block
    /// `state` at `pos` takes now that its neighbour in `direction` is
    /// `neighbour` (`BlockBehaviour.updateShape`). `state` may not be in the
    /// world yet (a piston landing it): decide from `state`, not from the
    /// block at `pos`. Answer only for your own blocks: put the new state in
    /// `new` (`state` keeps the block as it is; air breaks it, with its drops,
    /// on a neighbour update) and the block's own `updateShape` doesn't run,
    /// nor the ticks it would schedule. The level is read-only, as vanilla's
    /// `LevelReader`; ticks go through `native_schedule_block_tick` and
    /// `native_schedule_fluid_tick` with `ticks` (`ScheduledTickAccess`). It
    /// runs often, at least six times per `setBlock` that updates shapes
    /// (more for redstone wire's diagonals and knock-on updates): keep it
    /// short.
    #[inline(always)]
    #[allow(clippy::too_many_arguments)]
    pub(super) fn native_update_shape(
        &self,
        ticks: &mut Ticks,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        direction: Direction,
        neighbour_pos: BlockPos,
        neighbour: BlockStateId,
        new: &mut Option<BlockStateId>,
    ) {
        let _ = (
            ticks,
            dimension,
            pos,
            state,
            direction,
            neighbour_pos,
            neighbour,
            new,
        );
    }

    /// An authoritative server's `ServerLevel.tickBlock` is about to run a
    /// due scheduled tick of the block `state` at `pos` (`BlockState.tick`):
    /// the block is still the one the tick was scheduled for. Change the
    /// block with `native_set_block_property` and set `handled` to skip the
    /// block's own tick; a block changed without `handled` takes its own
    /// tick as it is now, if it is still the tick's block.
    #[inline(always)]
    pub(super) fn native_tick_block(
        &mut self,
        ticks: &mut Ticks,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        handled: &mut bool,
    ) {
        let _ = (ticks, dimension, pos, state, handled);
    }
}

/// The zoom a native mod asks for (`native_zoom`): the factor (1 is none),
/// eased through the field of view like the spyglass when `smooth`, and the
/// scale of mouse turning.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct NativeZoom {
    pub(crate) factor: f32,
    pub(crate) smooth: bool,
    pub(crate) turn: f32,
}

/// The minimap a native mod shows (`native_minimap`), drawn by Horizon's
/// minimap: its corner, its side in GUI pixels, blocks per map pixel,
/// turning with the player or north up, round or square.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct NativeMinimap {
    pub(crate) corner: Corner,
    pub(crate) size: f32,
    pub(crate) blocks_per_pixel: f32,
    pub(crate) rotate: bool,
    pub(crate) round: bool,
}

/// A mod block's screen over its menu (`native_screen`): its size, where
/// each of the container's slots is (`Slot.x`, `Slot.y`; none hides the
/// slot, and so do slots past the list; the server's menu does not have a
/// hidden slot either, so clicks and shift-clicks skip it), and the player inventory's first
/// row (vanilla's is 84; the hotbar is 58 below it) and left column, from
/// its top left.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NativeScreen {
    /// Vanilla's container screens are 176 wide.
    pub width: f32,
    pub height: f32,
    pub slots: Vec<Option<[f32; 2]>>,
    pub inventory_y: f32,
    /// The inventory's left column (vanilla's is 8).
    pub inventory_x: f32,
    /// The mod draws the title and the "Inventory" label itself
    /// (`native_screen_draw`); Horizon leaves them out.
    pub own_labels: bool,
}

/// The mod screen open on this client: the block it is for, the key its
/// slots are under, the window id of its menu, and the menu's data slots as
/// the server sent them.
pub(super) struct OpenNativeScreen {
    key: Pos,
    window: i32,
    block: String,
    screen: NativeScreen,
    data: Vec<i32>,
}

impl OpenNativeScreen {
    /// Data slot `id` of window `window`'s menu: false for another window's
    /// (an old or replaced one), which this screen leaves alone.
    fn set_data(&mut self, window: i32, id: i16, value: i16) -> bool {
        if window != self.window {
            return false;
        }
        // A menu's data slot ids are small; a bad one is dropped.
        if let Ok(i @ 0..256) = usize::try_from(id) {
            if self.data.len() <= i {
                self.data.resize(i + 1, 0);
            }
            self.data[i] = i32::from(value);
        }
        true
    }
}

/// The player as a native mod reads it.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NativePlayer {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) z: f64,
    /// Degrees; 0 faces south, 90 west (`Direction.fromYRot`).
    pub(crate) yaw: f32,
    pub(crate) pitch: f32,
    /// `minecraft:overworld`, `minecraft:the_nether`, `minecraft:the_end`.
    pub(crate) dimension: &'static str,
}

/// A native mod as Options > Mods lists it (`native_mods`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct NativeMod {
    /// Its id: its settings folder and its keys' lines.
    pub(crate) id: String,
    /// `Name 1.0`.
    pub(crate) title: String,
    pub(crate) description: String,
    /// Its keys: name, label and default key (`key.keyboard.z`). The page
    /// lists them first; `native_mod_key` reads the player's choice.
    pub(crate) keys: Vec<[String; 3]>,
}

/// An item the panel shows (`native_panel_items`): its id
/// (`minecraft:diamond`), a count (1 draws none) and its components beyond
/// the item's own, in vanilla's item syntax, or "".
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct NativeStack {
    pub(crate) item: String,
    pub(crate) count: u32,
    pub(crate) components: String,
}

/// How the panel lays out a recipe of `native_panel_recipes`.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NativeRecipeLayout {
    /// A crafting grid of `width` x `height` slots, an arrow and the result.
    Grid,
    /// One input over a flame, an arrow and the result (furnaces, smokers,
    /// campfires).
    Cooking,
    /// One input, an arrow and the result (the stonecutter).
    Single,
    /// Template, base and addition, an arrow and the result.
    Smithing,
    /// A potion and an ingredient, an arrow and the result.
    Brewing,
}

/// A recipe the panel shows (`native_panel_recipes`). Each slot, and the
/// result, cycles through its stacks, one a second; an empty slot shows
/// nothing.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NativeRecipeView {
    pub(crate) layout: NativeRecipeLayout,
    /// The grid's size (`Grid` only), at most 3 x 3.
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) slots: Vec<Vec<NativeStack>>,
    pub(crate) results: Vec<NativeStack>,
    /// A short line under the recipe (a cooking time), or "".
    pub(crate) note: String,
    /// The action button beside it: none, enabled, or shown greyed.
    pub(crate) action: Option<bool>,
}

/// What the player did on a native mod's panel (`native_panel_event`).
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum NativePanelEvent {
    /// A click on an item of the panel, by its index in
    /// `native_panel_items`' list.
    Clicked { index: u32, right: bool },
    /// The search box's text, each time the player changes it.
    Search(String),
    /// A key (`key.keyboard.r`; compare it with `native_mod_key`) went down
    /// while the pointer was on an item: a slot of the screen's, or the
    /// panel's.
    Key { key: String, stack: NativeStack },
    /// A click on a slot of a recipe the panel shows: the recipe's index in
    /// `native_panel_recipes`' list and the slot's in it (the results come
    /// after the slots), and the stack it showed then.
    SlotClicked {
        recipe: u32,
        slot: u32,
        stack: NativeStack,
        right: bool,
    },
    /// The action button of the recipe at this index.
    Action(u32),
    /// The player turned the recipes' page: the index of the first recipe
    /// now shown.
    Page(u32),
    /// The recipe page's back button.
    Back,
    /// The player picked the tab at this index.
    Tab(u32),
}

/// An item of [`NativeRecipes::items`], by its index there, with a count
/// and the components it has beyond the item's own, in vanilla's item
/// syntax (`[minecraft:potion_contents={potion:"minecraft:swiftness"}]`),
/// or "".
#[allow(dead_code)]
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct NativeRecipeStack {
    pub(crate) item: u32,
    pub(crate) count: u32,
    pub(crate) components: String,
}

/// The station a recipe is made at.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NativeRecipeKind {
    CraftingShaped,
    CraftingShapeless,
    Smelting,
    Blasting,
    Smoking,
    Campfire,
    Stonecutting,
    Smithing,
    Brewing,
}

/// A recipe of [`NativeRecipes`].
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NativeRecipe {
    /// `minecraft:oak_planks`.
    pub(crate) id: String,
    pub(crate) kind: NativeRecipeKind,
    /// The recipe book's tab (`crafting_building_blocks`, `furnace_food`),
    /// or "".
    pub(crate) category: String,
    /// Recipes the book shows on one button share a group, or "".
    pub(crate) group: String,
    /// A shaped recipe's grid: `slots` holds its cells row by row. 0 for
    /// the other kinds.
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// Each slot's stacks, any one of which fits; an empty list is an empty
    /// cell. Cooking and stonecutting: the input. Smithing: the template,
    /// the base, the addition. Brewing: the bottle, the reagent.
    pub(crate) slots: Vec<Vec<NativeRecipeStack>>,
    /// What it makes; several when that varies (a trim on any armour).
    pub(crate) results: Vec<NativeRecipeStack>,
    /// Cooking: game ticks and experience; 0 for the other kinds.
    pub(crate) cooking_time: u32,
    pub(crate) experience: f32,
    /// The recipe book's id for it (one per way the book shows it), as
    /// `native_known_recipes` lists them; -1 for the kinds the book doesn't
    /// show (campfire, stonecutting, smithing, brewing).
    pub(crate) book_id: i32,
}

/// An item of [`NativeRecipes::items`].
#[allow(dead_code)]
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct NativeItem {
    /// `minecraft:diamond`, `<mod>:<path>`.
    pub(crate) id: String,
    /// In the player's language.
    pub(crate) name: String,
    /// Its item tags, without the `#`.
    pub(crate) tags: Vec<String>,
}

/// Every item and recipe the game knows (`native_recipes`): `items` in the
/// registry's order, which each [`NativeRecipeStack::item`] indexes; the
/// recipes are the book's in its order, then campfire, stonecutting,
/// smithing and brewing.
#[allow(dead_code)]
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct NativeRecipes {
    pub(crate) items: Vec<NativeItem>,
    pub(crate) recipes: Vec<NativeRecipe>,
}

/// The mappings `native_mapping` answers for, with the key their default
/// binding is (none: a mouse button, attack or use).
#[allow(dead_code)]
const MAPPINGS: [(&str, Option<KeyCode>); 9] = [
    ("key.forward", Some(KeyCode::KeyW)),
    ("key.left", Some(KeyCode::KeyA)),
    ("key.back", Some(KeyCode::KeyS)),
    ("key.right", Some(KeyCode::KeyD)),
    ("key.jump", Some(KeyCode::Space)),
    ("key.sneak", Some(KeyCode::ShiftLeft)),
    ("key.sprint", Some(KeyCode::ControlLeft)),
    ("key.attack", None),
    ("key.use", None),
];

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Once a frame, in a world or not, after Horizon's extras.
    pub(super) fn native_frame(&mut self, elapsed: Duration) {
        let _ = elapsed;
    }
    /// Once a tick, in every `Game`: the window's (in single player, on LAN
    /// and on servers), and the server's own (`native_window` is false
    /// there: the single player server thread, a dedicated server).
    pub(super) fn native_tick(&mut self) {}
    /// The world's phases, read-only; `error_text` carries the engine's
    /// own open-failure text, empty otherwise. Server-side; the deprecated
    /// in-frame path delivers no lifecycle events. A server loop or final
    /// save failure fires `WorldCloseFailed` with the error text.
    pub(super) fn native_world_lifecycle(
        &mut self,
        context: &NativeLifecycleContext,
        phase: WorldPhase,
        error_text: &str,
    ) {
        let _ = (context, phase, error_text);
    }
    /// The authoritative server tick's phases: `ServerTickStart` before any
    /// level is ticked (NeoForge's `ServerTickEvent.Pre`), `ServerTickEnd`
    /// after the last (`.Post`). Read blocks with `native_server_block`, by
    /// dimension: which level `self.world` holds here is not defined. A paused
    /// pass fires nothing; a frozen world carries `world_running` false except
    /// during a requested step.
    pub(super) fn native_server_tick(
        &mut self,
        context: &NativeLifecycleContext,
        phase: ServerTickPhase,
    ) {
        let _ = (context, phase);
    }
    /// Before an authoritative server level's tick pass. `dimension` is its ID.
    pub(super) fn native_level_tick_pre(&mut self, dimension: &str) {
        let _ = dimension;
    }
    /// After a successful authoritative server level tick pass.
    pub(super) fn native_level_tick_post(&mut self, dimension: &str) {
        let _ = dimension;
    }
    /// The HUD (not drawn under F1), after the minimap: place a box with
    /// `corners.place` (boxes stack away from their edge) and draw it with
    /// `native_fill` and `native_text`. Skip a corner `native_corner_shown`
    /// says is taken.
    pub(super) fn native_hud(&mut self, batch: &mut HudBatch, corners: &mut Corners) {
        let _ = (batch, corners);
    }
    /// A player right-clicks the entity `entity` (`Entity.getId`) of type
    /// `kind` (`minecraft:cow`) with `self.hand`, holding `stack` (none in an
    /// empty hand): where `Player.interactOn` starts, before the entity's own
    /// `interact` and the item's `interactLivingEntity`. Set `claimed` to take
    /// the click: the hand swings and neither runs. Asked once per hand, as
    /// vanilla tries the main hand then the off hand. Server-side, for every
    /// player of the world (the helpers read the acting one). API 29.
    pub(super) fn native_interact_entity(
        &mut self,
        entity: i32,
        kind: &str,
        stack: Option<&ItemStack>,
        claimed: &mut bool,
    ) {
        let _ = (entity, kind, stack, claimed);
    }
    /// A player left-clicks the block `state` at `pos`, `face` the face hit
    /// (`[0, 1, 0]` is the top): where `ServerPlayerGameMode.handleBlockBreakAction`
    /// takes a dig's start, after its reach, spawn protection and world border
    /// checks, before `Block.attack` (a note block plays) or a creative break.
    /// Set `canceled` to ignore the click: the block isn't attacked, dug or
    /// broken on the server, and the player's client gets it back (its own
    /// predicted crack still shows to the clicking player). Server-side, for
    /// every player of the world (the helpers read the acting one). API 30.
    pub(super) fn native_left_click_block(
        &mut self,
        pos: [i32; 3],
        face: [i32; 3],
        state: BlockStateId,
        canceled: &mut bool,
    ) {
        let _ = (pos, face, state, canceled);
    }
    /// A living player's server tick ends (`ServerPlayer.tick`): once a tick
    /// for every player in the world, after their digging, item use,
    /// cooldowns, statistics and advancements. The helpers read that player.
    /// API 31.
    pub(super) fn native_player_tick(&mut self) {}
    /// The player-health HUD rows, once a frame the HUD draws them
    /// (`HealthHud::layout`, not under F1): `inputs` carries the health,
    /// food, air and vehicle state the layout reads, `wi`/`hi` the screen's
    /// GUI size. A mod that sets `draws` replaces every row — armour,
    /// hearts, food, air and the heights (`left_height`/`right_height`)
    /// above the hotbar are the answer's to give; `bubble_pop_sound` too.
    /// Nothing set draws Horizon's own.
    pub(super) fn native_health_hud(
        &mut self,
        inputs: &horizon_client::hud_layout::HealthInputs,
        wi: i32,
        hi: i32,
        draws: &mut Option<Vec<horizon_client::hud_layout::GuiDraw>>,
    ) {
        let _ = (inputs, wi, hi, draws);
    }
    /// A key or mouse button went down or up, by its vanilla name
    /// (`key.keyboard.z`, `key.mouse.left`): keys on any screen (check
    /// `native_playing`), mouse buttons while playing. Repeats are left out.
    pub(super) fn native_key(&mut self, key: &str, pressed: bool) {
        let _ = (key, pressed);
    }
    /// Wheel steps while playing: set `claimed` to keep them from the hotbar.
    pub(super) fn native_wheel(&mut self, steps: f32, claimed: &mut bool) {
        let _ = (steps, claimed);
    }
    /// A command typed or clicked in chat, without its slash, before the
    /// server and Horizon's commands see it: set `ran` when the mod ran it.
    pub(super) fn native_command(&mut self, command: &str, ran: &mut bool) {
        let _ = (command, ran);
    }
    /// The commands `native_command` runs, by name and without their slash:
    /// push the ones that are yours. Asked when a connection starts and
    /// again when the server sends its own tree, so the chat box colours,
    /// completes and lists them as the game's own; a name the game already
    /// has keeps that command's behaviour. API 35.
    pub(super) fn native_commands(&mut self, commands: &mut Vec<&'static str>) {
        let _ = commands;
    }
    /// The zoom a native mod asks for, applied while playing (the spyglass's
    /// own view wins): set `zoom` when it is none yet.
    pub(super) fn native_zoom(&self, zoom: &mut Option<NativeZoom>) {
        let _ = zoom;
    }
    /// The minimap a native mod shows: set `minimap` when it is none yet.
    pub(super) fn native_minimap(&self, minimap: &mut Option<NativeMinimap>) {
        let _ = minimap;
    }
    /// Options > Mods lists the native mods: push yours.
    pub(super) fn native_mods(&mut self, mods: &mut Vec<NativeMod>) {
        let _ = mods;
    }
    /// The buttons of mod `mod_id`'s settings page, under its keys: push
    /// their labels when it is yours. Asked again each time the page opens
    /// and after each click.
    pub(super) fn native_settings(&mut self, mod_id: &str, buttons: &mut Vec<String>) {
        let _ = (mod_id, buttons);
    }
    /// Button `button` of mod `mod_id`'s settings page was clicked.
    pub(super) fn native_settings_click(&mut self, mod_id: &str, button: usize) {
        let _ = (mod_id, button);
    }
    /// The player did `event` on mod `mod_id`'s panel (`native_panel_items`):
    /// act on it when it is yours. API 2.
    pub(super) fn native_panel_event(&mut self, mod_id: &str, event: &NativePanelEvent) {
        let _ = (mod_id, event);
    }
    /// Block `block` (`mymod:machine`) gets a block entity
    /// (`EntityBlock.newBlockEntity`): set `entity` to its id and saved
    /// compound when the block is yours and `entity` is none yet. The block's
    /// template must have a block entity (a chest, a furnace...), so that
    /// breaking the block removes it. API 15.
    pub(super) fn native_block_entity(
        &mut self,
        block: &str,
        entity: &mut Option<(String, Compound)>,
    ) {
        let _ = (block, entity);
    }
    /// Whether block `block`'s block entity ticks (`EntityBlock.getTicker`):
    /// set `ticks` when it is yours. Asked for each of the block's states
    /// when a world opens. API 15.
    pub(super) fn native_block_entity_ticks(&self, block: &str, ticks: &mut bool) {
        let _ = (block, ticks);
    }
    /// Block `block`'s block entity has item slots (a `WorldlyContainer`):
    /// set `container` when the block is yours. Asked once for each mod
    /// block when the ticking table is built; at most 256 slots, and slot
    /// numbers past the count are left out. Its `menu`, one of
    /// `containers::MOD_MENUS` with as many slots, opens when the block is
    /// used (its template must open a menu, as a chest or a furnace does);
    /// another is left out. API 17.
    pub(super) fn native_block_entity_container(
        &self,
        block: &str,
        container: &mut Option<ModContainer>,
    ) {
        let _ = (block, container);
    }
    /// A server tick of the block entity at `pos`, in vanilla's order among
    /// the others (`Level.tickBlockEntities`), when its `id` is yours: read
    /// and change its saved compound `nbt`, its item slots `items` (keep
    /// their count: as many as its container has, none without one) and the
    /// data slots its menu shows (`data`). It is out of the world during the
    /// call (the block entity at `pos` reads as a `Mod` with an empty id, and
    /// a save leaves it out); if the call removes or replaces it, `entity` is
    /// dropped. Clients, the window's own in single player included, don't
    /// get it yet. API 17.
    pub(super) fn native_block_entity_tick(&mut self, pos: [i32; 3], entity: &mut ModBlockEntity) {
        let _ = (pos, entity);
    }
    /// A menu opens on this client for block `block`, a mod's: set `screen`
    /// when the block is yours, to lay its screen out (`native_screen_draw`
    /// draws it). The server asks it too: a slot it hides is not in the menu
    /// there either, so shift-clicks never fill it. API 19.
    pub(super) fn native_screen(&self, block: &str, screen: &mut Option<NativeScreen>) {
        let _ = (block, screen);
    }
    /// Draw block `block`'s screen at `origin` (its top left, GUI pixels),
    /// over a plain panel with its slots' frames and under the items and
    /// labels, from the menu's data slots `data` (the block entity's `data`
    /// on the server): data slots are 16-bit on the wire as in vanilla (a
    /// bigger value wraps), and `data` is empty until the server's first
    /// values arrive, so read it with `get`. Draw with `native_sprite`, `native_fill` and
    /// `native_text`. API 19.
    pub(super) fn native_screen_draw(
        &self,
        block: &str,
        data: &[i32],
        batch: &mut HudBatch,
        origin: [f32; 2],
    ) {
        let _ = (block, data, batch, origin);
    }
    /// A left or right (`right`) press at `at` (GUI pixels from the screen's
    /// top left, so outside it too) on block `block`'s screen, a mod's, over
    /// no slot: set `button` to a menu button id when it is on one of yours.
    /// Horizon sends it to the server (`ContainerButtonClick`,
    /// `native_menu_button` there) and the press goes no further. API 20.
    pub(super) fn native_screen_click(
        &self,
        block: &str,
        data: &[i32],
        at: [f32; 2],
        right: bool,
        button: &mut Option<i32>,
    ) {
        let _ = (block, data, at, right, button);
    }
    /// The tooltip at `at` (as in `native_screen_click`) on block `block`'s
    /// screen, a mod's, over no item: push its lines when it is over
    /// something of yours. Legacy colour codes (§) work. API 20.
    pub(super) fn native_screen_tooltip(
        &self,
        block: &str,
        data: &[i32],
        at: [f32; 2],
        lines: &mut Vec<String>,
    ) {
        let _ = (block, data, at, lines);
    }
    /// A player pressed menu button `id` on the menu of the mod block entity
    /// at `pos` (`AbstractContainerMenu.clickMenuButton`), on the server:
    /// change `entity` when its id is yours. It is out of the world during
    /// the call, as in `native_block_entity_tick`. API 20.
    pub(super) fn native_menu_button(
        &mut self,
        pos: [i32; 3],
        entity: &mut ModBlockEntity,
        id: i32,
    ) {
        let _ = (pos, entity, id);
    }
    /// The commands native mods add to the server's command tree: push
    /// yours. Asked once when a world opens to players (a dedicated
    /// server, a LAN host, single player's server thread); every player's
    /// chat box then completes them as vanilla's, within their level. A
    /// name vanilla has is left out. API 8.
    pub(super) fn native_register_commands(&mut self, commands: &mut Vec<NativeCommand>) {
        let _ = commands;
    }
    /// A player's command on the server (`Commands.performCommand`),
    /// without its slash, for a command a mod registered and the player's
    /// level allows: set `ran` when it is yours. The player's state is
    /// swapped in for the call, so `native_player` and the other helpers
    /// read that player; `native_feedback` answers them. API 8.
    pub(super) fn native_perform_command(
        &mut self,
        player: &NativeServerPlayer,
        command: &str,
        ran: &mut bool,
    ) {
        let _ = (player, command, ran);
    }
    /// A click in a menu a mod opened (`native_open_menu`), which moved
    /// nothing: the menu's `slot` (its own first, then the player's
    /// inventory, `native_menu_inventory_slot`; -999 outside), the `button`
    /// and the click's `mode` (`ClickType`'s id) of the player `uuid`.
    /// No player's state is swapped in: the menu helpers take the player's
    /// UUID. API 10.
    pub(super) fn native_menu_clicked(
        &mut self,
        uuid: u128,
        window: i32,
        slot: i32,
        button: i32,
        mode: i32,
    ) {
        let _ = (uuid, window, slot, button, mode);
    }
    /// A menu a mod opened closed: the player closed it, the server or a mod
    /// opened another, or the player left. API 10.
    pub(super) fn native_menu_closed(&mut self, uuid: u128, window: i32) {
        let _ = (uuid, window);
    }
    /// An item's tooltip, wherever the game shows one (slots, the creative
    /// tabs, the recipe book, chat's hover): the stack, with its components
    /// (`stack.data()`: a shulker box's contents...), and its lines, the name
    /// first, each with a colour as `rgba` packs it (`u32::MAX` keeps the
    /// text's own). Add, change or take out lines; `Text::legacy` reads `§`
    /// codes. Asked each frame the tooltip shows, never for one the stack
    /// hides. API 12.
    pub(super) fn native_tooltip(&self, stack: &ItemStack, lines: &mut Vec<(Text, u32)>) {
        let _ = (stack, lines);
    }
    /// A player uses `stack` (right click) with `self.hand` on the block at
    /// `pos`, `face` the face clicked (`[0, 1, 0]` is the top), once the
    /// block itself let the click through (a chest still opens): where
    /// `ItemStack.useOn` starts. Set `claimed` to take the use: the hand
    /// swings and neither the item's `useOn` nor its `use` runs.
    /// Server-side, for every player of the world (the helpers read the
    /// acting one); an item on cooldown is never used. API 13.
    pub(super) fn native_use_item_on(
        &mut self,
        stack: &ItemStack,
        pos: [i32; 3],
        face: [i32; 3],
        claimed: &mut bool,
    ) {
        let _ = (stack, pos, face, claimed);
    }
    /// A player uses `stack` (right click) with `self.hand`: in the air, or
    /// on a block once `native_use_item_on` let it through, where `Item.use`
    /// starts. Each click reaches it once. Set `claimed` to take the use: the
    /// hand swings and the item's own use doesn't run. Server-side, for every
    /// player of the world; an item on cooldown is never used. API 13.
    pub(super) fn native_use_item(&mut self, stack: &ItemStack, claimed: &mut bool) {
        let _ = (stack, claimed);
    }
    /// A player breaks the block `state` at `pos`, once the game allowed it
    /// (the held item can break blocks, the debug stick didn't take the
    /// click): where `ServerPlayerGameMode.destroyBlock` starts. Set
    /// `canceled` to keep the block: nothing is destroyed or dropped, and a
    /// guest sees it back. Server-side, for every player of the world (the
    /// helpers read the acting one). API 14.
    pub(super) fn native_destroy_block(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        canceled: &mut bool,
    ) {
        let _ = (pos, state, canceled);
    }
    /// A player placed `stack` as the block `state` at `pos`: where
    /// `BlockItem.place` set it, before the stack shrinks. It only hears; a
    /// mod stops a placement by claiming `native_use_item_on`, which runs
    /// before it. Server-side, for every player of the world. API 14.
    pub(super) fn native_place_block(
        &mut self,
        stack: &ItemStack,
        pos: [i32; 3],
        state: BlockStateId,
    ) {
        let _ = (stack, pos, state);
    }
    /// A player right clicks the block at `pos` with `self.hand`, `face` the
    /// face clicked: where `ServerPlayerGameMode.useItemOn` starts, before
    /// the block's own use (a chest, a door, a lever) and before
    /// `native_use_item_on`. Set `canceled` to stop the click: neither the
    /// block nor the item acts, the hand doesn't swing, and a guest sees the
    /// block as it is. Server-side, for every player of the world
    /// (`native_acting_player` says who), once per hand. API 21.
    pub(super) fn native_right_click_block(
        &mut self,
        pos: [i32; 3],
        face: [i32; 3],
        canceled: &mut bool,
    ) {
        let _ = (pos, face, canceled);
    }
    /// An explosion at `center` in `dimension` (`minecraft:overworld`) is
    /// about to break `blocks`: `ServerExplosion.explode` with its `toBlow`,
    /// before any block is hit. Take out the positions to spare; they keep
    /// their block and get no fire. Entities are hurt as before. Server-side.
    /// API 22.
    pub(super) fn native_explosion_detonate(
        &mut self,
        dimension: &str,
        center: [f64; 3],
        blocks: &mut Vec<[i32; 3]>,
    ) {
        let _ = (dimension, center, blocks);
    }
    /// A player fills (`stack` an empty bucket) or empties a bucket at `pos`:
    /// `BucketItem.use`, where it asks `Level.mayInteract`. Set `canceled`
    /// and the bucket does nothing. Server-side, for every player of the
    /// world (`native_acting_player` says who). API 22.
    pub(super) fn native_fill_bucket(
        &mut self,
        stack: &ItemStack,
        pos: [i32; 3],
        canceled: &mut bool,
    ) {
        let _ = (stack, pos, canceled);
    }
    /// A player attacks the entity `entity` (`Entity.getId`) of type `kind`
    /// (`minecraft:item_frame`, `minecraft:cow`, `minecraft:player`) standing
    /// at `pos`: `ServerGamePacketListenerImpl.handleInteract`'s attack, once
    /// in reach, where `Player.attack` starts. Set `canceled` and the blow
    /// does nothing (no damage, no knockback, the frame keeps its item).
    /// Server-side, for every player of the world (`native_acting_player`
    /// says who). API 22.
    pub(super) fn native_attack_entity(
        &mut self,
        entity: i32,
        kind: &str,
        pos: [f64; 3],
        canceled: &mut bool,
    ) {
        let _ = (entity, kind, pos, canceled);
    }
    /// A message from `player`'s client on `channel` (`mod_id:name`), within
    /// the caps (`native_net.rs`): check `channel`, and treat `data` as the
    /// player's own bytes, checked by nothing. Runs in the tick after it
    /// arrives, with no player's state swapped in. Server-side. API 40.
    pub(super) fn native_net_from_client(&mut self, player: u128, channel: &str, data: &[u8]) {
        let _ = (player, channel, data);
    }
    /// A message from the server on `channel` (`mod_id:name`), whose `data`
    /// is the sender's own bytes. The window's game only. API 40.
    pub(super) fn native_net_from_server(&mut self, channel: &str, data: &[u8]) {
        let _ = (channel, data);
    }
    /// `player` died: where `ServerPlayer.die` starts, with the death
    /// message's key when the game knows it (`death.attack.lava`; none for a
    /// message it builds from the combat). It only hears. Server-side, for
    /// every player of the world, whether death messages are shown or not.
    /// API 40.
    pub(super) fn native_player_died(&mut self, player: u128, cause: Option<&str>) {
        let _ = (player, cause);
    }
    /// A new mob of `kind` joined `dimension` at `pos`: `natural` when the
    /// world's spawning made it, false when something else did (a breed, a
    /// raid, a spawner, a conversion, `/summon`, and the second mob of a
    /// jockey the world spawned). One call a mob: a `/summon` with passengers
    /// tells each. Not told of a mob loaded with its chunk, leaving a hive or
    /// changing level. It only hears; the mob is already among the game's
    /// mobs. Server-side. API 40.
    pub(super) fn native_mob_spawned(
        &mut self,
        dimension: &str,
        kind: &str,
        pos: [f64; 3],
        natural: bool,
    ) {
        let _ = (dimension, kind, pos, natural);
    }
    /// `player`'s melee blow killed the mob `entity` of `kind` (other deaths
    /// aren't told yet). It only hears, after the blow landed. Server-side.
    /// API 40.
    pub(super) fn native_mob_killed(&mut self, player: u128, entity: u32, kind: &str) {
        let _ = (player, entity, kind);
    }
    /// `player` threw `count` of `item` (`Player.drop`: the drop key, a
    /// menu's throw...). It only hears. Server-side. API 40.
    pub(super) fn native_item_dropped(&mut self, player: u128, item: &str, count: u32) {
        let _ = (player, item, count);
    }
    /// `player` is about to take `count` of `item` from the ground
    /// (`ItemEntity.playerTouch`). Set `canceled` to leave it there: the
    /// stack isn't offered again for 10 ticks. Server-side. API 40.
    pub(super) fn native_item_picked_up(
        &mut self,
        player: u128,
        item: &str,
        count: u32,
        canceled: &mut bool,
    ) {
        let _ = (player, item, count, canceled);
    }
    /// A player's hit landed on the mob `entity` of type `kind`
    /// (`minecraft:zombie`), holding `stack`: where `Item.hurtEnemy` is called,
    /// before the post-attack enchantments and the weapon's wear. It only
    /// hears. Another player as the target isn't told yet. Server-side, for
    /// every player of the world (the helpers read the acting one). API 39.
    pub(super) fn native_hurt_enemy(&mut self, stack: &ItemStack, entity: u32, kind: &str) {
        let _ = (stack, entity, kind);
    }
    /// A server tick of the stack a player carries in `slot`: where
    /// `Inventory.tick` calls `Item.inventoryTick`, for every stack, each tick,
    /// `/tick freeze` included (it stops the world, not the player). `slot` is
    /// as `Inventory` saves it (0 to 35; the armour 100 feet to 103 head; -106
    /// the off hand), and `selected` says it is the held slot.
    /// `native_carried_stack` reads and changes the stack, which stays in its
    /// slot, so the item helpers see it: a count of 0 empties the slot, and the
    /// player's client gets the change. This runs often, so check the item
    /// first. Server-side, for every living player of the world (the helpers
    /// read the acting one). API 39.
    pub(super) fn native_inventory_tick(&mut self, slot: i32, selected: bool) {
        let _ = (slot, selected);
    }
    /// A player's equipment slot `slot` (`head`, `chest`, `legs`, `feet`,
    /// `offhand`, and `mainhand` on a swap of the hands) took a different
    /// item, `new` (none when it was emptied): `LivingEntity.onEquipItem`,
    /// before the equip sound. It only hears. Never for a spectator.
    /// Server-side, for every player of the world (the helpers read the
    /// acting one). API 39.
    pub(super) fn native_on_equip(&mut self, slot: &str, new: Option<&ItemStack>) {
        let _ = (slot, new);
    }
}

fn dimension_id(dimension: LevelDimension) -> &'static str {
    match dimension {
        LevelDimension::Overworld => "minecraft:overworld",
        LevelDimension::Nether => "minecraft:the_nether",
        LevelDimension::End => "minecraft:the_end",
    }
}

/// Server-level helpers for native mods.
#[allow(dead_code)]
impl GameLevel<'_> {
    /// Play a registered vanilla sound for nearby players in this server level.
    /// The return value means queued, not heard. Unknown sound events are
    /// rejected at dispatch when the server registry is available.
    pub(super) fn native_play_sound(
        &mut self,
        sound: &str,
        at: [f64; 3],
        source: usize,
        volume: f32,
        pitch: f32,
    ) -> bool {
        if !self.native_authoritative || !sound.starts_with("minecraft:") {
            return false;
        }
        if source >= horizon_client::settings::SOUND_SOURCES.len() {
            return false;
        }
        let position_ok = at
            .into_iter()
            .all(|v| v.is_finite() && v.abs() <= f64::from(i32::MAX) / 8.0);
        let gain_ok = volume.is_finite() && volume >= 0.0 && pitch.is_finite() && pitch >= 0.0;
        if !position_ok || !gain_ok {
            return false;
        }
        self.native_sounds.push(super::creatures::NativeSound {
            dimension: dimension_id(self.dimension),
            sound: sound.to_owned(),
            at,
            source: source as i32,
            volume,
            pitch,
        });
        true
    }

    /// Queue particles in this authoritative level. A zero count sends one
    /// directional particle. The return value means accepted, not delivered.
    /// Block particle types require their block state; item, trail and
    /// vibration particles are not supported by the current network dispatch.
    pub(super) fn native_send_particles(
        &mut self,
        options: &ParticleOptions,
        state: Option<BlockStateId>,
        at: [f64; 3],
        spread: [f64; 3],
        speed: f64,
        count: i32,
    ) -> bool {
        use ParticleType as T;
        let supported = match options.ty {
            T::Block | T::BlockMarker | T::FallingDust | T::DustPillar | T::BlockCrumble => {
                state.is_some_and(|id| self.registry.state(id).is_some())
            }
            T::Item | T::Trail | T::Vibration => false,
            T::Geyser | T::GeyserPlume | T::GeyserBase | T::GeyserPoof => options
                .geyser
                .is_some_and(|(_, impulse)| impulse.is_finite()),
            _ => true,
        };
        if !self.native_authoritative
            || !supported
            || !at.into_iter().all(f64::is_finite)
            || !spread
                .into_iter()
                .all(|v| v.is_finite() && v.abs() <= f32::MAX as f64)
            || !speed.is_finite()
            || speed.abs() > f32::MAX as f64
            || !options.color.into_iter().all(f32::is_finite)
            || !options.to_color.into_iter().all(f32::is_finite)
            || !options.scale.is_finite()
        {
            return false;
        }
        self.native_particles
            .push(super::lan_particles::QueuedParticle {
                dimension: dimension_id(self.dimension),
                options: options.clone(),
                state,
                at,
                spread,
                speed,
                count,
            });
        true
    }
    /// Sets property `key` of the block at `pos` to `value` with
    /// `Level.setBlock(pos, state, flags)`, from a hook that gets the level's
    /// `ticks` (`native_random_tick`, `native_tick_block`). A crop's random
    /// tick grows it with `flags::UPDATE_CLIENTS`: neighbours' shapes update,
    /// but no neighbour gets `neighborChanged` and comparators aren't told;
    /// `flags::UPDATE_ALL` does both. Horizon sends every write to players
    /// whatever the flags: `UPDATE_CLIENTS`, `UPDATE_INVISIBLE` and
    /// `UPDATE_IMMEDIATE` change nothing yet. Any loaded position, not only
    /// the hook's; false off an authoritative server, when the block has no
    /// such property or value, or `pos` is not loaded; true without a write
    /// when the property already has `value`. Elsewhere,
    /// `Game::native_set_block_property_with` does the same. API 33.
    pub(super) fn native_set_block_property(
        &mut self,
        ticks: &mut Ticks,
        pos: [i32; 3],
        key: &str,
        value: &str,
        flags: i32,
    ) -> bool {
        if !self.native_authoritative {
            return false;
        }
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        let state = self.block_state(at);
        let Some(id) = with_property(self.registry, state, key, value) else {
            return false;
        };
        if id == state {
            return true;
        }
        let b = self.behaviours;
        b.set_block(self, ticks, at, id, flags)
    }
    /// Schedules a tick of `block` (a block name) at `pos` in `delay` game
    /// ticks, as a falling block's shape update schedules its fall
    /// (`ScheduledTickAccess.scheduleTick(pos, block, delay)`, normal
    /// priority); false for an unknown block or a position that isn't
    /// loaded. A tick of that block already queued there is kept and counts
    /// as scheduled. `ticks` is the hook's. API 32.
    pub(super) fn native_schedule_block_tick(
        &self,
        ticks: &mut Ticks,
        pos: [i32; 3],
        block: &str,
        delay: i32,
    ) -> bool {
        let Some(state) = self.registry.default_state(block) else {
            return false;
        };
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        let block = self.behaviours.block(state);
        let time = self.game_time();
        ticks.schedule_block(time, at, block, delay, TickPriority::Normal)
            != ScheduleResult::MissingChunk
    }
    /// Schedules a tick of `fluid` (`minecraft:water`, `minecraft:lava`,
    /// `minecraft:flowing_water`...) at `pos` in `delay` game ticks, as a
    /// waterlogged block's shape update schedules its water in 5
    /// (`ScheduledTickAccess.scheduleTick(pos, fluid, delay)`); false for
    /// an unknown fluid or a position that isn't loaded. A tick of that
    /// fluid already queued there is kept and counts as scheduled. `ticks` is
    /// the hook's. API 32.
    pub(super) fn native_schedule_fluid_tick(
        &self,
        ticks: &mut Ticks,
        pos: [i32; 3],
        fluid: &str,
        delay: i32,
    ) -> bool {
        let Some(kind) = FluidKind::from_name(fluid) else {
            return false;
        };
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        let time = self.game_time();
        ticks.schedule_fluid(time, at, kind, delay) != ScheduleResult::MissingChunk
    }
}

/// Helpers for native mods, kept as stable as the hook points.
// Native mods use these; nothing in Horizon does.
#[allow(dead_code)]
impl Game<'_> {
    /// The stack the acting player carries in `slot`, by
    /// `native_inventory_tick`'s numbers: none for an empty slot or another
    /// number. Change it as you like: a count of 0 empties the slot at the end
    /// of `native_inventory_tick`, and the player's client gets the change.
    /// API 39.
    pub(super) fn native_carried_stack(&mut self, slot: i32) -> Option<&mut ItemStack> {
        self.native_carried_slot(slot)?.as_mut()
    }
    /// Whether the player left mod `mod_id` on in Options > Mods. A mod
    /// turned off does nothing (check this first in each hook) until the
    /// launcher's next build leaves it out.
    pub(super) fn native_on(&self, mod_id: &str) -> bool {
        !self.settings.mods.disabled.contains(mod_id)
    }
    /// Sets a block in the current authoritative level through `Level.setBlock`.
    /// Returns false for another dimension, an invalid state, or no change.
    /// `flags` selects vanilla's neighbour and shape updates; Horizon sends
    /// every write to players whatever the flags. A block entity block gets
    /// its block entity. API 24.
    pub(super) fn native_set_block(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        state: BlockStateId,
        flags: i32,
    ) -> bool {
        if (self.serving.is_none() && self.options.dedicated.is_none())
            || dimension != self.dim.id()
            || self.pack.assets.registry.state(state).is_none()
        {
            return false;
        }
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        let previous = self.world.block(pos[0], pos[1], pos[2]);
        let changed =
            self.with_level(|b, level, ticks| b.set_block(level, ticks, at, state, flags));
        let new = self.res.facts().get(state).name.clone();
        let landed = self.world.block(pos[0], pos[1], pos[2]);
        if changed
            && self.res.facts().get(previous).name != new
            && self.res.facts().get(landed).name == new
            && let Some(entity) = self.new_block_entity(&new)
        {
            self.world
                .block_entities
                .insert((pos[0], pos[1], pos[2]), entity);
        }
        let _ = self.process_level_events();
        changed
    }
    /// Sets property `key` of the block at `pos` to `value`, as a furnace's
    /// tick sets its `lit` (`Level.setBlock` with `UPDATE_ALL`: neighbours
    /// and clients are told); false when the block has no such property or
    /// value. The block entity stays. API 18.
    pub(super) fn native_set_block_property(
        &mut self,
        pos: [i32; 3],
        key: &str,
        value: &str,
    ) -> bool {
        self.native_set_block_property_with(pos, key, value, flags::UPDATE_ALL)
    }
    /// `native_set_block_property` with `Level.setBlock`'s `flags`, as a
    /// brewing stand's tick sets its bottles with `flags::UPDATE_CLIENTS`.
    /// Players get the write whatever the flags (`flags`); false when this
    /// window isn't a server. API 33.
    pub(super) fn native_set_block_property_with(
        &mut self,
        pos: [i32; 3],
        key: &str,
        value: &str,
        flags: i32,
    ) -> bool {
        // As `native_set_block`: a window that isn't a server holds a copy.
        if self.serving.is_none() && self.options.dedicated.is_none() {
            return false;
        }
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let Some(id) = with_property(self.pack.assets.registry, state, key, value) else {
            return false;
        };
        if id == state {
            return true;
        }
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        self.with_level(|b, level, ticks| b.set_block(level, ticks, at, id, flags))
    }
    /// The block at `pos` changed the signal it gives: its neighbours, and
    /// theirs (a block it powers passes the signal on), read it again, as
    /// around a lever (`Level.updateNeighborsAt`). Call it when the answer
    /// changes and once the block is placed. Acts on the level in place:
    /// false (and nothing happens) when `dimension` is another, or this
    /// window isn't a server. API 38.
    pub(super) fn native_update_signal_neighbors(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
    ) -> bool {
        if (self.serving.is_none() && self.options.dedicated.is_none())
            || dimension != self.dim.id()
        {
            return false;
        }
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        let state = self.world.block(pos[0], pos[1], pos[2]);
        self.with_level(|b, level, ticks| {
            let block = b.block(state);
            b.update_neighbors_at(level, ticks, at, block);
            for [x, y, z] in [
                [0, -1, 0],
                [0, 1, 0],
                [0, 0, -1],
                [0, 0, 1],
                [-1, 0, 0],
                [1, 0, 0],
            ] {
                b.update_neighbors_at(level, ticks, at.offset(x, y, z), block);
            }
        });
        true
    }
    /// Runs `change` on the mod block entity at `pos` (a generator charging
    /// the machine beside it) and returns what it returns; none for any
    /// other block entity, and for the one ticking now, which is out of the
    /// world. Afterwards, as its `setChanged` would, its chunk is saved next
    /// time and, when its items changed, comparators read it again. API 18.
    pub(super) fn native_with_mod_block_entity<R>(
        &mut self,
        pos: [i32; 3],
        change: impl FnOnce(&mut ModBlockEntity) -> R,
    ) -> Option<R> {
        let key = (pos[0], pos[1], pos[2]);
        let Some(BlockEntity::Mod(m)) = self.world.block_entities.get_mut(&key) else {
            return None;
        };
        if m.id.is_empty() {
            return None;
        }
        let items = m.items.clone();
        let out = change(m);
        let moved = m.items != items;
        self.world.modified.insert((key.0 >> 4, key.2 >> 4));
        if moved {
            self.container_changed(key);
        }
        Some(out)
    }
    /// Playing, with nothing over the world: keys and zoom apply.
    pub(super) fn native_playing(&self) -> bool {
        self.screen == Screen::Playing
            && self.title.is_none()
            && self.sign_edit.is_none()
            && self.book.is_none()
    }
    /// This `Game` is the window's, not a server's.
    pub(super) fn native_window(&self) -> bool {
        self.options.execution_role == ExecutionRole::Client
    }
    /// Whether a box in `corner` shows: F3's debug screen takes both top
    /// corners (its left and right columns), and a bottom corner while one
    /// of its charts is drawn there.
    pub(super) fn native_corner_shown(&self, corner: Corner) -> bool {
        let charts = self.debug_charts.corners(!self.has_singleplayer_server());
        !debug_takes(corner, self.debug, charts)
    }
    /// The width of `text` in GUI pixels.
    pub(super) fn native_text_width(&self, text: &str) -> f32 {
        HudBatch::text_width(&self.gui.atlas, text)
    }
    /// A rectangle in GUI pixels, `argb` (0x78000000 is the extras' box).
    pub(super) fn native_fill(&self, batch: &mut HudBatch, rect: [f32; 4], argb: u32) {
        let [x, y, w, h] = rect;
        let [a, r, g, b] = argb.to_be_bytes();
        batch.fill(&self.gui.atlas, x, y, w, h, rgba(r, g, b, a));
    }
    /// Part `src` (`[x, y, w, h]` in the sprite's own pixels) of sprite
    /// `name` (vanilla's `gui/...`, or a content mod's GUI texture
    /// `mymod:gui/...`), drawn stretched into `dst` (`[x, y, w, h]` in GUI
    /// pixels). False when there is no such sprite. API 19.
    pub(super) fn native_sprite(
        &self,
        batch: &mut HudBatch,
        name: &str,
        src: [f32; 4],
        dst: [f32; 4],
    ) -> bool {
        let Some(mut r) = self.gui.atlas.sprites.get(name).copied() else {
            return false;
        };
        let (du, dv) = ((r.u1 - r.u0) / r.w as f32, (r.v1 - r.v0) / r.h as f32);
        r.u0 += src[0] * du;
        r.v0 += src[1] * dv;
        r.u1 = r.u0 + src[2] * du;
        r.v1 = r.v0 + src[3] * dv;
        batch.rect_uv(dst[0], dst[1], dst[2], dst[3], r, u32::MAX);
        true
    }
    /// Item `item`'s icon (`mymod:machine`) with its top left at `at`,
    /// `size` GUI pixels square (16 in a slot); false for an unknown item.
    /// API 20.
    pub(super) fn native_item(
        &self,
        batch: &mut HudBatch,
        item: &str,
        at: [f32; 2],
        size: f32,
    ) -> bool {
        if self.res.items().get(item).is_none() {
            return false;
        }
        let start = batch.vertices.len();
        self.draw_item_model(batch, &ItemStack::new(item, 1), at[0], at[1]);
        batch.scale_since(start, at[0], at[1], size / 16.0, size / 16.0);
        true
    }
    /// The open menu's title (the block's name, or the one it was given),
    /// as plain text. API 20.
    pub(super) fn native_screen_title(&self) -> Option<String> {
        self.remote_menu_title().map(|t| t.plain())
    }
    /// A data file of the game's resources, content mods' included
    /// (`data/mymod/machines.json`), when there is one: what a mod's content
    /// says, read at runtime. Only relative paths of plain names. API 20.
    pub(super) fn native_data(&self, path: &str) -> Option<Vec<u8>> {
        let plain = |p: &str| !p.is_empty() && p != "." && p != ".." && !p.contains('\\');
        if !path.starts_with("data/") || !path.split('/').all(plain) {
            return None;
        }
        fs::read(self.options.resources.join(path)).ok()
    }
    /// The block at `pos` (`mymod:cable`; air where nothing is loaded).
    /// API 20.
    pub(super) fn native_block(&self, pos: [i32; 3]) -> String {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        self.res.facts().get(state).name.to_string()
    }
    /// Property `key` of the block at `pos` (a machine's `facing`), if it
    /// has one. API 20.
    pub(super) fn native_block_property(&self, pos: [i32; 3], key: &str) -> Option<String> {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        self.pack
            .assets
            .registry
            .state(state)?
            .properties
            .get(key)
            .cloned()
    }
    /// Puts `stack` into the container at `pos` (a chest, a furnace, a mod
    /// container) through its side `face`, as a hopper would, and returns
    /// what did not fit: a machine's auto-output. API 20.
    pub(super) fn native_insert_item(
        &mut self,
        pos: [i32; 3],
        face: Direction,
        stack: ItemStack,
    ) -> Option<ItemStack> {
        self.insert_into_container((pos[0], pos[1], pos[2]), stack, face)
    }
    /// Where the mouse is, in GUI pixels. API 20.
    pub(super) fn native_cursor(&self) -> [f32; 2] {
        [self.cursor.0, self.cursor.1]
    }
    /// The size of sprite `name` in its own pixels, if there is one. API 19.
    pub(super) fn native_sprite_size(&self, name: &str) -> Option<[f32; 2]> {
        let r = self.gui.atlas.sprites.get(name)?;
        Some([r.w as f32, r.h as f32])
    }
    /// A line of text at `(x, y)` in GUI pixels, opaque `rgb`.
    pub(super) fn native_text(
        &self,
        batch: &mut HudBatch,
        text: &str,
        at: [f32; 2],
        rgb: u32,
        shadow: bool,
    ) {
        let [x, y] = at;
        let [_, r, g, b] = rgb.to_be_bytes();
        let color = rgba(r, g, b, 0xFF);
        batch.text_styled(&self.gui.atlas, &Text::literal(text), x, y, color, shadow);
    }
    /// The player, in a world.
    pub(super) fn native_player(&self) -> Option<NativePlayer> {
        let p = self.player.as_ref()?;
        Some(NativePlayer {
            x: p.position.x,
            y: p.position.y,
            z: p.position.z,
            yaw: self.yaw,
            pitch: self.pitch,
            dimension: self.dim.id(),
        })
    }
    /// The frame counter: frames in the last second, the slowest frame and
    /// the 1% low of the last 5 seconds, in frames per second.
    pub(super) fn native_frame_rate(&self) -> (u32, u32, u32) {
        self.frame_reading()
    }
    /// Attack and use clicks in the last second.
    pub(super) fn native_clicks(&mut self) -> (u32, u32) {
        self.mouse_reading().0
    }
    /// A vanilla mapping (`key.forward`, `key.jump`, `key.attack`…): whether
    /// it is held, and its key's label as Controls shows it.
    pub(super) fn native_mapping(&mut self, name: &str) -> Option<(bool, String)> {
        let &(_, key) = MAPPINGS.iter().find(|m| m.0 == name)?;
        let label = keybinds::display_name(self.settings.keys.get(name)).plain();
        let down = match key {
            Some(k) => self.keys.contains(&k),
            None => self.mouse_reading().1[usize::from(name == "key.use")],
        };
        Some((down, label))
    }
    /// The key the player chose for a mod's key `name`, else `default`
    /// (`key.keyboard.z`), as the WebAssembly mods' keys were kept
    /// (`key.<mod id>.<name>` in `horizon-mods.txt`).
    pub(super) fn native_mod_key(&self, mod_id: &str, name: &str, default: &str) -> String {
        let chosen = self
            .settings
            .mods
            .keys
            .get(mod_id)
            .and_then(|k| k.get(name));
        chosen.map_or(default, String::as_str).to_owned()
    }
    /// A line in chat from the client.
    pub(super) fn native_chat(&mut self, text: &str) {
        let focused = self.screen == Screen::Chat;
        let line = Text::literal(text);
        let atlas = &self.gui.atlas;
        self.ui
            .chat
            .add(atlas, line, None, ChatSource::Client, self.ticks, focused);
    }
    /// A native mod's own folder: `horizon-mods/.horizon-mod-data/<id>/`,
    /// where the WebAssembly mod of the same id kept its storage, so a
    /// player's settings carry over. Not created here.
    pub(super) fn native_data_dir(&self, mod_id: &str) -> PathBuf {
        let mods = Self::options_path().with_file_name(mod_list::DIR);
        mods.join(".horizon-mod-data").join(mod_id)
    }
    /// The folder mod `mod_id`'s post-build step wrote into when the launcher built the game
    /// (`native-data/<id>/` in the build's version folder); None when the game's own path is
    /// unknown. API 41.
    pub(super) fn native_built_data(&self, mod_id: &str) -> Option<PathBuf> {
        horizon_client::native_data::built(mod_id)
    }
    /// A mod's setting `key` (one file per key in `native_data_dir`).
    pub(super) fn native_setting(&self, mod_id: &str, key: &str) -> Option<String> {
        let path = self.native_data_dir(mod_id).join(key);
        fs::read_to_string(path).ok().map(|v| v.trim().to_owned())
    }
    /// Keeps a mod's setting `key`.
    pub(super) fn native_set_setting(&self, mod_id: &str, key: &str, value: &str) {
        let dir = self.native_data_dir(mod_id);
        let _ = fs::create_dir_all(&dir).and_then(|()| fs::write(dir.join(key), value));
    }
    /// The game's `config/` folder beside `options.txt`, shared by every mod
    /// as on Fabric and NeoForge: a mod names its own files in it
    /// (`config/<id>.json`, or a folder of its own). The same folder on the
    /// window, its server thread and a LAN host; a dedicated server's is in
    /// its own folder, beside `server.properties`. Made when missing. No
    /// `self`, so it works from any hook (`Game::native_config_dir()`), the
    /// level's too. API 26.
    pub(super) fn native_config_dir() -> Result<PathBuf, String> {
        config_dir(&Self::options_path())
    }
    /// Shows the panel beside container screens with these items (at most
    /// 8192), as the game's own slots in pages, with a search box; the
    /// player's clicks, searches and keys over items come back as
    /// `native_panel_event`. The panel is the last mod's to show one; empty
    /// items and no recipe page take yours away.
    pub(super) fn native_panel_items(&mut self, mod_id: &str, items: Vec<NativeStack>) {
        self.panel.show_items(mod_id, items);
    }
    /// Shows these recipes in the panel instead of its items, as many as
    /// fit a page, under `title` and a back button, with a row of `tabs`
    /// (each an icon; `tab` is the selected one) between them. At most 256
    /// recipes and 16 tabs; no recipes shows the items again. The same title
    /// and tab again keep the page the player turned to.
    pub(super) fn native_panel_recipes(
        &mut self,
        mod_id: &str,
        title: &str,
        tabs: Vec<NativeStack>,
        tab: u32,
        views: Vec<NativeRecipeView>,
    ) {
        let page = RecipePage::new(title, tabs, tab, views);
        self.panel.show_recipes(mod_id, page);
    }
    /// The recipes of type `kind` (`mymod:crusher`), a type outside `minecraft`,
    /// from `data/<namespace>/recipe/*.json` of the content mods (`.hmod`) and
    /// the world's data packs, in file order: each one's id and its file as
    /// JSON, for the mod to read as it likes. A data pack changes them on
    /// `/reload`, so look them up again when you need them. Empty for a type
    /// nobody uses and outside a world. These are the server's table on the
    /// server thread, a dedicated server or a LAN host; a client, the
    /// single-player window included, sees only the content mods' recipes, not
    /// the world's data packs'. API 36.
    pub(super) fn native_recipes_of_type(&self, kind: &str) -> &[CustomRecipe] {
        if self.player.is_none() {
            return &[];
        }
        self.recipes().custom.get(kind).map_or(&[], Vec::as_slice)
    }
    /// An ingredient as a recipe's JSON names it: an item, a `#tag`, or a list
    /// of them, a bare id as `minecraft:` (as vanilla reads it); `test` says
    /// whether a stack fits. Items the game doesn't know are left out; None
    /// when nothing is left (an unknown or empty tag, for one) and outside a
    /// world. API 36.
    pub(super) fn native_recipe_ingredient(&self, json: &serde_json::Value) -> Option<Ingredient> {
        self.player.as_ref()?;
        let qualify = |s: &str| match s.strip_prefix('#') {
            Some(tag) => String::from("#") + &qualified(tag),
            None => qualified(s),
        };
        let json = match json {
            serde_json::Value::String(s) => serde_json::Value::String(qualify(s)),
            serde_json::Value::Array(a) => serde_json::Value::Array(
                a.iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(|s| serde_json::Value::String(qualify(s)))
                    .collect(),
            ),
            _ => return None,
        };
        let items = self.items();
        let known: Vec<String> = Ingredient::parse(&json, items)?
            .1
            .into_iter()
            .filter(|item| items.get(item).is_some())
            .collect();
        (!known.is_empty()).then(|| Ingredient::of(known))
    }
    /// A stack as a recipe's `result` names it: `id` (a bare one as
    /// `minecraft:`), `count` (1 by default, up to 99) and `components`. None
    /// without an `id`, for an item the game doesn't know or a count outside
    /// that, and outside a world. API 36.
    pub(super) fn native_recipe_result(&self, json: &serde_json::Value) -> Option<ItemStack> {
        self.player.as_ref()?;
        let mut json = json.clone();
        if let Some(id) = json["id"].as_str() {
            json["id"] = qualified(id).into();
        }
        recipe_result(&json).filter(|s| self.items().get(&s.item).is_some())
    }
    /// Every item and recipe the game knows, made once (the same for every
    /// call); none outside a world. They are the window's own: the world's
    /// in single player, vanilla's on a server, which sends a client no
    /// others.
    pub(super) fn native_recipes(&mut self) -> Option<Arc<NativeRecipes>> {
        self.player.as_ref()?;
        self.made_recipes()
    }
    /// The `book_id`s of the recipes the player's recipe book holds now
    /// (the server's book on a server); empty outside a world.
    pub(super) fn native_known_recipes(&mut self) -> Arc<[i32]> {
        if self.player.is_none() {
            return Arc::from([]);
        }
        self.known_displays()
    }
    /// Places a recipe in the open screen's grid as a click in the recipe
    /// book does (`ServerboundPlaceRecipePacket`): the inventory's grid or a
    /// crafting table's. `all` places as many as the items allow, as with
    /// Shift. Refused while no such screen is open and for a recipe the
    /// player's book doesn't hold; one missing items shows as a ghost.
    pub(super) fn native_place_recipe(&mut self, book_id: i32, all: bool) -> Result<(), String> {
        self.recipe_place(book_id, all)
    }
    /// Creative mode only: gives the player `count` of `item`
    /// (`minecraft:diamond`) as taking it from the creative inventory does
    /// (`ServerboundSetCreativeModeSlot`): the selected slot first, then the
    /// hotbar, then the inventory. The count is cut to one stack.
    pub(super) fn native_give(&mut self, item: &str, count: u32) -> Result<(), String> {
        self.recipe_give(item, count)
    }
}

/// A mob's kind as the hook points name it (`minecraft:zombie`).
pub(super) fn mob_kind(mob: &horizon_client::mobs::Mob) -> String {
    ["minecraft:", mob.kind.id.trim_start_matches("minecraft:")].concat()
}

/// Where the game calls the hook points.
impl Game<'_> {
    /// The acting player as the hook points name one: the guest whose turn
    /// it is, else the host.
    pub(super) fn native_acting_id(&mut self) -> u128 {
        horizon_client::lan::playerdata::ints_uuid(self.acting_player_uuid())
    }
    /// Whether a native mod keeps the acting player from taking `stack`.
    pub(super) fn native_pickup_refused(&mut self, stack: &ItemStack) -> bool {
        let player = self.native_acting_id();
        let mut canceled = false;
        self.native_item_picked_up(player, &stack.item, stack.count, &mut canceled);
        canceled
    }
    /// The mobs from index `from` on are new in the world.
    pub(super) fn native_mobs_joined(&mut self, from: usize, natural: bool) {
        for i in from..self.mobs.len() {
            let m = &self.mobs[i];
            let (kind, pos) = (mob_kind(m), [m.position.x, m.position.y, m.position.z]);
            self.native_mob_spawned(self.dim.id(), &kind, pos, natural);
        }
    }
    /// The acting player's blow killed mob `index`.
    pub(super) fn native_kill_event(&mut self, index: usize) {
        let (entity, kind) = (self.mobs[index].id, mob_kind(&self.mobs[index]));
        let player = self.native_acting_id();
        self.native_mob_killed(player, entity, &kind);
    }
    /// The acting player's hit landed on mob `index`, holding `stack`.
    pub(super) fn native_hurt_enemy_event(&mut self, stack: &ItemStack, index: usize) {
        let mob = &self.mobs[index];
        let kind = ["minecraft:", mob.kind.id.trim_start_matches("minecraft:")].concat();
        self.native_hurt_enemy(stack, mob.id, &kind);
    }
    /// `Inventory.tick` for the acting player: each carried stack goes through
    /// `native_inventory_tick`.
    pub(super) fn native_inventory_ticks(&mut self) {
        if self.player.is_none() {
            return;
        }
        for i in 0..self.inventory.len() {
            self.native_slot_tick(i as i32, i == self.selected);
        }
        // Head first here; `Inventory` saves feet as 100 and head as 103.
        for i in 0..self.armor.len() {
            self.native_slot_tick(103 - i as i32, false);
        }
        self.native_slot_tick(-106, false);
    }
    fn native_slot_tick(&mut self, slot: i32, selected: bool) {
        if self.native_carried_slot(slot).is_none_or(|s| s.is_none()) {
            return;
        }
        self.native_inventory_tick(slot, selected);
        if let Some(stack) = self.native_carried_slot(slot)
            && stack.as_ref().is_some_and(|s| s.count == 0)
        {
            *stack = None;
        }
    }
    /// The slot a player's carried stack is in, by `native_inventory_tick`'s
    /// numbers: none for another number.
    fn native_carried_slot(&mut self, slot: i32) -> Option<&mut Option<ItemStack>> {
        match slot {
            0.. if (slot as usize) < self.inventory.len() => self.inventory.get_mut(slot as usize),
            100..=103 => self.armor.get_mut(103 - slot as usize),
            -106 => Some(&mut self.offhand),
            _ => None,
        }
    }
    /// `EntityBlock.newBlockEntity` of block `name`: Horizon's own, else a
    /// native mod's.
    pub(super) fn new_block_entity(&mut self, name: &str) -> Option<BlockEntity> {
        if let Some(entity) = BlockEntity::for_block(name) {
            return Some(entity);
        }
        let mut entity = None;
        self.native_block_entity(name, &mut entity);
        let (id, nbt) = entity?;
        Some(BlockEntity::Mod(Box::new(ModBlockEntity {
            id,
            nbt,
            ..ModBlockEntity::default()
        })))
    }
    /// The containers of the mod blocks' block entities, by block state
    /// (`native_block_entity_container`), checked.
    pub(super) fn native_containers(&self) -> FxHashMap<u32, Arc<ModContainer>> {
        let registry = self.pack.assets.registry;
        let mut by_block: FxHashMap<String, Option<Arc<ModContainer>>> = FxHashMap::default();
        let mut out = FxHashMap::default();
        for i in 0..registry.len() as u32 {
            let Some(state) = registry.state(horizon_world::registry::BlockStateId(i)) else {
                continue;
            };
            if state.name.starts_with("minecraft:") {
                continue;
            }
            let container = by_block.entry(state.name.clone()).or_insert_with(|| {
                let mut container = None;
                self.native_block_entity_container(&state.name, &mut container);
                container.map(|mut c| {
                    c.slots = c.slots.min(256);
                    let n = c.slots;
                    for list in c.faces.iter_mut().chain([&mut c.inputs, &mut c.outputs]) {
                        list.retain(|&s| s < n);
                    }
                    c.menu = c
                        .menu
                        .filter(|m| mod_menu(m).is_some_and(|(_, size)| size == n));
                    Arc::new(c)
                })
            });
            if let Some(c) = container {
                out.insert(i, Arc::clone(c));
            }
        }
        out
    }
    /// Whether a native mod's block `name` has a ticking block entity.
    pub(super) fn native_ticks(&self, name: &str) -> bool {
        let mut ticks = false;
        self.native_block_entity_ticks(name, &mut ticks);
        ticks
    }
    /// A mod block's container at `key` opens its menu: its slots grown to
    /// the menu's. False for any other block.
    pub(super) fn open_mod_menu(&mut self, key: Pos) -> bool {
        let state = self.world.block(key.0, key.1, key.2).0;
        let Some(c) = self.world.block_entities.mod_container(state) else {
            return false;
        };
        let slots = c.slots;
        if c.menu.is_none() {
            return false;
        }
        let Some(BlockEntity::Mod(m)) = self.world.block_entities.get_mut(&key) else {
            return false;
        };
        if m.items.len() < slots {
            m.items.resize(slots, None);
        }
        true
    }
    /// The menu type of the mod container at `key`, if it has one.
    pub(super) fn mod_menu_at(&self, key: Pos) -> Option<&'static str> {
        let state = self.world.block(key.0, key.1, key.2).0;
        let c = self.world.block_entities.mod_container(state)?;
        mod_menu(c.menu.as_deref()?).map(|(menu, _)| menu)
    }
    /// The container slots (of `size`) that the mod's screen for the block at
    /// `pos` does not show: the server's menu does not have them either, so a
    /// shift-click never moves an item where the player cannot see it.
    pub(super) fn native_hidden_slots(&self, pos: Pos, size: usize) -> Vec<usize> {
        if self.mod_menu_at(pos).is_none() {
            return Vec::new();
        }
        let block = self
            .res
            .facts()
            .get(self.world.block(pos.0, pos.1, pos.2))
            .name
            .to_string();
        let mut screen = None;
        self.native_screen(&block, &mut screen);
        let Some(screen) = screen else {
            return Vec::new();
        };
        (0..size)
            .filter(|&i| screen.slots.get(i).is_none_or(Option::is_none))
            .collect()
    }
    /// A menu opened on this client with its slots under `key`, on block
    /// `target`: a mod block's is laid out by its mod (`native_screen`).
    pub(super) fn open_native_screen(&mut self, key: Pos, target: Option<Pos>, window: i32) {
        self.mod_screen = None;
        let Some(t) = target else {
            return;
        };
        let block = self
            .res
            .facts()
            .get(self.world.block(t.0, t.1, t.2))
            .name
            .to_string();
        if block.starts_with("minecraft:") {
            return;
        }
        let mut screen = None;
        self.native_screen(&block, &mut screen);
        if let Some(screen) = screen {
            self.mod_screen = Some(OpenNativeScreen {
                key,
                window,
                block,
                screen,
                data: Vec::new(),
            });
        }
    }
    /// The mod screen showing now, if any.
    fn shown_native_screen(&self) -> Option<&OpenNativeScreen> {
        self.mod_screen
            .as_ref()
            .filter(|s| self.screen == Screen::Inventory && self.container == Some(s.key))
    }
    /// A data slot of window `window`'s menu (`ContainerSetData`); false
    /// when no mod screen is showing for that window.
    pub(super) fn native_screen_data(&mut self, window: i32, id: i16, value: i16) -> bool {
        if self.shown_native_screen().is_none() {
            return false;
        }
        self.mod_screen
            .as_mut()
            .is_some_and(|s| s.set_data(window, id, value))
    }
    /// The open mod screen's height.
    pub(super) fn native_screen_height(&self) -> Option<f32> {
        self.shown_native_screen().map(|s| s.screen.height)
    }
    /// Where the open mod screen puts container slot `slot`.
    pub(super) fn native_screen_slot(&self, slot: usize) -> Option<(f32, f32)> {
        let s = self.shown_native_screen()?;
        s.screen
            .slots
            .get(slot)
            .copied()
            .flatten()
            .map(|p| (p[0], p[1]))
    }
    /// The container slots the open mod screen shows (300.. ids).
    pub(super) fn native_screen_slots(&self) -> Option<Vec<usize>> {
        let s = self.shown_native_screen()?;
        let shown = s.screen.slots.iter().enumerate();
        Some(
            shown
                .filter(|(_, p)| p.is_some())
                .map(|(i, _)| 300 + i)
                .collect(),
        )
    }
    /// The first row of the player's inventory on the open mod screen.
    pub(super) fn native_screen_inventory_y(&self) -> Option<f32> {
        self.shown_native_screen().map(|s| s.screen.inventory_y)
    }
    /// The open mod screen's inventory's left column.
    pub(super) fn native_screen_inventory_x(&self) -> Option<f32> {
        self.shown_native_screen().map(|s| s.screen.inventory_x)
    }
    /// The open mod screen's width.
    pub(super) fn native_screen_width(&self) -> Option<f32> {
        self.shown_native_screen().map(|s| s.screen.width)
    }
    /// The open mod screen: a plain panel with its slots' frames, the mod's
    /// drawing over it, then the title and the inventory's label. False when
    /// none is showing.
    pub(super) fn draw_native_screen(&self, batch: &mut HudBatch, ox: f32, oy: f32) -> bool {
        let Some(s) = self.shown_native_screen() else {
            return false;
        };
        let atlas = &self.gui.atlas;
        let (w, h) = (s.screen.width, s.screen.height);
        batch.fill(atlas, ox, oy, w, h, rgba(0, 0, 0, 255));
        batch.fill(
            atlas,
            ox + 1.0,
            oy + 1.0,
            w - 2.0,
            h - 2.0,
            rgba(198, 198, 198, 255),
        );
        let container = self.native_screen_slots().unwrap_or_default();
        for i in (0..36).chain(container) {
            let (x, y) = self.slot_position(i);
            let (x, y) = (ox + x - 1.0, oy + y - 1.0);
            batch.fill(atlas, x, y, 18.0, 18.0, rgba(55, 55, 55, 255));
            batch.fill(
                atlas,
                x + 1.0,
                y + 1.0,
                17.0,
                17.0,
                rgba(255, 255, 255, 255),
            );
            batch.fill(
                atlas,
                x + 1.0,
                y + 1.0,
                16.0,
                16.0,
                rgba(139, 139, 139, 255),
            );
        }
        self.native_screen_draw(&s.block, &s.data, batch, [ox, oy]);
        if s.screen.own_labels {
            return true;
        }
        let grey = rgba(64, 64, 64, 255);
        if let Some(title) = self.remote_menu_title() {
            batch.text_styled(atlas, &title, ox + 8.0, oy + 6.0, grey, false);
        }
        let inventory = self.translate("container.inventory", "Inventory");
        let label_y = oy + s.screen.inventory_y - 11.0;
        batch.text(atlas, &inventory, ox + 8.0, label_y, grey, false);
        true
    }
    /// The tick of the mod block entity at `pos`.
    pub(super) fn tick_mod_block_entity(&mut self, pos: Pos) {
        let Some(BlockEntity::Mod(m)) = self.world.block_entities.get_mut(&pos) else {
            return;
        };
        // An empty id marks its place while the mod has it.
        let mut taken = std::mem::take(&mut **m);
        let items = taken.items.clone();
        let state = self.world.block(pos.0, pos.1, pos.2).0;
        let slots = self
            .world
            .block_entities
            .mod_container(state)
            .map_or(0, |c| c.slots);
        // Grown only: items a container no longer has room for stay saved.
        if taken.items.len() < slots {
            taken.items.resize(slots, None);
        }
        // Without a container, an `Items` item list is the mod's own field.
        if slots == 0 && !taken.items.is_empty() {
            let items = horizon_client::save::encode_items(&std::mem::take(&mut taken.items));
            taken.nbt.insert("Items".into(), items);
        }
        self.native_block_entity_tick([pos.0, pos.1, pos.2], &mut taken);
        self.put_back_mod_block_entity(pos, taken, &items);
    }
    /// Puts back a mod block entity taken out for a call, unless the call
    /// removed or replaced it, with what its `setChanged` does.
    fn put_back_mod_block_entity(
        &mut self,
        pos: Pos,
        mut taken: ModBlockEntity,
        items: &[Option<ItemStack>],
    ) {
        if let Some(BlockEntity::Mod(m)) = self.world.block_entities.get_mut(&pos)
            && m.id.is_empty()
        {
            let moved = taken.items != items;
            **m = taken;
            // BlockEntity.setChanged: the next save writes its chunk, and
            // comparators read a container whose items changed.
            self.world.modified.insert((pos.0 >> 4, pos.2 >> 4));
            if moved {
                self.container_changed(pos);
            }
        } else if taken.items.iter().any(Option::is_some) {
            // The mod removed or replaced its own block during the call. Vanilla
            // ticks it in the world and drops a container's contents on removal.
            let lost = std::mem::take(&mut taken.items);
            let at = BlockPos {
                x: pos.0,
                y: pos.1,
                z: pos.2,
            };
            self.with_level(|_, level, _| level.drop_lost_contents(at, lost));
            self.world.modified.insert((pos.0 >> 4, pos.2 >> 4));
        }
    }
    /// Menu button `id` on the mod container at `pos` (`clickMenuButton`).
    pub(super) fn mod_menu_button(&mut self, pos: Pos, id: i32) {
        if self.mod_menu_at(pos).is_none() {
            return;
        }
        let Some(BlockEntity::Mod(m)) = self.world.block_entities.get_mut(&pos) else {
            return;
        };
        if m.id.is_empty() {
            return;
        }
        let mut taken = std::mem::take(&mut **m);
        let items = taken.items.clone();
        self.native_menu_button([pos.0, pos.1, pos.2], &mut taken, id);
        self.put_back_mod_block_entity(pos, taken, &items);
    }
    /// The cursor from the open mod screen's top left, with its block and
    /// data, for its press and tooltip hooks.
    fn native_screen_at(&self) -> Option<(String, Vec<i32>, [f32; 2])> {
        let s = self.shown_native_screen()?;
        let r = self.renderer.as_ref()?;
        let batch = HudBatch::new([r.config.width, r.config.height]);
        let (ox, oy) = self.inventory_origin(&batch);
        let at = [self.cursor.0 - ox, self.cursor.1 - oy];
        Some((s.block.clone(), s.data.clone(), at))
    }
    /// A press on the open mod screen's own button (`native_screen_click`)
    /// goes to the server as a menu button click (true).
    pub(super) fn native_screen_press(&mut self, right: bool) -> bool {
        if self.inventory_target().0.is_some() {
            return false;
        }
        let Some((block, data, at)) = self.native_screen_at() else {
            return false;
        };
        let mut button = None;
        self.native_screen_click(&block, &data, at, right, &mut button);
        let Some(button) = button else {
            return false;
        };
        let window = self
            .remote
            .as_ref()
            .and_then(|r| r.menu.as_ref())
            .map(|(w, _)| *w);
        if let Some(container) = window {
            self.send(
                horizon_protocol::packets::play::Serverbound::ContainerButtonClick(
                    horizon_protocol::packets::play::ContainerButtonClick { container, button },
                ),
            );
        }
        self.audio.play_button_click();
        true
    }
    /// The open mod screen's tooltip under the cursor (`native_screen_tooltip`).
    pub(super) fn draw_native_screen_tooltip(&self, batch: &mut HudBatch) {
        if self.carried.is_some() || self.slot_at(batch, self.cursor.0, self.cursor.1).is_some() {
            return;
        }
        let Some((block, data, at)) = self.native_screen_at() else {
            return;
        };
        let mut lines = Vec::new();
        self.native_screen_tooltip(&block, &data, at, &mut lines);
        let lines: Vec<(horizon_client::text::Text, u32)> = lines
            .into_iter()
            .map(|l| (horizon_client::text::Text::literal(l), u32::MAX))
            .collect();
        self.draw_tooltip_lines(batch, &lines, self.cursor.0, self.cursor.1);
    }
    /// Options > Mods' rows for the native mods, after the others.
    pub(super) fn list_native_mods(&mut self) {
        let mut mods = Vec::new();
        self.native_mods(&mut mods);
        // A WebAssembly mod of the same id: the native row stands for it,
        // and alone toggles its `mod.<id>` line.
        self.settings
            .mods
            .installed
            .retain(|row| row.native.is_none() && !mods.iter().any(|m| m.id == row.id));
        for m in mods {
            let mut buttons = Vec::new();
            self.native_settings(&m.id, &mut buttons);
            let page = NativePage {
                keys: m.keys,
                buttons,
            };
            self.settings.mods.installed.push(Installed {
                id: m.id,
                title: m.title,
                description: m.description,
                problem: None,
                dev: false,
                native: Some(page),
            });
        }
    }
    /// A click on button `button` of native mod `i`'s page (its index in
    /// Options > Mods), and the page's labels after it.
    pub(super) fn native_settings_event(&mut self, i: usize, button: usize) {
        let Some(id) = self.settings.mods.installed.get(i).map(|m| m.id.clone()) else {
            return;
        };
        self.native_settings_click(&id, button);
        self.native_settings_refresh(i);
    }
    /// Native mod `i`'s page opened: its labels asked again, so that they
    /// follow the language (and the mod's state) since the list was made.
    pub(super) fn native_settings_refresh(&mut self, i: usize) {
        let Some(id) = self.settings.mods.installed.get(i).map(|m| m.id.clone()) else {
            return;
        };
        let mut buttons = Vec::new();
        self.native_settings(&id, &mut buttons);
        let row = self.settings.mods.installed.get_mut(i);
        if let Some(page) = row.and_then(|m| m.native.as_mut()) {
            page.buttons = buttons;
        }
    }
    /// A key event for `native_key`.
    pub(super) fn native_key_event(&mut self, key: KeyCode, pressed: bool, repeat: bool) {
        if repeat {
            return;
        }
        if let Some(name) = keybinds::key_name(key) {
            if pressed {
                self.native_panel_key(name);
            }
            self.native_key(name, pressed);
        }
    }
    /// A mouse button while playing, for `native_key`.
    pub(super) fn native_mouse_event(&mut self, button: MouseButton, pressed: bool) {
        let name = keybinds::mouse_name(button);
        self.native_key(&name, pressed);
    }
    /// Wheel steps while playing: whether a native mod took them.
    pub(super) fn native_wheel_event(&mut self, steps: f32) -> bool {
        let mut claimed = false;
        self.native_wheel(steps, &mut claimed);
        claimed
    }
    /// A native mod's command.
    pub(super) fn client_mod_command(&mut self, command: &str) -> bool {
        let mut ran = false;
        self.native_command(command, &mut ran);
        ran
    }
    /// The zoom in effect while playing (a native mod's); the spyglass's own
    /// view wins.
    pub(super) fn native_zoom_now(&self) -> Option<NativeZoom> {
        if !self.native_playing() || self.scoping() {
            return None;
        }
        let mut zoom = None;
        self.native_zoom(&mut zoom);
        // As the WebAssembly runtime clamped a mod's zoom.
        let z = zoom?;
        let factor = finite(z.factor, 1.0, 10.0);
        (factor > 1.0).then_some(NativeZoom {
            factor,
            smooth: z.smooth,
            turn: if z.turn.is_finite() {
                z.turn.clamp(0.05, 1.0)
            } else {
                1.0
            },
        })
    }
    /// The minimap a native mod shows.
    pub(super) fn native_minimap_now(&self) -> Option<NativeMinimap> {
        let mut minimap = None;
        self.native_minimap(&mut minimap);
        minimap.map(|m| NativeMinimap {
            size: finite(m.size, 16.0, 256.0),
            blocks_per_pixel: finite(m.blocks_per_pixel, 0.5, 4.0),
            ..m
        })
    }
}

/// Whether F3's debug screen, open when `debug`, covers `corner`
/// (`DebugScreenOverlay.extractRenderState`): its left and right columns
/// the top corners, and its charts the bottom corners `charts` (bottom left,
/// bottom right) says they are drawn in.
fn debug_takes(corner: Corner, debug: bool, charts: [bool; 2]) -> bool {
    debug
        && match corner {
            Corner::TopLeft | Corner::TopRight => true,
            Corner::BottomLeft => charts[0],
            Corner::BottomRight => charts[1],
        }
}

/// A menu of `MOD_MENUS`, by its name.
fn mod_menu(name: &str) -> Option<(&'static str, usize)> {
    MOD_MENUS.into_iter().find(|(menu, _)| *menu == name)
}

/// `state` with its property `key` set to `value`; none when the block has
/// no such property or value.
fn with_property(
    registry: &BlockRegistry,
    state: BlockStateId,
    key: &str,
    value: &str,
) -> Option<BlockStateId> {
    let mut s = registry.state(state)?.clone();
    match s.properties.get(key) {
        Some(v) if v == value => return Some(state),
        Some(_) => {}
        None => return None,
    }
    s.properties.insert(key.into(), value.into());
    registry.resolve(&s).ok()
}

/// `config/` beside `options`, made when missing.
fn config_dir(options: &Path) -> Result<PathBuf, String> {
    let dir = options.with_file_name("config");
    fs::create_dir_all(&dir).map_err(|e| dir.display().to_string() + ": " + &e.to_string())?;
    Ok(dir)
}

/// `v` within `lo..=hi`; `lo` when it is not a number.
fn finite(v: f32, lo: f32, hi: f32) -> f32 {
    if v.is_finite() { v.clamp(lo, hi) } else { lo }
}

#[cfg(test)]
mod tests {

    /// A mod's property write: none for a key or value the block lacks, the
    /// state itself when it already has the value, else the new state.
    #[test]
    fn with_property_finds_the_state() {
        let facts = br#"{"defaults":{"minecraft:air":0,"test:crop":1},"states":[
            ["air",true,false,"","minecraft:air"],
            ["crop0",false,false,"",{"id":"test:crop","properties":{"age":"0"}}],
            ["crop1",false,false,"",{"id":"test:crop","properties":{"age":"1"}}]
        ]}"#;
        let registry = horizon_world::registry::BlockRegistry::from_facts_json(&facts[..]).unwrap();
        use horizon_world::registry::BlockStateId;
        let (age0, age1) = (BlockStateId(1), BlockStateId(2));
        assert_eq!(
            super::with_property(&registry, age0, "age", "0"),
            Some(age0)
        );
        assert_eq!(
            super::with_property(&registry, age0, "age", "1"),
            Some(age1)
        );
        assert_eq!(super::with_property(&registry, age0, "age", "9"), None);
        assert_eq!(super::with_property(&registry, age0, "lit", "true"), None);
    }
    /// horizon-mixins refuses targets whose body has a `cfg` or a macro:
    /// the hook points and helpers must stay free of both. The launcher
    /// reads the API line as it is.
    /// A data packet for an old window leaves the open mod screen alone.
    #[test]
    fn mod_screen_data_is_its_windows_only() {
        let mut s = super::OpenNativeScreen {
            key: (0, 0, 0),
            window: 4,
            block: "gtmod:machine".into(),
            screen: super::NativeScreen::default(),
            data: Vec::new(),
        };
        assert!(s.set_data(4, 1, 7));
        assert!(!s.set_data(3, 1, 99));
        assert_eq!(s.data, [0, 7]);
    }

    #[test]
    fn hook_points_stay_hookable() {
        let source = include_str!("native_hooks.rs");
        let code = source.split("#[cfg(test)]").next().unwrap_or_default();
        assert!(code.contains("\npub(crate) const NATIVE_HOOKS_API: u32 = "));
        let lines = code.lines().filter(|l| !l.trim_start().starts_with("//"));
        for line in lines {
            assert!(!line.contains("cfg"), "a cfg in the hook points: {line}");
            for mac in ["!(", "![", "! {", "!{"] {
                assert!(!line.contains(mac), "a macro in the hook points: {line}");
            }
        }
    }

    #[test]
    fn lifecycle_source_stays_hookable() {
        let source = include_str!("native_lifecycle.rs");
        let code = source.split("#[cfg(test)]").next().unwrap_or_default();
        for line in code.lines().filter(|l| !l.trim_start().starts_with("//")) {
            assert!(!line.contains("cfg"), "a cfg in lifecycle source: {line}");
            for mac in ["!(", "![", "! {", "!{"] {
                assert!(!line.contains(mac), "a macro in lifecycle source: {line}");
            }
        }
    }

    #[test]
    fn catalog_matches_hook_points() {
        use std::collections::BTreeSet;

        let catalog: serde_json::Value =
            serde_json::from_str(include_str!("../../../../native-hooks.json")).unwrap();
        let hooks = catalog["hooks"].as_array().unwrap();
        let source = include_str!("native_hooks.rs");
        let section = source
            .split("/// The hook points. Each does nothing in Horizon.")
            .nth(1)
            .unwrap()
            .split("/// Server-level helpers for native mods.")
            .next()
            .unwrap();
        let level_section = source
            .split("impl GameLevel<'_> {")
            .nth(1)
            .unwrap()
            .split("\n}")
            .next()
            .unwrap();
        let level_hooks: BTreeSet<_> = level_section
            .lines()
            .filter_map(|line| {
                line.trim()
                    .strip_prefix("pub(super) fn native_")?
                    .split('(')
                    .next()
            })
            .collect();
        let declared: BTreeSet<_> = section
            .lines()
            .chain(level_section.lines())
            .filter_map(|line| {
                line.trim()
                    .strip_prefix("pub(super) fn native_")?
                    .split('(')
                    .next()
            })
            .collect();
        let mut names = BTreeSet::new();
        let mut contracts = BTreeSet::new();
        for hook in hooks {
            let name = hook["name"].as_str().unwrap();
            let version = hook["version"].as_u64().unwrap();
            assert!(
                version > 0 && contracts.insert((name, version)),
                "invalid or duplicate hook: {name} v{version}"
            );
            names.insert(name);
            let owner = if level_hooks.contains(name) {
                "GameLevel"
            } else {
                "Game"
            };
            assert_eq!(
                hook["target"].as_str().unwrap(),
                format!("horizon_client::game::native_hooks::{owner}<'_>::native_{name}")
            );
        }
        assert_eq!(names, declared);
    }

    /// A mod container opens a slot grid or a furnace's menu, by its slot
    /// count; no menu with rules of its own (a brewing stand's, a crafter's).
    #[test]
    fn mod_menus_are_slot_grids_and_furnaces() {
        use super::mod_menu;
        assert_eq!(mod_menu("minecraft:hopper"), Some(("minecraft:hopper", 5)));
        assert_eq!(mod_menu("minecraft:generic_9x3").map(|m| m.1), Some(27));
        assert_eq!(mod_menu("minecraft:furnace").map(|m| m.1), Some(3));
        assert_eq!(mod_menu("minecraft:brewing_stand"), None);
        assert_eq!(mod_menu("hopper"), None);
    }

    /// The config folder is `config/` beside `options.txt`, made when
    /// missing.
    #[test]
    fn config_dir_is_config_beside_options() {
        // Callable without a game, from any hook.
        let _: fn() -> Result<std::path::PathBuf, String> = super::Game::native_config_dir;
        use super::config_dir;
        let root = std::env::temp_dir().join(format!("horizon-config-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let options = root.join("options.txt");
        assert!(!root.join("config").exists());
        let dir = config_dir(&options).unwrap();
        assert_eq!(dir, root.join("config"));
        assert!(dir.is_dir());
        assert_eq!(config_dir(&options).unwrap(), dir);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Vanilla's F3 draws a column in each top corner (`extractLines`, left
    /// and right aligned), and its charts along the bottom.
    #[test]
    fn the_debug_screen_takes_both_top_corners_and_its_charts_corners() {
        use super::{Corner, debug_takes};
        for corner in Corner::ALL {
            assert!(!debug_takes(corner, false, [true, true]), "{corner:?}");
        }
        assert!(debug_takes(Corner::TopLeft, true, [false, false]));
        assert!(debug_takes(Corner::TopRight, true, [false, false]));
        assert!(!debug_takes(Corner::BottomLeft, true, [false, false]));
        assert!(!debug_takes(Corner::BottomRight, true, [false, false]));
        assert!(debug_takes(Corner::BottomLeft, true, [true, false]));
        assert!(!debug_takes(Corner::BottomRight, true, [true, false]));
        assert!(debug_takes(Corner::BottomRight, true, [false, true]));
    }
}
