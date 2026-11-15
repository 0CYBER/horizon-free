//! Scoped real registration/classification for resource-free actor routing tests.
//! This supplies application handlers, never a fake payload context or World.
use super::{Classified, Delivery, Endpoint, Incoming, Side, Transport};
use mod_api::{
    Hooks, IPayloadContext, IPayloadHandler, ModEntry, PayloadBytes, PayloadId, Registrar,
};
use std::sync::Arc;
pub(crate) const CHANNEL: &str = "raw_hub_probe:message";
struct Probe;
impl IPayloadHandler for Probe {
    fn handle(
        &self,
        _: PayloadBytes,
        _: &mut dyn IPayloadContext,
    ) -> Result<(), mod_api::PayloadHandleError> {
        Ok(())
    }
}
#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        registrar
            .payload_handlers()
            .registrar("1")
            .play_to_server(PayloadId(CHANNEL.into()), Arc::new(Probe));
    }
}
/// Keep the real owning mod enabled for the entire actor test, not just setup.
pub(crate) fn with_peer<R>(run: impl FnOnce(Arc<Transport>) -> R) -> R {
    mod_platform::testing::with_mods(
        vec![ModEntry::new("raw_hub_probe", Box::new(Probe))].into(),
        || {
            // with_mods installs callback ownership, not the load-owned Parts table.
            // Use the real registrar/installer before freezing this connection.
            mod_platform::__private::part_tests::install_parts_fixture(mod_api::__private::mods())
                .unwrap();
            let transport = Transport::capture(Side::Server).unwrap();
            assert!(
                transport.local.owns(CHANNEL),
                "fixture declaration was not installed"
            );
            transport
                .accept(transport.local.manifest.entries())
                .unwrap();
            transport.finish_configuration().unwrap();
            run(transport)
        },
    )
}
pub(crate) fn delivery(
    conn: u32,
    sender: crate::lan::PlayerSender,
    bytes: Vec<u8>,
) -> Box<Delivery> {
    let classified = Incoming::classify(
        sender.payload_transport(),
        horizon_protocol::payload_negotiation::Phase::Play,
        horizon_protocol::packets::configuration::CustomPayload {
            channel: CHANNEL.into(),
            data: bytes,
        },
    )
    .unwrap();
    let Classified::Application(incoming) = classified else {
        panic!("fixture registration must classify as application");
    };
    Box::new(Delivery {
        conn: Some(conn),
        endpoint: Endpoint::server(sender),
        incoming,
    })
}
