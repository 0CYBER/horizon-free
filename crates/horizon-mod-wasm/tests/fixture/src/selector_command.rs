use mod_api::*;
use std::sync::atomic::{AtomicU32, Ordering};

static CALLS: AtomicU32 = AtomicU32::new(0);

pub fn run(event: &mut RunModCommandEvent<'_>) -> Flow {
    let name = event.command.literals.first().map(String::as_str);
    if matches!(
        name,
        Some("wasmheal" | "wasmentity" | "wasmentities" | "wasmplayer")
    ) {
        return run_game(event);
    }
    if !matches!(
        name,
        Some(
            "selector_heal" | "selector_cow" | "selector_herd" | "selector_player" | "selector_id"
        )
    ) {
        return Flow::Continue;
    }
    assert_eq!(
        event.source,
        &CommandSource::Console {
            permission: PermissionLevel::All
        }
    );
    let call = CALLS.fetch_add(1, Ordering::Relaxed);
    assert_eq!(event.command.arguments.len(), 1);
    let epoch = WorldEpoch::new(73);
    match name.unwrap() {
        "selector_heal" => {
            assert_eq!(call, 0);
            let Some(CommandArgumentValue::Players(players)) = event.command.argument("targets")
            else {
                panic!("players must arrive as typed handles");
            };
            assert_eq!(players.len(), 2);
            assert_eq!(players[0].entity(), EntityRef::new(epoch, 91));
            assert_eq!(players[0].uuid(), 0x112233445566778899aabbccddeeff00);
            assert_eq!(players[1].entity(), EntityRef::new(epoch, 7));
            assert_eq!(players[1].uuid(), 0xffeeddccbbaa99887766554433221100);
        }
        "selector_cow" => {
            assert_eq!(call, 1);
            assert_eq!(
                event.command.argument("target"),
                Some(&CommandArgumentValue::Entity(EntityRef::new(epoch, 42)))
            );
        }
        "selector_herd" => {
            assert_eq!(call, 2);
            let Some(CommandArgumentValue::Entities(entities)) = event.command.argument("targets")
            else {
                panic!("entities must arrive as typed handles");
            };
            assert_eq!(entities.len(), 300);
            for (index, entity) in entities.iter().enumerate() {
                assert_eq!(*entity, EntityRef::new(epoch, 600 - index as u32));
            }
        }
        "selector_player" => {
            assert_eq!(call, 3);
            let Some(CommandArgumentValue::Player(player)) = event.command.argument("target")
            else {
                panic!("existing single-player variant must remain distinct");
            };
            assert_eq!(player.entity(), EntityRef::new(epoch, 7));
            assert_eq!(player.uuid(), 0xffeeddccbbaa99887766554433221100);
        }
        "selector_id" => {
            assert_eq!(call, 4);
            assert_eq!(
                event.command.argument("kind"),
                Some(&CommandArgumentValue::Id(
                    Identifier::parse("minecraft:zombie").unwrap()
                ))
            );
        }
        _ => unreachable!(),
    }
    Flow::Cancel
}

static GAME_CALLS: AtomicU32 = AtomicU32::new(0);

fn run_game(event: &mut RunModCommandEvent<'_>) -> Flow {
    let CommandSource::Player { player: source, .. } = event.source else {
        panic!("game selector fixture expects a player source");
    };
    assert_eq!(source.entity().id(), 12);
    let epoch = source.entity().epoch();
    let call = GAME_CALLS.fetch_add(1, Ordering::Relaxed);
    match event.command.literals[0].as_str() {
        "wasmheal" => {
            assert_eq!(call, 0);
            let Some(CommandArgumentValue::Players(players)) = event.command.argument("targets")
            else {
                panic!("expected resolved players");
            };
            assert_eq!(players.len(), 2);
            assert_eq!(players[0].entity(), EntityRef::new(epoch, 12));
            assert_eq!(players[1].entity(), EntityRef::new(epoch, 13));
            assert_ne!(players[0].uuid(), players[1].uuid());
        }
        "wasmentity" => {
            assert_eq!(call, 1);
            assert_eq!(
                event.command.argument("target"),
                Some(&CommandArgumentValue::Entity(EntityRef::new(epoch, 103)))
            );
        }
        "wasmentities" => {
            assert_eq!(call, 2);
            assert_eq!(
                event.command.argument("targets"),
                Some(&CommandArgumentValue::Entities(vec![
                    EntityRef::new(epoch, 103),
                    EntityRef::new(epoch, 102),
                    EntityRef::new(epoch, 101),
                ]))
            );
        }
        "wasmplayer" => {
            assert_eq!(call, 3, "refused selections must never invoke the guest");
            let Some(CommandArgumentValue::Player(player)) = event.command.argument("target")
            else {
                panic!("expected existing single player variant");
            };
            assert_eq!(player.entity(), EntityRef::new(epoch, 13));
        }
        _ => unreachable!(),
    }
    Flow::Cancel
}
