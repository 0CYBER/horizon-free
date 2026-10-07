//! Native Horizon development client. The scene and free camera are rendering
//! tools; they do not replace the authoritative gameplay implementation.
#![forbid(unsafe_code)]
mod camera;
mod mesh;
mod render;
use assets::Assets;
use camera::Camera;
use horizon_client::{assets, collisions, models, sprites, world};
use horizon_client::{
    clock::FixedStep,
    input,
    motion::{MotionResources, MotionWorld, PlayerMotion},
};
use horizon_core::geometry::Vec3;
use mesh::Mesh;
use render::Renderer;
use std::{
    collections::HashSet,
    fs,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    event::{DeviceEvent, DeviceId, ElementState, MouseButton, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{CursorGrabMode, Window, WindowId},
};
use world::TestWorld;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
struct Options {
    resources: PathBuf,
    radius: i32,
    frames: Option<u64>,
    report: Option<PathBuf>,
    timeout: Duration,
    trace_input: bool,
}
impl Options {
    fn read() -> Result<Option<Self>> {
        let sibling = std::env::current_exe()?
            .parent()
            .ok_or("missing executable directory")?
            .join("resources");
        let mut options = Self {
            resources: if sibling.join("horizon-resources.json").exists() {
                sibling
            } else {
                PathBuf::from(".local/horizon-client/26.3")
            },
            radius: 4,
            frames: None,
            report: None,
            timeout: Duration::from_secs(90),
            trace_input: false,
        };
        let mut args = std::env::args().skip(1);
        while let Some(argument) = args.next() {
            if argument == "--trace-input" {
                options.trace_input = true;
                continue;
            }
            if argument == "--help" || argument == "-h" {
                println!(
                    "Horizon native development client\n\n  --resources PATH       Prepared pinned resources\n  --radius CHUNKS        Controlled scene radius, 1..16 (default 4)\n  --frames COUNT         Exit after this many presented frames\n  --smoke-report PATH    Write a runtime report on exit\n  --timeout-seconds N    Deadline for a frame-limited run (default 90)\n  --trace-input          Log received keyboard, mouse-button and focus events\n\nControls: click to capture mouse; WASD move; Space/Shift rise/descend;\nCtrl boosts speed; arrow keys rotate; Esc releases mouse; Ctrl+Q quits; F3 shows diagnostics; F4 toggles camera collisions; F5 toggles walking test.\nWalking test: WASD move; Space jumps; gravity and collisions at 20 ticks/s.\n\nThis is a controlled development scene. Walking includes landing damage and stored health. Sprint/crouch, full death/respawn, interaction, saving and complete gameplay remain unfinished."
                );
                return Ok(None);
            }
            let value = args
                .next()
                .ok_or_else(|| format!("missing value after {argument}"))?;
            match argument.as_str() {
                "--resources" => options.resources = value.into(),
                "--radius" => {
                    options.radius = value.parse()?;
                    if !(1..=16).contains(&options.radius) {
                        return Err("radius must be 1..16 chunks".into());
                    }
                }
                "--frames" => {
                    let frames = value.parse()?;
                    if frames == 0 {
                        return Err("frame count must be positive".into());
                    }
                    options.frames = Some(frames);
                }
                "--smoke-report" => options.report = Some(value.into()),
                "--timeout-seconds" => {
                    let seconds: u64 = value.parse()?;
                    if seconds == 0 {
                        return Err("timeout must be positive".into());
                    }
                    options.timeout = Duration::from_secs(seconds);
                }
                _ => return Err(format!("unknown option {argument}").into()),
            }
        }
        Ok(Some(options))
    }
}
struct App<'a> {
    assets: &'a Assets,
    world: TestWorld<'a>,
    collision_binding: &'a horizon_world::block_collision::BlockCollisionRegistry<'a>,
    collision_camera: bool,
    motion_resources: &'a MotionResources,
    player: Option<PlayerMotion>,
    motion_clock: FixedStep,
    previous_player_position: Vec3,
    movement_ticks: u64,
    fall_damage_requests: u64,
    last_requested_fall_damage: Option<f32>,
    visibility: horizon_world::block_render::FaceVisibility,
    mesh_revision: u64,
    mesh: Mesh,
    options: Options,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    camera: Camera,
    keys: HashSet<KeyCode>,
    captured: bool,
    focused: bool,
    diagnostics: bool,
    started: Instant,
    previous: Instant,
    next_redraw: Instant,
    title_at: Instant,
    title_frames: u64,
    frames: u64,
    error: Option<String>,
}
impl App<'_> {
    fn toggle_walking(&mut self) -> Result<()> {
        if self.player.is_some() {
            self.player = None;
        } else {
            let mut collision = collisions::SceneCollisions {
                world: &self.world,
                binding: self.collision_binding,
            };
            if !collision.unobstructed(self.camera.collision_bounds())? {
                eprintln!(
                    "Horizon: move the inspection camera to an unobstructed position before walking."
                );
                return Ok(());
            }
            let p = self.camera.position;
            let feet = Vec3::new(p.x, p.y - f64::from(1.62_f32), p.z);
            self.player = Some(PlayerMotion::new(self.motion_resources, feet)?);
            self.previous_player_position = feet;
            self.collision_camera = false;
        }
        self.motion_clock.reset();
        self.previous = Instant::now();
        eprintln!("Horizon walking test: {}", self.player.is_some());
        Ok(())
    }
    fn move_player(&mut self, elapsed: Duration) -> Result<()> {
        let player = self.player.as_mut().ok_or("missing movement actor")?;
        let direction = input::modify_input(
            input::keyboard_direction(
                self.keys.contains(&KeyCode::KeyW),
                self.keys.contains(&KeyCode::KeyS),
                self.keys.contains(&KeyCode::KeyA),
                self.keys.contains(&KeyCode::KeyD),
            ),
            None,
            None,
        );
        for _ in 0..self.motion_clock.advance(elapsed) {
            self.previous_player_position = player.position;
            player.health.advance_clock();
            player.yaw = (-self.camera.yaw.to_degrees()) as f32;
            MotionWorld {
                player,
                scene: &mut self.world,
                assets: self.assets,
                collision_binding: self.collision_binding,
                resources: self.motion_resources,
            }
            .player_movement_tick(direction, self.keys.contains(&KeyCode::Space))?;
            // Health is already updated synchronously during landing. These
            // counters expose incoming requests; delivery consumers remain pending.
            for event in player.events.drain(..) {
                if let horizon_client::motion::MotionEvent::Fall(
                    horizon_client::landing::FallEvent::Damage { amount, .. },
                ) = event
                {
                    self.fall_damage_requests += 1;
                    self.last_requested_fall_damage = Some(amount);
                }
            }
            self.movement_ticks += 1;
        }
        let from = self.previous_player_position;
        let to = player.position;
        let alpha = self.motion_clock.fraction();
        self.camera.position = glam::DVec3::new(
            from.x + (to.x - from.x) * alpha,
            from.y + (to.y - from.y) * alpha + f64::from(1.62_f32),
            from.z + (to.z - from.z) * alpha,
        );
        if self.mesh_revision != self.world.revision {
            let mesh = mesh::build(&self.world, self.assets, &mut self.visibility)?;
            self.renderer
                .as_mut()
                .ok_or("missing renderer")?
                .replace_mesh(&mesh.cubes, &mesh.models)?;
            self.mesh = mesh;
            self.mesh_revision = self.world.revision;
        }
        Ok(())
    }
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: impl ToString) {
        self.error = Some(error.to_string());
        event_loop.exit();
    }
    fn capture(&mut self, capture: bool) {
        let Some(window) = &self.window else {
            return;
        };
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
    fn write_report(&self) -> Result<()> {
        let Some(path) = &self.options.report else {
            return Ok(());
        };
        let renderer = self.renderer.as_ref();
        let adapter = renderer.map(|r| {
            serde_json::json!({
                "name": r.info.name,
                "backend": format!("{:?}", r.info.backend),
                "device_type": format!("{:?}", r.info.device_type),
                "vendor": r.info.vendor,
                "device": r.info.device,
            })
        });
        let size = self.window.as_ref().map(|window| {
            let size = window.inner_size();
            [size.width, size.height]
        });
        let report = serde_json::json!({
            "schema": 1,
            "project": "Horizon",
            "scope": "Native resident-scene renderer with inspection camera and optional fixed-rate dry walking test; full gameplay remains unfinished",
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "adapter": adapter,
            "window_size": size,
            "presented_frames": self.frames,
            "elapsed_seconds": self.started.elapsed().as_secs_f64(),
            "resident_chunks": self.world.chunks.len(),
            "merged_quads": self.mesh.cubes.len(),
            "model_quads": self.mesh.models.len(),
            "visible_model_faces": self.mesh.visible_faces,
            "culled_model_faces": self.mesh.culled_faces,
            "model_blocks": self.mesh.model_blocks,
            "model_gallery": self.assets.model_gallery,
            "sprite_layers": self.assets.texture_layers,
            "animated_sprites": self.assets.animations.len(),
            "animation_uploads": self.renderer.as_ref().map_or(0, |r| r.animation_uploads),
            "camera_position": self.camera.position.to_array(),
            "camera_yaw": self.camera.yaw,
            "camera_pitch": self.camera.pitch,
            "collision_camera": self.collision_camera,
            "walking": self.player.is_some(),
            "movement_ticks": self.movement_ticks,
            "player_position": self.player.as_ref().map(|p| [p.position.x,p.position.y,p.position.z]),
            "player_velocity": self.player.as_ref().map(|p| [p.velocity.x,p.velocity.y,p.velocity.z]),
            "player_on_ground": self.player.as_ref().map(|p| p.ground.on_ground),
            "player_health": self.player.as_ref().map(|p| p.health.health()),
            "player_absorption": self.player.as_ref().map(|p| p.health.absorption()),
            "player_dead": self.player.as_ref().map(|p| p.health.dead),
            "player_hurt_cooldown": self.player.as_ref().map(|p| p.health.hurt.cooldown),
            "player_fall_distance": self.player.as_ref().map(|p| p.fall_distance),
            "fall_damage_requests": self.fall_damage_requests,
            "last_requested_fall_damage": self.last_requested_fall_damage,
            "world_revision": self.world.revision,
            "mesh_revision": self.mesh_revision,
            "error": self.error,
        });
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_vec_pretty(&report)?)?;
        Ok(())
    }
}
impl ApplicationHandler for App<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let window = match event_loop.create_window(
            Window::default_attributes()
                .with_title("Horizon — Development Client")
                .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 800.0)),
        ) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                self.fail(event_loop, error);
                return;
            }
        };
        match pollster::block_on(Renderer::new(
            window.clone(),
            self.assets,
            &self.mesh.cubes,
            &self.mesh.models,
            ((self.options.radius + 1) * 16) as f32,
        )) {
            Ok(renderer) => self.renderer = Some(renderer),
            Err(error) => {
                self.fail(event_loop, error);
                return;
            }
        }
        self.window = Some(window);
        self.previous = Instant::now();
        self.next_redraw = self.previous;
        self.title_at = self.previous;
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if self.options.trace_input
            && matches!(
                event,
                WindowEvent::Focused(_)
                    | WindowEvent::KeyboardInput { .. }
                    | WindowEvent::MouseInput { .. }
            )
        {
            eprintln!("Horizon input: {event:?}");
        }
        if self.window.as_ref().is_none_or(|window| window.id() != id) {
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size.width, size.height);
                }
                self.next_redraw = Instant::now();
            }
            WindowEvent::Focused(focused) => {
                self.focused = focused;
                if !focused {
                    self.keys.clear();
                    self.capture(false);
                }
                self.previous = Instant::now();
                self.motion_clock.reset();
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => self.capture(true),
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(key) = event.physical_key {
                    if event.state == ElementState::Pressed {
                        self.keys.insert(key);
                        if key == KeyCode::Escape {
                            self.capture(false);
                        }
                        if key == KeyCode::F5
                            && !event.repeat
                            && let Err(error) = self.toggle_walking()
                        {
                            self.fail(event_loop, error);
                        }
                        if key == KeyCode::F4 && !event.repeat && self.player.is_none() {
                            if self.collision_camera {
                                self.collision_camera = false;
                            } else {
                                let mut collision = collisions::SceneCollisions {
                                    world: &self.world,
                                    binding: self.collision_binding,
                                };
                                match collision.unobstructed(self.camera.collision_bounds()) {
                                    Ok(clear) => self.collision_camera = clear,
                                    Err(error) => self.fail(event_loop, error),
                                }
                            }
                            eprintln!(
                                "Horizon inspection camera collisions: {}",
                                self.collision_camera
                            );
                        }
                        if key == KeyCode::F3 && !event.repeat {
                            self.diagnostics = !self.diagnostics;
                            if !self.diagnostics {
                                self.window
                                    .as_ref()
                                    .unwrap()
                                    .set_title("Horizon — Development Client");
                            }
                        }
                        if key == KeyCode::KeyQ
                            && (self.keys.contains(&KeyCode::ControlLeft)
                                || self.keys.contains(&KeyCode::ControlRight))
                        {
                            event_loop.exit();
                        }
                    } else {
                        self.keys.remove(&key);
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                let elapsed = now.duration_since(self.previous);
                let seconds = elapsed.as_secs_f64().min(0.1);
                self.previous = now;
                if self.focused && self.player.is_some() {
                    self.camera.turn(&self.keys, seconds);
                    if let Err(error) = self.move_player(elapsed) {
                        self.fail(event_loop, error);
                        return;
                    }
                } else if self.focused {
                    let from = self.camera.position;
                    let bounds = self.camera.collision_bounds();
                    self.camera.update(&self.keys, seconds);
                    if self.collision_camera {
                        let delta = self.camera.position - from;
                        let mut collision = collisions::SceneCollisions {
                            world: &self.world,
                            binding: self.collision_binding,
                        };
                        match collision.resolve(
                            bounds,
                            horizon_core::geometry::Vec3::new(delta.x, delta.y, delta.z),
                            0.0,
                            false,
                        ) {
                            Ok(movement) => {
                                self.camera.position =
                                    from + glam::DVec3::new(movement.x, movement.y, movement.z)
                            }
                            Err(error) => {
                                self.fail(event_loop, error);
                                return;
                            }
                        }
                    }
                }
                let result = self.renderer.as_mut().unwrap().render(&self.camera);
                match result {
                    Ok(true) => {
                        self.frames += 1;
                        self.next_redraw = Instant::now();
                        if self
                            .options
                            .frames
                            .is_some_and(|target| self.frames >= target)
                        {
                            event_loop.exit();
                        }
                        if self.diagnostics && self.title_at.elapsed() >= Duration::from_secs(1) {
                            let fps = (self.frames - self.title_frames) as f64
                                / self.title_at.elapsed().as_secs_f64();
                            let health = self.player.as_ref().map_or_else(String::new, |p| {
                                format!(
                                    " | Health {}{}",
                                    p.health.health(),
                                    if p.health.dead_or_dying() {
                                        " (dead)"
                                    } else {
                                        ""
                                    }
                                )
                            });
                            self.window.as_ref().unwrap().set_title(&format!(
                                "Horizon — Development | {fps:.0} FPS | {} chunks | {} quads{health}",
                                self.world.chunks.len(),
                                self.mesh.cubes.len()
                            ));
                            self.title_at = Instant::now();
                            self.title_frames = self.frames;
                        }
                    }
                    Ok(false) => self.next_redraw = Instant::now() + Duration::from_millis(100),
                    Err(error) => self.fail(event_loop, error),
                }
            }
            _ => {}
        }
    }
    fn device_event(&mut self, _: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
        if self.captured
            && self.focused
            && let DeviceEvent::MouseMotion { delta: (x, y) } = event
        {
            self.camera.look(x, y);
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.options.frames.is_some() && self.started.elapsed() > self.options.timeout {
            self.fail(event_loop, "frame-limited client run timed out");
            return;
        }
        if Instant::now() >= self.next_redraw {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
            event_loop.set_control_flow(if self.options.frames.is_some() {
                ControlFlow::WaitUntil(self.started + self.options.timeout)
            } else {
                ControlFlow::Wait
            });
        } else {
            event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_redraw));
        }
    }
}
fn run() -> Result<()> {
    let Some(options) = Options::read()? else {
        return Ok(());
    };
    let assets = Assets::load(&options.resources).map_err(|error| {
        format!("{error}\nPrepare pinned resources with: python3 tools/prepare_native_client.py")
    })?;
    let world = TestWorld::new(&assets, options.radius)?;
    let collision_binding = collisions::load(&options.resources, &assets.registry)?;
    let motion_resources = MotionResources::load(&options.resources, &assets)?;
    let mut visibility = models::load_visibility(&options.resources, &assets.registry)?;
    let mesh = mesh::build(&world, &assets, &mut visibility)?;
    eprintln!(
        "Horizon development scene: {} resident chunks, {} merged cube faces, {} baked model faces.\nClick to capture mouse; WASD/Space/Shift move; arrows rotate; F4 toggles camera collisions; F5 toggles walking test; Esc releases; Ctrl+Q quits.",
        world.chunks.len(),
        mesh.cubes.len(),
        mesh.models.len()
    );
    let now = Instant::now();
    let mut app = App {
        assets: &assets,
        world,
        collision_binding: &collision_binding,
        collision_camera: false,
        motion_resources: &motion_resources,
        player: None,
        motion_clock: FixedStep::default(),
        previous_player_position: Vec3::ZERO,
        movement_ticks: 0,
        fall_damage_requests: 0,
        last_requested_fall_damage: None,
        visibility,
        mesh_revision: 0,
        mesh,
        options,
        window: None,
        renderer: None,
        camera: Camera::default(),
        keys: HashSet::new(),
        captured: false,
        focused: true,
        diagnostics: false,
        started: now,
        previous: now,
        next_redraw: now,
        title_at: now,
        title_frames: 0,
        frames: 0,
        error: None,
    };
    EventLoop::new()?.run_app(&mut app)?;
    app.write_report()?;
    if let Some(error) = app.error {
        return Err(error.into());
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("Horizon: {error}");
        std::process::exit(1);
    }
}
