use super::*;
fn id(value: &str) -> ResourceId {
    value.to_owned().try_into().unwrap()
}
fn window(nonce: u64) -> Window {
    Window {
        window: 7,
        nonce,
        menu: id("demo:pot"),
        book: id("demo:book"),
    }
}
fn open(client: &mut Client) -> CatalogIdentity {
    let Frame::Catalog {
        epoch,
        number,
        window,
        nonce,
        menu,
        book,
    } = client.open(window(8)).unwrap()
    else {
        panic!("catalog request")
    };
    CatalogIdentity {
        epoch,
        number,
        window,
        nonce,
        instance: 3,
        generation: 5,
        menu,
        book,
    }
}
fn client() -> Client {
    let mut client = Client::default();
    client.hello(11);
    assert!(client.ready(11, schema()));
    client
}
fn layout() -> mod_api::RecipeBookLayout {
    mod_api::RecipeBookLayout::Furnace {
        input: 0,
        fuel: 1,
        result: 2,
    }
}
fn complete(client: &mut Client, identity: &CatalogIdentity) {
    client.start(identity.clone(), layout(), 4).unwrap();
    assert!(
        client
            .part(identity.epoch, identity.number, 0, vec![0; 4])
            .unwrap()
            .is_some()
    );
    assert!(client.catalog().is_none());
    assert!(client.accept_catalog(identity.clone()));
}
fn display() -> DisplayId {
    DisplayId {
        recipe: id("demo:soup"),
        source: 2,
    }
}
fn select(client: &mut Client) -> Selection {
    let Frame::Place { selection, .. } = client.select(display(), false).unwrap() else {
        panic!("placement request")
    };
    selection
}
#[test]
fn negotiation_and_repeated_hello_preserve_monotonic_requests() {
    let mut client = Client::default();
    client.hello(11);
    assert!(!client.ready(12, schema()));
    assert!(!client.ready(11, schema() ^ 1));
    assert!(client.open(window(8)).is_none());
    assert!(client.ready(11, schema()));
    let first = open(&mut client);
    client.hello(11);
    let second = open(&mut client);
    assert!(second.number > first.number);
    let mut authority = Authority::new(11);
    assert_eq!(authority.hello(12, schema()), Err(Refusal::Stale));
    assert_eq!(authority.hello(11, schema() ^ 1), Err(Refusal::Unsupported));
    authority.hello(11, schema()).unwrap();
    authority.publish(first.clone()).unwrap();
    authority.hello(11, schema()).unwrap();
    assert_eq!(authority.publish(first), Err(Refusal::Stale));
}
#[test]
fn catalogue_authority_requires_a_completed_current_response() {
    let mut client = client();
    let identity = open(&mut client);
    assert!(!client.accept_catalog(identity.clone()));
    let mut wrong = identity.clone();
    wrong.instance = 0;
    assert_eq!(client.start(wrong, layout(), 4), Err(Refusal::Stale));
    let mut wrong = identity.clone();
    wrong.nonce += 1;
    assert_eq!(client.start(wrong, layout(), 4), Err(Refusal::Stale));
    complete(&mut client, &identity);
    let current = open(&mut client);
    assert!(!client.accept_catalog(identity));
    complete(&mut client, &current);
    assert_eq!(client.catalog(), Some(&current));
}
#[test]
fn bounded_fixed_fragments_cannot_publish_partial_or_reordered_data() {
    let mut client = client();
    let identity = open(&mut client);
    assert_eq!(
        client.start(identity.clone(), layout(), (MAX_CATALOG_BYTES + 1) as u32),
        Err(Refusal::TooLarge)
    );
    let data: Vec<_> = (0..CHUNK_BYTES + 5).map(|i| (i % 251) as u8).collect();
    client
        .start(identity.clone(), layout(), data.len() as u32)
        .unwrap();
    assert_eq!(
        client.start(identity.clone(), layout(), data.len() as u32),
        Err(Refusal::Invalid)
    );
    assert!(matches!(
        client.part(identity.epoch, identity.number, 1, vec![0]),
        Err(Refusal::Invalid)
    ));
    assert!(matches!(
        client.part(identity.epoch, identity.number, 0, vec![0]),
        Err(Refusal::Invalid)
    ));
    assert!(
        client
            .part(
                identity.epoch,
                identity.number,
                0,
                data[..CHUNK_BYTES].to_vec()
            )
            .unwrap()
            .is_none()
    );
    assert!(client.catalog().is_none());
    assert!(matches!(
        client.part(
            identity.epoch,
            identity.number - 1,
            CHUNK_BYTES as u32,
            vec![0; 5]
        ),
        Err(Refusal::Stale)
    ));
    let result = client
        .part(
            identity.epoch,
            identity.number,
            CHUNK_BYTES as u32,
            data[CHUNK_BYTES..].to_vec(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(result.identity, identity);
    assert_eq!(result.bytes, data);
    assert!(client.catalog().is_none());
    assert!(client.accept_catalog(result.identity));
}
#[test]
fn only_the_current_missing_response_can_install_a_ghost_once() {
    let mut client = client();
    let identity = open(&mut client);
    complete(&mut client, &identity);
    let old = select(&mut client);
    let current = select(&mut client);
    assert_eq!(client.reply(&old, Outcome::MissingIngredients), None);
    let mut wrong = current.clone();
    wrong.catalog.generation += 1;
    assert_eq!(client.reply(&wrong, Outcome::MissingIngredients), None);
    let mut wrong = current.clone();
    wrong.display.source += 1;
    assert_eq!(client.reply(&wrong, Outcome::MissingIngredients), None);
    assert_eq!(
        client.reply(&current, Outcome::MissingIngredients),
        Some(Outcome::MissingIngredients)
    );
    assert_eq!(client.reply(&current, Outcome::MissingIngredients), None);
    let current = select(&mut client);
    assert_eq!(
        client.reply(&current, Outcome::Placed),
        Some(Outcome::Placed)
    );
    let current = select(&mut client);
    assert_eq!(
        client.reply(&current, Outcome::Refused(Refusal::Denied)),
        Some(Outcome::Refused(Refusal::Denied))
    );
}
#[test]
fn click_close_reload_and_reconnect_invalidate_pending_ghost_replies() {
    let mut client = client();
    let identity = open(&mut client);
    complete(&mut client, &identity);
    let pending = select(&mut client);
    client.click();
    assert_eq!(client.reply(&pending, Outcome::MissingIngredients), None);
    let pending = select(&mut client);
    client.close();
    assert_eq!(client.reply(&pending, Outcome::MissingIngredients), None);
    let current = open(&mut client);
    complete(&mut client, &current);
    let pending = select(&mut client);
    assert!(client.invalidate(11));
    assert_eq!(client.reply(&pending, Outcome::MissingIngredients), None);
    let current = open(&mut client);
    complete(&mut client, &current);
    let pending = select(&mut client);
    client.hello(12);
    assert_eq!(client.reply(&pending, Outcome::MissingIngredients), None);
    assert!(client.catalog().is_none());
    assert!(!client.invalidate(11));
}
#[test]
fn server_replay_guard_binds_epoch_instance_generation_and_catalogue() {
    let mut client = client();
    let identity = open(&mut client);
    complete(&mut client, &identity);
    let mut authority = Authority::new(11);
    authority.hello(11, schema()).unwrap();
    authority.publish(identity.clone()).unwrap();
    let current = select(&mut client);
    let mut stale = current.clone();
    stale.catalog.instance += 1;
    assert_eq!(authority.place(&stale), Err(Refusal::Stale));
    let mut stale = current.clone();
    stale.catalog.generation += 1;
    assert_eq!(authority.place(&stale), Err(Refusal::Stale));
    authority.place(&current).unwrap();
    assert_eq!(authority.place(&current), Err(Refusal::Stale));
    authority.clear();
    assert_eq!(authority.place(&select(&mut client)), Err(Refusal::Stale));
    authority.hello(11, schema()).unwrap();
    assert_eq!(authority.publish(identity), Err(Refusal::Stale));
}
#[test]
fn placement_frames_share_the_bounded_settings_channel_without_vanilla_ids() {
    let mut client = client();
    let identity = open(&mut client);
    complete(&mut client, &identity);
    let selection = select(&mut client);
    let frame = Frame::Reply {
        selection,
        outcome: Outcome::MissingIngredients,
    };
    let message = super::super::settings_sync::Message::Placement(frame);
    let mut bytes = super::super::settings_codec::encode(&message).unwrap();
    assert_eq!(bytes[0], 8);
    assert_eq!(super::super::settings_codec::decode(&bytes), Some(message));
    bytes.push(0);
    assert!(super::super::settings_codec::decode(&bytes).is_none());
    assert!(ResourceId::try_from(format!("demo:{}", "a".repeat(RESOURCE_ID_BYTES - 5))).is_ok());
    assert!(ResourceId::try_from(format!("demo:{}", "a".repeat(RESOURCE_ID_BYTES - 4))).is_err());
    assert!(ResourceId::try_from("demo:bad path".to_owned()).is_err());
}

#[test]
fn invalidation_also_revokes_a_completed_but_unpublished_catalogue() {
    let mut client = client();
    let identity = open(&mut client);
    client.start(identity.clone(), layout(), 4).unwrap();
    assert!(
        client
            .part(identity.epoch, identity.number, 0, vec![0; 4])
            .unwrap()
            .is_some()
    );
    assert!(client.invalidate(identity.epoch));
    assert!(!client.accept_catalog(identity));
}
#[test]
fn catalogue_refusals_cannot_cancel_a_newer_request_or_completed_view() {
    let mut client = client();
    let old = open(&mut client);
    let current = open(&mut client);
    assert!(!client.refused(old.epoch, old.number, old.window, old.nonce));
    assert!(!client.refused(
        current.epoch,
        current.number,
        current.window,
        current.nonce + 1
    ));
    complete(&mut client, &current);
    assert!(!client.refused(current.epoch, current.number, current.window, current.nonce));
    assert!(!client.refused(current.epoch, 0, 0, 0));
    assert_eq!(client.catalog(), Some(&current));
    let current = open(&mut client);
    assert!(client.refused(current.epoch, current.number, current.window, current.nonce));
    assert_eq!(client.start(current, layout(), 4), Err(Refusal::Stale));
}
