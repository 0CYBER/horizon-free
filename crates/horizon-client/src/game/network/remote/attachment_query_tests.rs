use super::*;
use crate::game::native_entity_refs::tests::with_client_game;
use horizon_mod_runtime::attachments::AttachmentSync;
use horizon_protocol::attachment_sync::{Message, encode};
use mod_api::{AttachmentCodec, AttachmentType, Host};

static VALUE: AttachmentType<i32> = AttachmentType {
    namespace: "mirror",
    name: "value",
    default: || 73,
    codec: Some(AttachmentCodec {
        encode: |n| Ok([("n".into(), Tag::Int(*n))].into()),
        decode: |c| match c.get(&"n".into()) {
            Some(Tag::Int(n)) => Ok(*n),
            _ => Err(mod_api::AttachmentError::Decode),
        },
    }),
    copy_on_death: false,
    sync: true,
};
fn values(game: &Game<'_>) -> Vec<i32> {
    let host: &dyn Host = game;
    host.attachment_query("mirror", "value")
        .query(&VALUE)
        .unwrap()
        .iter()
        .map(|r| *r.unwrap().value)
        .collect()
}
fn client<'a>(game: &'a mut Game<'_>) -> &'a mut Remote {
    game.remote.as_mut().unwrap()
}
fn sync(game: &mut Game<'_>, holder: Holder, value: i32) {
    game.remote_attachment_sync(
        &encode(&Message {
            holder,
            replace: true,
            changes: vec![AttachmentSync {
                namespace: "mirror".into(),
                name: "value".into(),
                value: Some([("n".into(), Tag::Int(value))].into()),
            }],
        })
        .unwrap(),
    );
}
#[test]
#[ignore = "daily"]
fn synced_client_queries_follow_replacement_retirement_and_level_changes() {
    with_client_game(|game| {
        let (pipe, _server) = horizon_protocol::connection::Pipe::pair();
        game.connect_local(pipe, "QueryMirror").unwrap();
        let mut spawn = horizon_protocol::packets::play::SpawnInfo {
            dimension_type: 0,
            dimension: "test:pocket".into(),
            seed: 0,
            game_type: 1,
            previous_game_type: None,
            debug: false,
            flat: true,
            last_death: None,
            portal_cooldown: 0,
            sea_level: 63,
        };
        game.enter_remote_dimension(&spawn, false, horizon_client::level_loading::Reason::Other);
        client(game).logged_in = true;
        assert!(values(game).is_empty());
        let host: &mut dyn Host = game;
        VALUE.set(host.attachments(), Holder::Level, 73).unwrap();
        assert_eq!(values(game), [73]);
        client(game).attachments.clear();
        let host: &mut dyn Host = game;
        assert_eq!(VALUE.get(host.attachments(), Holder::Level), Ok(73));
        assert_eq!(values(game), [73]);
        client(game).attachments.clear();
        sync(game, Holder::Level, 41);
        assert_eq!(values(game), [41]);
        let captured = game.attachment_query("mirror", "value");
        let query = captured.query(&VALUE).unwrap();
        let row = query.iter().next().unwrap().unwrap();
        assert_eq!(
            row.holder,
            mod_api::AttachmentHolder::Level(game.level("test:pocket").unwrap())
        );
        let token = mod_api::AttachmentWriteToken::from(row.write_token());
        sync(game, Holder::Level, 42);
        assert_eq!(values(game), [42]);
        assert_eq!(*row.value, 41);
        client(game).attachments.remove(&Holder::Level);
        assert!(values(game).is_empty());
        assert_eq!(
            game.attachment_validate_write(token),
            Err(mod_api::AttachmentRowError::StaleHolder)
        );
        sync(game, Holder::Chunk([1, 2]), 51);
        assert_eq!(values(game), [51]);
        client(game).attachments.retain(|_, _| false);
        assert!(values(game).is_empty());
        sync(game, Holder::Level, 61);
        client(game).attachments.clear();
        assert!(values(game).is_empty());
        sync(game, Holder::Level, 71);
        spawn.dimension = "minecraft:the_nether".into();
        game.enter_remote_dimension(&spawn, false, horizon_client::level_loading::Reason::Other);
        assert!(values(game).is_empty());
        assert!(game.remote.as_ref().unwrap().attachments.is_empty());
        sync(game, Holder::Level, 81);
        assert_eq!(values(game), [81]);
    });
}
