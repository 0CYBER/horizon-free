//! The terrain's quad looks (`HORIZON_TERRAIN_LOOKS=1`) through pack changes on a real
//! offscreen renderer: what its arenas hold of a section, read back, is what the mesher handed
//! over.
use crate::game::native_entity_refs::tests::{run_fresh_cases, with_client_game};
use crate::game::native_hooks::particle_count_tests::frame_scene;
use horizon_client::chunk::Chunk;
use horizon_client::mesher::{Mesher, SectionMesh};
use std::time::Duration;

/// A 0.2 pack with a material stage, and `[shadow]` where `shadow`.
fn pack(shadow: bool) -> horizon_shader_api::Report {
    use horizon_shader_api::{MANIFEST, Pack, check};
    let manifest = format!(
        "schema = 1\nid = \"looks\"\nname = \"Looks\"\nversion = \"1\"\napi = \"0.2\"\n\
         [material]\nfile = \"material.wgsl\"\n{}",
        if shadow { "[shadow]\n" } else { "" }
    );
    let material = "fn material(input: MaterialInput) -> Surface { return Surface(input.albedo); }\n";
    let files = [(MANIFEST, manifest.as_str()), ("material.wgsl", material)]
        .map(|(name, text)| (name.to_owned(), text.as_bytes().to_vec()));
    check(&Pack::from_files(files.into_iter().collect()).unwrap()).unwrap()
}

fn install(game: &mut super::Game<'_>, shadow: bool) {
    let renderer = game.renderer.as_mut().unwrap();
    let pack = (pack(shadow), Vec::new(), Vec::new(), Vec::new(), Default::default());
    renderer.set_native_shader_pack(Some(pack)).unwrap();
}

/// Section `sy` of column (0, 0), meshed as the game's workers mesh it, with its quads' looks
/// while the game asks for them (`streaming.looks`).
fn mesh(game: &super::Game<'_>, sy: i32) -> SectionMesh {
    let context = game.worker_context();
    let chunk = |x: i32, z: i32| game.world.chunks.get(&(x, z)).map(|c| &**c);
    let region: [[Option<&Chunk>; 3]; 3] =
        std::array::from_fn(|dz| std::array::from_fn(|dx| chunk(dx as i32 - 1, dz as i32 - 1)));
    let s = &game.streaming;
    let mut mesher = Mesher {
        nether_shading: false,
        shader_pack_lighting: false,
        separate_ao: false,
        shader_pack: false,
        looks: s.looks,
        assets: &context.assets,
        render: &context.render,
        tables: &context.tables,
        biomes: &context.biomes,
        visibility: context.tables.visibility.clone(),
        zoom: s.biome_zoom,
        blend_radius: s.biome_blend,
        smooth_lighting: s.smooth_lighting,
        cutout_leaves: s.cutout_leaves,
        opaque_ice: false,
        translucent_layers: &[],
        leaf_culling: s.leaf_culling,
        hidden_states: &context.hidden_states,
        part_tables: &context.part_tables,
    };
    let mut mesh = mesher.mesh(&region, sy);
    mesh.sort_solid();
    mesh
}

/// A frame drawn, after the game's own work before it (the pack's inputs).
fn frame(game: &mut super::Game<'_>) {
    game.render_and_flip(Duration::from_secs_f64(1.0 / 60.0))
        .unwrap();
}

/// What the arenas hold of section `key`'s looks is its mesh's, solid and translucent, and
/// some are there; whether translucent quads have an arena of their own.
fn held_as_handed(game: &mut super::Game<'_>, key: (i32, i32, i32), case: &str) -> bool {
    let renderer = game.renderer.as_mut().unwrap();
    let (parts, split) = renderer.section_looks(key).expect(case);
    for (part, name) in parts.iter().zip(["solid", "translucent"]) {
        assert!(!part.handed.is_empty(), "{case}: {name} quads have looks");
        assert!(
            part.handed.iter().any(|l| l.0[0] != 0),
            "{case}: {name} looks are set"
        );
        let held = part.held.as_ref().unwrap_or_else(|| panic!("{case}: {name} has no looks"));
        let wrong = held.iter().zip(&part.handed).filter(|(a, b)| a != b).count();
        assert!(
            held.len() == part.handed.len() && wrong == 0,
            "{case}: {name}: {wrong} of {} slots differ",
            held.len()
        );
    }
    split
}

/// The offscreen game with a section of stone, water, glass, a torch and a lantern, whose
/// uploads the renderer keeps copies of: the event loop holding the window, and the section.
fn scene(game: &mut super::Game<'_>) -> (winit::event_loop::EventLoop<()>, (i32, i32, i32)) {
    let event_loop = frame_scene(game);
    for x in -1..=1 {
        for z in -1..=1 {
            let mut chunk = Chunk::empty(x, z);
            chunk.lit = true;
            game.world.chunks.insert((x, z), std::sync::Arc::new(chunk));
        }
    }
    let state = |name: &str| game.pack.assets.registry.default_state(name).unwrap();
    let [stone, water, glass, torch, lantern] = [
        "minecraft:stone",
        "minecraft:water",
        "minecraft:white_stained_glass",
        "minecraft:torch",
        "minecraft:lantern",
    ]
    .map(state);
    for x in 0..16 {
        for z in 0..16 {
            game.world.set(x, 62, z, stone).unwrap();
        }
    }
    for x in 4..8 {
        game.world.set(x, 63, 4, water).unwrap();
        game.world.set(x, 63, 8, glass).unwrap();
    }
    game.world.set(10, 63, 10, torch).unwrap();
    game.world.set(12, 63, 10, lantern).unwrap();
    let sy = (62 - game.world.bottom_y()).div_euclid(16);
    game.renderer.as_mut().unwrap().keep_capture_copies(true);
    (event_loop, (0, sy, 0))
}

/// Section `key` meshed as the game asks now, uploaded, and a frame drawn.
fn upload(game: &mut super::Game<'_>, key: (i32, i32, i32)) {
    let mesh = mesh(game, key.1);
    game.renderer.as_mut().unwrap().upload(key, &mesh);
    frame(game);
}

/// The game takes up the looks a frame or two after the pack asks for them, and meshes again:
/// the section is uploaded once it has.
fn install_and_upload(game: &mut super::Game<'_>, key: (i32, i32, i32), shadow: bool) {
    install(game, shadow);
    frame(game);
    frame(game);
    assert!(game.streaming.looks, "the game meshes with looks");
    upload(game, key);
}

/// A section's looks stay what the mesher handed over: when a pack that reads them replaces
/// another in one frame (the picker drops the old pack, then loads the new), when the same pack
/// loads again, and when the new pack lays translucent quads out in an arena of their own.
#[test]
#[ignore = "daily"]
fn terrain_looks_stay_with_their_quads_through_pack_changes() {
    let name = "game::shader_pack_looks_tests::terrain_looks_stay_with_their_quads_through_pack_changes";
    if run_fresh_cases(name, "HORIZON_TERRAIN_LOOKS", &["1"]) {
        return;
    }
    with_client_game(|game| {
        let (_event_loop, key) = scene(game);
        install_and_upload(game, key, true);
        held_as_handed(game, key, "first pack");
        // The picker's change of pack: the old one dropped, the new one loaded, one frame.
        game.renderer.as_mut().unwrap().drop_shader_packs();
        install(game, true);
        frame(game);
        frame(game);
        let split = held_as_handed(game, key, "another pack");
        install(game, true);
        frame(game);
        frame(game);
        held_as_handed(game, key, "the same pack again");
        // Without `[shadow]`, flat draws give translucent quads an arena of their own: the
        // layout change drops every section, which the game meshes again.
        install(game, false);
        frame(game);
        assert!(game.renderer.as_mut().unwrap().take_dropped_sections());
        upload(game, key);
        let now = held_as_handed(game, key, "translucent arena");
        assert!(!split && now, "the translucent arena came with the pack");
    });
}

/// A renderer made again on the same device for another frame takes the world
/// (`Renderer::take_world`, the game's `rebuild_renderer`): the sections' looks go with it,
/// and the pack loaded in it, which takes them too, keeps them as they are.
#[test]
#[ignore = "daily"]
fn terrain_looks_go_with_the_world_to_a_new_renderer() {
    let name = "game::shader_pack_looks_tests::terrain_looks_go_with_the_world_to_a_new_renderer";
    if run_fresh_cases(name, "HORIZON_TERRAIN_LOOKS", &["1"]) {
        return;
    }
    with_client_game(|game| {
        let (event_loop, key) = scene(game);
        install_and_upload(game, key, true);
        held_as_handed(game, key, "the first renderer");
        #[allow(deprecated)]
        let window = event_loop
            .create_window(
                winit::window::Window::default_attributes()
                    .with_visible(false)
                    .with_inner_size(winit::dpi::PhysicalSize::new(320, 240)),
            )
            .unwrap();
        let mut renderer = game.new_renderer(std::sync::Arc::new(window)).unwrap();
        assert!(renderer.take_world(game.renderer.as_mut().unwrap()));
        game.renderer = Some(renderer);
        install(game, true);
        frame(game);
        frame(game);
        assert!(!game.renderer.as_mut().unwrap().take_dropped_sections());
        held_as_handed(game, key, "the renderer made again");
    });
}

/// Sections uploaded without looks in a page with more slots than looks fit in a storage
/// binding (a stand-in limit; a translucent page may be past the real one): when a pack turns
/// looks on, the renderer drops every section at once, and the pages the game's meshes go to
/// then keep to the limit with their looks.
#[test]
#[ignore = "daily"]
fn pages_past_the_look_limit_are_dropped_when_looks_turn_on() {
    let name =
        "game::shader_pack_looks_tests::pages_past_the_look_limit_are_dropped_when_looks_turn_on";
    if run_fresh_cases(name, "HORIZON_TERRAIN_LOOKS", &["1"]) {
        return;
    }
    with_client_game(|game| {
        let (_event_loop, key) = scene(game);
        frame(game);
        assert!(!game.streaming.looks);
        upload(game, key);
        let renderer = game.renderer.as_mut().unwrap();
        assert!(renderer.section_looks(key).is_some(), "uploaded");
        // A translucent page holds 2^18 slots or more from its first: a quarter of that.
        assert!(renderer.section_looks(key).unwrap().1, "translucent quads apart");
        renderer.limit_translucent_look_pages(1 << 16);
        let _ = renderer.take_dropped_sections();
        // Without `[shadow]`, as without a pack: the same layout of translucent quads.
        install(game, false);
        frame(game);
        let renderer = game.renderer.as_mut().unwrap();
        assert!(renderer.take_dropped_sections(), "dropped when looks turned on");
        assert!(renderer.section_looks(key).is_none());
        frame(game);
        assert!(game.streaming.looks, "the game meshes with looks");
        upload(game, key);
        held_as_handed(game, key, "pages within the limit");
    });
}
