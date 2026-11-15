use super::*;
use crate::game::native_entity_refs::tests::with_game;
use mod_api::{AttachmentRowError, AttachmentType, AttachmentWriteToken};

static VALUE: AttachmentType<u32> = AttachmentType {
    namespace: "boundary",
    name: "value",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync: false,
};

fn capture(game: &Game<'_>) -> Vec<(AttachmentHolder, u32, AttachmentWriteToken)> {
    let batch = game.attachment_query("boundary", "value");
    let query = batch.query(&VALUE).unwrap();
    query
        .iter()
        .map(|row| {
            let row = row.unwrap();
            (
                row.holder,
                *row.value,
                AttachmentWriteToken::from(row.write_token()),
            )
        })
        .collect()
}

#[test]
#[ignore = "daily"]
fn current_dimension_capture_and_foreign_residency_validation() {
    with_game(|game| {
        VALUE
            .set(&mut game.world.attachments, Holder::Level, 7)
            .unwrap();
        game.native_level_opened(Dim::Overworld);
        let rows = capture(game);
        let holder = rows[0].0;
        let token = rows[0].2.clone();
        assert_eq!(game.attachment_validate_write(token.clone()), Ok(()));
        assert_eq!(rows[0].1, 7);
        let epoch = game.native_lifecycle.epoch().unwrap();
        let b = AttachmentHolder::Entity(mod_api::EntityRef::new(epoch, 313));
        let valid_b = game.attachment_indexes[&game.dim]
            .lock()
            .unwrap()
            .register(b, [])
            .into_result();
        let host: &dyn mod_api::ServerHost = game;
        assert_eq!(host.attachment_validate_write(valid_b.into()), Ok(()));

        let foreign = Arc::new(Mutex::new(AttachmentIndex::default()));
        let mut foreign_map = horizon_mod_runtime::attachments::Attachments::default();
        VALUE.set(&mut foreign_map, Holder::Level, 99).unwrap();
        foreign_map.bind_index(&foreign, holder);
        let original = game
            .attachment_indexes
            .insert(Dim::Overworld, foreign)
            .unwrap();
        assert_eq!(
            game.attachment_validate_write(token.clone()),
            Err(AttachmentRowError::StaleHolder)
        );
        assert_eq!(capture(game)[0].1, 99);
        game.attachment_indexes.insert(Dim::Overworld, original);
        assert_eq!(game.attachment_validate_write(token.clone()), Ok(()));

        game.dim = Dim::Nether;
        assert!(capture(game).is_empty());
        assert_eq!(
            game.attachment_validate_write(token),
            Err(AttachmentRowError::StaleHolder)
        );
        game.dim = Dim::Overworld;
    });
}
