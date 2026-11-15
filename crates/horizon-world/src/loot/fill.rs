use super::*;

/// Parameters supplied by the first inventory access. Automation has no entity
/// and zero luck; a player opening supplies THIS_ENTITY and effective luck.
#[derive(Clone, Copy, Default)]
pub struct ContainerLootContext<'a> {
    pub biome: Option<&'a str>,
    pub luck: f32,
    pub entity: Option<&'a LootEntity>,
}

impl LootData {
    /// LootTable.fill into an empty container of `slots` slots with
    /// `RandomSource.create(seed)`; `biome` is the container's biome for
    /// location checks (None: no origin, checks fail).
    pub fn fill(
        &self,
        table: &str,
        seed: i64,
        slots: usize,
        biome: Option<&str>,
    ) -> Vec<Option<LootStack>> {
        self.fill_with(table, &mut LegacyRandom::new(seed), slots, biome)
    }
    /// LootTable.fill drawing from `random` (a seeded random or the table's
    /// random sequence), which is advanced.
    pub fn fill_with<R: RandomSource + Clone>(
        &self,
        table: &str,
        random: &mut R,
        slots: usize,
        biome: Option<&str>,
    ) -> Vec<Option<LootStack>> {
        let mut out = vec![None; slots];
        if let Some(items) = self.roll_container_with(
            table,
            random,
            ContainerLootContext {
                biome,
                ..Default::default()
            },
        ) {
            self.place_container_loot(items, &mut out, (0..slots).collect(), random);
        }
        out
    }

    /// Roll container loot in table order, splitting to maximum stack sizes.
    /// Advances `random`; `None` means the table is missing (no draws).
    /// An existing table with no results returns `Some(Vec::new())`.
    pub fn roll_container_with<R: RandomSource + Clone>(
        &self,
        table: &str,
        random: &mut R,
        context: ContainerLootContext<'_>,
    ) -> Option<Vec<LootStack>> {
        let json = self.tables.get(table)?;
        let entity = context.entity.map(|this| EntityLootParams {
            this: this.clone(),
            ..Default::default()
        });
        let mut c = Context {
            query: query::current_query(),
            data: self,
            random: random.clone(),
            biome: context.biome,
            visited: Vec::new(),
            luck: context.luck,
            open_water: None,
            entity: entity.as_ref(),
            enchanted_level: None,
        };
        let mut items = Vec::new();
        c.table_raw(table, json, &mut |c, stack| c.data.split(stack, &mut items));
        c.query.finish(Some(table));
        *random = c.random;
        Some(items)
    }

    /// Shuffle free slots, split and shuffle rolled stacks, then place them.
    /// Continue with the roll's RNG. `available` must contain distinct, valid
    /// empty slot indices in container order; other slots are untouched.
    pub fn place_container_loot<R: RandomSource>(
        &self,
        mut items: Vec<LootStack>,
        slots: &mut [Option<LootStack>],
        mut available: Vec<usize>,
        random: &mut R,
    ) {
        shuffle(&mut available, random);
        shuffle_and_split(&mut items, available.len(), random);
        for stack in items {
            let Some(slot) = available.pop() else { break };
            slots[slot] = (!stack.is_empty()).then_some(stack);
        }
    }
}
