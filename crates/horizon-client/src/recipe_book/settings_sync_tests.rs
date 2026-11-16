use super::*;
use crate::recipe_book::settings_codec::{decode, encode};
fn flags(open: bool, filtering: bool) -> BookFlags {
    BookFlags { open, filtering }
}
fn registered() -> BTreeSet<String> {
    ["demo:pot", "demo:oven"].map(str::to_owned).into()
}
#[test]
fn authoritative_screen_pair_reconnect_and_pending_intent() {
    let mut saved = CustomBooks::default();
    let mut server = Authority::negotiate(7, registered().into_iter().collect(), &registered());
    let mut client = ClientView::default();
    assert!(client.receive(server.snapshot(&saved), &registered()));
    assert_eq!(client.books.get("demo:pot"), flags(false, false));
    let now = Instant::now();
    let (reply, changed) = server.request(&mut saved, 7, 1, "demo:pot", flags(true, false), now);
    assert!(changed);
    assert!(client.receive(reply, &registered()));
    let (reply, changed) = server.request(&mut saved, 7, 2, "demo:pot", flags(true, true), now);
    assert!(changed);
    assert!(client.receive(reply, &registered()));
    assert_eq!(saved.get("demo:pot"), flags(true, true));
    assert_eq!(saved.get("demo:oven"), flags(false, false));
    // A pending request is not passed to the authority and never reaches saved state.
    let _pending = Message::Request {
        epoch: 7,
        number: 3,
        key: "demo:pot".into(),
        flags: flags(false, true),
    };
    let reconnected = Authority::negotiate(8, registered().into_iter().collect(), &registered());
    let mut next = ClientView::default();
    assert!(next.receive(reconnected.snapshot(&saved), &registered()));
    assert_eq!(next.books.get("demo:pot"), flags(true, true));
    let (reply, changed) = server.request(&mut saved, 6, 99, "demo:pot", flags(false, false), now);
    assert!(!changed);
    assert!(matches!(reply, Message::Snapshot { .. }));
    assert!(
        !server
            .request(&mut saved, 7, 2, "demo:pot", flags(false, false), now)
            .1
    );
    assert!(
        !server
            .request(&mut saved, 7, 4, "unknown:book", flags(true, true), now)
            .1
    );
    assert_eq!(saved.get("demo:pot"), flags(true, true));
}
#[test]
fn rejects_duplicates_unknown_rate_excess_and_gaps() {
    let mut books = CustomBooks::default();
    books.set("absent:book", flags(true, false)).unwrap();
    let mut server = Authority::negotiate(
        1,
        vec!["demo:pot".into(), "absent:book".into()],
        &registered(),
    );
    let snapshot = server.snapshot(&books);
    assert!(
        matches!(&snapshot,Message::Snapshot { entries,.. } if entries==&vec![("demo:pot".into(),flags(false,false))])
    );
    let mut client = ClientView::default();
    assert!(client.receive(snapshot, &registered()));
    let now = Instant::now();
    for number in 1..=20 {
        assert!(
            !server
                .request(&mut books, 1, number, "demo:pot", flags(false, false), now)
                .1
        );
    }
    assert!(
        !server
            .request(&mut books, 1, 21, "demo:pot", flags(true, true), now)
            .1
    );
    assert_eq!(books.get("demo:pot"), flags(false, false));
    assert!(
        !server
            .request(
                &mut books,
                1,
                22,
                "demo:oven",
                flags(true, true),
                now + Duration::from_secs(1)
            )
            .1
    );
    assert!(!client.receive(
        Message::Update {
            epoch: 1,
            revision: 2,
            key: "demo:pot".into(),
            flags: flags(true, true)
        },
        &registered()
    ));
    assert!(!client.ready);
    assert_eq!(client.books.get("demo:pot"), flags(false, false));
    assert!(client.receive(server.snapshot(&books), &registered()));
}
#[test]
fn bounded_codec_refuses_malformed_without_mutation() {
    assert_eq!(
        encode(&Message::Offer(vec!["demo:pot".into()])),
        Some(vec![
            0, 0, 1, 0, 8, b'd', b'e', b'm', b'o', b':', b'p', b'o', b't'
        ])
    );
    assert_eq!(decode(&[4]), Some(Message::Resnapshot));
    let messages = [
        Message::Hello,
        Message::Ready,
        Message::Offer(vec!["demo:pot".into()]),
        Message::Snapshot {
            epoch: 4,
            revision: 0,
            entries: vec![("demo:pot".into(), flags(true, false))],
        },
        Message::Request {
            epoch: 4,
            number: 1,
            key: "demo:pot".into(),
            flags: flags(true, true),
        },
        Message::Update {
            epoch: 4,
            revision: 1,
            key: "demo:pot".into(),
            flags: flags(true, true),
        },
        Message::Resnapshot,
    ];
    for message in messages {
        let encoded = encode(&message).unwrap();
        assert_eq!(decode(&encoded), Some(message));
        for end in 0..encoded.len() {
            assert!(decode(&encoded[..end]).is_none());
        }
        let mut trailing = encoded.clone();
        trailing.push(0);
        assert!(decode(&trailing).is_none());
    }
    assert!(decode(&vec![0; MAX_BYTES + 1]).is_none());
    assert!(encode(&Message::Offer(vec!["demo:pot".into(); 1025])).is_none());
    let duplicate = Message::Snapshot {
        epoch: 1,
        revision: 0,
        entries: vec![("demo:pot".into(), flags(false, false)); 2],
    };
    assert!(decode(&encode(&duplicate).unwrap()).is_none());
    let mut malformed = encode(&Message::Request {
        epoch: 1,
        number: 1,
        key: "demo:pot".into(),
        flags: flags(false, false),
    })
    .unwrap();
    *malformed.last_mut().unwrap() = 2;
    assert!(decode(&malformed).is_none());
}

#[test]
fn payload_budget_bounds_all_snapshot_producing_routes() {
    let mut budget = RequestBudget::default();
    let now = Instant::now();
    for _ in 0..20 {
        assert!(budget.accept(now));
    }
    for _ in 0..100 {
        assert!(!budget.accept(now));
    }
    assert!(budget.accept(now + Duration::from_secs(1)));
    assert!(decode(&[5, 0]).is_none());
    assert!(decode(&[6, 2]).is_none());
}

#[test]
fn connected_replacement_removes_destination_only_flags() {
    let mut books = CustomBooks::default();
    books.set("demo:oven", flags(true, true)).unwrap();
    let mut server = Authority::negotiate(12, registered().into_iter().collect(), &registered());
    let mut client = ClientView::default();
    assert!(client.receive(server.snapshot(&books), &registered()));
    let mut imported = CustomBooks::default();
    imported.set("demo:pot", flags(true, false)).unwrap();
    books = imported;
    server.revision += 1;
    assert!(client.receive(server.snapshot(&books), &registered()));
    assert_eq!(client.books.get("demo:pot"), flags(true, false));
    assert_eq!(client.books.get("demo:oven"), flags(false, false));
    assert!(!client.receive(
        Message::Update {
            epoch: 12,
            revision: 1,
            key: "demo:oven".into(),
            flags: flags(true, true)
        },
        &registered()
    ));
    assert_eq!(client.books.get("demo:oven"), flags(false, false));
}
