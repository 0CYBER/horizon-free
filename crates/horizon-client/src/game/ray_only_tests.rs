//! The ray scene's entity layer takes the entities the camera doesn't draw (spec #16011 item 3,
//! step 2): one out of the camera's view or hidden, within the layer's reach, goes to the layer's
//! own list instead of being dropped, and nothing the camera draws changes; with the layer off,
//! the game decides and drops exactly as before. (A headless game has no baked mob models, so
//! the GPU proof shows whole models; these are the decisions every generator goes through.)
use crate::game::*;
use horizon_client::gfx::Frustum;
use horizon_client::gfx::native_entity::{EntityKind, EntityRuns};

fn game() -> Game<'static> {
    let root = std::path::PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES")
            .expect("set HORIZON_TEST_RESOURCES to prepared resources"),
    );
    let settings = Settings::default();
    let loaded = load_resources(
        root.clone(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        true,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
    let res: &'static _ = Box::leak(Box::new(loaded.resources));
    let mut game = Game::new(
        res,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&root),
        None,
    )
    .unwrap();
    game.build_registries();
    game.set_allow_commands(true);
    game
}

/// A model of four vertices around `z` (camera-relative), as a generator leaves it in `out`.
fn model(z: f32) -> Vec<Vertex> {
    let corner = |x: f32, y: f32| Vertex {
        pos: [x, y, z],
        uv: [0.0; 2],
        layer: 0,
        color: u32::MAX,
        light: 0,
        normal: [0; 4],
        glint_uv: [0.0; 2],
    };
    vec![
        corner(0.0, 0.0),
        corner(1.0, 0.0),
        corner(1.0, 1.0),
        corner(0.0, 1.0),
    ]
}

fn bytes(v: &[Vertex]) -> Vec<u8> {
    bytemuck::cast_slice::<Vertex, u8>(v).to_vec()
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn what_the_camera_drops_within_reach_goes_to_the_layer_and_nothing_else_changes() {
    use super::{EntityCullDecision as D, Sight};
    let mut game = game();
    // The camera's view: everything toward -z. A box in front, one behind, one far behind.
    game.view_frustum = Some(Frustum::boxed(
        glam::Vec3::splat(-64.0),
        glam::Vec3::new(64.0, 64.0, 0.0),
    ));
    let at = |z: f32| {
        (
            glam::Vec3::new(0.0, 0.0, z),
            glam::Vec3::new(1.0, 2.0, z + 1.0),
        )
    };
    let ((front_lo, front_hi), (back_lo, back_hi), (far_lo, far_hi)) =
        (at(-5.0), at(5.0), at(300.0));
    let hidden = || D::Hidden;

    // The layer off: the camera's view alone decides, and a hidden entity stays hidden.
    game.native_shaders.ray_reach = None;
    let front = game.sight(front_lo, front_hi);
    assert_eq!(
        front,
        Some(Sight {
            camera: true,
            rays: false
        })
    );
    assert_eq!(game.sight(back_lo, back_hi), None);
    assert!(game.sighted_decision(front.unwrap(), hidden) == D::Hidden);
    assert!(game.sighted_decision(front.unwrap(), || D::CheckVertices) == D::CheckVertices);
    // Occlusion drops a model as before, kept for the layer or not.
    let mut out = [model(-9.0), model(-5.0)].concat();
    game.culled_for_rays(|| game.drop_culled(&mut out, 4));
    assert_eq!(bytes(&out), bytes(&model(-9.0)));
    assert!(game.native_shaders.ray_only.get_mut().is_empty());

    // The layer on, reaching 144 blocks: the box behind is the layer's alone, and so is one in
    // view that occlusion hides; the far one is no one's.
    let reach = Frustum::boxed(glam::Vec3::splat(-144.0), glam::Vec3::splat(144.0));
    game.native_shaders.ray_reach = Some(reach);
    let front = game.sight(front_lo, front_hi).unwrap();
    assert_eq!(
        front,
        Sight {
            camera: true,
            rays: true
        }
    );
    let back = game.sight(back_lo, back_hi).unwrap();
    assert_eq!(
        back,
        Sight {
            camera: false,
            rays: true
        }
    );
    assert_eq!(game.sight(far_lo, far_hi), None);
    let mut asked = false;
    assert!(
        game.sighted_decision(back, || {
            asked = true;
            D::CheckVertices
        }) == D::Rays
    );
    assert!(
        !asked,
        "no occlusion query for an entity out of the camera's view"
    );
    assert!(game.sighted_decision(front, hidden) == D::Rays);
    assert!(game.sighted_decision(front, || D::CheckVertices) == D::CheckVertices);
    assert!(game.sighted_decision(front, || D::Bypass) == D::Bypass);
    // A model the layer takes alone leaves what the camera draws as it was before it.
    let mut out = [model(-9.0), model(5.0)].concat();
    game.ray_only(&mut out, 4);
    assert_eq!(bytes(&out), bytes(&model(-9.0)));
    // A hidden one in reach goes there too while a lit entity is culled; one out of reach, or
    // culled outside `culled_for_rays` (an item), is dropped.
    let mut out = [model(-9.0), model(-5.0)].concat();
    game.culled_for_rays(|| game.drop_culled(&mut out, 4));
    assert_eq!(bytes(&out), bytes(&model(-9.0)));
    let mut out = [model(-9.0), model(-500.0)].concat();
    game.culled_for_rays(|| game.drop_culled(&mut out, 4));
    let mut item = [model(-9.0), model(-6.0)].concat();
    game.drop_culled(&mut item, 4);
    assert_eq!(bytes(&out), bytes(&model(-9.0)));
    assert_eq!(bytes(&item), bytes(&model(-9.0)));
    assert!(!game.native_shaders.ray_keeps.get());
    // The layer's list: the two models it took, one entity's run each, merged.
    let ray = std::mem::take(game.native_shaders.ray_only.get_mut());
    assert_eq!(bytes(&ray), bytes(&[model(5.0), model(-5.0)].concat()));
    let mut runs = EntityRuns::default();
    runs.push(0..8, EntityKind::Entity);
    assert_eq!(*game.native_shaders.ray_runs.get_mut(), runs);
}
