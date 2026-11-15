mod api {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct BlockEntityTypeKey(pub u32);
}

use mod_api::BlockKey;
use mod_api::mod_api::BlockEntityTypeKey;

#[test]
fn block_entity_additions_collect_in_order_and_quiet_dispatch_preserves_input() {
    struct Listener(BlockKey);
    #[mod_api::__subscriptions]
    impl Hooks for Listener {
        fn block_entity_type_add_blocks(
            &self,
            _: &RegistrationContext,
            event: &mut BlockEntityTypeAddBlocksEvent<'_>,
        ) {
            for &kind in event.types.iter().filter(|kind| kind.0 < 2) {
                event.additions.push((kind, self.0));
            }
        }
    }
    let types = [
        BlockEntityTypeKey(0),
        BlockEntityTypeKey(1),
        BlockEntityTypeKey(2),
    ];
    let mut additions = vec![(types[2], BlockKey::Other)];
    install(Vec::<ModEntry>::new().into());
    dispatch_block_entity_type_add_blocks(
        &RegistrationContext("bootstrap"),
        &types,
        &mut additions,
    );
    assert_eq!(additions, [(types[2], BlockKey::Other)]);
    install(
        vec![
            ModEntry::new("first", Box::new(Listener(BlockKey::Test))),
            ModEntry::new("second", Box::new(Listener(BlockKey::Other))),
        ]
        .into(),
    );
    dispatch_block_entity_type_add_blocks(
        &RegistrationContext("bootstrap"),
        &types,
        &mut additions,
    );
    assert_eq!(
        additions,
        [
            (types[2], BlockKey::Other),
            (types[0], BlockKey::Test),
            (types[1], BlockKey::Test),
            (types[0], BlockKey::Other),
            (types[1], BlockKey::Other)
        ]
    );
}
