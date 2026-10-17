//! Non-player entities simulated on the client world. Dropped items follow
//! `ItemEntity.tick`: gravity 0.04, 0.98 drag (block friction on the ground),
//! a ground bounce of -0.5, water buoyancy, pickup delay and 6000-tick despawn.
use crate::{
    blocks::{BlockFacts, Fluid},
    collisions::SceneCollisions,
    items::ItemStack,
    world::World,
};
use horizon_core::geometry::{Aabb, Vec3};
use horizon_world::block_collision::BlockCollisionRegistry;

#[derive(Clone, Debug)]
pub struct ItemEntity {
    pub health: i32,
    pub fire_ticks: i32,
    pub id: u32,
    pub stack: ItemStack,
    pub thrower: Option<[i32; 4]>,
    pub position: Vec3,
    pub previous: Vec3,
    pub velocity: Vec3,
    pub on_ground: bool,
    pub age: u32,
    pub pickup_delay: u32,
    pub bob_offset: f32,
}
/// Environment results needed by the entity owner, including lethal lava damage.
#[derive(Debug)]
pub struct ItemEnvironmentTick {
    pub alive: bool,
    pub lava_hurt: bool,
}
impl ItemEnvironmentTick {
    pub fn should_play_lava_hurt_sound(&self, tick: u32) -> bool {
        self.lava_hurt && (!self.alive || tick.is_multiple_of(10))
    }
}
impl ItemEntity {
    pub const WIDTH: f64 = 0.25;
    pub fn bounds(&self) -> Aabb {
        let h = Self::WIDTH / 2.0;
        Aabb::new(
            Vec3::new(self.position.x - h, self.position.y, self.position.z - h),
            Vec3::new(
                self.position.x + h,
                self.position.y + Self::WIDTH,
                self.position.z + h,
            ),
        )
    }
    pub fn environment_tick(
        &mut self,
        world: &World,
        facts: &BlockFacts,
        registry: &horizon_world::registry::BlockRegistry,
        items: &crate::items::Items,
    ) -> ItemEnvironmentTick {
        let at = world.block(
            self.position.x.floor() as i32,
            (self.position.y + 0.1).floor() as i32,
            self.position.z.floor() as i32,
        );
        let fluid = facts.get(at).fluid;
        let fire_immune = items.resists_fire(&self.stack, "minecraft:in_fire");
        if fluid == Fluid::Water || fire_immune {
            self.fire_ticks = 0;
        }
        if self.fire_ticks > 0 {
            if self.fire_ticks % 20 == 0
                && fluid != Fluid::Lava
                && !items.resists_fire(&self.stack, "minecraft:on_fire")
            {
                self.health -= 1;
            }
            self.fire_ticks -= 1;
        }
        let mut lava_hurt = false;
        if fluid == Fluid::Lava && !fire_immune {
            self.fire_ticks = self.fire_ticks.max(300);
            if !items.resists_fire(&self.stack, "minecraft:lava") {
                self.health -= 4;
                lava_hurt = true;
            }
        }
        let fire_damage = crate::motion::environment::fire_contact(world, registry, self.bounds());
        if fire_damage > 0.0 && !fire_immune {
            self.fire_ticks = self.fire_ticks.max(160);
            if !items.resists_fire(&self.stack, "minecraft:in_fire") {
                self.health -= fire_damage as i32;
            }
        }
        ItemEnvironmentTick {
            alive: self.health > 0,
            lava_hurt,
        }
    }
    /// Returns false when the item should be removed.
    pub fn tick(
        &mut self,
        world: &World,
        facts: &BlockFacts,
        collision: &BlockCollisionRegistry<'_>,
    ) -> bool {
        self.previous = self.position;
        if self.pickup_delay > 0 && self.pickup_delay != u32::MAX {
            self.pickup_delay -= 1;
        }
        let eye = world.block(
            self.position.x.floor() as i32,
            (self.position.y + 0.1).floor() as i32,
            self.position.z.floor() as i32,
        );
        let in_water = facts.get(eye).fluid == Fluid::Water;
        if in_water {
            // ItemEntity.setUnderwaterMovement
            self.velocity = Vec3::new(
                self.velocity.x * 0.99,
                self.velocity.y + if self.velocity.y < 0.06 { 5.0e-4 } else { 0.0 },
                self.velocity.z * 0.99,
            );
        } else if facts.get(eye).fluid == Fluid::Lava {
            self.velocity = Vec3::new(
                self.velocity.x * 0.95,
                self.velocity.y + if self.velocity.y < 0.06 { 5.0e-4 } else { 0.0 },
                self.velocity.z * 0.95,
            );
        } else {
            self.velocity.y -= 0.04;
        }
        let horizontal = self.velocity.x * self.velocity.x + self.velocity.z * self.velocity.z;
        if !self.on_ground || horizontal > 1.0e-5 || (self.age + self.id).is_multiple_of(4) {
            let mut scene = SceneCollisions {
                world,
                binding: collision,
                subject: None,
            };
            let wanted = self.velocity;
            let moved = scene
                .resolve(self.bounds(), wanted, 0.0, self.on_ground)
                .unwrap_or(Vec3::ZERO);
            self.position = Vec3::new(
                self.position.x + moved.x,
                self.position.y + moved.y,
                self.position.z + moved.z,
            );
            self.on_ground = wanted.y < 0.0 && moved.y != wanted.y;
            if moved.x != wanted.x {
                self.velocity.x = 0.0;
            }
            if moved.z != wanted.z {
                self.velocity.z = 0.0;
            }
            if moved.y != wanted.y {
                self.velocity.y = 0.0;
            }
            let mut friction = 0.98;
            if self.on_ground {
                let below = world.block(
                    self.position.x.floor() as i32,
                    (self.position.y - 0.999_999).floor() as i32,
                    self.position.z.floor() as i32,
                );
                friction = f64::from(facts.get(below).friction) * 0.98;
            }
            self.velocity = Vec3::new(
                self.velocity.x * friction,
                self.velocity.y * 0.98,
                self.velocity.z * friction,
            );
            if self.on_ground && self.velocity.y < 0.0 {
                self.velocity.y *= -0.5;
            }
        }
        self.age += 1;
        self.age < 6000 && self.position.y > -128.0
    }
}
/// ExperienceOrb: gravity 0.03, homing towards the player within 8 blocks.
#[derive(Clone, Debug)]
pub struct ExperienceOrb {
    pub position: Vec3,
    pub previous: Vec3,
    pub velocity: Vec3,
    pub value: u32,
    pub age: u32,
    pub on_ground: bool,
}
impl ExperienceOrb {
    /// ExperienceOrb.getExperienceValue: the largest orb size not above `value`.
    pub fn split(value: u32) -> u32 {
        [2477, 1237, 617, 307, 149, 73, 37, 17, 7, 3, 1]
            .into_iter()
            .find(|&v| value >= v)
            .unwrap_or(1)
    }
    /// ExperienceOrb.getIcon
    pub fn icon(&self) -> u32 {
        match self.value {
            2477.. => 10,
            1237.. => 9,
            617.. => 8,
            307.. => 7,
            149.. => 6,
            73.. => 5,
            37.. => 4,
            17.. => 3,
            7.. => 2,
            3.. => 1,
            _ => 0,
        }
    }
    pub fn tick(
        &mut self,
        world: &World,
        facts: &BlockFacts,
        collision: &BlockCollisionRegistry<'_>,
        player: Option<[f64; 3]>,
    ) -> bool {
        self.previous = self.position;
        let block = world.block(
            self.position.x.floor() as i32,
            self.position.y.floor() as i32,
            self.position.z.floor() as i32,
        );
        if facts.get(block).fluid == Fluid::Water {
            self.velocity = Vec3::new(
                self.velocity.x * 0.99,
                (self.velocity.y + 5.0e-4).min(0.06),
                self.velocity.z * 0.99,
            );
        } else {
            self.velocity.y -= 0.03;
        }
        if let Some(p) = player {
            let d = [
                (p[0] - self.position.x) / 8.0,
                (p[1] - self.position.y) / 8.0,
                (p[2] - self.position.z) / 8.0,
            ];
            let length = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            let power = 1.0 - length;
            if power > 0.0 && length > 1e-6 {
                let power = power * power;
                self.velocity = Vec3::new(
                    self.velocity.x + d[0] / length * power * 0.1,
                    self.velocity.y + d[1] / length * power * 0.1,
                    self.velocity.z + d[2] / length * power * 0.1,
                );
            }
        }
        let h = 0.25;
        let bounds = Aabb::new(
            Vec3::new(self.position.x - h, self.position.y, self.position.z - h),
            Vec3::new(
                self.position.x + h,
                self.position.y + 0.5,
                self.position.z + h,
            ),
        );
        let mut scene = SceneCollisions {
            world,
            binding: collision,
            subject: None,
        };
        let wanted = self.velocity;
        let moved = scene
            .resolve(bounds, wanted, 0.0, self.on_ground)
            .unwrap_or(Vec3::ZERO);
        self.position = Vec3::new(
            self.position.x + moved.x,
            self.position.y + moved.y,
            self.position.z + moved.z,
        );
        self.on_ground = wanted.y < 0.0 && moved.y != wanted.y;
        let mut friction = 0.98;
        if self.on_ground {
            let below = world.block(
                self.position.x.floor() as i32,
                (self.position.y - 1.0).floor() as i32,
                self.position.z.floor() as i32,
            );
            friction = f64::from(facts.get(below).friction) * 0.98;
        }
        self.velocity = Vec3::new(
            self.velocity.x * friction,
            self.velocity.y * 0.98,
            self.velocity.z * friction,
        );
        if self.on_ground {
            self.velocity.y *= -0.9;
        }
        self.age += 1;
        self.age < 6000
    }
}
/// Player experience (Player.giveExperiencePoints / getXpNeededForNextLevel).
#[derive(Clone, Copy, Debug, Default)]
pub struct Experience {
    pub level: i32,
    pub progress: f32,
    pub total: i32,
}
impl Experience {
    /// `Player.getBaseExperienceReward`: keep-inventory and spectators drop none.
    pub fn death_reward(&self, keep: bool) -> u32 {
        if keep {
            0
        } else {
            self.level.wrapping_mul(7).clamp(0, 100) as u32
        }
    }
    /// `ServerPlayer.restoreFrom`: only keep-inventory and spectators retain XP.
    pub fn respawn(&mut self, keep: bool) {
        if !keep {
            *self = Self::default();
        }
    }
    pub fn needed(level: i32) -> i32 {
        if level >= 30 {
            112 + (level - 30) * 9
        } else if level >= 15 {
            37 + (level - 15) * 5
        } else {
            7 + level * 2
        }
    }
    /// Returns true when a level was gained.
    pub fn give(&mut self, points: i32) -> bool {
        let start = self.level;
        self.progress += points as f32 / Self::needed(self.level) as f32;
        self.total = self.total.saturating_add(points).max(0);
        while self.progress >= 1.0 {
            self.progress = (self.progress - 1.0) * Self::needed(self.level) as f32;
            self.level += 1;
            self.progress /= Self::needed(self.level) as f32;
        }
        self.level > start
    }
}
/// A painting (`Painting`, a `HangingEntity`): the block it hangs in, the
/// 2D data value of the direction it faces, and its variant.
#[derive(Clone, Debug)]
pub struct Painting {
    pub id: u32,
    pub pos: [i32; 3],
    /// Direction.get2DDataValue: south 0, west 1, north 2, east 3.
    pub facing: u8,
    pub variant: std::sync::Arc<str>,
    pub width: i32,
    pub height: i32,
    /// BlockAttachedEntity.checkInterval.
    pub check_interval: u32,
}
impl Painting {
    pub fn step(&self) -> [i32; 3] {
        [[0, 0, 1], [-1, 0, 0], [0, 0, -1], [1, 0, 0]][usize::from(self.facing & 3)]
    }
    /// Direction.getCounterClockWise (the painting's left as it faces out).
    pub fn left(&self) -> [i32; 3] {
        [[1, 0, 0], [0, 0, 1], [-1, 0, 0], [0, 0, -1]][usize::from(self.facing & 3)]
    }
    /// Painting.calculateBoundingBox: (min, max).
    pub fn bounds(&self) -> ([f64; 3], [f64; 3]) {
        let s = self.step().map(f64::from);
        let l = self.left().map(f64::from);
        let off = |n: i32| if n % 2 == 0 { 0.5 } else { 0.0 };
        let (h, v) = (off(self.width), off(self.height));
        let c = [
            f64::from(self.pos[0]) + 0.5 - s[0] * 0.46875 + l[0] * h,
            f64::from(self.pos[1]) + 0.5 + v,
            f64::from(self.pos[2]) + 0.5 - s[2] * 0.46875 + l[2] * h,
        ];
        let along_x = s[0] != 0.0;
        let size = [
            if along_x {
                0.0625
            } else {
                f64::from(self.width)
            },
            f64::from(self.height),
            if along_x {
                f64::from(self.width)
            } else {
                0.0625
            },
        ];
        (
            std::array::from_fn(|i| c[i] - size[i] / 2.0),
            std::array::from_fn(|i| c[i] + size[i] / 2.0),
        )
    }
}
/// An item frame or glow item frame (`ItemFrame`).
#[derive(Clone, Debug)]
pub struct ItemFrame {
    pub id: u32,
    pub pos: [i32; 3],
    /// Direction.get3DDataValue: down 0, up 1, north 2, south 3, west 4, east 5.
    pub facing: u8,
    pub glow: bool,
    pub item: Option<crate::items::ItemStack>,
    /// 0..8, in 45 degree steps.
    pub rotation: u8,
    pub drop_chance: f32,
    pub fixed: bool,
    pub invisible: bool,
    pub check_interval: u32,
}
impl ItemFrame {
    pub fn step(&self) -> [i32; 3] {
        [
            [0, -1, 0],
            [0, 1, 0],
            [0, 0, -1],
            [0, 0, 1],
            [-1, 0, 0],
            [1, 0, 0],
        ][usize::from(self.facing.min(5))]
    }
    pub fn kind(&self) -> &'static str {
        if self.glow {
            "minecraft:glow_item_frame"
        } else {
            "minecraft:item_frame"
        }
    }
    /// ItemFrame.createBoundingBox (without a framed map): (min, max).
    pub fn bounds(&self) -> ([f64; 3], [f64; 3]) {
        let s = self.step().map(f64::from);
        let c: [f64; 3] = std::array::from_fn(|i| f64::from(self.pos[i]) + 0.5 - s[i] * 0.46875);
        let size: [f64; 3] = std::array::from_fn(|i| if s[i] != 0.0 { 0.0625 } else { 0.75 });
        (
            std::array::from_fn(|i| c[i] - size[i] / 2.0),
            std::array::from_fn(|i| c[i] + size[i] / 2.0),
        )
    }
}
/// An armor stand (`ArmorStand`).
#[derive(Clone, Debug)]
pub struct ArmorStand {
    pub fire_ticks: i32,
    pub health: f32,
    pub id: u32,
    pub position: Vec3,
    pub velocity: Vec3,
    pub on_ground: bool,
    pub yaw: f32,
    /// mainhand, offhand, feet, legs, chest, head (EquipmentSlot order).
    pub equipment: [Option<crate::items::ItemStack>; 6],
    pub small: bool,
    pub show_arms: bool,
    pub no_base_plate: bool,
    pub invisible: bool,
    pub marker: bool,
    /// Game time of the last hit (the wobble and the second-hit window).
    pub last_hit: i64,
}
impl ArmorStand {
    pub const SLOTS: [&'static str; 6] = ["mainhand", "offhand", "feet", "legs", "chest", "head"];
    pub fn new(id: u32, position: Vec3, yaw: f32) -> Self {
        Self {
            fire_ticks: 0,
            health: 20.0,
            id,
            position,
            velocity: Vec3::ZERO,
            on_ground: true,
            yaw,
            equipment: Default::default(),
            small: false,
            show_arms: false,
            no_base_plate: false,
            invisible: false,
            marker: false,
            last_hit: i64::MIN / 2,
        }
    }
    pub fn environment_tick(
        &mut self,
        world: &World,
        facts: &BlockFacts,
        registry: &horizon_world::registry::BlockRegistry,
    ) -> bool {
        let (lo, hi) = (self.bounds().min_position(), self.bounds().max_position());
        let mut water = false;
        let mut lava = false;
        for x in lo.x.floor() as i32..=hi.x.floor() as i32 {
            for y in lo.y.floor() as i32..hi.y.ceil() as i32 {
                for z in lo.z.floor() as i32..=hi.z.floor() as i32 {
                    match facts.get(world.block(x, y, z)).fluid {
                        Fluid::Water => water = true,
                        Fluid::Lava => lava = true,
                        _ => {}
                    }
                }
            }
        }
        if water {
            self.fire_ticks = 0;
        }
        if self.fire_ticks > 0 {
            if self.fire_ticks % 20 == 0
                && !lava
                && !self.invisible
                && !self.marker
                && self.health > 0.5
            {
                self.health -= 4.0;
            }
            self.fire_ticks -= 1;
        }
        if lava {
            self.fire_ticks = self.fire_ticks.max(300);
        }
        let fire = crate::motion::environment::fire_contact(world, registry, self.bounds()) > 0.0;
        if fire && !self.invisible && !self.marker {
            if self.fire_ticks > 0 {
                self.health -= 0.15;
            } else {
                self.fire_ticks = 100;
            }
        }
        self.health > 0.5
    }
    /// EntityType.ARMOR_STAND dimensions (0.5 x 1.975, half for small).
    pub fn bounds(&self) -> Aabb {
        let s = if self.small { 0.5 } else { 1.0 };
        let (w, h) = (0.25 * s, 1.975 * s);
        let p = self.position;
        Aabb::new(
            Vec3::new(p.x - w, p.y, p.z - w),
            Vec3::new(p.x + w, p.y + h, p.z + w),
        )
    }
    /// LivingEntity.travel with gravity 0.08 and 0.98 drag, falling until it
    /// lands (armor stands have no other movement).
    pub fn tick(&mut self, scene: &mut crate::collisions::SceneCollisions) {
        if self.marker {
            return;
        }
        let mut v = self.velocity;
        v.y -= 0.08;
        let wanted = v;
        if let Ok(moved) = scene.resolve(self.bounds(), wanted, 0.0, self.on_ground) {
            self.position = Vec3::new(
                self.position.x + moved.x,
                self.position.y + moved.y,
                self.position.z + moved.z,
            );
            self.on_ground = wanted.y < 0.0 && moved.y > wanted.y;
            v = Vec3::new(0.0, if moved.y != wanted.y { 0.0 } else { v.y * 0.98 }, 0.0);
        }
        self.velocity = v;
    }
}

#[cfg(test)]
mod experience_tests {
    use super::*;

    #[test]
    fn death_reward_uses_levels_and_caps_the_recoverable_points() {
        for (level, reward) in [(0, 0), (1, 7), (14, 98), (15, 100), (30, 100)] {
            let xp = Experience {
                level,
                progress: 0.75,
                total: 10_000,
            };
            assert_eq!(xp.death_reward(false), reward);
            assert_eq!(xp.death_reward(true), 0);
        }
    }

    #[test]
    fn respawn_clears_all_xp_unless_it_is_kept() {
        let mut xp = Experience::default();
        xp.give(155);
        let before = (xp.level, xp.progress, xp.total);
        xp.respawn(true);
        assert_eq!((xp.level, xp.progress, xp.total), before);
        xp.respawn(false);
        assert_eq!((xp.level, xp.progress, xp.total), (0, 0.0, 0));
        xp.give(7);
        assert_eq!((xp.level, xp.progress, xp.total), (1, 0.0, 7));
    }
}

#[cfg(test)]
mod fire_environment_tests {
    use super::*;
    #[test]
    fn ordinary_items_burn_but_native_netherite_resists_lava() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client/26.3");
        if !root.join("items.json").exists() {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let facts = BlockFacts::load(&root, assets.registry).unwrap();
        let items = crate::items::Items::load(&root, assets.registry, &facts).unwrap();
        let mut world = World::test_scene(&assets, &render, 3).unwrap();
        let at = Vec3::new(20.5, 4.0, 20.5);
        world
            .set(
                20,
                4,
                20,
                assets.registry.default_state("minecraft:lava").unwrap(),
            )
            .unwrap();
        let make = |id: &str| ItemEntity {
            id: 1,
            thrower: None,
            stack: ItemStack {
                item: std::sync::Arc::from(id),
                count: 1,
                damage: 0,
                extra: None,
            },
            health: 5,
            fire_ticks: 0,
            position: at,
            previous: at,
            velocity: Vec3::ZERO,
            on_ground: false,
            age: 0,
            pickup_delay: 0,
            bob_offset: 0.0,
        };
        let mut wood = make("minecraft:oak_planks");
        let mut metal = make("minecraft:netherite_ingot");
        let first = wood.environment_tick(&world, &facts, assets.registry, &items);
        assert!(first.alive && first.lava_hurt);
        assert!(
            !first.should_play_lava_hurt_sound(1),
            "ordinary entry damage must not repeat the burn sound"
        );
        assert!(first.should_play_lava_hurt_sound(10));
        assert!(
            wood.fire_ticks > 0,
            "lava contact must ignite dropped items"
        );
        let last = wood.environment_tick(&world, &facts, assets.registry, &items);
        assert!(
            !last.alive && last.lava_hurt,
            "lethal lava damage still emits burn sound"
        );
        assert!(
            last.should_play_lava_hurt_sound(2),
            "the lethal hit must always play the sound"
        );
        for _ in 0..40 {
            let result = metal.environment_tick(&world, &facts, assets.registry, &items);
            assert!(result.alive && !result.lava_hurt);
            assert!(!result.should_play_lava_hurt_sound(10));
        }
        assert_eq!(metal.health, 5);
        assert_eq!(metal.fire_ticks, 0, "fire-resistant items must not ignite");
        world
            .set(
                20,
                4,
                20,
                assets.registry.default_state("minecraft:fire").unwrap(),
            )
            .unwrap();
        let mut burning = make("minecraft:oak_planks");
        let result = burning.environment_tick(&world, &facts, assets.registry, &items);
        assert!(result.alive && !result.lava_hurt);
        assert!(burning.fire_ticks > 0);
        world
            .set(
                20,
                4,
                20,
                assets
                    .registry
                    .default_state("minecraft:soul_fire")
                    .unwrap(),
            )
            .unwrap();
        let mut soul = make("minecraft:oak_planks");
        soul.environment_tick(&world, &facts, assets.registry, &items);
        assert_eq!(soul.health, 3);
        let mut stand = ArmorStand::new(2, at, 0.0);
        world
            .set(
                20,
                4,
                20,
                assets.registry.default_state("minecraft:lava").unwrap(),
            )
            .unwrap();
        for _ in 0..150 {
            assert!(stand.environment_tick(&world, &facts, assets.registry));
        }
        assert_eq!(stand.health, 20.0, "lava does not hurt armor stands");
    }
}
