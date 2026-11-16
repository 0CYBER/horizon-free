#![cfg(test)]
//! Prepared-resource proof of the API request, server queue and client model resolution.
use super::*;
use crate::game::api_values::{TryIntoApi, TryIntoEngine};

#[test]
#[ignore = "daily"]
fn stack_particle_crosses_server_packet_and_resolves_apple() {
    let mut transfer = None;
    crate::game::native_entity_refs::tests::with_game(|game| {
        mod_api::__private::resolve_particles(["minecraft:item"], |id| {
            horizon_client::particles::ParticleType::from_name(id)
                .map(|ty| mod_api::__private::ParticleKind::Vanilla(ty as u32))
        })
        .unwrap();
        let stack = ItemStack::new("minecraft:apple", 1);
        let api_stack = stack.try_into_api().unwrap();
        let key = mod_api::__private::particle_named_key("minecraft:item").unwrap();
        let options = mod_api::ParticleOptions::item_stack(key, api_stack.clone());
        let level = game.level(game.dim.id()).unwrap();
        assert!(level.send_particles(game, &options, None, [0.0, 64.0, 0.0], [0.2; 3], 0.0, 5));
        let queued = game.creatures.lan_particles.pop().unwrap();
        let mut sync = horizon_client::lan::data::SyncData::empty();
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        sync.builtin = horizon_protocol::registry::Builtin::load(
            &horizon_protocol::registry::Builtin::find(&root).unwrap(),
        )
        .unwrap();
        let sync = std::sync::Arc::new(sync);
        let (hub, _, _) = crate::game::lan_particles::tests::test_hub(
            "Item particle",
            vec![],
            Some(sync.clone()),
        );
        let particle = hub.particle(&queued.options, None).unwrap();
        use horizon_protocol::packets::play::{self, Clientbound};
        let packet = Clientbound::LevelParticles(play::LevelParticles {
            particle,
            override_limiter: false,
            always_show: false,
            pos: queued.at,
            dist: queued.spread.map(|v| v as f32),
            max_speed: queued.speed.map(|v| v as f32),
            count: queued.count,
            randomization: 0,
        })
        .encode();
        let mut oversized = api_stack;
        oversized.set_count(99).unwrap();
        let bad = mod_api::ParticleOptions::item_stack(key, oversized);
        assert!(bad.try_into_engine(game.items()).is_err());
        assert!(!level.send_particles(game, &bad, None, [0.0; 3], [0.0; 3], 0.0, 1));
        assert!(game.creatures.lan_particles.is_empty());
        transfer = Some((packet, sync));
    });
    let (packet, sync) = transfer.unwrap();
    crate::game::native_entity_refs::tests::with_client_game(|game| {
        use horizon_protocol::packets::play::Clientbound;
        let mut reader = horizon_protocol::buf::Reader::new(&packet);
        let id = reader.varint().unwrap();
        let Clientbound::LevelParticles(decoded) = Clientbound::decode(id, reader.rest()).unwrap()
        else {
            panic!("not a particle packet");
        };
        assert_eq!(decoded.count, 5);
        let particle = decoded.particle;
        let horizon_protocol::entity_data::ParticleData::Item(network_stack) =
            particle.data.unwrap()
        else {
            panic!("not an item particle")
        };
        assert_eq!(
            sync.builtin
                .get("minecraft:item")
                .unwrap()
                .name(network_stack.item),
            Some("minecraft:apple")
        );
        let received =
            horizon_client::item_sync::from_network(&network_stack, &sync.builtin, &sync.dynamic);
        assert_eq!(&*received.item, "minecraft:apple");
        assert_eq!(
            game.item_particle_options(&received).unwrap().item,
            Some(vec!["item/apple".into()])
        );
    });
}
