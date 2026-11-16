//! `MobEffect.particleFactory`: Wind Charged, Weaving, Oozing and Infested
//! are registered with a particle of their own (`MobEffects`), and every
//! other effect an entity effect in its colour, faint when ambient. This is
//! the list `LivingEntity.tickEffects` swirls around the window's player and
//! around the mobs of this game.
use super::effect_particle;
use super::*;
use crate::game::*;
use horizon_client::effects::EffectInstance;
use horizon_client::motion::PlayerMotion;
use horizon_client::particles::{ParticleOptions, ParticleType};
use std::path::PathBuf;
use std::time::Instant;

/// The particle type an effect shows, or a panic naming the effect.
fn ty(effect: &str, ambient: bool) -> ParticleType {
    match effect_particle(effect, ambient) {
        Some(p) => p.ty,
        None => panic!("{effect} shows no particle"),
    }
}

#[test]
fn the_four_party_effects_show_their_own_particle_types() {
    // `WindChargedMobEffect` passes `ParticleTypes.SMALL_GUST`, `Weaving`
    // `ITEM_COBWEB`, `Oozing` `ITEM_SLIME` and `Infested` `INFESTED`.
    let own = [
        ("oozing", ParticleType::ItemSlime),
        ("weaving", ParticleType::ItemCobweb),
        ("wind_charged", ParticleType::SmallGust),
        ("infested", ParticleType::Infested),
    ];
    for (effect, want) in own {
        for ambient in [false, true] {
            assert_eq!(ty(effect, ambient), want, "{effect} ambient {ambient}");
            // The effects' ids also arrive namespaced.
            let named = format!("minecraft:{effect}");
            assert_eq!(ty(&named, ambient), want, "{named} ambient {ambient}");
        }
    }
}

#[test]
fn every_other_effect_stays_an_entity_effect_in_its_colour() {
    // `MobEffects.SPEED` is 0x33EBFF, `SLOWNESS` 0x8BAFE0; an ambient
    // instance's alpha is `AMBIENT_ALPHA` (38) of 255.
    let channel = |color: u32, shift: u32| ((color >> shift) & 255) as f32 / 255.0;
    let entity = |color: u32, alpha: f32| {
        ParticleOptions::colored(
            ParticleType::EntityEffect,
            [
                channel(color, 16),
                channel(color, 8),
                channel(color, 0),
                alpha,
            ],
        )
    };
    for (effect, color) in [("speed", 0x33_EBFF), ("slowness", 0x8B_AFE0)] {
        let shown = effect_particle(effect, false).unwrap();
        let want = entity(color, 1.0);
        assert_eq!(shown.ty, want.ty, "{effect}");
        assert_eq!(shown.color, want.color, "{effect}");
        let faint = effect_particle(effect, true).unwrap();
        assert_eq!(faint.color, entity(color, 38.0 / 255.0).color, "{effect}");
    }
}

/// A game with the prepared vanilla resources (`HORIZON_TEST_RESOURCES`): the
/// tests that use it are `#[ignore]`d, as CI has no resources.
fn with_game(test: impl FnOnce(&mut Game<'_>)) {
    let root = PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let loaded = load_resources(
        horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        Instant::now(),
        false,
        crate::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
    let mut g = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
            root.to_path_buf(),
        )),
        None,
    )
    .unwrap();
    g.options.dedicated = Some(Vec::new());
    g.world = World::new(g.res.facts().air, 1);
    test(&mut g);
}

/// `LivingEntity.tickEffects` on a client: the window's own player, oozing,
/// swirls a slime ball crumb (`OozingMobEffect` passes `ITEM_SLIME`) at its
/// own box, and the crumb has the slime ball's model to draw
/// (`BreakingItemParticle.SlimeProvider`).
#[test]
#[ignore = "daily"]
fn an_oozing_player_swirls_slime_ball_crumbs() {
    with_game(|g| {
        let oozing = horizon_client::brewing::effect("oozing").unwrap().id;
        g.potions.effects.add(EffectInstance::new(oozing, 400, 0));
        g.player = Some(PlayerMotion::new(g.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
        // `ParticleEngine.add` records what the level is asked for instead of
        // drawing it, so the swirls are read exactly.
        g.particles.requested = Some(Vec::new());
        // One tick in four swirls one particle.
        for _ in 0..64 {
            g.effect_swirls();
        }
        let asked = g.particles.requested.take().unwrap();
        assert!(
            asked
                .iter()
                .any(|(options, ..)| options.ty == ParticleType::ItemSlime),
            "no slime ball crumb: {:?}",
            asked.iter().map(|(o, ..)| o.ty).collect::<Vec<_>>()
        );
        // The crumb draws the slime ball's model, not the missing sprite.
        let sprites = g.particles.item_sprites.get(&ParticleType::ItemSlime);
        assert!(sprites.is_some_and(|s| !s.is_empty()), "{sprites:?}");
    });
}
