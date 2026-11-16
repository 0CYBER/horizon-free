//! Deliver a replacement generation or end the connection with its refusal reason.
use super::*;
use crate::lan::conn::PlayerSender;
use horizon_protocol::packets::{configuration as cfg, play};

pub fn reload_connection(sender: &PlayerSender) {
    match reload_payload(sender.datapack_offer.as_deref()) {
        Step::Nothing => {}
        Step::Send(data) => match chunks(&data) {
            Ok(packets) => sender.send_many(packets.map(|data| {
                play::Clientbound::CustomPayload(cfg::CustomPayload {
                    channel: CHANNEL.into(),
                    data,
                })
                .encode()
            })),
            Err(reason) => {
                sender.disconnect(horizon_core::nbt::Tag::String(reason.as_str().into()))
            }
        },
        Step::Refuse(reason) => {
            sender.disconnect(horizon_core::nbt::Tag::String(reason.as_str().into()))
        }
    }
}
