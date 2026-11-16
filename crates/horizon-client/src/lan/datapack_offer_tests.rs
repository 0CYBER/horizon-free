use super::PlayerSender;
use std::sync::Arc;

#[test]
fn registry_offers_follow_live_connections_without_evicting_other_players() {
    let mut first = PlayerSender::disabled();
    first.datapack_offer = Some(Arc::from([1_u8, 2, 3]));
    let weak = Arc::downgrade(first.datapack_offer.as_ref().unwrap());
    let held = first.clone();
    for n in 0..4100_u32 {
        let mut joining = PlayerSender::disabled();
        joining.datapack_offer = Some(Arc::from(n.to_le_bytes()));
        assert_eq!(held.datapack_offer.as_deref(), Some(&[1, 2, 3][..]));
    }
    drop(first);
    assert!(weak.upgrade().is_some());
    drop(held);
    assert!(
        weak.upgrade().is_none(),
        "disconnect releases the connection's offer"
    );
}
