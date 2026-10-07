//! The playable game loop: streaming world, player simulation at 20 ticks/s,
//! block interaction, inventory and HUD.
use horizon_client::{
    Result,
    assets::Assets,
    audio::Audio,
    biome::Biomes,
    blocks::{BlockFacts, Fluid},
    chunk::{HEIGHT, MIN_Y},
    clock::FixedStep,
    entities::ItemEntity,
    particles::Particle,
    crafting::Recipes,
    collisions,
    environment::Environment,
    gfx::{Frame, Renderer},
    hud::{HudAtlas, HudBatch, rgba},
    items::{ItemStack, Items},
    light::LightTables,
    mesher::{MeshTables, Vertex},
    models,
    motion::{MotionEvent, MotionResources, MotionWorld, PlayerMotion},
    sprint::Keys,
    streaming::{Streaming, WorkerContext},
    save::{Level, Persistent, SaveDir},
    testgen::TestTerrain,
    vanillagen::VanillaTerrain,
    world::World,
};
use horizon_core::{geometry::Vec3, random::RandomSource};
use horizon_world::{block_collision::BlockCollisionRegistry, chunk::BlockState, registry::BlockStateId};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    event::{DeviceEvent, DeviceId, ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{CursorGrabMode, Window, WindowId},
};

struct Options {
    resources: PathBuf,
    distance: i32,
    seed: i64,
    creative: bool,
    test_terrain: bool,
    frames: Option<u64>,
    screenshot_after: Option<u64>,
    timeout: Duration,
    report: Option<PathBuf>,
    world: String,
}
impl Options {
    fn read() -> Result<Option<Self>> {
        let sibling = std::env::current_exe()?
            .parent()
            .ok_or("missing executable directory")?
            .join("resources");
        let mut o = Self {
            resources: if sibling.join("horizon-resources.json").exists() {
                sibling
            } else {
                PathBuf::from(".local/horizon-client/26.3")
            },
            distance: 12,
            seed: 0,
            creative: false,
            test_terrain: false,
            frames: None,
            screenshot_after: None,
            timeout: Duration::from_secs(120),
            report: None,
            world: "New World".into(),
        };
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--help" | "-h" => {
                    println!(
                        "Horizon — Minecraft Java Edition 26.3 in Rust\n\n  --resources PATH        Prepared resources (tools/prepare_native_client.py)\n  --render-distance N     Chunks, 2..64 (default 12)\n  --world NAME            World folder under saves/ (default \"New World\")\n  --seed N                World seed for new worlds (default 0)\n  --creative              Creative mode (default survival)\n  --test-terrain          Simple test terrain instead of vanilla generation\n  --frames N              Exit after N frames (testing)\n  --smoke-report PATH     Write a JSON report on exit\n\nControls: mouse look, WASD move, Space jump, Left Shift sneak, Left Ctrl sprint,\nleft click break, right click place/use, 1-9/scroll hotbar, E inventory,\nQ drop, F3 debug, Esc pause."
                    );
                    return Ok(None);
                }
                "--creative" => o.creative = true,
                "--test-terrain" => o.test_terrain = true,
                _ => {
                    let value = args.next().ok_or_else(|| format!("missing value after {arg}"))?;
                    match arg.as_str() {
                        "--resources" => o.resources = value.into(),
                        "--render-distance" | "--radius" => {
                            o.distance = value.parse()?;
                            if !(2..=64).contains(&o.distance) {
                                return Err("render distance must be 2..64".into());
                            }
                        }
                        "--seed" => {
                            o.seed = value.parse().unwrap_or_else(|_| {
                                // Text seeds hash like String.hashCode.
                                value
                                    .encode_utf16()
                                    .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(i32::from(c)))
                                    .into()
                            })
                        }
                        "--frames" => o.frames = Some(value.parse()?),
                        "--timeout-seconds" => o.timeout = Duration::from_secs(value.parse()?),
                        "--smoke-report" => o.report = Some(value.into()),
                        "--screenshot-after" => o.screenshot_after = Some(value.parse()?),
                        "--world" => o.world = value,
                        _ => return Err(format!("unknown option {arg}").into()),
                    }
                }
            }
        }
        Ok(Some(o))
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Playing,
    Paused,
    Inventory,
    Dead,
}
#[derive(Clone)]
enum Icon {
    Flat(String),
    Cube { top: String, left: String, right: String, tint: u32 },
}
struct Mining {
    pos: [i32; 3],
    progress: f32,
    ticks: u32,
}
struct Hit {
    pos: [i32; 3],
    face: [i32; 3],
    point: [f64; 3],
}
pub struct Resources {
    assets: Arc<Assets>,
    facts: BlockFacts,
    items: Items,
    recipes: Recipes,
    biomes: Arc<Biomes>,
    environment: Environment,
    atlas: HudAtlas,
    icons: HashMap<String, Option<Icon>>,
    layer_names: HashMap<u32, String>,
}
struct Game<'a> {
    res: &'a mut Resources,
    collision: &'a BlockCollisionRegistry<'a>,
    motion: &'a MotionResources,
    world: World,
    streaming: Streaming,
    options: Options,
    player: Option<PlayerMotion>,
    spawn: Option<Vec3>,
    clock: FixedStep,
    previous_position: Vec3,
    eye: f32,
    previous_eye: f32,
    yaw: f32,
    pitch: f32,
    inventory: Vec<Option<ItemStack>>,
    selected: usize,
    carried: Option<ItemStack>,
    craft_grid: Vec<Option<ItemStack>>,
    craft_size: usize,
    mining: Option<Mining>,
    destroy_delay: u32,
    use_delay: u32,
    creative: bool,
    keys: HashSet<KeyCode>,
    left: bool,
    right: bool,
    cursor: (f32, f32),
    captured: bool,
    focused: bool,
    debug: bool,
    screen: Screen,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    started: Instant,
    previous: Instant,
    frames: u64,
    fps: f32,
    fps_frames: u64,
    fps_at: Instant,
    ticks: u64,
    error: Option<String>,
    message: Option<(String, Instant)>,
    save: Arc<SaveDir>,
    level: Level,
    audio: Audio,
    dropped: Vec<ItemEntity>,
    next_entity: u32,
    particles: Vec<Particle>,
    spawn_search: u32,
}
fn view_vector(yaw: f32, pitch: f32) -> [f64; 3] {
    let (y, p) = (f64::from(yaw).to_radians(), f64::from(pitch).to_radians());
    [-y.sin() * p.cos(), -p.sin(), y.cos() * p.cos()]
}
impl Game<'_> {
    fn eye_position(&self) -> Option<[f64; 3]> {
        let p = self.player.as_ref()?;
        Some([p.position.x, p.position.y + f64::from(p.dimensions.eye_height), p.position.z])
    }
    fn reach(&self) -> f64 {
        if self.creative { 5.0 } else { 4.5 }
    }
    /// Voxel traversal against non-air, non-fluid blocks (outline = full cube).
    fn pick(&self) -> Option<Hit> {
        let origin = self.eye_position()?;
        let dir = view_vector(self.yaw, self.pitch);
        let reach = self.reach();
        let mut pos = origin.map(|v| v.floor() as i32);
        let step = dir.map(|d| if d > 0.0 { 1 } else { -1 });
        let delta = dir.map(|d| if d == 0.0 { f64::INFINITY } else { (1.0 / d).abs() });
        let mut t_max = [0.0; 3];
        for i in 0..3 {
            let boundary = if dir[i] > 0.0 { f64::from(pos[i]) + 1.0 } else { f64::from(pos[i]) };
            t_max[i] = if dir[i] == 0.0 { f64::INFINITY } else { (boundary - origin[i]) / dir[i] };
        }
        let mut face = [0; 3];
        let mut t = 0.0;
        while t <= reach {
            let state = self.world.block(pos[0], pos[1], pos[2]);
            let f = self.res.facts.get(state);
            if !self.res.facts.is_air(state)
                && !(f.fluid != Fluid::None && !f.waterlogged && &*f.name != "minecraft:kelp" && f.render_shape != horizon_client::blocks::RenderShape::Model)
                && !matches!(&*f.name, "minecraft:water" | "minecraft:lava" | "minecraft:bubble_column")
            {
                return Some(Hit {
                    pos,
                    face,
                    point: [origin[0] + dir[0] * t, origin[1] + dir[1] * t, origin[2] + dir[2] * t],
                });
            }
            let axis = if t_max[0] < t_max[1] {
                if t_max[0] < t_max[2] { 0 } else { 2 }
            } else if t_max[1] < t_max[2] {
                1
            } else {
                2
            };
            t = t_max[axis];
            t_max[axis] += delta[axis];
            pos[axis] += step[axis];
            face = [0; 3];
            face[axis] = -step[axis];
        }
        None
    }
    fn held(&self) -> Option<&ItemStack> {
        self.inventory[self.selected].as_ref()
    }
    /// Inventory.add: returns the stack that did not fit.
    fn give(&mut self, mut stack: ItemStack) -> Option<ItemStack> {
        let max = self.res.items.get(&stack.item).map_or(64, |i| i.max_stack);
        // Prefer the selected slot, then the hotbar, then the main inventory.
        let order: Vec<usize> = std::iter::once(self.selected).chain((0..36).filter(|&i| i != self.selected)).collect();
        for &i in &order {
            if let Some(slot) = &mut self.inventory[i]
                && slot.item == stack.item
                && slot.damage == stack.damage
                && slot.count < max
            {
                let moved = (max - slot.count).min(stack.count);
                slot.count += moved;
                stack.count -= moved;
                if stack.count == 0 {
                    return None;
                }
            }
        }
        for i in 0..36 {
            if self.inventory[i].is_none() {
                let moved = stack.count.min(max);
                self.inventory[i] = Some(ItemStack { count: moved, ..stack.clone() });
                stack.count -= moved;
                if stack.count == 0 {
                    return None;
                }
            }
        }
        Some(stack)
    }
    /// The block's particle texture (approximated by its first model face) and tint.
    fn particle_sprite(&self, state: BlockStateId) -> Option<(String, u32)> {
        let models = self.res.assets.model_catalog.collect(state, horizon_core::position::BlockPos::new(0, 0, 0)).ok()?;
        let face = models.iter().flat_map(|(_, m)| m.faces.iter()).find(|f| f.quad.facing == horizon_assets::model::Direction::Up)
            .or_else(|| models.iter().flat_map(|(_, m)| m.faces.iter()).next())?;
        let name = self.res.layer_names.get(&face.layer)?.clone();
        let tint = if face.quad.tint_index >= 0 {
            let n = &self.res.facts.get(state).name;
            if n.contains("leaves") || n.contains("vine") { 0x48B518 } else { 0x91BD59 }
        } else {
            0xFFFFFF
        };
        Some((name, tint))
    }
    /// ParticleEngine.destroy: a 4x4x4 burst.
    fn destroy_particles(&mut self, pos: [i32; 3], state: BlockStateId) {
        let Some((sprite, tint)) = self.particle_sprite(state) else { return };
        for x in 0..4 {
            for y in 0..4 {
                for z in 0..4 {
                    let d = |i: i32| (f64::from(i) + 0.5) / 4.0;
                    let p = [f64::from(pos[0]) + d(x), f64::from(pos[1]) + d(y), f64::from(pos[2]) + d(z)];
                    let v = [d(x) - 0.5, d(y) - 0.5, d(z) - 0.5];
                    let particle = Particle::terrain(&mut self.world.random, p, v, sprite.clone(), tint);
                    self.particles.push(particle);
                }
            }
        }
    }
    /// ParticleEngine.crack: one crumb on the face being mined.
    fn crack_particle(&mut self, pos: [i32; 3], face: [i32; 3], state: BlockStateId) {
        let Some((sprite, tint)) = self.particle_sprite(state) else { return };
        let r = &mut self.world.random;
        let mut p = [0.0; 3];
        for i in 0..3 {
            let base = f64::from(pos[i]);
            p[i] = match face[i] {
                1 => base + 1.0 + 0.1,
                -1 => base - 0.1,
                _ => base + 0.1 + r.next_double() * 0.8,
            };
        }
        let mut particle = Particle::terrain(r, p, [0.0; 3], sprite, tint);
        for c in &mut particle.velocity {
            *c *= 0.2;
        }
        particle.velocity[1] = (particle.velocity[1] - 0.1) * 0.2 + 0.1;
        particle.size *= 0.6;
        self.particles.push(particle);
    }
    fn spawn_item(&mut self, stack: ItemStack, position: [f64; 3], velocity: [f64; 3], pickup_delay: u32) {
        self.next_entity += 1;
        let p = Vec3::new(position[0], position[1], position[2]);
        let bob = self.world.random.next_float() * std::f32::consts::PI * 2.0;
        self.dropped.push(ItemEntity {
            id: self.next_entity,
            stack,
            position: p,
            previous: p,
            velocity: Vec3::new(velocity[0], velocity[1], velocity[2]),
            on_ground: false,
            age: 0,
            pickup_delay,
            bob_offset: bob,
        });
    }
    /// Block.popResource
    fn pop_resource(&mut self, pos: [i32; 3], stack: ItemStack) {
        let r = &mut self.world.random;
        let mut offset = || f64::from(r.next_float()) * 0.5 - 0.25;
        let p = [
            f64::from(pos[0]) + 0.5 + offset(),
            f64::from(pos[1]) + 0.5 + offset() - 0.125,
            f64::from(pos[2]) + 0.5 + offset(),
        ];
        let v = [r.next_double() * 0.2 - 0.1, 0.2, r.next_double() * 0.2 - 0.1];
        self.spawn_item(stack, p, v, 10);
    }
    /// Player.drop: thrown from the eyes along the view direction.
    fn drop_held(&mut self, all: bool) {
        let Some(eye) = self.eye_position() else { return };
        let Some(stack) = self.inventory[self.selected].as_mut() else { return };
        let count = if all { stack.count } else { 1 };
        let thrown = ItemStack { count, ..stack.clone() };
        stack.count -= count;
        if stack.count == 0 {
            self.inventory[self.selected] = None;
        }
        let (yaw, pitch) = (f64::from(self.yaw).to_radians(), f64::from(self.pitch).to_radians());
        let r = &mut self.world.random;
        let spread = f64::from(r.next_float()) * std::f64::consts::TAU;
        let strength = 0.02 * f64::from(r.next_float());
        let v = [
            -yaw.sin() * pitch.cos() * 0.3 + spread.cos() * strength,
            -pitch.sin() * 0.3 + 0.1 + f64::from(r.next_float() - r.next_float()) * 0.1,
            yaw.cos() * pitch.cos() * 0.3 + spread.sin() * strength,
        ];
        self.spawn_item(thrown, [eye[0], eye[1] - 0.3, eye[2]], v, 40);
    }
    fn tick_items(&mut self) {
        let mut items = std::mem::take(&mut self.dropped);
        items.retain_mut(|item| item.tick(&self.world, &self.res.facts, self.collision));
        let Some(p) = self.player.as_ref() else {
            self.dropped = items;
            return;
        };
        if p.health.dead_or_dying() {
            self.dropped = items;
            return;
        }
        let w = f64::from(p.dimensions.width) / 2.0;
        let (min, max) = (
            [p.position.x - w - 1.0, p.position.y - 0.5, p.position.z - w - 1.0],
            [p.position.x + w + 1.0, p.position.y + f64::from(p.dimensions.height) + 0.5, p.position.z + w + 1.0],
        );
        let feet = [p.position.x, p.position.y, p.position.z];
        let mut kept = Vec::with_capacity(items.len());
        for mut item in items {
            let b = item.bounds();
            let touching = b.min_position().x < max[0] && b.max_position().x > min[0]
                && b.min_position().y < max[1] && b.max_position().y > min[1]
                && b.min_position().z < max[2] && b.max_position().z > min[2];
            if touching && item.pickup_delay == 0 {
                let before = item.stack.count;
                match self.give(item.stack.clone()) {
                    None => {
                        let pitch = ((self.world.random.next_float() - self.world.random.next_float()) * 0.7 + 1.0) * 2.0;
                        self.audio.play("minecraft:entity.item.pickup", Some(feet), 0.2, pitch);
                        continue;
                    }
                    Some(rest) => {
                        if rest.count < before {
                            self.audio.play("minecraft:entity.item.pickup", Some(feet), 0.2, 1.0);
                        }
                        item.stack = rest;
                    }
                }
            }
            kept.push(item);
        }
        self.dropped = kept;
    }
    fn break_block(&mut self, pos: [i32; 3]) -> Result<()> {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let facts = self.res.facts.get(state).clone();
        if !self.creative {
            let held = self.held().cloned();
            let harvest = !facts.requires_tool
                || self.res.items.correct_for_drops(held.as_ref(), &facts.name);
            if harvest {
                let drops = self.res.items.block_drops(
                    &self.res.assets.registry,
                    state,
                    held.as_ref(),
                    &mut self.world.random,
                );
                for d in drops {
                    if &*d.item != "minecraft:air" {
                        self.pop_resource(pos, d);
                    }
                }
            }
            // Tools lose durability on blocks with nonzero hardness.
            if facts.hardness > 0.0
                && let Some(stack) = self.inventory[self.selected].as_mut()
                && let Some(info) = self.res.items.get(&stack.item)
                && info.max_damage > 0
                && !info.tool.is_empty()
            {
                stack.damage += 1;
                if stack.damage >= info.max_damage {
                    self.inventory[self.selected] = None;
                }
            }
        }
        let replacement = if facts.waterlogged {
            self.res.assets.registry.default_state("minecraft:water")
        } else {
            None
        }
        .unwrap_or(self.world.air);
        self.world.set(pos[0], pos[1], pos[2], replacement)?;
        self.destroy_particles(pos, state);
        let center = [f64::from(pos[0]) + 0.5, f64::from(pos[1]) + 0.5, f64::from(pos[2]) + 0.5];
        self.audio.play(&facts.sounds.break_sound, Some(center), (facts.sounds.volume + 1.0) / 2.0, facts.sounds.pitch * 0.8);
        // Two-block plants and doors break their other half.
        if let Some(props) = self.res.assets.registry.state(state).map(|s| s.properties.clone())
            && let Some(half) = props.get("half")
        {
            let other = match half.as_str() {
                "upper" => Some(pos[1] - 1),
                "lower" => Some(pos[1] + 1),
                _ => None,
            };
            if let Some(y) = other {
                let o = self.world.block(pos[0], y, pos[2]);
                if self.res.facts.get(o).name == facts.name {
                    self.world.set(pos[0], y, pos[2], self.world.air)?;
                }
            }
        }
        // Unsupported plants above fall (snow, flowers, grass): simple support rule.
        let above = self.world.block(pos[0], pos[1] + 1, pos[2]);
        let af = self.res.facts.get(above);
        if !self.res.facts.is_air(above) && af.hardness == 0.0 && !af.full_collision && af.fluid == Fluid::None {
            self.break_block([pos[0], pos[1] + 1, pos[2]])?;
        }
        Ok(())
    }
    /// `BlockItem.getPlacementState` for the common placement properties.
    fn placement_state(&self, block: BlockStateId, hit: &Hit) -> BlockStateId {
        let registry = &self.res.assets.registry;
        let Some(state) = registry.state(block) else { return block };
        let mut s = state.clone();
        let yaw = self.yaw.rem_euclid(360.0);
        let facing = match ((yaw + 45.0) / 90.0) as i32 % 4 {
            0 => "south",
            1 => "west",
            2 => "north",
            _ => "east",
        };
        let opposite = |d: &str| match d {
            "north" => "south",
            "south" => "north",
            "east" => "west",
            "west" => "east",
            "up" => "down",
            _ => "up",
        };
        let clicked = match hit.face {
            [0, 1, 0] => "up",
            [0, -1, 0] => "down",
            [0, 0, -1] => "north",
            [0, 0, 1] => "south",
            [-1, 0, 0] => "west",
            _ => "east",
        };
        let frac_y = hit.point[1] - hit.point[1].floor();
        let top_half = hit.face == [0, -1, 0] || (hit.face[1] == 0 && frac_y > 0.5);
        let name = s.name.clone();
        let props = &mut s.properties;
        if props.contains_key("axis") {
            let axis = match hit.face {
                [_, 0, 0] if hit.face[0] != 0 => "x",
                [0, _, 0] if hit.face[1] != 0 => "y",
                _ => "z",
            };
            if props.get("axis").is_some() {
                props.insert("axis".into(), axis.into());
            }
        }
        if let Some(f) = props.get("facing").cloned() {
            let wants = if name.ends_with("_stairs") || name.contains("fence_gate") || name.ends_with("_door") || name.ends_with("_trapdoor") {
                facing.to_owned()
            } else if name.ends_with("torch") || name.ends_with("ladder") || name.contains("wall_") {
                clicked.to_owned()
            } else if name.contains("piston") || name.contains("observer") || name.contains("dispenser") || name.contains("dropper") || name.ends_with("rod") || name.contains("amethyst") {
                clicked.to_owned()
            } else {
                opposite(facing).to_owned()
            };
            props.insert("facing".into(), wants);
            let mut probe = BlockState { name: name.clone(), properties: props.clone() };
            if registry.resolve(&probe).is_err() {
                probe.properties.insert("facing".into(), f);
                *props = probe.properties;
            }
        }
        if name.ends_with("_slab") {
            props.insert("type".into(), if top_half { "top" } else { "bottom" }.into());
        }
        if (name.ends_with("_stairs") || name.ends_with("_trapdoor")) && props.contains_key("half") {
            props.insert("half".into(), if top_half { "top" } else { "bottom" }.into());
        }
        // Torches on walls become wall torches.
        if name.ends_with("torch") && hit.face[1] == 0 {
            let wall = name.replace("torch", "wall_torch");
            if let Some(wall_state) = registry.default_state(&wall)
                && let Some(ws) = registry.state(wall_state)
            {
                let mut w = ws.clone();
                w.properties.insert("facing".into(), clicked.into());
                if let Ok(id) = registry.resolve(&w) {
                    return id;
                }
            }
        }
        if let Some(w) = props.get_mut("waterlogged") {
            let target = [hit.pos[0] + hit.face[0], hit.pos[1] + hit.face[1], hit.pos[2] + hit.face[2]];
            let existing = self.world.block(target[0], target[1], target[2]);
            let f = self.res.facts.get(existing);
            *w = (f.fluid == Fluid::Water && f.fluid_level == 8 && !f.fluid_falling).to_string();
        }
        registry.resolve(&s).unwrap_or(block)
    }
    fn use_item(&mut self) -> Result<()> {
        let sneaking = self.keys.contains(&KeyCode::ShiftLeft);
        if !sneaking
            && let Some(hit) = self.pick()
        {
            let clicked = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
            if &*self.res.facts.get(clicked).name == "minecraft:crafting_table" {
                self.craft_size = 3;
                self.screen = Screen::Inventory;
                self.capture(false);
                self.right = false;
                return Ok(());
            }
        }
        let Some(stack) = self.held().cloned() else { return Ok(()) };
        let Some(info) = self.res.items.get(&stack.item).cloned() else { return Ok(()) };
        if let Some((nutrition, saturation)) = info.food
            && let Some(p) = self.player.as_mut()
            && (p.health.food.food < 20 || self.creative)
        {
            let food = &mut p.health.food;
            food.food = (food.food + nutrition as i32).min(20);
            food.saturation = (food.saturation + saturation).min(food.food as f32);
            self.consume_held();
            return Ok(());
        }
        let Some(hit) = self.pick() else { return Ok(()) };
        let Some(block) = info.block else { return Ok(()) };
        let clicked_state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        let target = if self.res.facts.get(clicked_state).replaceable {
            hit.pos
        } else {
            [hit.pos[0] + hit.face[0], hit.pos[1] + hit.face[1], hit.pos[2] + hit.face[2]]
        };
        if target[1] < MIN_Y || target[1] >= MIN_Y + HEIGHT || !self.world.loaded(target[0], target[2]) {
            return Ok(());
        }
        let existing = self.world.block(target[0], target[1], target[2]);
        if !self.res.facts.get(existing).replaceable {
            return Ok(());
        }
        let state = self.placement_state(block, &hit);
        // Do not place collidable blocks inside the player.
        if self.res.facts.get(state).full_collision
            && let Some(p) = &self.player
        {
            let w = f64::from(p.dimensions.width) / 2.0;
            let (min, max) = (
                [p.position.x - w, p.position.y, p.position.z - w],
                [p.position.x + w, p.position.y + f64::from(p.dimensions.height), p.position.z + w],
            );
            let overlaps = (0..3).all(|i| {
                f64::from(target[i]) < max[i] - 1e-7 && f64::from(target[i]) + 1.0 > min[i] + 1e-7
            });
            if overlaps {
                return Ok(());
            }
        }
        self.world.set(target[0], target[1], target[2], state)?;
        let sounds = self.res.facts.get(state).sounds.clone();
        let center = [f64::from(target[0]) + 0.5, f64::from(target[1]) + 0.5, f64::from(target[2]) + 0.5];
        self.audio.play(&sounds.place, Some(center), (sounds.volume + 1.0) / 2.0, sounds.pitch * 0.8);
        // Two-block items place their upper half.
        if let Some(s) = self.res.assets.registry.state(state)
            && s.properties.get("half").map(String::as_str) == Some("lower")
        {
            let mut upper = s.clone();
            upper.properties.insert("half".into(), "upper".into());
            if let Ok(u) = self.res.assets.registry.resolve(&upper)
                && self.res.facts.get(self.world.block(target[0], target[1] + 1, target[2])).replaceable
            {
                self.world.set(target[0], target[1] + 1, target[2], u)?;
            }
        }
        self.consume_held();
        Ok(())
    }
    fn consume_held(&mut self) {
        if self.creative {
            return;
        }
        if let Some(s) = self.inventory[self.selected].as_mut() {
            s.count -= 1;
            if s.count == 0 {
                self.inventory[self.selected] = None;
            }
        }
    }
    fn tick_mining(&mut self) -> Result<()> {
        if self.destroy_delay > 0 {
            self.destroy_delay -= 1;
        }
        if !self.left || self.screen != Screen::Playing {
            self.mining = None;
            return Ok(());
        }
        let Some(hit) = self.pick() else {
            self.mining = None;
            return Ok(());
        };
        if self.creative {
            if self.destroy_delay == 0 {
                self.break_block(hit.pos)?;
                self.destroy_delay = 5;
            }
            return Ok(());
        }
        if self.destroy_delay > 0 {
            return Ok(());
        }
        if self.mining.as_ref().is_none_or(|m| m.pos != hit.pos) {
            self.mining = Some(Mining { pos: hit.pos, progress: 0.0, ticks: 0 });
        }
        let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        let facts = self.res.facts.get(state);
        if facts.hardness < 0.0 {
            return Ok(());
        }
        // Player.getDestroySpeed / BlockBehaviour.getDestroyProgress.
        let held = self.held();
        let mut speed = self.res.items.mining_speed(held, &facts.name);
        let player = self.player.as_ref().ok_or("no player")?;
        if !player.ground.on_ground {
            speed /= 5.0;
        }
        let eye = self.eye_position().unwrap_or_default();
        if self.res.facts.get(self.world.block(eye[0].floor() as i32, eye[1].floor() as i32, eye[2].floor() as i32)).fluid == Fluid::Water {
            speed /= 5.0;
        }
        let harvest = !facts.requires_tool || self.res.items.correct_for_drops(held, &facts.name);
        let progress = if facts.hardness == 0.0 {
            1.0
        } else {
            speed / facts.hardness / if harvest { 30.0 } else { 100.0 }
        };
        let hit_sound = facts.sounds.clone();
        let m = self.mining.as_mut().ok_or("mining")?;
        m.progress += progress;
        m.ticks += 1;
        let crack = (m.pos, hit.face);
        if m.ticks % 4 == 1 {
            let c = [f64::from(m.pos[0]) + 0.5, f64::from(m.pos[1]) + 0.5, f64::from(m.pos[2]) + 0.5];
            self.audio.play(&hit_sound.hit, Some(c), (hit_sound.volume + 1.0) / 8.0, hit_sound.pitch * 0.5);
        }
        self.crack_particle(crack.0, crack.1, state);
        let m = self.mining.as_mut().ok_or("mining")?;
        if m.progress >= 1.0 {
            let pos = m.pos;
            self.mining = None;
            self.break_block(pos)?;
            self.destroy_delay = 5;
        }
        Ok(())
    }
    /// The i-th world spawn search point: a square spiral in 48-block steps.
    fn spawn_candidate(i: u32) -> (i32, i32) {
        if i == 0 {
            return (0, 0);
        }
        let mut k = 1u32;
        let mut ring = 1i32;
        loop {
            let points = 8 * ring as u32;
            if i < k + points {
                let j = (i - k) as i32;
                let side = j / (2 * ring);
                let t = j % (2 * ring) - ring;
                let (x, z) = match side {
                    0 => (t, -ring),
                    1 => (ring, t),
                    2 => (-t, ring),
                    _ => (-ring, -t),
                };
                return (x * 48, z * 48);
            }
            k += points;
            ring += 1;
        }
    }
    /// A column whose surface is a full solid, non-fluid, non-leaf block open to the sky.
    fn land_column(&self, x: i32, z: i32) -> Option<i32> {
        let mut y = MIN_Y + HEIGHT - 1;
        while y > MIN_Y {
            let s = self.world.block(x, y, z);
            if !self.res.facts.is_air(s) {
                let f = self.res.facts.get(s);
                let ok = f.full_collision && f.fluid == Fluid::None && !f.name.contains("leaves") && f.hardness >= 0.0;
                return ok.then_some(y + 1);
            }
            y -= 1;
        }
        None
    }
    fn spawn_player(&mut self) -> Result<()> {
        if self.player.is_some() {
            return Ok(());
        }
        let saved = self.level.position.map(|p| Vec3::new(p[0], p[1], p[2]));
        if self.spawn.is_none() {
            self.spawn = self.level.spawn.map(|s| Vec3::new(s[0] + 0.5, s[1], s[2] + 0.5));
        }
        let anchor = match saved.or(self.spawn) {
            Some(p) => (p.x.floor() as i32, p.z.floor() as i32),
            None => Self::spawn_candidate(self.spawn_search),
        };
        let (cx, cz) = (anchor.0 >> 4, anchor.1 >> 4);
        self.streaming.center = (cx, cz);
        if !(-1..=1).all(|dz| (-1..=1).all(|dx| self.streaming.is_lit(cx + dx, cz + dz))) {
            return Ok(());
        }
        let feet = if let Some(p) = saved {
            self.level.position = None;
            p
        } else if let Some(s) = self.spawn {
            // WorldSpawn: the stored column, lifted onto its current surface.
            let y = self.land_column(s.x.floor() as i32, s.z.floor() as i32).map_or(s.y, f64::from);
            Vec3::new(s.x, y.max(s.y), s.z)
        } else {
            let mut best: Option<(i64, i32, i32, i32)> = None;
            for z in (cz - 1) * 16..(cz + 2) * 16 {
                for x in (cx - 1) * 16..(cx + 2) * 16 {
                    if let Some(y) = self.land_column(x, z) {
                        let d = i64::from(x - anchor.0).pow(2) + i64::from(z - anchor.1).pow(2);
                        if best.is_none_or(|b| d < b.0) {
                            best = Some((d, x, y, z));
                        }
                    }
                }
            }
            match best {
                Some((_, x, y, z)) => {
                    let spawn = Vec3::new(f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5);
                    self.spawn = Some(spawn);
                    self.level.spawn = Some([f64::from(x), f64::from(y), f64::from(z)]);
                    spawn
                }
                None => {
                    self.spawn_search += 1;
                    return Ok(());
                }
            }
        };
        let mut player = PlayerMotion::new(self.motion, feet)?;
        player.yaw = self.yaw;
        if self.level.health > 0.0 && self.ticks < 2 {
            player.set_health(self.level.health, false)?;
            player.health.food.food = self.level.food;
            player.health.food.saturation = self.level.saturation;
        }
        self.previous_position = feet;
        self.player = Some(player);
        self.eye = 1.62;
        self.previous_eye = 1.62;
        self.clock.reset();
        Ok(())
    }
    fn tick(&mut self) -> Result<()> {
        self.ticks += 1;
        self.world.game_time += 1;
        if self.ticks % 6000 == 0 {
            self.save_game()?;
        }
        self.world.day_time += 1;
        if self.use_delay > 0 {
            self.use_delay -= 1;
        }
        let Some(player) = self.player.as_mut() else { return Ok(()) };
        // Hold position until the surrounding terrain exists.
        let (cx, cz) = ((player.position.x.floor() as i32) >> 4, (player.position.z.floor() as i32) >> 4);
        if !self.streaming.is_lit(cx, cz) {
            return Ok(());
        }
        let playing = self.screen == Screen::Playing;
        let held = |k| playing && self.keys.contains(&k);
        let keys = Keys {
            forward: held(KeyCode::KeyW),
            backward: held(KeyCode::KeyS),
            left: held(KeyCode::KeyA),
            right: held(KeyCode::KeyD),
            jump: held(KeyCode::Space),
            shift: held(KeyCode::ShiftLeft),
            sprint: held(KeyCode::ControlLeft),
        };
        self.previous_position = player.position;
        player.yaw = self.yaw;
        MotionWorld {
            player,
            scene: &mut self.world,
            assets: &self.res.assets,
            collision_binding: self.collision,
            resources: self.motion,
        }
        .controlled_simulation_tick(keys)?;
        let events = std::mem::take(&mut player.events);
        let feet = [player.position.x, player.position.y, player.position.z];
        self.previous_eye = self.eye;
        self.eye += (player.dimensions.eye_height - self.eye) * 0.5;
        let crouching = player.pose == horizon_client::pose::Pose::Crouching;
        for event in events {
            match event {
                MotionEvent::Emission { position, state, .. } if !crouching => {
                    // Entity.playStepSound: the block's step sound at 0.15 volume.
                    let sounds = self.res.facts.get(state).sounds.clone();
                    self.audio.play(&sounds.step, Some(feet), sounds.volume * 0.15, sounds.pitch);
                    let _ = position;
                }
                MotionEvent::Fall(horizon_client::landing::FallEvent::Sound { sound, volume, pitch, .. }) => {
                    let name = match sound {
                        horizon_client::landing::SceneSound::BigFall => "minecraft:entity.player.big_fall",
                        horizon_client::landing::SceneSound::SmallFall => "minecraft:entity.player.small_fall",
                        _ => continue,
                    };
                    self.audio.play(name, Some(feet), volume, pitch);
                }
                MotionEvent::Fall(horizon_client::landing::FallEvent::Damage { .. }) => {
                    self.audio.play("minecraft:entity.player.hurt", Some(feet), 1.0, 1.0);
                }
                _ => {}
            }
        }
        let Some(player) = self.player.as_mut() else { return Ok(()) };
        if player.position.y < f64::from(MIN_Y - 64) {
            // Out of the world.
            let _ = player.set_health(0.0, false);
        }
        if player.health.dead_or_dying() && self.screen != Screen::Dead {
            self.screen = Screen::Dead;
            self.capture(false);
        }
        self.tick_mining()?;
        self.tick_items();
        let facts = &self.res.facts;
        let world = &self.world;
        self.particles.retain_mut(|p| p.tick(world, |id| facts.states[id as usize].full_collision));
        if self.particles.len() > 4000 {
            let excess = self.particles.len() - 4000;
            self.particles.drain(..excess);
        }
        if self.right && self.use_delay == 0 && self.screen == Screen::Playing {
            self.use_item()?;
            self.use_delay = 4;
        }
        Ok(())
    }
    fn capture(&mut self, capture: bool) {
        let Some(window) = &self.window else { return };
        if capture {
            self.captured = window
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| window.set_cursor_grab(CursorGrabMode::Confined))
                .is_ok();
        } else {
            let _ = window.set_cursor_grab(CursorGrabMode::None);
            self.captured = false;
        }
        window.set_cursor_visible(!self.captured);
    }
    fn icon(&mut self, item: &str) -> Option<Icon> {
        if let Some(icon) = self.res.icons.get(item) {
            return icon.clone();
        }
        let icon = self.resolve_icon(item);
        self.res.icons.insert(item.to_owned(), icon.clone());
        icon
    }
    fn resolve_icon(&self, item: &str) -> Option<Icon> {
        let root = &self.options.resources;
        let name = item.trim_start_matches("minecraft:");
        let info = self.res.items.get(item);
        let model_name = info.map_or(name.to_owned(), |i| i.model.trim_start_matches("minecraft:").to_owned());
        let def: serde_json::Value = serde_json::from_slice(
            &std::fs::read(root.join(format!("assets/minecraft/items/{model_name}.json"))).ok()?,
        )
        .ok()?;
        // Find the first plain model reference in the item model definition.
        fn first_model(v: &serde_json::Value) -> Option<String> {
            match v {
                serde_json::Value::Object(o) => {
                    if o.get("type").and_then(|t| t.as_str()) == Some("minecraft:model") {
                        return o.get("model").and_then(|m| m.as_str()).map(str::to_owned);
                    }
                    for key in ["model", "fallback", "on_false", "on_true", "cases", "entries", "tints"] {
                        if let Some(m) = o.get(key).and_then(first_model) {
                            return Some(m);
                        }
                    }
                    o.values().find_map(first_model)
                }
                serde_json::Value::Array(a) => a.iter().find_map(first_model),
                _ => None,
            }
        }
        let model = first_model(&def["model"])?;
        let path = model.trim_start_matches("minecraft:").to_owned();
        let mut current = path.clone();
        let mut textures = serde_json::Map::new();
        for _ in 0..16 {
            let text = std::fs::read(root.join(format!("assets/minecraft/models/{current}.json"))).ok()?;
            let m: serde_json::Value = serde_json::from_slice(&text).ok()?;
            if let Some(t) = m["textures"].as_object() {
                for (k, v) in t {
                    textures.entry(k.clone()).or_insert(v.clone());
                }
            }
            match m["parent"].as_str() {
                Some(p) if p.ends_with("item/generated") || p.ends_with("item/handheld") || p.contains("handheld") => {
                    let layer = textures.get("layer0")?.as_str()?;
                    return Some(Icon::Flat(layer.trim_start_matches("minecraft:").to_owned()));
                }
                Some(p) => current = p.trim_start_matches("minecraft:").to_owned(),
                None => break,
            }
        }
        if path.starts_with("block/") {
            let block = self.res.assets.registry.default_state(&format!("minecraft:{name}"))
                .or_else(|| info.and_then(|i| i.block))?;
            let models = self.res.assets.model_catalog.collect(block, horizon_core::position::BlockPos::new(0, 0, 0)).ok()?;
            let mut faces: HashMap<usize, (u32, i32)> = HashMap::new();
            for (_, m) in &models {
                for f in &m.faces {
                    faces.entry(f.quad.facing as usize).or_insert((f.layer, f.quad.tint_index));
                }
            }
            use horizon_assets::model::Direction as D;
            let get = |d: D| faces.get(&(d as usize)).and_then(|(l, _)| self.res.layer_names.get(l)).cloned();
            let top = get(D::Up)?;
            let tinted = faces.get(&(D::Up as usize)).is_some_and(|(_, t)| *t >= 0);
            let tint = if tinted {
                if name.contains("leaves") || name == "vine" { 0x48B518 } else { 0x91BD59 }
            } else {
                0xFFFFFF
            };
            return Some(Icon::Cube {
                left: get(D::South).unwrap_or(top.clone()),
                right: get(D::East).unwrap_or(top.clone()),
                top,
                tint,
            });
        }
        textures
            .get("layer0")
            .or_else(|| textures.get("all"))
            .or_else(|| textures.get("texture"))
            .and_then(|t| t.as_str())
            .map(|t| Icon::Flat(t.trim_start_matches("minecraft:").to_owned()))
    }
    fn draw_item(&mut self, batch: &mut HudBatch, stack: &ItemStack, x: f32, y: f32) {
        let icon = self.icon(&stack.item);
        let atlas = &self.res.atlas;
        match icon {
            Some(Icon::Flat(name)) => batch.sprite_scaled(atlas, &name, x, y, 16.0, 16.0, u32::MAX),
            Some(Icon::Cube { top, left, right, tint }) => {
                let color = |b: f32| {
                    let c = |s: u32| (((tint >> s) & 255) as f32 * b) as u8;
                    rgba(c(16), c(8), c(0), 255)
                };
                let (cx, top_y) = (x + 8.0, y + 1.0);
                let quad = |batch: &mut HudBatch, name: &str, p: [[f32; 2]; 4], c: u32| {
                    if let Some(r) = atlas.sprites.get(name).copied() {
                        batch.quad(p, [[r.u0, r.v0], [r.u1, r.v0], [r.u1, r.v1], [r.u0, r.v1]], c);
                    }
                };
                let (w, h) = (7.0, 3.6);
                quad(batch, &top, [[cx, top_y], [cx + w, top_y + h], [cx, top_y + 2.0 * h], [cx - w, top_y + h]], color(1.0));
                let side = 8.0;
                quad(batch, &left, [[cx - w, top_y + h], [cx, top_y + 2.0 * h], [cx, top_y + 2.0 * h + side], [cx - w, top_y + h + side]], color(0.8));
                quad(batch, &right, [[cx, top_y + 2.0 * h], [cx + w, top_y + h], [cx + w, top_y + h + side], [cx, top_y + 2.0 * h + side]], color(0.6));
            }
            None => batch.fill(atlas, x + 2.0, y + 2.0, 12.0, 12.0, rgba(200, 0, 200, 255)),
        }
        if stack.count > 1 {
            let text = stack.count.to_string();
            let w = HudBatch::text_width(atlas, &text);
            batch.text(atlas, &text, x + 17.0 - w, y + 9.0, u32::MAX, true);
        }
        if let Some(info) = self.res.items.get(&stack.item)
            && info.max_damage > 0
            && stack.damage > 0
        {
            let f = 1.0 - stack.damage as f32 / info.max_damage as f32;
            let width = (13.0 * f).round();
            let hue = f / 3.0;
            let (r, g) = ((255.0 * (1.0 - hue * 3.0).clamp(0.0, 1.0)) as u8, (255.0 * (hue * 3.0).clamp(0.0, 1.0)) as u8);
            batch.fill(atlas, x + 2.0, y + 13.0, 13.0, 2.0, rgba(0, 0, 0, 255));
            batch.fill(atlas, x + 2.0, y + 13.0, width, 1.0, rgba(r, g, 0, 255));
        }
    }
    fn hud(&mut self, size: [u32; 2]) -> HudBatch {
        let mut batch = HudBatch::new(size);
        let (w, h) = (batch.width, batch.height);
        let atlas_ptr: *const HudAtlas = &self.res.atlas;
        let _ = atlas_ptr;
        if self.player.is_none() {
            let text = if self.spawn_search > 0 { "Finding a place to spawn..." } else { "Loading terrain..." };
            let atlas = &self.res.atlas;
            let tw = HudBatch::text_width(atlas, text);
            batch.fill(atlas, 0.0, 0.0, w, h, rgba(20, 20, 20, 255));
            batch.text(atlas, text, (w - tw) / 2.0, h / 2.0 - 4.0, u32::MAX, true);
            return batch;
        }
        {
            let atlas = &self.res.atlas;
            // Crosshair.
            batch.sprite(atlas, "gui/sprites/hud/crosshair", (w - 15.0) / 2.0, (h - 15.0) / 2.0);
            // Hotbar.
            batch.sprite(atlas, "gui/sprites/hud/hotbar", w / 2.0 - 91.0, h - 22.0);
            batch.sprite(atlas, "gui/sprites/hud/hotbar_selection", w / 2.0 - 91.0 - 1.0 + self.selected as f32 * 20.0, h - 23.0);
        }
        for i in 0..9 {
            if let Some(stack) = self.inventory[i].clone() {
                self.draw_item(&mut batch, &stack, w / 2.0 - 90.0 + i as f32 * 20.0 + 2.0, h - 19.0);
            }
        }
        let atlas = &self.res.atlas;
        if !self.creative
            && let Some(p) = &self.player
        {
            // Experience bar and hearts/food (Gui.renderPlayerHealth).
            batch.sprite(atlas, "gui/sprites/hud/experience_bar_background", w / 2.0 - 91.0, h - 29.0);
            let health = p.health.health().ceil() as i32;
            let top = h - 39.0;
            for i in 0..10 {
                let x = w / 2.0 - 91.0 + i as f32 * 8.0;
                let y = if health <= 4 { top + ((self.ticks + i as u64 * 7) % 3) as f32 - 1.0 } else { top };
                batch.sprite(atlas, "gui/sprites/hud/heart/container", x, y);
                let v = health - i * 2;
                if v >= 2 {
                    batch.sprite(atlas, "gui/sprites/hud/heart/full", x, y);
                } else if v == 1 {
                    batch.sprite(atlas, "gui/sprites/hud/heart/half", x, y);
                }
            }
            let food = p.health.food.food;
            for i in 0..10 {
                let x = w / 2.0 + 91.0 - i as f32 * 8.0 - 9.0;
                batch.sprite(atlas, "gui/sprites/hud/food_empty", x, top);
                let v = food - i * 2;
                if v >= 2 {
                    batch.sprite(atlas, "gui/sprites/hud/food_full", x, top);
                } else if v == 1 {
                    batch.sprite(atlas, "gui/sprites/hud/food_half", x, top);
                }
            }
        }
        if self.debug {
            let p = self.player.as_ref().map(|p| p.position).unwrap_or(Vec3::ZERO);
            let block = [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32];
            let biome = self
                .world
                .chunks
                .get(&(block[0] >> 4, block[2] >> 4))
                .map(|c| self.res.biomes.get(c.biome(block[0], block[1], block[2])).name.clone())
                .unwrap_or_default();
            let light = self.world.light(block[0], block[1], block[2]);
            let facing = match ((self.yaw.rem_euclid(360.0) + 45.0) / 90.0) as i32 % 4 {
                0 => "south (Towards positive Z)",
                1 => "west (Towards negative X)",
                2 => "north (Towards negative Z)",
                _ => "east (Towards positive X)",
            };
            let lines = [
                format!("Horizon 26.3 (Rust) {:.0} fps", self.fps),
                format!("C: {} chunks, {} sections drawn / {}", self.world.chunks.len(), self.renderer.as_ref().map_or(0, |r| r.drawn_sections), self.renderer.as_ref().map_or(0, |r| r.section_count())),
                format!("Pending jobs: {}", self.streaming.pending()),
                String::new(),
                format!("XYZ: {:.3} / {:.5} / {:.3}", p.x, p.y, p.z),
                format!("Block: {} {} {}", block[0], block[1], block[2]),
                format!("Chunk: {} {} {} in {} {} {}", block[0] & 15, block[1] & 15, block[2] & 15, block[0] >> 4, block[1] >> 4, block[2] >> 4),
                format!("Facing: {facing} ({:.1} / {:.1})", self.yaw.rem_euclid(360.0) - if self.yaw.rem_euclid(360.0) > 180.0 { 360.0 } else { 0.0 }, self.pitch),
                format!("Light: {} (sky {}, block {})", (light >> 4).max(light & 15), light >> 4, light & 15),
                format!("Biome: {biome}"),
                format!("Day {} time {}", self.world.day_time / 24000, self.world.day_time % 24000),
            ];
            for (i, line) in lines.iter().enumerate() {
                if line.is_empty() {
                    continue;
                }
                let tw = HudBatch::text_width(atlas, line);
                batch.fill(atlas, 1.0, 2.0 + i as f32 * 9.0 - 1.0, tw + 2.0, 9.0, rgba(80, 80, 80, 144));
                batch.text(atlas, line, 2.0, 2.0 + i as f32 * 9.0, rgba(224, 224, 224, 255), false);
            }
        }
        if let Some((text, at)) = &self.message
            && at.elapsed() < Duration::from_secs(3)
        {
            let tw = HudBatch::text_width(atlas, text);
            batch.text(atlas, text, (w - tw) / 2.0, h - 59.0, u32::MAX, true);
        }
        match self.screen {
            Screen::Paused => {
                batch.fill(atlas, 0.0, 0.0, w, h, rgba(16, 16, 16, 160));
                let title = "Game Menu";
                let tw = HudBatch::text_width(atlas, title);
                batch.text(atlas, title, (w - tw) / 2.0, 40.0, u32::MAX, true);
                for (i, label) in ["Back to Game", "Save and Quit to Title"].iter().enumerate() {
                    let (bx, by) = (w / 2.0 - 100.0, h / 4.0 + 24.0 + i as f32 * 24.0 + 16.0);
                    let hover = self.cursor.0 >= bx && self.cursor.0 < bx + 200.0 && self.cursor.1 >= by && self.cursor.1 < by + 20.0;
                    let sprite = if hover { "gui/sprites/widget/button_highlighted" } else { "gui/sprites/widget/button" };
                    batch.sprite_scaled(atlas, sprite, bx, by, 200.0, 20.0, u32::MAX);
                    let tw = HudBatch::text_width(atlas, label);
                    batch.text(atlas, label, bx + (200.0 - tw) / 2.0, by + 6.0, u32::MAX, true);
                }
            }
            Screen::Dead => {
                batch.fill(atlas, 0.0, 0.0, w, h, rgba(80, 0, 0, 120));
                let title = "You Died!";
                let tw = HudBatch::text_width(atlas, title);
                batch.text(atlas, title, (w - tw) / 2.0, 60.0, u32::MAX, true);
                let (bx, by) = (w / 2.0 - 100.0, h / 4.0 + 72.0);
                batch.sprite_scaled(atlas, "gui/sprites/widget/button", bx, by, 200.0, 20.0, u32::MAX);
                let label = "Respawn";
                let tw = HudBatch::text_width(atlas, label);
                batch.text(atlas, label, bx + (200.0 - tw) / 2.0, by + 6.0, u32::MAX, true);
            }
            Screen::Inventory => {
                self.draw_inventory(&mut batch);
            }
            Screen::Playing => {}
        }
        batch
    }
    fn inventory_origin(&self, batch: &HudBatch) -> (f32, f32) {
        ((batch.width - 176.0) / 2.0, (batch.height - 166.0) / 2.0)
    }
    /// Slot ids: 0..36 player inventory, 100..109 crafting grid, 200 result.
    fn slots(&self) -> Vec<usize> {
        let mut v: Vec<usize> = (0..36).collect();
        v.extend((0..self.craft_size * self.craft_size).map(|i| 100 + i));
        v.push(200);
        v
    }
    /// InventoryMenu / CraftingMenu slot layouts.
    fn slot_position(&self, i: usize) -> (f32, f32) {
        let table = self.craft_size == 3;
        match i {
            0..9 => (8.0 + i as f32 * 18.0, 142.0),
            9..36 => (8.0 + ((i - 9) % 9) as f32 * 18.0, 84.0 + ((i - 9) / 9) as f32 * 18.0),
            200 => if table { (124.0, 35.0) } else { (154.0, 28.0) },
            _ => {
                let g = i - 100;
                let (r, c) = (g / self.craft_size, g % self.craft_size);
                if table {
                    (30.0 + c as f32 * 18.0, 17.0 + r as f32 * 18.0)
                } else {
                    (98.0 + c as f32 * 18.0, 18.0 + r as f32 * 18.0)
                }
            }
        }
    }
    fn slot_at(&self, batch: &HudBatch, x: f32, y: f32) -> Option<usize> {
        let (ox, oy) = self.inventory_origin(batch);
        self.slots().into_iter().find(|&i| {
            let (sx, sy) = self.slot_position(i);
            x >= ox + sx && x < ox + sx + 16.0 && y >= oy + sy && y < oy + sy + 16.0
        })
    }
    fn slot_mut(&mut self, i: usize) -> &mut Option<ItemStack> {
        if i >= 100 && i < 109 {
            &mut self.craft_grid[i - 100]
        } else {
            &mut self.inventory[i]
        }
    }
    fn craft_result(&self) -> Option<ItemStack> {
        let n = self.craft_size;
        let grid: Vec<Option<ItemStack>> = (0..n * n).map(|i| self.craft_grid[i].clone()).collect();
        self.res.recipes.craft(&grid, n)
    }
    fn draw_inventory(&mut self, batch: &mut HudBatch) {
        let (ox, oy) = self.inventory_origin(batch);
        {
            let atlas = &self.res.atlas;
            batch.fill(atlas, 0.0, 0.0, batch.width, batch.height, rgba(16, 16, 16, 160));
            let texture = if self.craft_size == 3 { "gui/container/crafting_table" } else { "gui/container/inventory" };
            if let Some(r) = atlas.sprites.get(texture).copied() {
                // Container textures are 256x256 with the screen in the top-left 176x166.
                let mut r = r;
                r.u1 = r.u0 + (r.u1 - r.u0) * 176.0 / r.w as f32;
                r.v1 = r.v0 + (r.v1 - r.v0) * 166.0 / r.h as f32;
                batch.rect_uv(ox, oy, 176.0, 166.0, r, u32::MAX);
            }
            let (title, tx) = if self.craft_size == 3 { ("Crafting", 29.0) } else { ("Crafting", 97.0) };
            batch.text(atlas, title, ox + tx, oy + if self.craft_size == 3 { 6.0 } else { 8.0 }, rgba(64, 64, 64, 255), false);
            batch.text(atlas, "Inventory", ox + 8.0, oy + 72.0, rgba(64, 64, 64, 255), false);
        }
        for i in self.slots() {
            let (sx, sy) = self.slot_position(i);
            let stack = if i == 200 { self.craft_result() } else { self.slot_mut(i).clone() };
            if let Some(stack) = stack {
                self.draw_item(batch, &stack, ox + sx, oy + sy);
            }
        }
        if let Some(slot) = self.slot_at(batch, self.cursor.0, self.cursor.1) {
            let (sx, sy) = self.slot_position(slot);
            batch.fill(&self.res.atlas, ox + sx, oy + sy, 16.0, 16.0, rgba(255, 255, 255, 128));
        }
        if let Some(stack) = self.carried.clone() {
            self.draw_item(batch, &stack, self.cursor.0 - 8.0, self.cursor.1 - 8.0);
        }
    }
    fn close_container(&mut self) {
        let mut returned: Vec<ItemStack> = self.carried.take().into_iter().collect();
        returned.extend(self.craft_grid.iter_mut().filter_map(Option::take));
        for stack in returned {
            if let Some(rest) = self.give(stack)
                && let Some(eye) = self.eye_position()
            {
                self.spawn_item(rest, [eye[0], eye[1] - 0.3, eye[2]], [0.0, 0.1, 0.0], 40);
            }
        }
        self.craft_size = 2;
    }
    fn click_inventory(&mut self, right: bool) {
        let Some(size) = self.renderer.as_ref().map(|r| [r.config.width, r.config.height]) else { return };
        let batch = HudBatch::new(size);
        let Some(slot) = self.slot_at(&batch, self.cursor.0, self.cursor.1) else { return };
        let max = |items: &Items, s: &ItemStack| items.get(&s.item).map_or(64, |i| i.max_stack);
        let shift = self.keys.contains(&KeyCode::ShiftLeft) || self.keys.contains(&KeyCode::ShiftRight);
        if slot == 200 {
            // ResultSlot: take the crafted stack and consume one of each ingredient.
            let mut crafted = 0;
            while let Some(result) = self.craft_result() {
                if shift {
                    if self.give(result).is_some() {
                        break;
                    }
                } else {
                    match &mut self.carried {
                        None => self.carried = Some(result),
                        Some(c) if c.item == result.item && c.count + result.count <= max(&self.res.items, c) => c.count += result.count,
                        _ => return,
                    }
                }
                for cell in self.craft_grid.iter_mut() {
                    if let Some(s) = cell {
                        s.count -= 1;
                        if s.count == 0 {
                            *cell = None;
                        }
                    }
                }
                crafted += 1;
                if !shift || crafted >= 64 {
                    break;
                }
            }
            return;
        }
        if shift && slot < 36 {
            // Quick move between hotbar and main inventory.
            if let Some(stack) = self.inventory[slot].take() {
                let range = if slot < 9 { 9..36 } else { 0..9 };
                let mut stack = stack;
                for i in range.clone() {
                    if let Some(s) = &mut self.inventory[i]
                        && s.item == stack.item
                        && s.damage == stack.damage
                    {
                        let moved = max(&self.res.items, s).saturating_sub(s.count).min(stack.count);
                        s.count += moved;
                        stack.count -= moved;
                    }
                }
                if stack.count > 0
                    && let Some(i) = range.clone().find(|&i| self.inventory[i].is_none())
                {
                    self.inventory[i] = Some(stack);
                    stack = ItemStack { count: 0, ..self.inventory[i].clone().unwrap() };
                }
                if stack.count > 0 {
                    self.inventory[slot] = Some(stack);
                }
            }
            return;
        }
        let carried = self.carried.take();
        let current = self.slot_mut(slot).take();
        let items = &self.res.items;
        let (new_slot, new_carried) = match (carried, current) {
            (None, Some(mut s)) if right => {
                let half = s.count.div_ceil(2);
                s.count -= half;
                let taken = ItemStack { count: half, ..s.clone() };
                ((s.count > 0).then_some(s), Some(taken))
            }
            (None, s) => (None, s),
            (Some(mut c), None) if right => {
                let one = ItemStack { count: 1, ..c.clone() };
                c.count -= 1;
                (Some(one), (c.count > 0).then_some(c))
            }
            (Some(c), None) => (Some(c), None),
            (Some(mut c), Some(mut s)) if c.item == s.item && c.damage == s.damage => {
                let room = max(items, &s).saturating_sub(s.count);
                let moved = if right { 1.min(room).min(c.count) } else { room.min(c.count) };
                s.count += moved;
                c.count -= moved;
                (Some(s), (c.count > 0).then_some(c))
            }
            (Some(c), Some(s)) => (Some(c), Some(s)),
        };
        *self.slot_mut(slot) = new_slot;
        self.carried = new_carried;
    }
    /// ItemEntityRenderer: items bob and spin; blocks render as small cubes.
    fn entity_geometry(&mut self, camera: glam::DVec3, alpha: f64) -> Vec<Vertex> {
        let mut out = Vec::new();
        let items = self.dropped.clone();
        for item in &items {
            let p = item.previous;
            let q = item.position;
            let pos = glam::DVec3::new(
                p.x + (q.x - p.x) * alpha,
                p.y + (q.y - p.y) * alpha,
                p.z + (q.z - p.z) * alpha,
            ) - camera;
            if pos.length_squared() > 64.0 * 64.0 {
                continue;
            }
            let age = item.age as f32 + alpha as f32;
            let bob = (age / 10.0 + item.bob_offset).sin() * 0.1 + 0.1;
            let spin = age / 20.0 + item.bob_offset;
            let light = self.world.light(q.x.floor() as i32, (q.y + 0.1).floor() as i32, q.z.floor() as i32);
            let packed = u32::from(light >> 4) * 16 | (u32::from(light & 15) * 16) << 8;
            let center = pos.as_vec3() + glam::Vec3::new(0.0, bob + 0.125, 0.0);
            let rot = glam::Mat3::from_rotation_y(spin);
            let icon = self.icon(&item.stack.item);
            let atlas = &self.res.atlas;
            let mut quad = |corners: [glam::Vec3; 4], name: &str, shade: f32, tint: u32| {
                let Some(r) = atlas.sprites.get(name).copied() else { return };
                let uv = [[r.u0, r.v1], [r.u1, r.v1], [r.u1, r.v0], [r.u0, r.v0]];
                let c = |sft: u32| (((tint >> sft) & 255) as f32 * shade) as u32;
                let color = 0xFF00_0000 | c(0) << 16 | c(8) << 8 | c(16);
                for (corner, uv) in corners.iter().zip(uv) {
                    let v = center + rot * *corner;
                    out.push(Vertex { pos: v.to_array(), uv, layer: 0, color, light: packed });
                }
            };
            let copies = match item.stack.count {
                0..=1 => 1,
                2..=16 => 2,
                17..=32 => 3,
                _ => 4,
            };
            for k in 0..copies {
                let o = glam::Vec3::new(k as f32 * 0.03, k as f32 * 0.03, k as f32 * 0.05);
                match &icon {
                    Some(Icon::Cube { top, left, right, tint }) => {
                        let h = 0.125;
                        let v = |x: f32, y: f32, z: f32| glam::Vec3::new(x * h, y * h, z * h) + o;
                        quad([v(-1., 1., 1.), v(1., 1., 1.), v(1., 1., -1.), v(-1., 1., -1.)], top, 1.0, *tint);
                        quad([v(-1., -1., -1.), v(1., -1., -1.), v(1., -1., 1.), v(-1., -1., 1.)], top, 0.5, *tint);
                        quad([v(-1., -1., 1.), v(1., -1., 1.), v(1., 1., 1.), v(-1., 1., 1.)], left, 0.8, 0xFFFFFF);
                        quad([v(1., -1., -1.), v(-1., -1., -1.), v(-1., 1., -1.), v(1., 1., -1.)], left, 0.8, 0xFFFFFF);
                        quad([v(1., -1., 1.), v(1., -1., -1.), v(1., 1., -1.), v(1., 1., 1.)], right, 0.6, 0xFFFFFF);
                        quad([v(-1., -1., -1.), v(-1., -1., 1.), v(-1., 1., 1.), v(-1., 1., -1.)], right, 0.6, 0xFFFFFF);
                    }
                    Some(Icon::Flat(name)) => {
                        let h = 0.25;
                        let v = |x: f32, y: f32| glam::Vec3::new(x * h, y * h + 0.125, 0.0) + o;
                        quad([v(-1., -1.), v(1., -1.), v(1., 1.), v(-1., 1.)], name, 1.0, 0xFFFFFF);
                    }
                    None => {}
                }
            }
        }
        // Camera-facing particle quads.
        let (yaw, pitch) = (self.yaw.to_radians(), self.pitch.to_radians());
        let forward = glam::Vec3::new(-yaw.sin() * pitch.cos(), -pitch.sin(), yaw.cos() * pitch.cos());
        let right = forward.cross(glam::Vec3::Y).normalize_or_zero();
        let up = right.cross(forward).normalize_or_zero();
        let atlas = &self.res.atlas;
        for p in &self.particles {
            let Some(r) = atlas.sprites.get(&p.sprite).copied() else { continue };
            let a = alpha;
            let pos = glam::DVec3::new(
                p.previous[0] + (p.position[0] - p.previous[0]) * a,
                p.previous[1] + (p.position[1] - p.previous[1]) * a,
                p.previous[2] + (p.position[2] - p.previous[2]) * a,
            ) - camera;
            let c = pos.as_vec3();
            let light = self.world.light(p.position[0].floor() as i32, p.position[1].floor() as i32, p.position[2].floor() as i32);
            let packed = u32::from(light >> 4) * 16 | (u32::from(light & 15) * 16) << 8;
            // TerrainParticle colour: 0.6 base brightness times the block tint.
            let ch = |sft: u32| (((p.tint >> sft) & 255) as f32 * 0.6) as u32;
            let color = 0xFF00_0000 | ch(0) << 16 | ch(8) << 8 | ch(16);
            let (du, dv) = (r.u1 - r.u0, r.v1 - r.v0);
            let (u0, v0, u1, v1) = (r.u0 + du * p.uv[0], r.v0 + dv * p.uv[1], r.u0 + du * p.uv[2], r.v0 + dv * p.uv[3]);
            let s = p.size;
            for (k, uv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].iter().zip([[u0, v1], [u1, v1], [u1, v0], [u0, v0]]) {
                let v = c + right * (k.0 * s) + up * (k.1 * s);
                out.push(Vertex { pos: v.to_array(), uv, layer: 0, color, light: packed });
            }
        }
        // Breaking progress overlay (destroy_stage_0..9).
        if let Some(m) = &self.mining
            && m.progress > 0.0
        {
            let stage = ((m.progress * 10.0) as u32).min(9);
            if let Some(r) = atlas.sprites.get(&format!("block/destroy_stage_{stage}")).copied() {
                let o = (glam::DVec3::new(f64::from(m.pos[0]), f64::from(m.pos[1]), f64::from(m.pos[2])) - camera).as_vec3();
                let e = 0.002;
                let corner = |x: f32, y: f32, z: f32| o + glam::Vec3::new(x * (1.0 + 2.0 * e) - e, y * (1.0 + 2.0 * e) - e, z * (1.0 + 2.0 * e) - e);
                let faces = [
                    [corner(0., 1., 0.), corner(1., 1., 0.), corner(1., 1., 1.), corner(0., 1., 1.)],
                    [corner(0., 0., 1.), corner(1., 0., 1.), corner(1., 0., 0.), corner(0., 0., 0.)],
                    [corner(0., 0., 0.), corner(1., 0., 0.), corner(1., 1., 0.), corner(0., 1., 0.)],
                    [corner(1., 0., 1.), corner(0., 0., 1.), corner(0., 1., 1.), corner(1., 1., 1.)],
                    [corner(0., 0., 1.), corner(0., 0., 0.), corner(0., 1., 0.), corner(0., 1., 1.)],
                    [corner(1., 0., 0.), corner(1., 0., 1.), corner(1., 1., 1.), corner(1., 1., 0.)],
                ];
                for f in faces {
                    for (v, uv) in f.iter().zip([[r.u0, r.v1], [r.u1, r.v1], [r.u1, r.v0], [r.u0, r.v0]]) {
                        out.push(Vertex { pos: v.to_array(), uv, layer: 0, color: 0xB4FF_FFFF, light: 15 * 16 | (15 * 16) << 8 });
                    }
                }
            }
        }
        out
    }
    fn save_game(&mut self) -> Result<()> {
        let chunks: Vec<_> = self
            .world
            .modified
            .drain()
            .filter_map(|k| self.world.chunks.get(&k).cloned())
            .collect();
        self.save.save(&chunks, &self.res.assets.registry, &self.res.biomes)?;
        let level = &mut self.level;
        level.creative = self.creative;
        level.time = self.world.game_time;
        level.day_time = self.world.day_time;
        level.spawn = self.spawn.map(|s| [s.x, s.y, s.z]);
        level.rotation = [self.yaw, self.pitch];
        if let Some(p) = &self.player {
            level.position = Some([p.position.x, p.position.y, p.position.z]);
            level.health = p.health.health();
            level.food = p.health.food.food;
            level.saturation = p.health.food.saturation;
        }
        level.inventory = self
            .inventory
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.as_ref().map(|s| (i as u8, s.item.to_string(), s.count, s.damage)))
            .collect();
        level.write(&self.save.root)?;
        self.message = Some(("Saved the game".into(), Instant::now()));
        Ok(())
    }
    fn render(&mut self) -> Result<bool> {
        let Some(size) = self.renderer.as_ref().map(|r| [r.config.width, r.config.height]) else {
            return Ok(false);
        };
        let alpha = self.clock.fraction();
        let (camera, underwater) = match &self.player {
            Some(p) => {
                let (a, b) = (self.previous_position, p.position);
                let eye = self.previous_eye + alpha as f32 * (self.eye - self.previous_eye);
                let c = glam::DVec3::new(
                    a.x + (b.x - a.x) * alpha,
                    a.y + (b.y - a.y) * alpha + f64::from(eye),
                    a.z + (b.z - a.z) * alpha,
                );
                let s = self.world.block(c.x.floor() as i32, c.y.floor() as i32, c.z.floor() as i32);
                (c, self.res.facts.get(s).fluid == Fluid::Water)
            }
            None => (glam::DVec3::new(0.5, 100.0, 0.5), false),
        };
        self.audio.listener = [camera.x, camera.y, camera.z];
        self.audio.listener_yaw = self.yaw;
        let block = [camera.x.floor() as i32, camera.y.floor() as i32, camera.z.floor() as i32];
        let biome = self
            .world
            .chunks
            .get(&(block[0] >> 4, block[2] >> 4))
            .map(|c| c.biome(block[0], block[1], block[2]))
            .unwrap_or(0);
        let info = self.res.biomes.get(biome).clone();
        let sky = self.res.environment.sample(self.world.day_time, info.sky, info.fog);
        let outline = if self.screen == Screen::Playing {
            self.pick().map(|h| {
                let p = h.pos.map(f64::from);
                [p[0], p[1], p[2], p[0] + 1.0, p[1] + 1.0, p[2] + 1.0]
            })
        } else {
            None
        };
        let hud = self.hud(size);
        let entities = self.entity_geometry(camera, alpha);
        let water_fog = {
            let c = info.water_fog;
            [(c >> 16 & 255) as f32 / 255.0, (c >> 8 & 255) as f32 / 255.0, (c & 255) as f32 / 255.0]
        };
        let frame = Frame {
            camera,
            yaw: self.yaw,
            pitch: self.pitch,
            fov_degrees: 70.0,
            sky,
            distance: (self.streaming.radius * 16) as f32,
            underwater,
            water_fog,
            outline,
            hud: &hud,
            entities: &entities,
            game_time: self.world.game_time as f32,
            ticks: self.ticks,
        };
        self.renderer.as_mut().ok_or("renderer")?.render(&frame)
    }
    fn update_world(&mut self) {
        if let Some(p) = &self.player {
            self.streaming.center = ((p.position.x.floor() as i32) >> 4, (p.position.z.floor() as i32) >> 4);
        }
        let (uploads, unloaded) = self.streaming.update(&mut self.world);
        let modified: Vec<_> = unloaded
            .iter()
            .filter(|c| self.world.modified.remove(&(c.x, c.z)))
            .cloned()
            .collect();
        if !modified.is_empty()
            && let Err(e) = self.save.save(&modified, &self.res.assets.registry, &self.res.biomes)
        {
            eprintln!("Horizon: failed to save unloaded chunks: {e}");
        }
        if let Some(renderer) = self.renderer.as_mut() {
            for (key, mesh) in uploads {
                renderer.upload(key, &mesh);
            }
            for c in unloaded {
                for sy in 0..(HEIGHT / 16) {
                    renderer.remove((c.x, sy, c.z));
                }
            }
        }
    }
}
impl ApplicationHandler for Game<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let window = match event_loop.create_window(
            Window::default_attributes()
                .with_title("Horizon")
                .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0)),
        ) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                self.error = Some(e.to_string());
                event_loop.exit();
                return;
            }
        };
        let assets = &self.res.assets;
        match pollster::block_on(Renderer::new(
            window.clone(),
            &assets.pixels,
            assets.sprite_size,
            assets.texture_layers,
            &assets.animations,
            (&self.res.atlas.pixels, self.res.atlas.width, self.res.atlas.height),
        )) {
            Ok(r) => self.renderer = Some(r),
            Err(e) => {
                self.error = Some(e.to_string());
                event_loop.exit();
                return;
            }
        }
        self.window = Some(window);
        self.previous = Instant::now();
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if self.window.as_ref().is_none_or(|w| w.id() != id) {
            return;
        }
        let result: Result<()> = (|| {
            match event {
                WindowEvent::CloseRequested => {
                    self.save_game()?;
                    event_loop.exit();
                }
                WindowEvent::Resized(size) => {
                    if let Some(r) = &mut self.renderer {
                        r.resize(size.width, size.height);
                    }
                }
                WindowEvent::Focused(focused) => {
                    self.focused = focused;
                    if !focused {
                        self.keys.clear();
                        self.left = false;
                        self.right = false;
                        self.capture(false);
                        if self.screen == Screen::Playing && self.player.is_some() {
                            self.screen = Screen::Paused;
                        }
                    }
                }
                WindowEvent::CursorMoved { position, .. } => {
                    if let Some(r) = &self.renderer {
                        let scale = HudBatch::new([r.config.width, r.config.height]).scale;
                        self.cursor = (position.x as f32 / scale, position.y as f32 / scale);
                    }
                }
                WindowEvent::MouseWheel { delta, .. } => {
                    let d = match delta {
                        MouseScrollDelta::LineDelta(_, y) => y,
                        MouseScrollDelta::PixelDelta(p) => p.y as f32,
                    };
                    if self.screen == Screen::Playing && d != 0.0 {
                        self.selected = (self.selected as i32 - d.signum() as i32).rem_euclid(9) as usize;
                    }
                }
                WindowEvent::MouseInput { state, button, .. } => {
                    let pressed = state == ElementState::Pressed;
                    match self.screen {
                        Screen::Playing => {
                            if pressed && !self.captured {
                                self.capture(true);
                                return Ok(());
                            }
                            match button {
                                MouseButton::Left => {
                                    self.left = pressed;
                                    if pressed {
                                        self.destroy_delay = 0;
                                    }
                                }
                                MouseButton::Right => {
                                    self.right = pressed;
                                    if pressed {
                                        self.use_item()?;
                                        self.use_delay = 4;
                                    }
                                }
                                MouseButton::Middle if pressed && self.creative => {
                                    if let Some(hit) = self.pick() {
                                        let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
                                        let item = self.res.facts.get(state).item.clone();
                                        if &*item != "minecraft:air" {
                                            self.inventory[self.selected] = Some(ItemStack { item, count: 1, damage: 0 });
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        Screen::Inventory if pressed => {
                            self.click_inventory(button == MouseButton::Right);
                        }
                        Screen::Paused if pressed => {
                            let size = self.renderer.as_ref().map(|r| [r.config.width, r.config.height]).unwrap_or([1, 1]);
                            let b = HudBatch::new(size);
                            for i in 0..2 {
                                let (bx, by) = (b.width / 2.0 - 100.0, b.height / 4.0 + 24.0 + i as f32 * 24.0 + 16.0);
                                if self.cursor.0 >= bx && self.cursor.0 < bx + 200.0 && self.cursor.1 >= by && self.cursor.1 < by + 20.0 {
                                    if i == 0 {
                                        self.screen = Screen::Playing;
                                        self.capture(true);
                                    } else {
                                        self.save_game()?;
                                        event_loop.exit();
                                    }
                                }
                            }
                        }
                        Screen::Dead if pressed => {
                            self.player = None;
                            self.screen = Screen::Playing;
                            if !self.creative {
                                self.inventory.iter_mut().for_each(|s| *s = None);
                            }
                            self.spawn_player()?;
                            self.capture(true);
                        }
                        _ => {}
                    }
                }
                WindowEvent::KeyboardInput { event, .. } => {
                    let PhysicalKey::Code(key) = event.physical_key else { return Ok(()) };
                    if event.state != ElementState::Pressed {
                        self.keys.remove(&key);
                        return Ok(());
                    }
                    self.keys.insert(key);
                    if event.repeat {
                        return Ok(());
                    }
                    match key {
                        KeyCode::Escape => match self.screen {
                            Screen::Playing => {
                                self.screen = Screen::Paused;
                                self.capture(false);
                            }
                            Screen::Paused | Screen::Inventory => {
                                self.close_container();
                                self.screen = Screen::Playing;
                                self.capture(true);
                            }
                            Screen::Dead => {}
                        },
                        KeyCode::KeyE => match self.screen {
                            Screen::Playing => {
                                self.screen = Screen::Inventory;
                                self.capture(false);
                            }
                            Screen::Inventory => {
                                self.close_container();
                                self.screen = Screen::Playing;
                                self.capture(true);
                            }
                            _ => {}
                        },
                        KeyCode::F3 => self.debug = !self.debug,
                        KeyCode::KeyQ if self.screen == Screen::Playing => {
                            let all = self.keys.contains(&KeyCode::ControlLeft);
                            self.drop_held(all);
                        }
                        KeyCode::Digit1 | KeyCode::Digit2 | KeyCode::Digit3 | KeyCode::Digit4 | KeyCode::Digit5 | KeyCode::Digit6 | KeyCode::Digit7 | KeyCode::Digit8 | KeyCode::Digit9 => {
                            self.selected = match key {
                                KeyCode::Digit1 => 0,
                                KeyCode::Digit2 => 1,
                                KeyCode::Digit3 => 2,
                                KeyCode::Digit4 => 3,
                                KeyCode::Digit5 => 4,
                                KeyCode::Digit6 => 5,
                                KeyCode::Digit7 => 6,
                                KeyCode::Digit8 => 7,
                                _ => 8,
                            };
                        }
                        _ => {}
                    }
                }
                WindowEvent::RedrawRequested => {
                    let now = Instant::now();
                    let elapsed = now.duration_since(self.previous);
                    self.previous = now;
                    // Arrow keys turn the camera (accessibility / scripted testing).
                    let turn = |k| f32::from(u8::from(self.keys.contains(&k)));
                    let secs = elapsed.as_secs_f32().min(0.1);
                    self.yaw += (turn(KeyCode::ArrowRight) - turn(KeyCode::ArrowLeft)) * 90.0 * secs;
                    self.pitch = (self.pitch + (turn(KeyCode::ArrowDown) - turn(KeyCode::ArrowUp)) * 90.0 * secs).clamp(-90.0, 90.0);
                    self.update_world();
                    self.spawn_player()?;
                    let steps = self.clock.advance(elapsed);
                    for _ in 0..steps {
                        self.tick()?;
                    }
                    if self.render()? {
                        self.frames += 1;
                        self.fps_frames += 1;
                        if self.fps_at.elapsed() >= Duration::from_secs(1) {
                            self.fps = self.fps_frames as f32 / self.fps_at.elapsed().as_secs_f32();
                            self.fps_frames = 0;
                            self.fps_at = Instant::now();
                        }
                        if self.options.frames.is_some_and(|f| self.frames >= f) {
                            event_loop.exit();
                        }
                    }
                }
                _ => {}
            }
            Ok(())
        })();
        if let Err(e) = result {
            self.error = Some(e.to_string());
            event_loop.exit();
        }
    }
    fn device_event(&mut self, _: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
        if self.captured
            && self.focused
            && self.screen == Screen::Playing
            && let DeviceEvent::MouseMotion { delta: (x, y) } = event
        {
            // MouseHandler.turnPlayer at default sensitivity 0.5: 0.15 degrees per count.
            let s = 0.5_f64 * 0.6 + 0.2;
            let f = s * s * s * 8.0;
            self.yaw += (x * f * 0.15) as f32;
            self.pitch = (self.pitch + (y * f * 0.15) as f32).clamp(-90.0, 90.0);
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.options.frames.is_some() && self.started.elapsed() > self.options.timeout {
            self.error = Some("frame-limited run timed out".into());
            event_loop.exit();
            return;
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
        event_loop.set_control_flow(ControlFlow::Poll);
    }
}
pub fn run() -> Result<()> {
    let Some(options) = Options::read()? else { return Ok(()) };
    let started = Instant::now();
    let root = options.resources.clone();
    let assets = Arc::new(Assets::load(&root).map_err(|e| {
        format!("{e}\nPrepare resources with: python3 tools/prepare_native_client.py --prepare")
    })?);
    eprintln!("load assets: {:.2}s", started.elapsed().as_secs_f32());
    let facts = BlockFacts::load(&root, &assets.registry)?;
    eprintln!("load facts: {:.2}s", started.elapsed().as_secs_f32());
    let items = Items::load(&root, &assets.registry, &facts)?;
    eprintln!("load items: {:.2}s", started.elapsed().as_secs_f32());
    let recipes = Recipes::load(&root, &items)?;
    let biomes = Arc::new(Biomes::load(&root)?);
    let environment = Environment::load(&root)?;
    eprintln!("load biomes+env: {:.2}s", started.elapsed().as_secs_f32());
    let visibility = models::load_visibility(&root, &assets.registry)?;
    eprintln!("load visibility: {:.2}s", started.elapsed().as_secs_f32());
    let tables = Arc::new(MeshTables::new(&assets, &facts, visibility)?);
    let light = Arc::new(LightTables::new(&facts));
    eprintln!("load tables: {:.2}s", started.elapsed().as_secs_f32());
    let atlas = HudAtlas::build(&root)?;
    eprintln!("load atlas: {:.2}s", started.elapsed().as_secs_f32());
    let layer_names = assets
        .sprite_layers
        .iter()
        .map(|(name, layer)| (*layer, name.trim_start_matches("minecraft:").to_owned()))
        .collect();
    let registry_owner = assets.clone();
    let collision = collisions::load(&root, &registry_owner.registry)?;
    let motion = MotionResources::load(&root, &registry_owner)?;
    eprintln!("load collision+motion: {:.2}s", started.elapsed().as_secs_f32());
    let context = Arc::new(WorkerContext {
        assets: assets.clone(),
        tables,
        biomes: biomes.clone(),
        light,
    });
    let threads = std::thread::available_parallelism().map_or(2, |n| n.get()).saturating_sub(1).max(1);
    let save_root = std::env::current_dir()?.join("saves").join(&options.world);
    let level = Level::read(&save_root).unwrap_or_else(|| Level {
        name: options.world.clone(),
        seed: options.seed,
        creative: options.creative,
        day_time: 1000,
        health: 20.0,
        food: 20,
        saturation: 5.0,
        ..Level::default()
    });
    let save = Arc::new(SaveDir::new(save_root));
    let mut streaming = if options.test_terrain {
        let inner = Arc::new(TestTerrain::new(assets.registry, &biomes)?);
        Streaming::new(
            Arc::new(Persistent { inner, save: save.clone(), registry: assets.clone(), biomes: biomes.clone() }),
            context,
            threads,
        )
    } else {
        let inner = Arc::new(VanillaTerrain::new(&root.join("worldgen"), assets.registry, &biomes, level.seed).map_err(|e| {
            format!("{e}\nPrepare the worldgen bundle with: python3 tools/prepare_native_client.py")
        })?);
        eprintln!("load worldgen: {:.2}s", started.elapsed().as_secs_f32());
        Streaming::new(
            Arc::new(Persistent { inner, save: save.clone(), registry: assets.clone(), biomes: biomes.clone() }),
            context,
            threads,
        )
    };
    streaming.radius = options.distance;
    eprintln!(
        "Horizon: resources loaded in {:.1}s; {} block states, {} items, {} biomes; {} worker threads",
        started.elapsed().as_secs_f32(),
        facts.states.len(),
        items.items.len(),
        biomes.list.len(),
        threads
    );
    let audio = Audio::new(&root);
    let mut world = World::new(facts.air, level.seed);
    world.game_time = level.time;
    world.day_time = level.day_time;
    let creative = level.creative || options.creative;
    let mut resources = Resources {
        assets,
        facts,
        items,
        recipes,
        biomes,
        environment,
        atlas,
        icons: HashMap::new(),
        layer_names,
    };
    let mut inventory = vec![None; 36];
    for (slot, id, count, damage) in &level.inventory {
        if let Some(cell) = inventory.get_mut(usize::from(*slot)) {
            *cell = Some(ItemStack { item: Arc::from(id.as_str()), count: *count, damage: *damage });
        }
    }
    let fresh = level.inventory.is_empty() && level.position.is_none();
    if creative && fresh {
        for (i, name) in ["minecraft:grass_block", "minecraft:stone", "minecraft:oak_planks", "minecraft:glass", "minecraft:torch", "minecraft:oak_log", "minecraft:cobblestone", "minecraft:water_bucket", "minecraft:diamond_pickaxe"].iter().enumerate() {
            inventory[i] = Some(ItemStack { item: Arc::from(*name), count: 1, damage: 0 });
        }
    }
    let now = Instant::now();
    let mut game = Game {
        res: &mut resources,
        collision: &collision,
        motion: &motion,
        world,
        streaming,
        options,
        player: None,
        spawn: None,
        clock: FixedStep::default(),
        previous_position: Vec3::ZERO,
        eye: 1.62,
        previous_eye: 1.62,
        yaw: level.rotation[0],
        pitch: level.rotation[1],
        inventory,
        selected: 0,
        carried: None,
        craft_grid: vec![None; 9],
        craft_size: 2,
        mining: None,
        destroy_delay: 0,
        use_delay: 0,
        creative,
        keys: HashSet::new(),
        left: false,
        right: false,
        cursor: (0.0, 0.0),
        captured: false,
        focused: true,
        debug: false,
        screen: Screen::Playing,
        window: None,
        renderer: None,
        started: now,
        previous: now,
        frames: 0,
        fps: 0.0,
        fps_frames: 0,
        fps_at: now,
        ticks: 0,
        error: None,
        message: None,
        save,
        level,
        audio,
        dropped: Vec::new(),
        next_entity: 0,
        particles: Vec::new(),
        spawn_search: 0,
    };
    EventLoop::new()?.run_app(&mut game)?;
    if let Some(path) = &game.options.report {
        let report = serde_json::json!({
            "frames": game.frames,
            "elapsed_seconds": game.started.elapsed().as_secs_f64(),
            "chunks": game.world.chunks.len(),
            "generated": game.streaming.generated_total,
            "meshed_sections": game.streaming.meshed_total,
            "player": game.player.as_ref().map(|p| [p.position.x, p.position.y, p.position.z]),
            "error": game.error,
        });
        std::fs::write(path, serde_json::to_vec_pretty(&report)?)?;
    }
    if let Some(e) = game.error {
        return Err(e.into());
    }
    Ok(())
}
