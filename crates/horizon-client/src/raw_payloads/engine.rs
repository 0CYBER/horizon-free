//! Engine declarations in the negotiated payload table (arismoko/horizon#813 step 4a).
//!
//! An engine service that sends on a registered channel declares it here, so its channel is
//! admitted, refused and epoch-bound exactly like a mod's declaration: it is part of every
//! Horizon installation's Offer, and a peer that does not offer it (a vanilla peer, or a
//! Horizon build without the service) has it absent, so a send returns `UnsupportedPeer`
//! without a packet. Every engine declaration is optional: a missing one never refuses a join.
//!
//! The receiving side is the engine's own decoder, not a mod callback: a negotiated incoming
//! engine message is handed on as an ordinary custom payload (`Classified::Other`). The channel
//! stays reserved, so no mod may declare it (`Local::capture`), and the generic packet API
//! still refuses it.
use horizon_protocol::{
    attachment_sync,
    payload_negotiation::{Error, Flow, ManifestBuilder, Phase, RegistrationPhase},
};

/// One engine declaration: phase, channel, flow and exact version.
pub(super) struct Declaration {
    pub phase: Phase,
    pub id: &'static str,
    pub flow: Flow,
    pub version: &'static str,
}

/// Every engine declaration. Attachment sync: play, clientbound only.
pub(super) const DECLARATIONS: &[Declaration] = &[Declaration {
    phase: Phase::Play,
    id: attachment_sync::CHANNEL,
    flow: Flow::ToClient,
    version: attachment_sync::VERSION,
}];

/// Add the engine declarations to an installation's manifest, as optional entries.
pub(super) fn register(manifest: &mut ManifestBuilder) -> Result<(), Error> {
    for declaration in DECLARATIONS {
        let phase = match declaration.phase {
            Phase::Configuration => RegistrationPhase::Configuration,
            Phase::Play => RegistrationPhase::Play,
        };
        manifest.register(
            phase,
            declaration.id,
            declaration.flow,
            declaration.version,
            true,
        )?;
    }
    Ok(())
}

/// Whether `transport`'s current epoch is in play with engine channel `id` negotiated: the
/// test an engine service uses in place of the peer's brand.
pub fn negotiated(transport: &super::Transport, id: &str) -> bool {
    transport
        .epoch()
        .is_some_and(|epoch| epoch.phase == Phase::Play && epoch.peer.engine_sends(Phase::Play, id))
}

/// Whether `id` in `phase` is an engine declaration.
pub fn is_engine(phase: Phase, id: &str) -> bool {
    DECLARATIONS
        .iter()
        .any(|declaration| declaration.phase == phase && declaration.id == id)
}
