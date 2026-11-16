//! The game's own answer to the fluid capability (`FLUIDS`, docs/modding/src/how-to/native-mods.md): the tanks a
//! mod block declared through `native_block_entity_fluids`, and a cauldron, read through a
//! side, filled and drained in millibuckets. They act where the world is simulated, never on a
//! client's copy, and only on the level in place.
use super::Game;
use super::cauldron_contents::{self, CauldronContent, CauldronContents, EMPTY_CAULDRON};
use horizon_client::containers::{BlockEntity, ModFluidTanks};
use horizon_core::geometry::Direction;
use horizon_core::nbt::{Compound, JavaString, Tag};
use mod_api::{BlockSide, FluidStack, Transaction};
use std::sync::Arc;

/// What holds fluids at a position: a mod block's tanks, or a cauldron
/// (`CauldronFluids`). An empty cauldron is fillable; a cauldron block with no
/// content in the table holds none: powder snow is not a fluid (`FluidKind`
/// has none), so a powder snow cauldron has no content.
enum FluidAt {
    Mod(Arc<ModFluidTanks>),
    Cauldron(CauldronFluids),
}
/// A cauldron: the table of contents, and what it holds, a content's index
/// in that table and its level; none for an empty cauldron.
#[derive(Clone)]
struct CauldronFluids {
    contents: Arc<CauldronContents>,
    held: Option<(usize, u32)>,
}
impl CauldronFluids {
    /// What it holds, and at which level.
    fn held(&self) -> Option<(&CauldronContent, u32)> {
        let (index, level) = self.held?;
        Some((self.contents.get(index), level))
    }
    /// The same cauldron holding `held`.
    fn with(&self, held: Option<(usize, u32)>) -> Self {
        Self {
            contents: self.contents.clone(),
            held,
        }
    }
    /// What a fill of `amount` of `fluid` takes, and what the cauldron then holds: whole
    /// levels of the content `fluid` has, into an empty cauldron or one holding that content,
    /// so a content only ever full fills an empty cauldron only whole. None when it takes
    /// nothing.
    fn fill(&self, fluid: &str, amount: i32) -> Option<(i32, CauldronFluids)> {
        let (index, content) = self.contents.of_fluid(fluid)?;
        let level = match self.held {
            None => 0,
            Some((held, level)) if held == index => level,
            // Water into lava would turn it to stone in vanilla's world, not in a tank.
            Some(_) => return None,
        };
        let new = content.fill_level(level, amount.max(0));
        (new > level).then(|| {
            (
                content.amount(new) - content.amount(level),
                self.with(Some((index, new))),
            )
        })
    }
    /// What a drain of at most `amount` takes: the fluid, how much, and what the cauldron then
    /// holds. Whole levels, so a content only ever full drains only whole, and the last level
    /// leaves the cauldron empty. None when it gives nothing.
    fn drain(&self, amount: i32) -> Option<(&str, i32, CauldronFluids)> {
        let (index, level) = self.held?;
        let content = self.contents.get(index);
        let (taken, left) = content.drain_take(level, amount);
        (taken > 0).then(|| {
            (
                content.fluid.as_str(),
                taken,
                self.with((left > 0).then_some((index, left))),
            )
        })
    }
}

/// The tanks of a mod block its side `side` reaches (none: every tank), in the order the side
/// numbers them for the fluid capability.
fn side_tanks(tanks: &ModFluidTanks, side: Option<Direction>) -> impl Iterator<Item = usize> + '_ {
    let n = tanks.tanks.len();
    let face = side.map(|face| &tanks.faces[face as usize]);
    (0..face.map_or(n, Vec::len)).filter_map(move |i| {
        let tank = face.map_or(i, |face| face[i]);
        (tank < n).then_some(tank)
    })
}

/// Whether the tanks at `at` take `fluid`. A mod block's saved tanks keep an id and an amount,
/// so they take vanilla's fluids and a mod's own, without components. A cauldron takes only
/// the fluids that have a content (water, lava, and the mods'), discarding their components:
/// the one place a fluid's components are dropped, and what comes out of a cauldron is plain.
fn game_holds(fluid: &FluidStack, at: &FluidAt) -> bool {
    match at {
        FluidAt::Mod(_) => {
            !fluid.has_components()
                && fluid.key().and_then(mod_api::__private::fluid_id) != Some("minecraft:empty")
        }
        FluidAt::Cauldron(cauldron) => fluid
            .key()
            .and_then(mod_api::__private::fluid_id)
            .is_some_and(|id| cauldron.contents.of_fluid(id).is_some()),
    }
}

fn fluid_identity(id: &[u16]) -> Option<mod_api::FluidIdentity> {
    if let Some(key) = mod_api::__private::fluid_key_utf16(id) {
        return Some(mod_api::FluidIdentity::Resolved(key));
    }
    mod_api::__private::lookup_unresolved_fluid_utf16(id)
        .map(mod_api::FluidIdentity::Unresolved)
        .or_else(|| {
            static WARNED: std::sync::Once = std::sync::Once::new();
            WARNED.call_once(|| log::warn!("An unrecognized live fluid id is unavailable"));
            None
        })
}

/// A capability amount in the fluid helpers' millibuckets.
fn mb(amount: u32) -> i32 {
    i32::try_from(amount).unwrap_or(i32::MAX)
}

/// The saved state of one tank: its fluid and amount in millibuckets.
type Tank = (Option<String>, i32);

/// The tanks in `nbt`, each `n` long: missing entries read as empty, and a
/// save with more tanks than the block now declares keeps the ones that
/// still exist (a mod update that shrank its tanks loses nothing it kept).
fn read_tanks(nbt: &Compound, n: usize) -> Vec<Tank> {
    let list = match nbt.get(&JavaString::from("fluids")) {
        Some(Tag::List(list)) => list,
        _ => return vec![(None, 0); n],
    };
    let read: Vec<Tank> = list
        .iter()
        .take(n)
        .map(|tag| match tag {
            Tag::Compound(map) => (
                match map.get(&JavaString::from("Fluid")) {
                    Some(Tag::String(fluid)) if fluid.to_string_lossy() != "minecraft:empty" => {
                        Some(fluid.to_string_lossy())
                    }
                    _ => None,
                },
                match map.get(&JavaString::from("Amount")) {
                    Some(Tag::Int(amount)) => *amount,
                    _ => 0,
                },
            ),
            _ => (None, 0),
        })
        .collect();
    let mut tanks = read;
    tanks.resize(n, (None, 0));
    tanks
}

/// `read_tanks`'s writer: the list back into `nbt`, an empty tank as
/// `minecraft:empty` so the tank numbers stay in place.
fn write_tanks(nbt: &mut Compound, tanks: &[Tank]) {
    let list = tanks
        .iter()
        .map(|&(ref fluid, amount)| {
            let mut map = Compound::new();
            map.insert(
                JavaString::from("Fluid"),
                Tag::String(JavaString::from(
                    fluid.as_deref().unwrap_or("minecraft:empty"),
                )),
            );
            map.insert(JavaString::from("Amount"), Tag::Int(amount));
            Tag::Compound(map)
        })
        .collect();
    nbt.insert(JavaString::from("fluids"), Tag::List(list));
}

// Native mods use these; nothing in Horizon does.
#[allow(dead_code)]
impl Game<'_> {
    /// What holds fluids at `pos` in the level in place: a mod block's tanks (its block
    /// entity's, and not the placeholder standing in while it ticks), or a cauldron.
    fn native_fluid_at(&self, pos: [i32; 3]) -> Option<FluidAt> {
        if !self.native_simulates() {
            return None;
        }
        let key = (pos[0], pos[1], pos[2]);
        let state = self.world.block(pos[0], pos[1], pos[2]);
        if let Some(tanks) = self.world.block_entities.mod_fluids(state.0) {
            // The block entity is out of the world while it ticks.
            return matches!(
                self.world.block_entities.get(&key),
                Some(horizon_client::containers::BlockEntity::Mod(m)) if !m.id().is_empty()
            )
            .then(|| FluidAt::Mod(Arc::clone(tanks)));
        }
        let info = self.pack.assets.registry.state(state)?;
        let contents = cauldron_contents::current();
        let held = if info.name == EMPTY_CAULDRON {
            None
        } else {
            let (index, content) = contents.of_block(&info.name)?;
            let level = match &content.level {
                Some((property, _)) => info.properties.get(property.as_str())?.parse().ok()?,
                None => 1,
            };
            Some((index, level))
        };
        Some(FluidAt::Cauldron(CauldronFluids { contents, held }))
    }
    /// The cauldron at `pos` set to hold what `to` holds. Whether it changed.
    fn cauldron_set(&mut self, pos: [i32; 3], to: &CauldronFluids) -> bool {
        let mut properties = std::collections::BTreeMap::new();
        let name = match to.held() {
            None | Some((_, 0)) => EMPTY_CAULDRON,
            Some((content, level)) => {
                if let Some((property, _)) = &content.level {
                    properties.insert(property.clone(), level.to_string());
                }
                content.block.as_str()
            }
        };
        let state = horizon_world::chunk::BlockState {
            name: name.to_owned(),
            properties: properties.into(),
        };
        match self.pack.assets.registry.resolve(&state) {
            Ok(id) => self.journal_set_block(pos, id),
            Err(_) => false,
        }
    }
    /// The saved tanks of the mod block entity at `pos`, `n` of them.
    fn tank_state(&self, pos: [i32; 3], n: usize) -> Option<Vec<Tank>> {
        match self.world.block_entities.get(&(pos[0], pos[1], pos[2]))? {
            BlockEntity::Mod(m) => Some(read_tanks(&m.nbt, n)),
            _ => None,
        }
    }
    /// Read one tank directly from the authoritative NBT without copying its list.
    fn tank_entry(&self, pos: [i32; 3], index: usize) -> Option<(Option<&JavaString>, i32)> {
        let BlockEntity::Mod(entity) = self.world.block_entities.get(&(pos[0], pos[1], pos[2]))?
        else {
            return None;
        };
        let Some(Tag::List(list)) = entity
            .nbt
            .iter()
            .find(|(key, _)| key.0.iter().copied().eq("fluids".encode_utf16()))
            .map(|(_, value)| value)
        else {
            return Some((None, 0));
        };
        let Some(entry) = list.get(index) else {
            return Some((None, 0));
        };
        let id = match entry.get("Fluid") {
            Some(Tag::String(id)) if !id.0.iter().copied().eq("minecraft:empty".encode_utf16()) => {
                Some(id)
            }
            _ => None,
        };
        let amount = match entry.get("Amount") {
            Some(Tag::Int(amount)) => *amount,
            _ => 0,
        };
        Some((id, amount))
    }

    /// `tank_state`'s writer.
    fn set_tank_state(&mut self, pos: [i32; 3], state: &[Tank]) {
        let key = (pos[0], pos[1], pos[2]);
        self.journal_block_entity(key);
        if let Some(BlockEntity::Mod(m)) = self.world.block_entities.get_mut(&key) {
            write_tanks(&mut m.nbt, state);
            self.world.modified.insert((key.0 >> 4, key.2 >> 4));
        }
    }
    /// The tanks the game's own fluid capability reaches at `at`: in the level in place only.
    fn capability_tanks(&self, at: BlockSide) -> Option<FluidAt> {
        if !self.level_in_place(at.level) {
            return None;
        }
        self.native_fluid_at(at.pos)
    }
    /// How many tanks of the block at `at` in the level in place its side reaches
    /// (none: every tank), for the game's own fluid capability: a mod block's by its table, a
    /// cauldron's one. None where no tanks are.
    pub(in crate::game) fn fluid_tank_count(&self, at: BlockSide) -> Option<usize> {
        match self.capability_tanks(at)? {
            FluidAt::Mod(tanks) => Some(side_tanks(&tanks, at.side).count()),
            FluidAt::Cauldron(_) => Some(1),
        }
    }
    /// What tank `tank` of those `at`'s side reaches holds (`fluid_tank_count`).
    pub(in crate::game) fn fluid_tank(&self, at: BlockSide, tank: usize) -> Option<FluidStack> {
        match self.capability_tanks(at)? {
            FluidAt::Mod(tanks) => {
                let real = side_tanks(&tanks, at.side).nth(tank)?;
                let (fluid, amount) = self.tank_entry(at.pos, real)?;
                let amount = u32::try_from(amount).ok().filter(|&amount| amount > 0)?;
                let id = &fluid?.0;
                let identity = fluid_identity(id)?;
                Some(FluidStack {
                    fluid: identity,
                    amount,
                    components: None,
                })
            }
            // All a cauldron holds is its content's whole levels.
            FluidAt::Cauldron(cauldron) if tank == 0 => {
                let (content, level) = cauldron.held()?;
                Some(FluidStack::new(
                    mod_api::__private::fluid_key(&content.fluid)?,
                    content.amount(level).unsigned_abs(),
                ))
            }
            FluidAt::Cauldron(_) => None,
        }
    }
    /// How much of `fluid` tank `tank` of those `at`'s side reaches holds when empty: a mod
    /// block's fillable tank its capacity, a cauldron what its content of `fluid` holds when
    /// full; none for another tank, a drain-only one, or a fluid the tanks there don't take:
    /// one with no cauldron content, or with components into a mod block's tanks.
    pub(in crate::game) fn fluid_tank_capacity(
        &self,
        at: BlockSide,
        tank: usize,
        fluid: &FluidStack,
    ) -> u32 {
        match self.capability_tanks(at) {
            Some(at) if !game_holds(fluid, &at) => 0,
            Some(FluidAt::Mod(tanks)) => side_tanks(&tanks, at.side)
                .nth(tank)
                .filter(|real| tanks.inputs.contains(real))
                .map_or(0, |real| tanks.tanks[real].max(0).unsigned_abs()),
            Some(FluidAt::Cauldron(cauldron)) if tank == 0 => fluid
                .key()
                .and_then(mod_api::__private::fluid_id)
                .and_then(|id| cauldron.contents.of_fluid(id))
                .map_or(0, |(_, content)| content.total),
            _ => 0,
        }
    }
    /// Puts as much of `fluid` as tank `tank` of those `at`'s side reaches takes: a mod
    /// block's fillable tank, empty or holding that fluid, up to its capacity; a cauldron by
    /// its content's whole levels (water's thirds, lava only whole into an empty one),
    /// discarding the fluid's components. What didn't fit. The write is kept by
    /// transaction `tx`.
    pub(in crate::game) fn fluid_tank_insert(
        &mut self,
        at: BlockSide,
        tank: usize,
        fluid: FluidStack,
        tx: &Transaction,
    ) -> Option<FluidStack> {
        if !self.transaction_holds(tx) {
            return Some(fluid).filter(|fluid| fluid.amount > 0);
        }
        if fluid.amount == 0 {
            return None;
        }
        let amount = mb(fluid.amount);
        let took = match self.capability_tanks(at) {
            Some(at) if !game_holds(&fluid, &at) => 0,
            Some(FluidAt::Mod(tanks)) => {
                let real = side_tanks(&tanks, at.side)
                    .nth(tank)
                    .filter(|real| tanks.inputs.contains(real));
                let Some(real) = real else {
                    return Some(fluid);
                };
                let Some((held, current)) = self.tank_entry(at.pos, real) else {
                    return Some(fluid);
                };
                if current > 0
                    && !held.is_some_and(|held| {
                        mod_api::__private::fluid_identity_matches_utf16(fluid.fluid, &held.0)
                    })
                {
                    return Some(fluid);
                }
                let add = (tanks.tanks[real] - current).clamp(0, amount);
                if add > 0 {
                    let Some(mut state) = self.tank_state(at.pos, tanks.tanks.len()) else {
                        return Some(fluid);
                    };
                    state[real] = (
                        Some(mod_api::__private::fluid_identity_name(fluid.fluid)),
                        current + add,
                    );
                    self.set_tank_state(at.pos, &state);
                }
                add
            }
            Some(FluidAt::Cauldron(cauldron)) if tank == 0 => match cauldron.fill(
                mod_api::__private::fluid_id(fluid.key().expect("validated cauldron fluid"))
                    .expect("resolved fluid key"),
                amount,
            ) {
                Some((took, to)) if self.cauldron_set(at.pos, &to) => took,
                _ => 0,
            },
            _ => 0,
        };
        let rest = fluid.amount - took.unsigned_abs();
        (rest > 0).then(|| fluid.with_amount(rest))
    }
    /// Takes up to `max` out of tank `tank` of those `at`'s side reaches: a mod block's
    /// drainable tank, a cauldron by its content's whole levels (water's thirds, lava only
    /// whole). What came out. The write is kept by transaction `tx`.
    pub(in crate::game) fn fluid_tank_extract(
        &mut self,
        at: BlockSide,
        tank: usize,
        max: u32,
        tx: &Transaction,
    ) -> Option<FluidStack> {
        if !self.transaction_holds(tx) {
            return None;
        }
        let max = mb(max);
        match self.capability_tanks(at)? {
            FluidAt::Mod(tanks) => {
                let real = side_tanks(&tanks, at.side)
                    .nth(tank)
                    .filter(|real| tanks.outputs.contains(real))?;
                let (name, amount) = self.tank_entry(at.pos, real)?;
                let id = &name?.0;
                let identity = fluid_identity(id)?;
                let mut state = self.tank_state(at.pos, tanks.tanks.len())?;
                let entry = &mut state[real];
                let take = amount.clamp(0, max);
                if take == 0 {
                    return None;
                }
                entry.1 -= take;
                if entry.1 == 0 {
                    entry.0 = None;
                }
                self.set_tank_state(at.pos, &state);
                Some(FluidStack {
                    fluid: identity,
                    amount: take.unsigned_abs(),
                    components: None,
                })
            }
            FluidAt::Cauldron(cauldron) if tank == 0 => {
                let (fluid, took, to) = cauldron.drain(max)?;
                let key = mod_api::__private::fluid_key(fluid)?;
                self.cauldron_set(at.pos, &to)
                    .then(|| FluidStack::new(key, took.unsigned_abs()))
            }
            FluidAt::Cauldron(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::HostTransaction;
    use mod_api::ServerHost;

    /// Vanilla's table, with an empty cauldron, water at `level` and lava on it: what the
    /// cauldron holds as (water level, lava) for the assertions.
    fn vanilla_cauldrons() -> (
        CauldronFluids,
        impl Fn(u32) -> CauldronFluids,
        CauldronFluids,
        impl Fn(&CauldronFluids) -> (u32, bool),
    ) {
        let contents = Arc::new(CauldronContents::vanilla());
        let water = contents.of_block("minecraft:water_cauldron").unwrap().0;
        let lava = contents.of_block("minecraft:lava_cauldron").unwrap().0;
        let empty = CauldronFluids {
            contents,
            held: None,
        };
        let level = move |cauldron: &CauldronFluids| match cauldron.held {
            None => (0, false),
            Some((index, level)) if index == water => (level, false),
            Some(_) => (0, true),
        };
        (
            empty.clone(),
            {
                let empty = empty.clone();
                move |level| empty.with(Some((water, level)))
            },
            empty.with(Some((lava, 1))),
            level,
        )
    }

    #[test]
    fn a_cauldron_takes_and_gives_water_by_whole_thirds_and_lava_only_whole() {
        let (empty, water, lava, level) = vanilla_cauldrons();
        let fill = |kind: &CauldronFluids, fluid, amount| {
            kind.fill(fluid, amount)
                .map(|(took, to)| (took, level(&to)))
        };
        // 500 mB raises an empty cauldron one level, 333 mB; a full one takes none.
        assert_eq!(
            fill(&empty, "minecraft:water", 500),
            Some((333, (1, false)))
        );
        assert_eq!(
            fill(&water(1), "minecraft:water", 1000),
            Some((667, (3, false)))
        );
        assert_eq!(fill(&water(3), "minecraft:water", 1000), None);
        assert_eq!(fill(&empty, "minecraft:lava", 999), None);
        assert_eq!(
            fill(&empty, "minecraft:lava", 1500),
            Some((1000, (0, true)))
        );
        assert_eq!(fill(&water(1), "minecraft:lava", 1000), None);
        assert_eq!(fill(&lava, "minecraft:water", 1000), None);
        // A fluid with no content, powder snow's or a mod's, never goes in.
        assert_eq!(fill(&empty, "minecraft:powder_snow", 1000), None);
        let drain = |kind: &CauldronFluids, amount| {
            kind.drain(amount)
                .map(|(fluid, took, to)| (fluid.to_owned(), took, level(&to)))
        };
        assert_eq!(
            drain(&water(3), 500),
            Some(("minecraft:water".to_owned(), 334, (2, false)))
        );
        assert_eq!(drain(&water(1), 332), None);
        assert_eq!(
            drain(&water(2), 1000),
            Some(("minecraft:water".to_owned(), 666, (0, false)))
        );
        assert_eq!(drain(&lava, 999), None);
        assert_eq!(
            drain(&lava, 1000),
            Some(("minecraft:lava".to_owned(), 1000, (0, false)))
        );
        assert_eq!(drain(&empty, i32::MAX), None);
    }

    #[test]
    #[ignore = "daily"]
    fn a_cauldron_move_the_target_takes_part_of_gives_the_whole_levels_that_fit() {
        use super::super::super::native_entity_refs::tests::with_game;
        use horizon_client::chunk::Chunk;
        use mod_api::{BlockUpdateFlags, FLUIDS, move_resources};
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let water = |game: &Game<'_>, n: u32| {
                let state = horizon_world::chunk::BlockState {
                    name: "minecraft:water_cauldron".to_owned(),
                    properties: [("level".to_owned(), n.to_string())].into_iter().collect(),
                };
                game.pack.assets.registry.resolve(&state).unwrap()
            };
            let (full, one) = (water(game, 3), water(game, 1));
            let (from, to) = ([0, 64, 0], [2, 64, 0]);
            let flags = BlockUpdateFlags::NEIGHBORS;
            assert_eq!(game.level_set_block(level, from, full, flags), Ok(true));
            assert_eq!(game.level_set_block(level, to, one, flags), Ok(true));
            // The target has room for two levels: the bucket the source first gives doesn't
            // fit, so the move is undone and tried again with what fit, two levels' worth.
            let host: &mut dyn ServerHost = game;
            let source = level.capability(host, &FLUIDS, from, None).unwrap();
            let target = level.capability(host, &FLUIDS, to, None).unwrap();
            let moved = host
                .transaction(|host, tx| move_resources(host, &source, &target, |_| true, 1000, tx));
            assert_eq!(moved, Ok(667));
            assert_eq!(game.world.block(from[0], from[1], from[2]), one);
            assert_eq!(game.world.block(to[0], to[1], to[2]), full);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_mods_cauldron_fills_and_drains_by_its_levels_and_a_refusal_names_both_sides() {
        use super::super::super::native_entity_refs::tests::{
            record_key, run_fresh_cases, with_game_at,
        };
        if run_fresh_cases(
            "game::machines::native_fluids::tests::a_mods_cauldron_fills_and_drains_by_its_levels_and_a_refusal_names_both_sides",
            "HORIZON_CAULDRON_CONTENT_CHILD",
            &["valid"],
        ) {
            return;
        }
        use horizon_client::chunk::Chunk;
        use mod_api::{
            BlockKey, BlockUpdateFlags, FLUIDS, FluidKey, FluidResource, Hooks, ModEntry, Mods,
            RegisterCauldronFluidContentEvent, Registrar, RegistrationContext,
        };

        /// The keys the mod's `register` requested, as a real mod keeps them for its hooks.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        struct Keys {
            snow_cauldron: BlockKey,
            water_cauldron: BlockKey,
            composter: BlockKey,
            stone: BlockKey,
            syrup: FluidKey,
            honey: FluidKey,
            water: FluidKey,
        }
        #[derive(Default)]
        struct SyrupCauldron(std::sync::OnceLock<Keys>);
        #[mod_api::__subscriptions]
        impl Hooks for SyrupCauldron {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                let keys = Keys {
                    snow_cauldron: r.block_key("minecraft:powder_snow_cauldron"),
                    water_cauldron: r.block_key("minecraft:water_cauldron"),
                    composter: r.block_key("minecraft:composter"),
                    stone: r.block_key("minecraft:stone"),
                    syrup: r.declare_fluid("syrup"),
                    honey: r.declare_fluid("honey"),
                    water: r.fluid_key("minecraft:water"),
                };
                // Its provider is declared only in the later cauldron-content event below.
                // Coverage queries must not execute that future callback early.
                assert!(!r.is_block_capability_registered(&FLUIDS, &keys.snow_cauldron));
                record_key(&self.0, keys);
            }
            fn register_cauldron_fluid_content(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterCauldronFluidContentEvent<'_>,
            ) {
                let keys = self.0.get().unwrap();
                // The powder snow cauldron holds no fluid here, so it can be the syrup
                // cauldron: 3000 mB over its `level` 1..3.
                event
                    .contents
                    .register(keys.snow_cauldron, keys.syrup, 3000, Some("level"));
                // Refused: the water cauldron already has a content, water already has a
                // cauldron, and stone has no level property.
                event
                    .contents
                    .register(keys.water_cauldron, keys.honey, 1000, Some("level"));
                event
                    .contents
                    .register(keys.composter, keys.water, 8000, Some("level"));
                event
                    .contents
                    .register(keys.stone, keys.honey, 1000, Some("level"));
            }
        }
        let mods: Mods = vec![ModEntry::new(
            "cauldron_test",
            Box::new(SyrupCauldron::default()),
        )]
        .into();
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            let base =
                std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
            let root = super::super::super::mods::content_root(&base, &Default::default())
                .unwrap()
                .prepared()
                .to_path_buf();
            with_game_at(root, |game| {
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let level = game.level("minecraft:overworld").unwrap();
                let flags = BlockUpdateFlags::NEIGHBORS;
                let block = |game: &Game<'_>, name: &str, level: Option<u32>| {
                    let state = horizon_world::chunk::BlockState {
                        name: name.to_owned(),
                        properties: level
                            .map(|n| ("level".to_owned(), n.to_string()))
                            .into_iter()
                            .collect(),
                    };
                    game.pack.assets.registry.resolve(&state).unwrap()
                };
                let resource = |fluid: &str| {
                    FluidResource::of(&FluidStack::new(
                        mod_api::__private::fluid_key(fluid).unwrap(),
                        1,
                    ))
                };
                let (syrup, water, lava) = (
                    resource("cauldron_test:syrup"),
                    resource("minecraft:water"),
                    resource("minecraft:lava"),
                );

                // The refusals name both sides; the rest of the mod's contents still apply,
                // and water and lava keep theirs.
                let (contents, refusals) =
                    super::super::super::mods::collect_cauldron_fluid_contents(
                        game.pack.assets.registry,
                    );
                assert_eq!(refusals.len(), 3, "{refusals:?}");
                for (refusal, named) in refusals.iter().zip([
                    [
                        "minecraft:water_cauldron",
                        "minecraft:water",
                        "cauldron_test:honey",
                    ],
                    [
                        "minecraft:water",
                        "minecraft:water_cauldron",
                        "minecraft:composter",
                    ],
                    ["minecraft:stone", "level", "cauldron_test:honey"],
                ]) {
                    assert!(
                        named.iter().all(|name| refusal.contains(name)),
                        "{refusal}: names {named:?}"
                    );
                }
                let content = |block: &str| {
                    contents.of_block(block).map(|(_, content)| {
                        (content.fluid.clone(), content.total, content.levels())
                    })
                };
                assert_eq!(
                    content("minecraft:powder_snow_cauldron"),
                    Some(("cauldron_test:syrup".to_owned(), 3000, 3))
                );
                assert_eq!(
                    content("minecraft:water_cauldron"),
                    Some(("minecraft:water".to_owned(), 1000, 3))
                );
                assert_eq!(
                    content("minecraft:lava_cauldron"),
                    Some(("minecraft:lava".to_owned(), 1000, 1))
                );
                assert_eq!(content("minecraft:composter"), None);
                assert_eq!(content("minecraft:stone"), None);

                // The handler on the syrup cauldron reports syrup and a whole number of
                // levels, 1000 mB each, at each level, with room for 3000 mB of syrup.
                let pos = [0, 64, 0];
                let mut handler = None;
                for (n, held) in [(1, 1000), (2, 2000), (3, 3000)] {
                    let cauldron = block(game, "minecraft:powder_snow_cauldron", Some(n));
                    assert_eq!(game.level_set_block(level, pos, cauldron, flags), Ok(true));
                    // The handler is the position's: one answers every cauldron there.
                    let handler = handler
                        .get_or_insert_with(|| level.capability(game, &FLUIDS, pos, None).unwrap());
                    assert_eq!(handler.size(game), 1);
                    assert_eq!(handler.resource(game, 0), Some(syrup.clone()));
                    assert_eq!(handler.amount(game, 0), held);
                    assert_eq!(handler.capacity(game, 0, &syrup), 3000);
                    assert_eq!(handler.capacity(game, 0, &water), 1000);
                }
                let handler = handler.unwrap();
                let extract = |game: &mut Game<'_>, resource: &FluidResource, amount| {
                    game.transaction(|host, tx| {
                        Ok::<_, ()>(handler.extract(host, tx, 0, resource, amount))
                    })
                    .unwrap()
                };
                let insert = |game: &mut Game<'_>, resource: &FluidResource, amount| {
                    game.transaction(|host, tx| {
                        Ok::<_, ()>(handler.insert(host, tx, 0, resource, amount))
                    })
                    .unwrap()
                };
                let at = |game: &Game<'_>| game.world.block(pos[0], pos[1], pos[2]);
                // A drain takes whole levels: 1500 mB asked takes one level, 999 none, and the
                // last level leaves an empty cauldron.
                assert_eq!(extract(game, &syrup, 1500), 1000);
                assert_eq!(
                    at(game),
                    block(game, "minecraft:powder_snow_cauldron", Some(2))
                );
                assert_eq!(extract(game, &syrup, 999), 0);
                assert_eq!(extract(game, &water, 1000), 0);
                assert_eq!(
                    at(game),
                    block(game, "minecraft:powder_snow_cauldron", Some(2))
                );
                assert_eq!(extract(game, &syrup, 2000), 2000);
                assert_eq!(at(game), block(game, "minecraft:cauldron", None));
                assert_eq!(handler.resource(game, 0), None);
                assert_eq!(handler.amount(game, 0), 0);
                // An empty cauldron filled with syrup becomes the syrup cauldron at the level
                // the amount reaches; honey has no content and goes nowhere.
                assert_eq!(insert(game, &resource("cauldron_test:honey"), 3000), 0);
                assert_eq!(at(game), block(game, "minecraft:cauldron", None));
                assert_eq!(insert(game, &syrup, 2500), 2000);
                assert_eq!(
                    at(game),
                    block(game, "minecraft:powder_snow_cauldron", Some(2))
                );
                assert_eq!(insert(game, &water, 1000), 0);
                assert_eq!(insert(game, &syrup, 1000), 1000);
                assert_eq!(
                    at(game),
                    block(game, "minecraft:powder_snow_cauldron", Some(3))
                );
                assert_eq!(insert(game, &syrup, 1000), 0);

                // Water and lava answer exactly as before: water by thirds of a bucket, the
                // top third 334 mB, lava a bucket only whole, and syrup into neither.
                assert_eq!(extract(game, &syrup, 3000), 3000);
                assert_eq!(insert(game, &water, 500), 333);
                assert_eq!(at(game), block(game, "minecraft:water_cauldron", Some(1)));
                assert_eq!(insert(game, &syrup, 1000), 0);
                assert_eq!(insert(game, &water, 1000), 667);
                assert_eq!(at(game), block(game, "minecraft:water_cauldron", Some(3)));
                assert_eq!(handler.resource(game, 0), Some(water.clone()));
                assert_eq!(handler.amount(game, 0), 1000);
                assert_eq!(extract(game, &water, 500), 334);
                assert_eq!(at(game), block(game, "minecraft:water_cauldron", Some(2)));
                assert_eq!(extract(game, &water, 1000), 666);
                assert_eq!(at(game), block(game, "minecraft:cauldron", None));
                assert_eq!(insert(game, &lava, 999), 0);
                assert_eq!(insert(game, &lava, 1500), 1000);
                assert_eq!(at(game), block(game, "minecraft:lava_cauldron", None));
                assert_eq!(handler.resource(game, 0), Some(lava.clone()));
                assert_eq!(handler.amount(game, 0), 1000);
                assert_eq!(handler.capacity(game, 0, &lava), 1000);
                assert_eq!(insert(game, &syrup, 1000), 0);
                assert_eq!(extract(game, &lava, 999), 0);
                assert_eq!(extract(game, &lava, 1000), 1000);
                assert_eq!(at(game), block(game, "minecraft:cauldron", None));
            })
        });
    }

    #[test]
    fn a_cauldron_takes_water_and_lava_whatever_their_components_and_a_mod_tank_none_with_them() {
        use super::{FluidAt, FluidStack, game_holds};
        mod_platform::__private::part_tests::game_thread();
        let (empty, water, _, _) = vanilla_cauldrons();
        let potion = |fluid| {
            use horizon_core::nbt::{Compound, Tag};
            let mut potion = Compound::new();
            potion.insert("potion".into(), Tag::String("minecraft:healing".into()));
            let mut components = Compound::new();
            components.insert("minecraft:potion_contents".into(), Tag::Compound(potion));
            FluidStack {
                components: Some(Box::new(components.into())),
                ..FluidStack::new(mod_api::__private::fluid_key(fluid).unwrap(), 1000)
            }
        };
        let cauldron = FluidAt::Cauldron(empty);
        let tank = FluidAt::Mod(Arc::default());
        for fluid in ["minecraft:water", "minecraft:lava"] {
            assert!(game_holds(&potion(fluid), &cauldron), "{fluid}");
            assert!(
                game_holds(
                    &FluidStack::new(mod_api::__private::fluid_key(fluid).unwrap(), 1000),
                    &tank
                ),
                "{fluid}"
            );
            assert!(!game_holds(&potion(fluid), &tank), "{fluid}");
        }
        assert!(!game_holds(&potion("test:potion"), &cauldron));
        // What a cauldron gives is named by its fluid alone: its components are gone.
        let full = water(3);
        let (fluid, amount, _) = full.drain(1000).unwrap();
        assert_eq!(
            FluidStack::new(
                mod_api::__private::fluid_key(fluid).unwrap(),
                amount.unsigned_abs()
            ),
            FluidStack::new(
                mod_api::__private::fluid_key("minecraft:water").unwrap(),
                1000
            )
        );
    }

    /// A scoped provider using the block entity's saved tanks. Capacity describes the
    /// storage; validity independently limits new contents to plain water. Old contents
    /// remain drainable even when they would no longer be accepted.
    struct WaterTank {
        entity: mod_api::BlockEntityRef,
        reached: Vec<usize>,
    }

    impl WaterTank {
        fn state(&self, host: &dyn mod_api::Host) -> Vec<Tank> {
            read_tanks(&self.entity.data(host).unwrap(), 2)
        }

        fn write(&self, host: &mut dyn ServerHost, state: &[Tank]) -> bool {
            let mut nbt = Compound::new();
            write_tanks(&mut nbt, state);
            self.entity.merge_data(host, nbt).unwrap()
        }
    }

    impl mod_api::ResourceHandler<mod_api::FluidResource> for WaterTank {
        fn size(&self, _: &mut dyn ServerHost) -> usize {
            self.reached.len()
        }
        fn resource(
            &self,
            host: &mut dyn ServerHost,
            index: usize,
        ) -> Option<mod_api::FluidResource> {
            let real = *self.reached.get(index)?;
            let (fluid, amount) = self.state(host).swap_remove(real);
            fluid.filter(|_| amount > 0).map(|fluid| {
                mod_api::FluidResource::of(&FluidStack::new(
                    mod_api::__private::fluid_key(&fluid).unwrap(),
                    1,
                ))
            })
        }
        fn amount(&self, host: &mut dyn ServerHost, index: usize) -> u32 {
            self.reached
                .get(index)
                .map_or(0, |&real| self.state(host)[real].1.max(0).unsigned_abs())
        }
        fn capacity(
            &self,
            _: &mut dyn ServerHost,
            index: usize,
            _: &mod_api::FluidResource,
        ) -> u32 {
            if index < self.reached.len() { 4000 } else { 0 }
        }
        fn is_valid(
            &self,
            _: &mut dyn ServerHost,
            index: usize,
            resource: &mod_api::FluidResource,
        ) -> bool {
            index < self.reached.len()
                && resource
                    == &mod_api::FluidResource::of(&FluidStack::new(
                        mod_api::__private::fluid_key("minecraft:water").unwrap(),
                        1,
                    ))
        }
        fn insert(
            &self,
            host: &mut dyn ServerHost,
            _: &Transaction,
            index: usize,
            resource: &mod_api::FluidResource,
            amount: u32,
        ) -> u32 {
            if !self.is_valid(host, index, resource) {
                return 0;
            }
            let mut state = self.state(host);
            let entry = &mut state[self.reached[index]];
            if entry.1 > 0 && entry.0.as_deref() != Some("minecraft:water") {
                return 0;
            }
            let took = amount.min((4000 - entry.1).max(0).unsigned_abs());
            if took == 0 {
                return 0;
            }
            entry.0 = Some("minecraft:water".into());
            entry.1 += took as i32;
            if self.write(host, &state) { took } else { 0 }
        }
        fn extract(
            &self,
            host: &mut dyn ServerHost,
            _: &Transaction,
            index: usize,
            resource: &mod_api::FluidResource,
            amount: u32,
        ) -> u32 {
            let Some(&real) = self.reached.get(index) else {
                return 0;
            };
            if self.resource(host, index).as_ref() != Some(resource) {
                return 0;
            }
            let mut state = self.state(host);
            let entry = &mut state[real];
            let took = amount.min(entry.1.max(0).unsigned_abs());
            if took == 0 {
                return 0;
            }
            entry.1 -= took as i32;
            if entry.1 == 0 {
                entry.0 = None;
            }
            if self.write(host, &state) { took } else { 0 }
        }
    }

    #[test]
    #[ignore = "daily"]
    fn a_scoped_tank_can_refuse_a_fluid() {
        use super::super::super::native_entity_refs::tests::{run_fresh_cases, with_game_at};
        if run_fresh_cases(
            "game::machines::native_fluids::tests::a_scoped_tank_can_refuse_a_fluid",
            "HORIZON_SCOPED_FLUID_CHILD",
            &["valid"],
        ) {
            return;
        }
        use horizon_client::{
            chunk::Chunk,
            containers::{ModBlockEntity, ModBlockEntityFields},
            fast_hash::FxHashMap,
        };
        use mod_api::{
            BlockUpdateFlags, FLUIDS, FluidResource, Hooks, ModEntry, Mods, Registrar,
            move_resources,
        };

        struct WaterTanks;
        #[mod_api::__subscriptions]
        impl Hooks for WaterTanks {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                r.declare_fluid("chocolate");
                let kind = r
                    .block_entity_type("water_tank")
                    .blocks(&["fluid_filter_test:water_tank"])
                    .build();
                r.block_entity_capability_before_game(&FLUIDS, &kind, |host, at| {
                    // The same block entity type elsewhere keeps the game's default handler.
                    (at.pos == [2, 64, 0]).then(|| {
                        Box::new(WaterTank {
                            entity: host.block_entity(at.level, at.pos).unwrap(),
                            reached: if at.side == Some(Direction::Down) {
                                vec![1, 0]
                            } else {
                                vec![0, 1]
                            },
                        })
                            as Box<dyn mod_api::ResourceHandler<FluidResource>>
                    })
                });
            }
            fn blocks(
                &self,
                _: &mod_api::RegistrationContext,
                event: &mut mod_api::BlocksEvent<'_>,
            ) {
                event.blocks.push(mod_api::Block::new(
                    "fluid_filter_test:water_tank",
                    "minecraft:stone",
                ));
            }
        }
        let mods: Mods = vec![ModEntry::new("fluid_filter_test", Box::new(WaterTanks))].into();
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            let base =
                std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
            let root = super::super::super::mods::content_root(&base, &Default::default())
                .unwrap()
                .prepared()
                .to_path_buf();
            with_game_at(root, |game| {
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let level = game.level("minecraft:overworld").unwrap();
                let block = game
                    .pack
                    .assets
                    .registry
                    .default_state("fluid_filter_test:water_tank")
                    .unwrap();
                let mut table = ModFluidTanks::default();
                table.tanks = vec![4000, 4000];
                table.inputs = vec![0, 1];
                table.outputs = vec![0, 1];
                table.faces = std::array::from_fn(|_| vec![1, 0]);
                game.world
                    .block_entities
                    .set_mod_fluids(FxHashMap::from_iter([(block.0, Arc::new(table))]));
                let (from, to) = ([0, 64, 0], [2, 64, 0]);
                for pos in [from, to] {
                    assert_eq!(
                        game.level_set_block(level, pos, block, BlockUpdateFlags::NEIGHBORS),
                        Ok(true)
                    );
                    game.world.block_entities.insert(
                        (pos[0], pos[1], pos[2]),
                        BlockEntity::Mod(Box::new(ModBlockEntity::new(
                            "fluid_filter_test:water_tank".into(),
                            ModBlockEntityFields::default(),
                        ))),
                    );
                }
                // The default takes a mod's fluid; a scoped answer does not affect it.
                let source = level.capability(game, &FLUIDS, from, None).unwrap();
                let chocolate = FluidResource::of(&FluidStack::new(
                    mod_api::__private::fluid_key("fluid_filter_test:chocolate").unwrap(),
                    1,
                ));
                let water = FluidResource::of(&FluidStack::new(
                    mod_api::__private::fluid_key("minecraft:water").unwrap(),
                    1,
                ));
                assert_eq!(source.owner(), "the game");
                let empty = FluidResource::of(&FluidStack::new(
                    mod_api::__private::fluid_key("minecraft:empty").unwrap(),
                    1000,
                ));
                assert_eq!(source.capacity(game, 0, &empty), 0);
                assert_eq!(
                    game.transaction(|host, tx| Ok::<_, ()>(
                        source.insert(host, tx, 0, &empty, 1000)
                    )),
                    Ok(0)
                );
                assert_eq!(source.resource(game, 0), None);
                for id in ["mystery", "minecraft:mystery"] {
                    game.set_tank_state(from, &tanks(&[(Some(id), 1000), (None, 0)]));
                    assert!(source.resource(game, 0).is_none());
                    assert_eq!(
                        game.tank_state(from, 2).unwrap()[0],
                        (Some(id.to_owned()), 1000)
                    );
                }
                let handle = mod_api::__private::intern_unresolved_fluid("othermod:fluid").unwrap();
                game.set_tank_state(from, &tanks(&[(Some("othermod:fluid"), 1000), (None, 0)]));
                let pipe_target = [4, 64, 0];
                assert_eq!(
                    game.level_set_block(level, pipe_target, block, BlockUpdateFlags::NEIGHBORS),
                    Ok(true)
                );
                game.world.block_entities.insert(
                    (4, 64, 0),
                    BlockEntity::Mod(Box::new(ModBlockEntity::new(
                        "fluid_filter_test:water_tank".into(),
                        ModBlockEntityFields::default(),
                    ))),
                );
                let target = level.capability(game, &FLUIDS, pipe_target, None).unwrap();
                assert_eq!(
                    game.transaction(|host, tx| move_resources(
                        host,
                        &source,
                        &target,
                        |_| true,
                        300,
                        tx
                    )),
                    Ok(300)
                );
                assert_eq!(
                    game.tank_state(pipe_target, 2).unwrap()[0],
                    (Some("othermod:fluid".into()), 300)
                );
                let mut saved = Chunk::empty(0, 0);
                saved.set(4, 64, 0, block.0);
                saved.block_entities.push((
                    (4, 64, 0),
                    game.world.block_entities.get(&(4, 64, 0)).unwrap().clone(),
                ));
                let mut disk = horizon_client::save::SaveDir::new(
                    std::env::temp_dir()
                        .join(format!("horizon-fluid-roundtrip-{}", std::process::id())),
                    horizon_client::save::LevelShape::OVERWORLD,
                );
                disk.enable_trusted_fluid_load();
                disk.save(
                    &[Arc::new(saved)],
                    game.pack.assets.registry,
                    game.res.biomes(),
                )
                .unwrap();
                let loaded = disk
                    .load(0, 0, game.pack.assets.registry, game.res.biomes())
                    .unwrap();
                let BlockEntity::Mod(loaded) = &loaded.block_entities[0].1 else {
                    panic!("saved mod tank")
                };
                assert_eq!(
                    read_tanks(&loaded.nbt, 2)[0],
                    (Some("othermod:fluid".into()), 300)
                );
                let unresolved = source.resource(game, 0).unwrap();
                assert_eq!(
                    *unresolved.fluid(),
                    mod_api::FluidIdentity::Unresolved(handle)
                );
                assert_eq!(source.amount(game, 0), 700);
                assert_eq!(
                    game.transaction(|host, tx| Ok::<_, ()>(source.extract(
                        host,
                        tx,
                        0,
                        &unresolved,
                        200
                    ))),
                    Ok(200)
                );
                assert_eq!(
                    game.tank_state(from, 2).unwrap()[0],
                    (Some("othermod:fluid".into()), 500)
                );
                game.set_tank_state(from, &tanks(&[(Some("water"), 1000), (None, 0)]));
                assert_eq!(source.resource(game, 0), Some(water.clone()));
                assert_eq!(source.amount(game, 0), 1000);
                assert_eq!(
                    game.transaction(|host, tx| Ok::<_, ()>(
                        source.extract(host, tx, 0, &water, 1000)
                    )),
                    Ok(1000)
                );
                assert_eq!(source.resource(game, 0), None);
                assert!(source.is_valid(game, 0, &chocolate));
                assert_eq!(
                    game.transaction(|host, tx| Ok::<_, ()>(
                        source.insert(host, tx, 0, &chocolate, 1000)
                    )),
                    Ok(1000)
                );
                assert_eq!(source.resource(game, 0), Some(chocolate.clone()));
                assert_eq!(source.amount(game, 0), 1000);

                // A refusal must protect both a previously filled tank and an empty one.
                game.set_tank_state(
                    to,
                    &tanks(&[(Some("fluid_filter_test:chocolate"), 1000), (None, 0)]),
                );
                let target = level.capability(game, &FLUIDS, to, None).unwrap();
                assert_eq!(target.owner(), "fluid_filter_test");
                for index in 0..2 {
                    assert_eq!(target.capacity(game, index, &chocolate), 4000);
                    assert!(!target.is_valid(game, index, &chocolate));
                    assert_eq!(
                        game.transaction(|host, tx| Ok::<_, ()>(
                            target.insert(host, tx, index, &chocolate, 500)
                        )),
                        Ok(0)
                    );
                }
                assert_eq!(target.amount(game, 0), 1000);
                assert_eq!(target.resource(game, 0), Some(chocolate.clone()));
                assert_eq!(target.amount(game, 1), 0);
                assert_eq!(target.resource(game, 1), None);
                assert_eq!(
                    game.transaction(|host, tx| move_resources(
                        host,
                        &source,
                        &target,
                        |_| true,
                        500,
                        tx
                    )),
                    Ok(0)
                );
                assert_eq!(source.amount(game, 0), 1000);
                assert_eq!(target.amount(game, 0), 1000);
                assert_eq!(target.amount(game, 1), 0);

                // Water is valid but cannot replace another fluid; it can fill the empty tank.
                assert!(target.is_valid(game, 0, &water));
                assert_eq!(
                    game.transaction(|host, tx| Ok::<_, ()>(
                        target.insert(host, tx, 0, &water, 500)
                    )),
                    Ok(0)
                );
                assert!(target.is_valid(game, 1, &water));
                assert_eq!(
                    game.transaction(|host, tx| Ok::<_, ()>(
                        target.insert(host, tx, 1, &water, 500)
                    )),
                    Ok(500)
                );
                assert_eq!(target.resource(game, 1), Some(water.clone()));
                assert_eq!(target.amount(game, 1), 500);
                let sided = level
                    .capability(game, &FLUIDS, to, Some(Direction::Down))
                    .unwrap();
                assert_eq!(sided.resource(game, 0), Some(water.clone()));
                assert_eq!(sided.amount(game, 0), 500);
                // A drain does not consult acceptance, so an old refused fluid is recoverable.
                assert_eq!(
                    game.transaction(|host, tx| Ok::<_, ()>(
                        target.extract(host, tx, 0, &chocolate, 100)
                    )),
                    Ok(100)
                );
                assert_eq!(target.amount(game, 0), 900);
                assert_eq!(
                    game.transaction(|host, tx| Ok::<_, ()>(
                        source.insert(host, tx, 1, &water, 1000)
                    )),
                    Ok(1000)
                );
                assert_eq!(
                    game.transaction(|host, tx| move_resources(
                        host,
                        &source,
                        &target,
                        |resource| resource == &water,
                        200,
                        tx
                    )),
                    Ok(200)
                );
                assert_eq!(source.amount(game, 1), 800);
                assert_eq!(target.amount(game, 1), 700);
                let abort: Result<(), ()> = game.transaction(|host, tx| {
                    assert_eq!(target.insert(host, tx, 1, &water, 200), 200);
                    Err(())
                });
                assert!(abort.is_err());
                assert_eq!(target.amount(game, 1), 700);
                assert_eq!(
                    game.tank_state(from, 2),
                    Some(tanks(&[
                        (Some("fluid_filter_test:chocolate"), 1000),
                        (Some("minecraft:water"), 800)
                    ]))
                );
                assert_eq!(
                    game.tank_state(to, 2),
                    Some(tanks(&[
                        (Some("fluid_filter_test:chocolate"), 900),
                        (Some("minecraft:water"), 700)
                    ]))
                );
            })
        });
    }

    fn tanks(list: &[(Option<&str>, i32)]) -> Vec<Tank> {
        list.iter()
            .map(|&(fluid, amount)| (fluid.map(str::to_owned), amount))
            .collect()
    }

    #[test]
    fn tank_state_round_trips_through_nbt_and_missing_entries_read_empty() {
        let mut nbt = Compound::new();
        let state = tanks(&[(Some("minecraft:water"), 3334), (None, 0)]);
        write_tanks(&mut nbt, &state);
        assert_eq!(read_tanks(&nbt, 2), state);
        // A tank the save never heard of reads as empty, not as gone.
        let longer = read_tanks(&nbt, 3);
        assert_eq!(longer[2], (None, 0));
        // Anything else under `fluids` is the mod's own, not a tank list.
        nbt.insert(JavaString::from("fluids"), Tag::Int(0));
        assert_eq!(read_tanks(&nbt, 2), vec![(None, 0), (None, 0)]);
    }

    #[test]
    fn a_shrunk_tank_table_keeps_the_tanks_that_still_exist() {
        // A mod update that declares one tank where the save has two keeps
        // the first tank's contents, instead of reading all as empty and
        // writing the wipe back.
        let mut nbt = Compound::new();
        let saved = tanks(&[
            (Some("minecraft:water"), 4000),
            (Some("minecraft:lava"), 1000),
        ]);
        write_tanks(&mut nbt, &saved);
        assert_eq!(
            read_tanks(&nbt, 1),
            tanks(&[(Some("minecraft:water"), 4000)])
        );
    }

    #[test]
    fn an_empty_tank_writes_the_empty_fluid_not_none() {
        let mut nbt = Compound::new();
        write_tanks(
            &mut nbt,
            &tanks(&[(Some("minecraft:lava"), 1000), (None, 0)]),
        );
        let Some(Tag::List(list)) = nbt.get(&JavaString::from("fluids")) else {
            panic!("no list");
        };
        let Some(Tag::Compound(empty)) = list.get(1) else {
            panic!("no compound");
        };
        assert_eq!(
            empty.get(&JavaString::from("Fluid")),
            Some(&Tag::String(JavaString::from("minecraft:empty")))
        );
    }
}
