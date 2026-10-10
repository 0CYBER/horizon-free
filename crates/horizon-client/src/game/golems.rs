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
                let mut blocks = vec![head];
                blocks.extend((-1..=1).map(|s| side(s, 1)));
                blocks.push(side(0, 2));
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
        for p in blocks {
            let state = self.world.block(p[0], p[1], p[2]);
            self.destroy_particles(p, state);
            self.level_set(p, self.world.air)?;
        }
        let at = [
            f64::from(base[0]) + 0.5,
            f64::from(base[1]) + 0.05,
            f64::from(base[2]) + 0.5,
        ];
        if let Some(id) = self.spawn_mob(kind, at, false)
            && let Some(m) = self.mobs.iter_mut().find(|m| m.id == id)
        {
            m.yaw = 0.0;
            m.body_yaw = 0.0;
            m.head_yaw = 0.0;
            m.previous_yaw = 0.0;
            m.previous_body_yaw = 0.0;
            m.previous_head_yaw = 0.0;
        }
        Ok(())
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
