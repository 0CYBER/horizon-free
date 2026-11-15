//! A typed declaration's schema on the negotiated route: the real registrar's manifest against a
//! peer whose value type has another shape.
use super::{Local, Side};
use horizon_protocol::payload_negotiation::{
    self as negotiation, Control, Flow, Manifest, Mismatch, Negotiation, Phase, RegistrationPhase,
};
use mod_api::{
    Hooks, IPayloadContext, ModEntry, PayloadCodec, PayloadId, PayloadSendError, Registrar, VarInt,
};

#[derive(PayloadCodec)]
struct Counter {
    counter: VarInt,
}
/// What an older peer sends on the same ids: a fixed-width number.
#[derive(PayloadCodec)]
struct OldCounter {
    counter: i32,
}

struct Typed;
#[mod_api::__subscriptions]
impl Hooks for Typed {
    fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let ignore = |_: Counter, _: &mut dyn IPayloadContext| Ok(());
        let mut payloads = registrar.payload_handlers();
        payloads
            .registrar("1")
            .play_to_server_typed(PayloadId("typed_schema:required".into()), ignore);
        payloads
            .registrar("1")
            .optional()
            .play_to_server_typed(PayloadId("typed_schema:optional".into()), ignore);
    }
}

fn peer(optional_too: bool) -> Manifest {
    let old = mod_api::typed_payload_version("1", mod_api::payload_schema::<OldCounter>());
    let current = mod_api::typed_payload_version("1", mod_api::payload_schema::<Counter>());
    let mut builder = Manifest::builder();
    let required = if optional_too { &current } else { &old };
    builder
        .register(
            RegistrationPhase::Play,
            "typed_schema:required",
            Flow::ToServer,
            required,
            false,
        )
        .unwrap();
    builder
        .register(
            RegistrationPhase::Play,
            "typed_schema:optional",
            Flow::ToServer,
            &old,
            true,
        )
        .unwrap();
    builder.finish().unwrap()
}

#[test]
fn another_value_schema_is_refused_when_required_and_unsupported_when_optional() {
    assert_eq!(negotiation::SCHEMA_MARKER, mod_api::PAYLOAD_SCHEMA_MARKER);
    mod_platform::testing::with_mods(
        vec![ModEntry::new("typed_schema", Box::new(Typed))].into(),
        || {
            mod_platform::__private::part_tests::install_parts_fixture(mod_api::__private::mods())
                .unwrap();
            let local = Local::capture(Side::Client).unwrap();
            // Required schema 2 against the peer's schema 1: admission names both versions.
            let refused = Negotiation::new(local.manifest.clone())
                .receive(Control::Offer(peer(false)))
                .unwrap_err();
            let negotiation::Error::Incompatible {
                id,
                reason,
                local: ours,
                peer: theirs,
                ..
            } = refused
            else {
                panic!("{refused}");
            };
            assert_eq!(
                (id.as_str(), reason),
                ("typed_schema:required", Mismatch::Schema)
            );
            assert_ne!(ours.unwrap().0, theirs.unwrap().0);
            // The optional one with another schema is absent: a send is UnsupportedPeer.
            let mut admitted = Negotiation::new(local.manifest.clone());
            admitted.receive(Control::Offer(peer(true))).unwrap();
            let accepted = admitted.accepted().unwrap();
            let ids: Vec<_> = accepted.iter().map(|entry| entry.id()).collect();
            assert_eq!(ids, ["typed_schema:required"]);
            let table = local.freeze(accepted, Side::Client).unwrap();
            let optional = PayloadId("typed_schema:optional".into());
            assert_eq!(
                table.can_send(Phase::Play, &optional),
                Err(PayloadSendError::UnsupportedPeer(optional.clone()))
            );
        },
    );
}
