//! Container block entities: chests/barrels (27 slots) and the three furnace
//! kinds, ticking as `AbstractFurnaceBlockEntity.serverTick` with fuel times
//! from the data-driven `context_int_provider`/`context_float_provider` files.
use crate::{
    crafting::Recipes,
    items::{ItemStack, Items},
};
use serde_json::Value;
use std::{collections::HashMap, fs, path::Path, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FurnaceKind {
    Furnace,
    BlastFurnace,
    Smoker,
}
impl FurnaceKind {
    pub fn recipe_type(self) -> &'static str {
        match self {
            Self::Furnace => "minecraft:smelting",
            Self::BlastFurnace => "minecraft:blasting",
            Self::Smoker => "minecraft:smoking",
        }
    }
    pub fn fast(self) -> bool {
        self != Self::Furnace
    }
    pub fn from_block(name: &str) -> Option<Self> {
        match name {
            "minecraft:furnace" => Some(Self::Furnace),
            "minecraft:blast_furnace" => Some(Self::BlastFurnace),
            "minecraft:smoker" => Some(Self::Smoker),
            _ => None,
        }
    }
}
#[derive(Clone, Debug)]
pub struct Furnace {
    pub kind: FurnaceKind,
    /// input, fuel, result
    pub items: [Option<ItemStack>; 3],
    pub lit_remaining: i32,
    pub lit_total: i32,
    pub cooking_timer: i32,
    pub cooking_total: i32,
    pub speed: f32,
    /// `AbstractFurnaceBlockEntity.recipesUsed`: how many of each recipe this furnace has
    /// made since a player last took from its output. The experience is owed per recipe
    /// rather than per item, and a save keeps it as the `RecipesUsed` compound.
    pub recipes_used: HashMap<String, u32>,
}
/// A hopper's slots: the only plain container with five.
pub const HOPPER_SLOTS: usize = 5;

#[derive(Clone, Debug)]
pub enum BlockEntity {
    Chest(Vec<Option<ItemStack>>),
    Furnace(Furnace),
    Brewing(Box<crate::brewing::BrewingStand>),
    /// Signs and hanging signs (`SignBlockEntity`).
    Sign(crate::signs::SignData),
    /// `JukeboxBlockEntity`: the record and its `JukeboxSongPlayer`.
    Jukebox(Jukebox),
    /// `BeaconBlockEntity`.
    Beacon(Box<Beacon>),
    /// `LecternBlockEntity`: the book and the open page.
    Lectern(Lectern),
    /// Sculk sensors, shriekers and catalysts (`crate::sculk`).
    Sculk(Box<crate::sculk::SculkEntity>),
    /// `CreakingHeartBlockEntity`: its protector and the hurt emitter.
    CreakingHeart(Box<CreakingHeart>),
    /// `BannerBlockEntity`: the pattern layers.
    Banner(Banner),
    /// `CrafterBlockEntity`.
    Crafter(Crafter),
    /// `SpawnerBlockEntity` (`crate::mobs::spawner`).
    Spawner(Box<crate::mobs::spawner::Spawner>),
    /// `TrialSpawnerBlockEntity` (`crate::mobs::trial_spawner`).
    TrialSpawner(Box<crate::mobs::trial_spawner::TrialSpawner>),
    /// `VaultBlockEntity` (`crate::vault`).
    Vault(Box<crate::vault::Vault>),
    /// `BeehiveBlockEntity` of beehives and bee nests (`crate::beehive`).
    Beehive(Box<crate::beehive::Beehive>),
    /// `CommandBlockEntity` of the three command blocks (`crate::command_block`).
    CommandBlock(Box<crate::command_block::CommandBlock>),
    /// `SkullBlockEntity` of every skull and head block.
    Skull(Skull),
}
/// `SkullBlockEntity`: the player profile the head shows (kept as loaded:
/// a compound, or a string when it names a player only), the note block
/// sound and `custom_name`.
#[derive(Clone, Debug, Default)]
pub struct Skull {
    pub profile: Option<horizon_world::nbt::Tag>,
    pub note_block_sound: Option<String>,
    pub custom_name: Option<horizon_world::nbt::Tag>,
    /// What `profile` draws, read from it once rather than every frame.
    pub skin: Option<SkullSkin>,
}
impl Skull {
    pub fn new(
        profile: Option<horizon_world::nbt::Tag>,
        note_block_sound: Option<String>,
        custom_name: Option<horizon_world::nbt::Tag>,
    ) -> Self {
        let skin = profile.as_ref().map(SkullSkin::of);
        Self {
            profile,
            note_block_sound,
            custom_name,
            skin,
        }
    }
}
/// `SkullBlockRenderer.getPlayerSkin` of a head's profile: its skin's URL
/// with the `skins/<sha1>` sprite it downloads to, and the
/// `DefaultPlayerSkin` drawn until that sprite is there (steve for a
/// profile with no id).
#[derive(Clone, Debug)]
pub struct SkullSkin {
    pub download: Option<(String, String)>,
    pub fallback: &'static str,
}
impl SkullSkin {
    pub fn of(profile: &horizon_world::nbt::Tag) -> Self {
        let download = crate::skins::profile_skin_url_nbt(profile).and_then(|url| {
            let location = crate::skins::skin_location(&crate::skins::texture_hash(&url)?);
            Some((url, location))
        });
        // DefaultPlayerSkin.get(id), or getDefaultSkin (slim steve) without an id.
        let fallback = crate::skins::profile_uuid_nbt(profile)
            .map_or("entity/player/slim/steve", crate::tab_list::default_skin);
        Self { download, fallback }
    }
}
/// The skull and head blocks (`SkullBlock`, `WallSkullBlock`): every
/// `*_skull` and `*_head` but the piston's head.
fn is_skull(name: &str) -> bool {
    name.ends_with("_skull") || (name.ends_with("_head") && name != "minecraft:piston_head")
}
/// `CrafterBlockEntity`: nine slots that can be disabled, the powered flag
/// and the ticks left in the crafting animation.
#[derive(Clone, Debug, Default)]
pub struct Crafter {
    pub items: [Option<ItemStack>; 9],
    pub disabled: [bool; 9],
    pub triggered: bool,
    pub crafting_ticks: i32,
    /// The block state's `triggered` when last ticked (not saved): the block
    /// entity flag follows it only when it changes (`CrafterBlock.neighborChanged`).
    pub state_triggered: Option<bool>,
}
impl Crafter {
    /// `slotCanBeDisabled`: only empty slots.
    pub fn slot_can_be_disabled(&self, slot: usize) -> bool {
        slot < 9 && self.items[slot].is_none()
    }
    /// `setSlotState`.
    pub fn set_slot_state(&mut self, slot: usize, enabled: bool) {
        if self.slot_can_be_disabled(slot) {
            self.disabled[slot] = !enabled;
        }
    }
    /// `getRedstoneSignal`: filled or disabled slots.
    pub fn redstone_signal(&self) -> i32 {
        (0..9)
            .filter(|&i| self.items[i].is_some() || self.disabled[i])
            .count() as i32
    }
    /// `canPlaceItem`: a hopper fills the smallest stacks first.
    pub fn can_place(&self, slot: usize, _stack: &ItemStack, max_stack: u32) -> bool {
        if slot >= 9 || self.disabled[slot] {
            return false;
        }
        let Some(current) = &self.items[slot] else {
            return true;
        };
        if current.count >= max_stack {
            return false;
        }
        !(slot + 1..9).any(|i| {
            !self.disabled[i]
                && match &self.items[i] {
                    None => true,
                    Some(other) => {
                        other.count < current.count && other.same_item_same_components(current)
                    }
                }
        })
    }
}
#[derive(Clone, Debug, Default)]
pub struct Banner {
    pub layers: Vec<crate::banner::BannerLayer>,
    pub name: Option<String>,
    /// Components not consumed by BannerBlockEntity's implicit fields.
    pub components: horizon_world::nbt::Compound,
}
/// `CreakingHeartBlockEntity` (the logic is in `game/creaking.rs`).
#[derive(Clone, Debug, Default)]
pub struct CreakingHeart {
    /// The protector: its entity id once resolved, and its UUID (the saved form).
    pub creaking: Option<(Option<u32>, [i32; 4])>,
    pub ticks_existed: i64,
    pub ticker: i32,
    /// Ticks left of the hurt effect (`emitter`) and where it heads.
    pub emitter: i32,
    pub emitter_target: Option<[f64; 3]>,
    pub output_signal: i32,
}
#[derive(Clone, Debug, Default)]
pub struct Lectern {
    pub book: Option<ItemStack>,
    pub page: i32,
}
#[derive(Clone, Debug, Default)]
pub struct Beacon {
    pub levels: i32,
    /// Effect ids (`minecraft:speed`).
    pub primary: Option<String>,
    pub secondary: Option<String>,
    /// Beam sections (RGB colour, height) and the ones being rebuilt.
    pub sections: Vec<(u32, i32)>,
    pub checking: Vec<(u32, i32)>,
    pub last_check_y: i32,
}
#[derive(Clone, Debug, Default)]
pub struct Jukebox {
    pub record: Option<ItemStack>,
    /// `ticks_since_song_started`, while a song plays.
    pub ticks: i64,
    pub playing: bool,
}
impl BlockEntity {
    pub fn for_block(name: &str) -> Option<Self> {
        if let Some(hanging) = crate::signs::sign_kind(name) {
            return Some(Self::Sign(crate::signs::SignData {
                hanging,
                ..Default::default()
            }));
        }
        if let Some(kind) = FurnaceKind::from_block(name) {
            return Some(Self::Furnace(Furnace {
                kind,
                items: [None, None, None],
                lit_remaining: 0,
                lit_total: 0,
                cooking_timer: 0,
                cooking_total: 0,
                speed: 1.0,
                recipes_used: HashMap::new(),
            }));
        }
        if name == "minecraft:creaking_heart" {
            return Some(Self::CreakingHeart(Box::default()));
        }
        if name == "minecraft:spawner" {
            return Some(Self::Spawner(Box::default()));
        }
        if name == "minecraft:trial_spawner" {
            return Some(Self::TrialSpawner(Box::default()));
        }
        if name == "minecraft:vault" {
            return Some(Self::Vault(Box::default()));
        }
        if name == "minecraft:beehive" || name == "minecraft:bee_nest" {
            return Some(Self::Beehive(Box::default()));
        }
        if name == "minecraft:lectern" {
            return Some(Self::Lectern(Lectern::default()));
        }
        if crate::command_block::Mode::of_block(name).is_some() {
            // CommandBlock.newBlockEntity: setAutomatic(this.automatic).
            return Some(Self::CommandBlock(Box::new(
                crate::command_block::CommandBlock {
                    auto: crate::command_block::automatic_block(name),
                    ..Default::default()
                },
            )));
        }
        if let Some(sculk) = crate::sculk::SculkEntity::for_block(name) {
            return Some(Self::Sculk(Box::new(sculk)));
        }
        if name == "minecraft:crafter" {
            // CrafterBlock.newBlockEntity copies the placed state's flag: the
            // first tick sees it as a change from untriggered.
            return Some(Self::Crafter(Crafter {
                state_triggered: Some(false),
                ..Crafter::default()
            }));
        }
        if name.ends_with("_banner") {
            return Some(Self::Banner(Banner::default()));
        }
        if is_skull(name) {
            return Some(Self::Skull(Skull::default()));
        }
        if name == "minecraft:beacon" {
            return Some(Self::Beacon(Box::new(Beacon {
                last_check_y: i32::MIN / 2,
                ..Default::default()
            })));
        }
        if name == "minecraft:brewing_stand" {
            return Some(Self::Brewing(Box::new(crate::brewing::BrewingStand::new())));
        }
        let size = match name {
            "minecraft:chest" | "minecraft:trapped_chest" | "minecraft:barrel" => 27,
            n if n.ends_with("copper_chest") => 27,
            "minecraft:hopper" => HOPPER_SLOTS,
            "minecraft:dispenser" | "minecraft:dropper" => 9,
            n if n.ends_with("shulker_box") => 27,
            _ => return None,
        };
        Some(Self::Chest(vec![None; size]))
    }
    pub fn id(&self) -> &'static str {
        match self {
            Self::Chest(items) => match items.len() {
                HOPPER_SLOTS => "minecraft:hopper",
                9 => "minecraft:dispenser",
                _ => "minecraft:chest",
            },
            Self::Furnace(f) => match f.kind {
                FurnaceKind::Furnace => "minecraft:furnace",
                FurnaceKind::BlastFurnace => "minecraft:blast_furnace",
                FurnaceKind::Smoker => "minecraft:smoker",
            },
            Self::Brewing(_) => "minecraft:brewing_stand",
            Self::Sign(s) => s.id(),
            Self::Jukebox(_) => "minecraft:jukebox",
            Self::Beacon(_) => "minecraft:beacon",
            Self::Lectern(_) => "minecraft:lectern",
            Self::Sculk(s) => s.id(),
            Self::Banner(_) => "minecraft:banner",
            Self::Crafter(_) => "minecraft:crafter",
            Self::CreakingHeart(_) => "minecraft:creaking_heart",
            Self::Spawner(_) => "minecraft:mob_spawner",
            Self::TrialSpawner(_) => "minecraft:trial_spawner",
            Self::Vault(_) => "minecraft:vault",
            Self::Beehive(_) => "minecraft:beehive",
            Self::CommandBlock(_) => "minecraft:command_block",
            Self::Skull(_) => "minecraft:skull",
        }
    }
    pub fn contents(&self) -> Vec<ItemStack> {
        match self {
            Self::Chest(items) => items.iter().flatten().cloned().collect(),
            Self::Furnace(f) => f.items.iter().flatten().cloned().collect(),
            Self::Brewing(b) => b.items.iter().flatten().cloned().collect(),
            Self::Sign(_) => Vec::new(),
            Self::Jukebox(j) => j.record.iter().cloned().collect(),
            Self::Beacon(_)
            | Self::CreakingHeart(_)
            | Self::Spawner(_)
            | Self::TrialSpawner(_)
            | Self::Vault(_)
            | Self::Beehive(_)
            | Self::CommandBlock(_)
            | Self::Skull(_) => Vec::new(),
            Self::Lectern(l) => l.book.iter().cloned().collect(),
            Self::Sculk(_) => Vec::new(),
            Self::Banner(_) => Vec::new(),
            Self::Crafter(c) => c.items.iter().flatten().cloned().collect(),
        }
    }
    pub fn slots(&self) -> &[Option<ItemStack>] {
        match self {
            Self::Chest(items) => items,
            Self::Furnace(f) => &f.items,
            Self::Brewing(b) => &b.items,
            Self::Sign(_)
            | Self::Jukebox(_)
            | Self::Beacon(_)
            | Self::Lectern(_)
            | Self::Sculk(_)
            | Self::Banner(_)
            | Self::Skull(_) => &[],
            Self::Crafter(c) => &c.items,
            Self::CreakingHeart(_)
            | Self::Spawner(_)
            | Self::TrialSpawner(_)
            | Self::Vault(_)
            | Self::Beehive(_)
            | Self::CommandBlock(_) => &[],
        }
    }
    pub fn slots_mut(&mut self) -> &mut [Option<ItemStack>] {
        match self {
            Self::Chest(items) => items,
            Self::Furnace(f) => &mut f.items,
            Self::Brewing(b) => &mut b.items,
            Self::Sign(_)
            | Self::Jukebox(_)
            | Self::Beacon(_)
            | Self::Lectern(_)
            | Self::Sculk(_)
            | Self::Banner(_)
            | Self::Skull(_) => &mut [],
            Self::Crafter(c) => &mut c.items,
            Self::CreakingHeart(_)
            | Self::Spawner(_)
            | Self::TrialSpawner(_)
            | Self::Vault(_)
            | Self::Beehive(_)
            | Self::CommandBlock(_) => &mut [],
        }
    }
}
/// Evaluator for `context_int_provider` / `context_float_provider` values.
pub struct Providers {
    ints: HashMap<String, Value>,
    floats: HashMap<String, Value>,
}
fn load_dir(dir: &Path, prefix: &str, out: &mut HashMap<String, Value>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        let name = p
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_owned();
        if p.is_dir() {
            load_dir(&p, &format!("{prefix}{name}/"), out);
        } else if let Ok(bytes) = fs::read(&p)
            && let Ok(v) = serde_json::from_slice(&bytes)
        {
            out.insert(format!("minecraft:{prefix}{name}"), v);
        }
    }
}
impl Providers {
    pub fn load(root: &Path) -> Self {
        let mut ints = HashMap::new();
        let mut floats = HashMap::new();
        load_dir(
            &root.join("data/minecraft/context_int_provider"),
            "",
            &mut ints,
        );
        load_dir(
            &root.join("data/minecraft/context_float_provider"),
            "",
            &mut floats,
        );
        Self { ints, floats }
    }
    fn eval(v: &Value, fast: bool, table: &HashMap<String, Value>, depth: u32) -> f64 {
        if depth > 16 {
            return 0.0;
        }
        match v {
            Value::Number(n) => n.as_f64().unwrap_or(0.0),
            Value::String(name) => table
                .get(name)
                .map_or(0.0, |v| Self::eval(v, fast, table, depth + 1)),
            Value::Object(o) => {
                let f =
                    |k: &str| Self::eval(o.get(k).unwrap_or(&Value::Null), fast, table, depth + 1);
                match o.get("type").and_then(Value::as_str).unwrap_or("") {
                    "minecraft:div" => {
                        let r = f("right");
                        if r == 0.0 {
                            0.0
                        } else {
                            (f("left") / r).trunc()
                        }
                    }
                    "minecraft:mul" => f("left") * f("right"),
                    "minecraft:add" | "minecraft:sum" => f("left") + f("right"),
                    "minecraft:sub" => f("left") - f("right"),
                    "minecraft:constant" => f("value"),
                    "minecraft:conditional" => {
                        // The only condition in the pinned data: block/fast_cooking.
                        let condition = o.get("condition").and_then(Value::as_str)
                            == Some("minecraft:block/fast_cooking");
                        if condition && fast {
                            f("on_true")
                        } else {
                            f("on_false")
                        }
                    }
                    _ => 0.0,
                }
            }
            _ => 0.0,
        }
    }
    /// A plain `context_int_provider` value or reference (no context conditions).
    pub fn int_value(&self, v: &Value) -> i32 {
        Self::eval(v, false, &self.ints, 0) as i32
    }
    /// A plain `context_float_provider` value or reference; `default` when absent.
    pub fn float_value(&self, v: &Value, default: f32) -> f32 {
        if v.is_null() {
            default
        } else {
            Self::eval(v, false, &self.floats, 0) as f32
        }
    }
    #[cfg(test)]
    pub fn empty() -> Self {
        Self {
            ints: HashMap::new(),
            floats: HashMap::new(),
        }
    }
    pub fn burn_time(&self, items: &Items, stack: &ItemStack, fast: bool) -> i32 {
        items
            .get(&stack.item)
            .and_then(|i| i.fuel.as_ref())
            .map_or(0, |(time, _)| Self::eval(time, fast, &self.ints, 0) as i32)
    }
    pub fn speed(&self, items: &Items, stack: &ItemStack, fast: bool) -> f32 {
        items
            .get(&stack.item)
            .and_then(|i| i.fuel.as_ref())
            .map_or(1.0, |(_, speed)| {
                Self::eval(speed, fast, &self.floats, 0) as f32
            })
    }
}
fn remainder(item: &str) -> Option<&'static str> {
    match item {
        "minecraft:lava_bucket" | "minecraft:water_bucket" | "minecraft:milk_bucket" => {
            Some("minecraft:bucket")
        }
        _ => None,
    }
}
impl Furnace {
    /// `AbstractFurnaceBlockEntity.setItem` on the input slot: anything but
    /// the same item with the same components restarts cooking, with the
    /// total time of the new input's recipe (200 ticks without one).
    pub fn set_input(&mut self, recipes: &Recipes, stack: Option<ItemStack>) {
        let same = match (&self.items[0], &stack) {
            (Some(old), Some(new)) => old.same_item_same_components(new),
            _ => false,
        };
        self.items[0] = stack;
        if !same {
            self.cooking_total = self.items[0]
                .as_ref()
                .and_then(|input| recipes.cook(self.kind.recipe_type(), input))
                .map_or(200, |recipe| total_time(recipe.time, self.speed));
            self.cooking_timer = 0;
        }
    }
    /// `recipesUsed`, each recipe with its `experience` (0 for one this data pack does
    /// not have), in the order `getRecipesToAwardAndPopExperience` pays them.
    pub fn recipe_experience(&self, recipes: &Recipes) -> Vec<(u32, f32)> {
        self.recipes_used
            .iter()
            .map(|(id, count)| (*count, recipes.cooking_experience(id)))
            .collect()
    }
    /// A menu click wrote its slots into `items`, and `before` was the input
    /// slot until then: the menu called `setItem` on it only if it changed
    /// (`Slot.setByPlayer`; partial takes and merges keep the stack).
    pub fn input_clicked(&mut self, recipes: &Recipes, before: Option<ItemStack>) {
        if self.items[0] != before {
            let after = std::mem::replace(&mut self.items[0], before);
            self.set_input(recipes, after);
        }
    }
    /// One server tick. Returns `Some(lit)` when the LIT block state must change.
    pub fn tick(
        &mut self,
        recipes: &Recipes,
        items: &Items,
        providers: &Providers,
    ) -> Option<bool> {
        let was_lit = self.lit_remaining > 0;
        let mut lit = false;
        if self.lit_remaining > 0 {
            self.lit_remaining -= 1;
            lit = self.lit_remaining > 0;
        }
        let fast = self.kind.fast();
        let has_ingredient = self.items[0].is_some();
        let has_fuel = self.items[1].is_some();
        if lit || has_fuel && has_ingredient {
            if let Some(ingredient) = self.items[0].clone() {
                if let Some(recipe) = recipes.cook(self.kind.recipe_type(), &ingredient) {
                    if self.cooking_timer == 0 {
                        self.cooking_total = total_time(recipe.time, self.speed);
                    }
                    let max = items.get(&recipe.result.item).map_or(64, |i| i.max_stack);
                    let can_burn = match &self.items[2] {
                        None => true,
                        Some(out) => {
                            out.item == recipe.result.item && out.count + recipe.result.count <= max
                        }
                    };
                    if can_burn {
                        if !lit {
                            let fuel = self.items[1].clone();
                            let time = fuel
                                .as_ref()
                                .map_or(0, |f| providers.burn_time(items, f, fast));
                            let speed = fuel
                                .as_ref()
                                .map_or(1.0, |f| providers.speed(items, f, fast));
                            self.lit_remaining = time;
                            self.lit_total = time;
                            self.speed = speed;
                            if self.cooking_total > 0 && self.cooking_timer < self.cooking_total {
                                let ratio = self.cooking_timer as f32 / self.cooking_total as f32;
                                self.cooking_total = total_time(recipe.time, self.speed);
                                self.cooking_timer =
                                    (ratio * self.cooking_total as f32).ceil() as i32;
                            }
                            if time > 0 {
                                if let Some(f) = self.items[1].as_mut() {
                                    let item = f.item.clone();
                                    f.count -= 1;
                                    if f.count == 0 {
                                        self.items[1] = remainder(&item).map(|r| ItemStack {
                                            item: Arc::from(r),
                                            count: 1,
                                            damage: 0,
                                            extra: None,
                                        });
                                    }
                                }
                                lit = true;
                            }
                        }
                        if lit {
                            self.cooking_timer += 1;
                            if self.cooking_timer >= self.cooking_total {
                                self.cooking_timer = 0;
                                self.cooking_total = total_time(recipe.time, self.speed);
                                match &mut self.items[2] {
                                    Some(out) => out.count += recipe.result.count,
                                    None => self.items[2] = Some(recipe.result.clone()),
                                }
                                *self.recipes_used.entry(recipe.id.clone()).or_insert(0) += 1;
                                if let Some(i) = self.items[0].as_mut() {
                                    i.count -= 1;
                                    if i.count == 0 {
                                        self.items[0] = None;
                                    }
                                }
                            }
                        } else {
                            self.cooking_timer = 0;
                        }
                    } else {
                        self.cooking_timer = 0;
                    }
                }
            } else {
                self.cooking_timer = 0;
            }
        } else if self.cooking_timer > 0 {
            self.cooking_timer = (self.cooking_timer - 2).clamp(0, self.cooking_total);
        }
        (was_lit != lit).then_some(lit)
    }
}
fn total_time(time: u32, speed: f32) -> i32 {
    if speed > 0.0 {
        (time as f32 / speed).ceil() as i32
    } else {
        time as i32
    }
}

/// `AbstractFurnaceBlockEntity.createExperience`: `count * experience`, floored, and one
/// more with the chance of what is left over (a level random draw only when there is some).
pub fn create_experience(
    count: u32,
    experience: f32,
    random: &mut impl horizon_core::random::RandomSource,
) -> u32 {
    let total = count as f32 * experience;
    let whole = total.floor();
    let fraction = total - whole;
    let mut xp = whole.max(0.0) as u32;
    if fraction != 0.0 && random.next_float() < fraction {
        xp += 1;
    }
    xp
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::random::{LegacyRandom, RandomSource};

    /// A whole amount pays itself without a draw; a fraction is a chance of one more.
    #[test]
    fn create_experience_rounds_the_fraction_by_chance() {
        let mut random = LegacyRandom::new(7);
        let mut untouched = LegacyRandom::new(7);
        assert_eq!(create_experience(4, 0.5, &mut random), 2);
        assert_eq!(create_experience(3, 0.0, &mut random), 0);
        assert_eq!(random.next_long(), untouched.next_long());
        let paid: u32 = (0..1000)
            .map(|_| create_experience(1, 0.35, &mut random))
            .sum();
        assert!((250..450).contains(&paid), "{paid}");
        assert!((0..100).all(|_| (1..=2).contains(&create_experience(1, 1.7, &mut random))));
    }
}
