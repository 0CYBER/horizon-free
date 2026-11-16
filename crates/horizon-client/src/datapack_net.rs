//! The registries of mods' datapack registries on the wire (`horizon:datapack_registries`,
//! arismoko/horizon#811, stage 4): at the join, a Horizon client offers the synchronized
//! registries it declared, with their schemas (`horizon:datapack_offer`), and the server answers
//! with a snapshot of the ones both sides have, before any registry data. A required registry
//! the client did not offer ends the join with a reason; an optional one is left out and
//! nothing is sent in its place. The client checks the snapshot against its own declarations
//! before it keeps anything, and a snapshot that fails ends the join. The set a server last
//! published and the set a client received are held here, for the lan thread and the game.
//!
//! A `/reload` that publishes the next generation sends each joined Horizon client a new snapshot
//! on the same channel, built from the offer owned by its live connection. The
//! client checks it as at the join and swaps its set in one step, or ends the connection. See
//! `docs/design/mod-api/mod-datapack-registries.md`.
use std::sync::{Arc, Mutex, atomic::AtomicU64};

/// The snapshot, server to client.
pub const CHANNEL: &str = "horizon:datapack_registries";
/// The client's offer, client to server.
pub const OFFER_CHANNEL: &str = "horizon:datapack_offer";

type Held = Mutex<Option<Arc<mod_api::DatapackRegistrySet>>>;
/// What this process's server last published.
static SERVED: Held = Mutex::new(None);
/// What this process's client last received from a server.
static RECEIVED: Held = Mutex::new(None);

#[cfg(feature = "test-publication-scope")]
thread_local! {
    static SCOPED_SERVED: std::cell::RefCell<Option<Option<Arc<mod_api::DatapackRegistrySet>>>> = const { std::cell::RefCell::new(None) };
}

/// Isolates this thread's publication for resource-backed game fixtures.
#[cfg(feature = "test-publication-scope")]
pub fn isolate_server_publication() -> impl Drop {
    struct Scope(
        Option<Option<Arc<mod_api::DatapackRegistrySet>>>,
        std::marker::PhantomData<std::rc::Rc<()>>,
    );
    impl Drop for Scope {
        fn drop(&mut self) {
            SCOPED_SERVED.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    Scope(
        SCOPED_SERVED.with(|slot| slot.replace(Some(None))),
        std::marker::PhantomData,
    )
}
static SESSION: AtomicU64 = AtomicU64::new(1);
/// The snapshot of the set now served for `player`, from the offer they made at the join, when
/// there is one to send.
pub fn reload_payload(offer: Option<&[u8]>) -> Step {
    server_step(true, offer)
}

/// A new id for a world session's set, whether this process published it or received it, so
/// no two sets of one process share a session and a handle never reads another world's entry.
pub fn next_session() -> u64 {
    SESSION.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

fn held(slot: &Held) -> Option<Arc<mod_api::DatapackRegistrySet>> {
    slot.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn put(slot: &Held, set: Option<Arc<mod_api::DatapackRegistrySet>>) {
    *slot.lock().unwrap_or_else(|e| e.into_inner()) = set;
}

/// Keeps the set a server just published, for the clients that join it.
pub fn serve(set: Option<Arc<mod_api::DatapackRegistrySet>>) {
    #[cfg(feature = "test-publication-scope")]
    if SCOPED_SERVED.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some(local) = slot.as_mut() {
            *local = set.clone();
            true
        } else {
            false
        }
    }) {
        return;
    }
    put(&SERVED, set);
}

/// The set this process's server last published.
pub fn served() -> Option<Arc<mod_api::DatapackRegistrySet>> {
    #[cfg(feature = "test-publication-scope")]
    if let Some(local) = SCOPED_SERVED.with(|slot| slot.borrow().clone()) {
        return local;
    }
    held(&SERVED)
}

/// The set this process's client received from the server it joined, if the server sent one.
pub fn received() -> Option<Arc<mod_api::DatapackRegistrySet>> {
    held(&RECEIVED)
}

/// Forgets what an earlier join received, as a new join starts.
pub fn clear_received() {
    put(&RECEIVED, None);
    clear_transfer();
}

/// What the client sends its server: its synchronized registries and their schemas, or `None`
/// when it declared none.
pub fn offer() -> Option<Vec<u8>> {
    let declarations = mod_api::__private::datapack_registry_declarations();
    declarations
        .iter()
        .any(|declaration| declaration.sync != mod_api::DatapackRegistrySync::ServerOnly)
        .then(|| mod_api::DatapackSnapshot::encode_offer(declarations))
        .flatten()
}

/// What the server does for a joining client.
#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    /// Nothing: no mod declared a synchronized registry, or none applies to this client.
    Nothing,
    /// Send these bytes on [`CHANNEL`], before the registry data.
    Send(Vec<u8>),
    /// End the join with this reason, before any payload.
    Refuse(String),
}

/// The server's step for a client of the given brand, with the offer it sent if it sent one.
pub fn server_step(horizon_client: bool, offer: Option<&[u8]>) -> Step {
    let declarations = mod_api::__private::datapack_registry_declarations();
    step_for(declarations, served().as_deref(), horizon_client, offer)
}

fn step_for(
    declarations: &[mod_api::DatapackRegistryDeclaration],
    set: Option<&mod_api::DatapackRegistrySet>,
    horizon_client: bool,
    offer: Option<&[u8]>,
) -> Step {
    use mod_api::DatapackRegistrySync;
    if declarations
        .iter()
        .all(|declaration| declaration.sync == DatapackRegistrySync::ServerOnly)
    {
        return Step::Nothing;
    }
    let Some(set) = set else {
        return Step::Nothing;
    };
    // A vanilla client, or a Horizon one that sent no offer, has no registries to match.
    let offered = match (horizon_client, offer) {
        (true, Some(bytes)) => match mod_api::DatapackSnapshot::decode_offer(bytes) {
            Ok(offered) => offered,
            Err(_) => return Step::Refuse("The client's mod registries are unreadable.".into()),
        },
        _ => Vec::new(),
    };
    match set.snapshot_for(declarations, &offered) {
        Err(mod_api::DatapackNegotiationError::RequiredRegistry { registry, mod_id }) => {
            Step::Refuse(format!(
                "Install the mod {mod_id} of the server: it needs the registry {registry}."
            ))
        }
        Err(_) => Step::Refuse("The client's mod registries are unreadable.".into()),
        Ok(snapshot) if snapshot.registries.is_empty() => Step::Nothing,
        Ok(snapshot) => match snapshot.encode() {
            Some(bytes) => Step::Send(bytes),
            None => Step::Refuse(
                "The server's datapack registry snapshot exceeds the 64 MiB transport limit."
                    .into(),
            ),
        },
    }
}

/// The client's side of a snapshot: checks it against what the client declared and keeps it as
/// the set its mods read, or says why the join must end. Nothing is kept on failure.
pub fn accept(bytes: &[u8]) -> Result<(), String> {
    let declarations = mod_api::__private::datapack_registry_declarations();
    let received = receive(declarations, received().as_deref(), bytes)?;
    put(&RECEIVED, Some(Arc::new(received)));
    Ok(())
}

/// A snapshot after the first of a join keeps the session, so a handle from the generation
/// before fails as stale; one that does not come after the generation held is refused.
fn receive(
    declarations: &[mod_api::DatapackRegistryDeclaration],
    previous: Option<&mod_api::DatapackRegistrySet>,
    bytes: &[u8],
) -> Result<mod_api::DatapackRegistrySet, String> {
    let refused = |error: mod_api::DatapackSnapshotError| {
        format!("The server's mod registries do not match this client's: {error:?}")
    };
    let snapshot = mod_api::DatapackSnapshot::decode(bytes).map_err(refused)?;
    let candidate = snapshot.candidate(declarations).map_err(refused)?;
    if let Some(previous) = previous {
        return previous
            .received_next(&snapshot, candidate)
            .ok_or_else(|| "The server's mod registries arrived out of order.".to_owned());
    }
    let session = next_session();
    Ok(mod_api::DatapackRegistrySet::received(
        &snapshot, candidate, session,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::{
        DatapackCandidate, DatapackField, DatapackFieldKind, DatapackRegistryDeclaration,
        DatapackRegistrySet, DatapackRegistrySync,
    };

    fn spell(sync: DatapackRegistrySync) -> DatapackRegistryDeclaration {
        DatapackRegistryDeclaration {
            mod_id: "example".into(),
            key: "example:spell".into(),
            fields: vec![DatapackField::required(
                "power",
                DatapackFieldKind::Int { min: 0, max: 100 },
            )],
            sync,
        }
    }

    fn set(declaration: &DatapackRegistryDeclaration) -> DatapackRegistrySet {
        let files = vec![(
            "data/addon/example/spell/fire.json".to_owned(),
            vec![("pack".to_owned(), br#"{"power": 7}"#.to_vec())],
        )];
        DatapackRegistrySet::first(
            DatapackCandidate::build(std::slice::from_ref(declaration), &files).unwrap(),
            1,
        )
    }

    #[test]
    fn a_client_with_the_mod_receives_the_registry_and_reads_it() {
        let declaration = spell(DatapackRegistrySync::Required);
        let declarations = std::slice::from_ref(&declaration);
        let server = set(&declaration);
        let offer = mod_api::DatapackSnapshot::encode_offer(declarations).unwrap();
        let Step::Send(bytes) = step_for(declarations, Some(&server), true, Some(&offer)) else {
            panic!("a snapshot is sent");
        };
        let client = receive(declarations, None, &bytes).unwrap();
        let handle = client.handle("example:spell", "addon:fire").unwrap();
        assert_eq!(
            client.get(&handle).unwrap().get("power"),
            Some(&mod_api::DatapackValue::Int(7))
        );
        // Truncated bytes are refused and nothing is kept.
        assert!(receive(declarations, None, &bytes[..bytes.len() - 1]).is_err());
    }

    #[test]
    fn a_reload_snapshot_replaces_the_set_under_the_same_session() {
        let declaration = spell(DatapackRegistrySync::Required);
        let declarations = std::slice::from_ref(&declaration);
        let first = set(&declaration);
        let offer = mod_api::DatapackSnapshot::encode_offer(declarations).unwrap();
        let send = |server: &DatapackRegistrySet| {
            let Step::Send(bytes) = step_for(declarations, Some(server), true, Some(&offer)) else {
                panic!("a snapshot is sent");
            };
            bytes
        };
        let joined = receive(declarations, None, &send(&first)).unwrap();
        let held = joined.handle("example:spell", "addon:fire").unwrap();
        let reloaded = first
            .reload(
                DatapackCandidate::build(
                    declarations,
                    &[(
                        "data/addon/example/spell/fire.json".to_owned(),
                        vec![("pack".to_owned(), br#"{"power": 9}"#.to_vec())],
                    )],
                )
                .unwrap(),
            )
            .unwrap();
        let bytes = send(&reloaded);
        let next = receive(declarations, Some(&joined), &bytes).unwrap();
        assert_eq!(next.generation(), 2);
        // The handle from generation 1 is stale, not from another session.
        assert_eq!(
            next.get(&held),
            Err(mod_api::DatapackReadError::StaleGeneration)
        );
        let fresh = next.handle("example:spell", "addon:fire").unwrap();
        assert_eq!(
            next.get(&fresh).unwrap().get("power"),
            Some(&mod_api::DatapackValue::Int(9))
        );
        // The same snapshot again, or an older one, changes nothing.
        assert!(receive(declarations, Some(&next), &bytes).is_err());
        assert!(receive(declarations, Some(&next), &send(&first)).is_err());
    }

    #[test]
    fn a_reload_cannot_change_which_registries_a_client_is_sent() {
        let declaration = spell(DatapackRegistrySync::Required);
        let declarations = std::slice::from_ref(&declaration);
        let first = set(&declaration);
        let reloaded = first
            .reload(DatapackCandidate::build(declarations, &[]).unwrap())
            .unwrap();
        let offer = mod_api::DatapackSnapshot::decode_offer(
            &mod_api::DatapackSnapshot::encode_offer(declarations).unwrap(),
        )
        .unwrap();
        let shape = |set: &DatapackRegistrySet| {
            let snapshot = set.snapshot_for(declarations, &offer).unwrap();
            snapshot
                .registries
                .iter()
                .map(|registry| registry.key.clone())
                .collect::<Vec<_>>()
        };
        // The entries go, the registry (and so the negotiation) stays.
        assert_eq!(shape(&first), shape(&reloaded));
        assert_eq!(shape(&reloaded), vec!["example:spell".to_owned()]);
    }

    #[test]
    fn a_required_registry_refuses_a_client_that_lacks_it() {
        let declaration = spell(DatapackRegistrySync::Required);
        let declarations = std::slice::from_ref(&declaration);
        let server = set(&declaration);
        for (horizon, offer) in [(false, None), (true, None)] {
            let Step::Refuse(reason) = step_for(declarations, Some(&server), horizon, offer) else {
                panic!("the join is refused");
            };
            assert!(reason.contains("example") && reason.contains("example:spell"));
        }
        // Bytes that are not an offer are refused too.
        assert!(matches!(
            step_for(declarations, Some(&server), true, Some(&[1, 2, 3])),
            Step::Refuse(_)
        ));
    }

    #[test]
    fn optional_and_server_only_registries_never_end_a_join() {
        let optional = spell(DatapackRegistrySync::Optional);
        assert_eq!(
            step_for(
                std::slice::from_ref(&optional),
                Some(&set(&optional)),
                false,
                None
            ),
            Step::Nothing
        );
        let only = spell(DatapackRegistrySync::ServerOnly);
        assert_eq!(
            step_for(std::slice::from_ref(&only), Some(&set(&only)), true, None),
            Step::Nothing
        );
        // No world published yet: nothing to send.
        assert_eq!(
            step_for(std::slice::from_ref(&optional), None, true, None),
            Step::Nothing
        );
    }
}

#[path = "datapack_delivery.rs"]
mod delivery;
pub use delivery::reload_connection;

#[path = "datapack_transport.rs"]
mod transport;
pub use transport::{accept_chunk, chunks, clear_transfer, finish_transfer};
