use super::*;
use mod_api::BlockMetadata as _;
use mod_api::{
    BlockState, Compound, DrawContext, EntityRef, GuiEntity, Scene, SceneBlock, SceneClock,
    SceneEntity, SceneError, SceneParticle, SceneView,
};

/// A screen's drawing that keeps each scene it is asked to draw and answers with the
/// refusal its clock's id names (`REFUSALS[id]`), so every named refusal crosses back.
#[derive(Default)]
struct Draw(Vec<([f32; 4], Scene, SceneView)>);
impl DrawContext for Draw {
    fn draw_scene(
        &mut self,
        rect: [f32; 4],
        scene: &Scene,
        view: SceneView,
    ) -> Result<(), SceneError> {
        self.0.push((rect, scene.clone(), view));
        match scene.clock {
            Some(clock) => REFUSALS.get(clock.id as usize).copied().unwrap_or(Ok(())),
            None => Ok(()),
        }
    }
}

/// The answer for each clock id.
const REFUSALS: [Result<(), SceneError>; 16] = [
    Ok(()),
    Err(SceneError::Unavailable),
    Err(SceneError::BadView),
    Err(SceneError::TooManyBlocks),
    Err(SceneError::TooManyEntities),
    Err(SceneError::TooManyParticles),
    Err(SceneError::TooManyScenes),
    Err(SceneError::BadBlock { index: 3 }),
    Err(SceneError::UnknownType { index: 1 }),
    Err(SceneError::BadData { index: 2 }),
    Err(SceneError::NoEntity { index: 0 }),
    Err(SceneError::BadEntity { index: 5 }),
    Err(SceneError::BadParticle { index: 4 }),
    Err(SceneError::BadClock),
    Err(SceneError::NoClock),
    Ok(()),
];

/// What a guide page draws: a scene of blocks alone, then a living one with each part (a
/// block, an entity by type with data, one of the level, a particle and a clock) on each
/// clock id, each answered with its refusal.
fn asks(
    block: BlockState,
    smoke: mod_api::ParticleKey,
) -> Vec<([f32; 4], Scene, SceneView, Result<(), SceneError>)> {
    let rect = [12.0, 20.0, 140.0, 110.0];
    let view = SceneView::new([1.0, 0.5, 0.5], 225.0, 30.0, 18.0);
    let blocks = vec![
        SceneBlock::new([0, 0, 0], block, None),
        SceneBlock::new([1, 0, -2], block, None),
    ];
    let mut data = Compound::new();
    data.insert("IsBaby".into(), mod_api::Tag::Byte(1));
    let entities = vec![
        SceneEntity::new(GuiEntity::with_data("minecraft:cow", data), [1.5, 0.0, 0.5])
            .turned(135.0, 150.0, -12.5),
        SceneEntity::new(
            GuiEntity::existing(EntityRef::new(mod_api::WorldEpoch::new(3), 42)),
            [-0.5, 1.0, 0.25],
        ),
    ];
    let particles = vec![SceneParticle::new(
        mod_api::ParticleOptions::simple(smoke),
        [0.5, 1.25, 0.5],
        [0.0, 0.0625, -0.125],
    )];
    let mut asks = vec![(rect, Scene::new(blocks.clone()), view, Ok(()))];
    for (id, answer) in (0u32..).zip(REFUSALS) {
        let clock = SceneClock::new(id, 40 + u64::from(id), 0.375).animate_blocks(id % 2 == 0);
        let scene = Scene::new(blocks.clone())
            .with_entities(entities.clone())
            .with_particles(particles.clone())
            .with_clock(clock);
        asks.push((rect, scene, view, answer));
    }
    asks
}

struct Register;
#[mod_api::__subscriptions]
impl Hooks for Register {
    fn register(&self, registrar: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        registrar.block_key("wire_test:stone");
        registrar.particle_key("minecraft:smoke");
    }
}

/// The installed block's default state and the smoke's key, as a loaded game has them.
fn keys(mods: &mod_api::Mods) -> (BlockState, mod_api::ParticleKey) {
    mod_api::__private::part_tests::install_parts_fixture(mods).unwrap();
    let properties = std::collections::BTreeMap::new();
    mod_api::__private::resolve_block_keys(["wire_test:stone"], |_| {
        horizon_mod_runtime::block_definitions::BlockDefinition::new(&[&properties], 0)
            .map_err(|e| format!("{e:?}"))
    })
    .unwrap();
    mod_api::__private::resolve_particles(["minecraft:smoke"], |_| {
        Some(horizon_mod_runtime::registry_keys::ParticleKind::Vanilla(5))
    })
    .unwrap();
    let metadata = mod_api::__private::block_definitions().unwrap();
    let block = BlockState::from(
        metadata
            .of(
                mod_api::__private::part_tests::block_key("wire_test:stone"),
                &[],
            )
            .unwrap(),
    );
    let smoke = mod_api::__private::particle_named_key("minecraft:smoke").expect("smoke's key");
    (block, smoke)
}

/// A native mod's screen draw: the hook's draw context is the game's, called directly.
struct Screen;
#[mod_api::__subscriptions]
impl Hooks for Screen {
    fn gui_render_post(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut mod_api::GuiRenderPostEvent<'_>,
    ) {
        let (block, smoke) = KEYS.get().copied().unwrap();
        for (rect, scene, view, answer) in asks(block, smoke) {
            assert_eq!(event.batch.draw_scene(rect, &scene, view), answer);
        }
    }
}

static KEYS: std::sync::OnceLock<(BlockState, mod_api::ParticleKey)> = std::sync::OnceLock::new();

struct Client;
impl mod_api::Host for Client {}
mod_api::impl_host_areas!(Client);
impl mod_api::ClientHost for Client {}
mod_api::impl_client_host_areas!(Client);

#[test]
fn a_native_mods_screen_draw_reaches_the_draw_contexts_scene() {
    let mods: mod_api::Mods = vec![
        mod_api::ModEntry::new("wire_test", Box::new(Register)),
        mod_api::ModEntry::new("screen", Box::new(Screen)),
    ]
    .into();
    mod_api::testing::with_mods(mods.clone(), || {
        let (block, smoke) = keys(&mods);
        let _ = KEYS.set((block, smoke));
        let mut draw = Draw::default();
        mod_api::dispatch_gui_render_post(&Client, &mut draw, false, true);
        let sent: Vec<_> = asks(block, smoke)
            .into_iter()
            .map(|(r, s, v, _)| (r, s, v))
            .collect();
        assert_eq!(draw.0, sent);
    });
}

#[test]
fn a_wasm_mods_scene_crosses_whole_with_each_part_and_each_refusal_comes_back() {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("wire_test", Box::new(Register))].into();
    mod_api::testing::with_mods(mods.clone(), || {
        let (block, smoke) = keys(&mods);
        let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
        let mut draw = Draw::default();
        let mut cx = Box::new(boundary::Cx::default());
        let mut frame = boundary::Frame::default();
        frame.set_draw_context_mut(&mut draw);
        cx.frames.push(frame);
        let pointer = &mut *cx as *mut boundary::Cx;
        guest::set_transport(Some(Rc::new(move |id, bytes| {
            let mut out = Writer::new();
            // SAFETY: cx and its frame's draw remain live for this synchronous transport.
            unsafe { boundary::serve(id, pointer, bytes, &mut out) }.unwrap();
            out.into_bytes()
        })));
        let mut sent = Vec::new();
        let mut answers = Vec::new();
        for (rect, scene, view, answer) in asks(block, smoke) {
            answers.push(DrawContext::draw_scene(
                &mut guest::Guest,
                rect,
                &scene,
                view,
            ));
            assert_eq!(answers.last(), Some(&answer));
            sent.push((rect, scene, view));
        }
        guest::set_transport(None);
        drop(cx);
        // Every named refusal came back as itself, and the host was handed exactly what
        // the guest drew: blocks, both entities, the particle and the clock.
        let mut expected = vec![Ok(())];
        expected.extend(REFUSALS);
        assert_eq!(answers, expected);
        assert_eq!(draw.0.len(), 17);
        assert_eq!(draw.0[1].1.blocks.len(), 2);
        assert_eq!(draw.0[1].1.entities.len(), 2);
        assert_eq!(draw.0[1].1.particles.len(), 1);
        assert_eq!(draw.0, sent);
    });
}
