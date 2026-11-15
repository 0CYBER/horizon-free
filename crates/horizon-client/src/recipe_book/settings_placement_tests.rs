use super::super::super::{
    placement_sync::{Frame, schema},
    settings_sync::Message,
};
use super::*;
#[test]
fn placement_requires_the_existing_negotiation_and_shares_its_budget() {
    let mut connection = Connection::default();
    let now = Instant::now();
    let hello = settings_codec::encode(&Message::Placement(Frame::Hello {
        epoch: 7,
        schema: schema(),
    }))
    .unwrap();
    assert!(
        Connection::default()
            .receive_placement(&hello, now)
            .is_none()
    );
    let mut books = CustomBooks::default();
    for message in [Message::Hello, Message::Offer(vec!["demo:book".into()])] {
        assert!(
            connection
                .receive(
                    &settings_codec::encode(&message).unwrap(),
                    &mut books,
                    || ["demo:book".into()].into(),
                    || 7,
                    now
                )
                .is_some()
        );
    }
    for _ in 0..17 {
        assert!(matches!(
            connection.receive_placement(&hello, now),
            Some(Frame::Hello { epoch: 7, .. })
        ));
    }
    let wrong_direction = settings_codec::encode(&Message::Placement(Frame::Ready {
        epoch: 7,
        schema: schema(),
    }))
    .unwrap();
    assert!(
        connection
            .receive_placement(&wrong_direction, now)
            .is_none()
    );
    assert!(connection.receive_placement(&hello, now).is_none());
    let reply = connection
        .receive(
            &settings_codec::encode(&Message::Resnapshot).unwrap(),
            &mut books,
            Default::default,
            || 8,
            now,
        )
        .unwrap();
    assert_eq!(reply.message, Message::Busy);
    assert!(
        connection
            .receive_placement(&hello, now + std::time::Duration::from_secs(1))
            .is_some()
    );
    assert_eq!(connection.authority.as_ref().unwrap().epoch, 7);
}
