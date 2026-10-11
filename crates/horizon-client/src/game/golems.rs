//! Building golems from blocks (`CarvedPumpkinBlock.trySpawnGolem`): a carved
//! pumpkin or jack o'lantern on two snow blocks makes a snow golem, and on a
//! T of four iron blocks (arms along X or Z, nothing beside the legs) an iron
//! golem. Players placing the pumpkin and dispensers share it.
use super::Game;
use horizon_client::Result;
use horizon_world::chunk::BlockState;

impl Game<'_> {
    fn golem_block(&self, p: [i32; 3]) -> &str {
        let state = self.world.block(p[0], p[1], p[2]);
        &self.res.facts.get(state).name
    }
    fn is_golem_head(&self, p: [i32; 3]) -> bool {
        matches!(
            self.golem_block(p),
            "minecraft:carved_pumpkin" | "minecraft:jack_o_lantern"
        )
    }
    /// The blocks of the golem whose head is at `head` and its spawn base
    /// (`BlockPattern.find`), snow golems first.
    fn find_golem(&self, head: [i32; 3]) -> Option<(&'static str, Vec<[i32; 3]>, [i32; 3])> {
        let below = |n: i32| [head[0], head[1] - n, head[2]];
        if self.golem_block(below(1)) == "minecraft:snow_block"
            && self.golem_block(below(2)) == "minecraft:snow_block"
        {
            // The pattern ("^", "#", "#") is one column.
            return Some((
                "minecraft:snow_golem",
                vec![head, below(1), below(2)],
                below(2),
            ));
        }
        for (ax, az) in [(1, 0), (0, 1)] {
            let side = |s: i32, row: i32| [head[0] + ax * s, head[1] - row, head[2] + az * s];
            let arms = (-1..=1).all(|s| self.golem_block(side(s, 1)) == "minecraft:iron_block");
            let air = |p: [i32; 3]| self.res.facts.is_air(self.world.block(p[0], p[1], p[2]));
            if arms
                && self.golem_block(side(0, 2)) == "minecraft:iron_block"
                && [-1, 1].iter().all(|&s| air(side(s, 0)) && air(side(s, 2)))
            {
                // Every cell of ("~^~", "###", "~#~"), the air ones included.
                let blocks = (-1..=1)
                    .flat_map(|s| (0..3).map(move |row| (s, row)))
                    .map(|(s, row)| side(s, row))
                    .collect();
                return Some(("minecraft:iron_golem", blocks, side(0, 2)));
            }
        }
        None
    }
    /// `CarvedPumpkinBlock.canSpawnGolem`.
    pub(super) fn can_spawn_golem(&self, head: [i32; 3]) -> bool {
        self.find_golem(head).is_some()
    }
    /// `CarvedPumpkinBlock.trySpawnGolem`, after a pumpkin was placed.
    pub(super) fn check_golem_spawn(&mut self, head: [i32; 3]) -> Result<()> {
        if !self.is_golem_head(head) {
            return Ok(());
        }
        let Some((kind, blocks, base)) = self.find_golem(head) else {
            return self.try_spawn_copper_golem(head);
        };
        // CarvedPumpkinBlock.spawnGolemInWorld
        self.clear_pattern_blocks(&blocks)?;
        let at = [
            f64::from(base[0]) + 0.5,
            f64::from(base[1]) + 0.05,
            f64::from(base[2]) + 0.5,
        ];
        self.create_triggered(kind, at, 0.0);
        self.update_pattern_blocks(&blocks)
    }
    /// Whether placing a golem head at `head` completes a snow or iron golem.
    pub(super) fn completes_golem(&self, head: [i32; 3]) -> bool {
        self.find_golem(head).is_some()
    }
    /// `EntityType.create(level, TRIGGERED)` and `snapTo`: a built golem or
    /// wither has no spawn finalization and draws nothing from the level.
    pub(super) fn create_triggered(&mut self, kind: &str, at: [f64; 3], yaw: f32) -> Option<u32> {
        let t = self.res.mob_types.get(kind).cloned()?;
        self.next_entity += 1;
        let mut mob = horizon_client::mobs::Mob::new(
            self.next_entity,
            t,
            horizon_core::geometry::Vec3::new(at[0], at[1], at[2]),
            yaw,
        );
        mob.head_yaw = yaw;
        mob.body_yaw = yaw;
        mob.previous_yaw = yaw;
        mob.previous_body_yaw = yaw;
        mob.previous_head_yaw = yaw;
        let id = mob.id;
        self.mobs.push(mob);
        Some(id)
    }
    /// `CarvedPumpkinBlock.clearPatternBlocks`: every cell of the match
    /// becomes air (flags 2) with its destroy event (2001).
    pub(super) fn clear_pattern_blocks(&mut self, cells: &[[i32; 3]]) -> Result<()> {
        let air = self.world.air;
        for &p in cells {
            let state = self.world.block(p[0], p[1], p[2]);
            let pos = horizon_core::position::BlockPos::new(p[0], p[1], p[2]);
            self.with_level(|b, level, ticks| b.set_block(level, ticks, pos, air, 2));
            self.level_events
                .push(horizon_world::level::LevelEvent::LevelEvent {
                    id: 2001,
                    pos,
                    data: state.0 as i32,
                });
        }
        self.process_level_events()
    }
    /// `CarvedPumpkinBlock.updatePatternBlocks`: neighbours of every cell
    /// hear that air arrived.
    pub(super) fn update_pattern_blocks(&mut self, cells: &[[i32; 3]]) -> Result<()> {
        let air = self.world.air;
        for &p in cells {
            let pos = horizon_core::position::BlockPos::new(p[0], p[1], p[2]);
            self.with_level(|b, level, ticks| b.update_neighbors_at(level, ticks, pos, b.block(air)));
        }
        self.process_level_events()
    }
    /// A pumpkin on a copper block: the block turns into the matching copper
    /// chest (facing the pumpkin's way) and a copper golem appears where the
    /// pumpkin was (`CarvedPumpkinBlock.spawnGolemInWorld` with
    /// `CopperChestBlock.getFromCopperBlock`).
    fn try_spawn_copper_golem(&mut self, head: [i32; 3]) -> Result<()> {
        let below = [head[0], head[1] - 1, head[2]];
        let registry = self.res.assets.registry;
        let name = self.golem_block(below).to_string();
        let Some(base) = name.strip_prefix("minecraft:") else {
            return Ok(());
        };
        let waxed = base.strip_prefix("waxed_").unwrap_or(base);
        if !matches!(
            waxed,
            "copper_block" | "exposed_copper" | "weathered_copper" | "oxidized_copper"
        ) {
            return Ok(());
        }
        let chest = format!(
            "minecraft:{}_chest",
            base.strip_suffix("_block").unwrap_or(base)
        );
        let pumpkin = self.world.block(head[0], head[1], head[2]);
        let facing = registry
            .state(pumpkin)
            .and_then(|s| s.properties.get("facing").cloned())
            .unwrap_or_else(|| "north".into());
        let chest_state = BlockState {
            name: chest.clone(),
            properties: [
                ("facing".to_string(), facing),
                ("type".to_string(), "single".to_string()),
                ("waterlogged".to_string(), "false".to_string()),
            ]
            .into(),
        };
        let Some(chest_id) = registry
            .id(&chest_state)
            .or_else(|| registry.default_state(&chest))
        else {
            return Ok(());
        };
        self.destroy_particles(head, pumpkin);
        self.level_set(head, self.world.air)?;
        self.level_set(below, chest_id)?;
        let at = [
            f64::from(head[0]) + 0.5,
            f64::from(head[1]) + 0.05,
            f64::from(head[2]) + 0.5,
        ];
        let _ = self.spawn_mob("minecraft:copper_golem", at, false);
        Ok(())
    }
}
