//! Viewed levels, client side (arismoko/horizon#3716): the answer to a Horizon server's `horizon:levels` query in
//! configuration (`horizon_protocol::viewed_levels`), sent before the client acknowledges
//! `FinishConfiguration`. The server sends a client nothing more on the channel than its answer
//! allows; the game keeps a world for each level it opens (`game::viewed_worlds`).
use horizon_protocol::{packets::configuration as cfg, viewed_levels};

/// Levels besides its own this client can view: the other two of vanilla's three dimensions.
/// The game refuses an `open` past it.
pub const MAX_LEVELS: u8 = 2;

/// Whether `payload` is the server's viewed-levels query.
pub fn asked(payload: &cfg::CustomPayload) -> bool {
    payload.channel == viewed_levels::CHANNEL
}

/// The answer to the query `data`, or the reason that closes the connection.
pub fn answer(data: &[u8]) -> Result<cfg::Serverbound, String> {
    viewed_levels::read_query(data).map_err(|error| error.to_string())?;
    Ok(cfg::Serverbound::CustomPayload(cfg::CustomPayload {
        channel: viewed_levels::CHANNEL.into(),
        data: viewed_levels::answer(MAX_LEVELS),
    }))
}
