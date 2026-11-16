//! The menu-extra revision capability, server side. In configuration the server asks a client
//! whose brand is Horizon's which `horizon:menu_extra` payload layout it reads, and keeps the
//! answer for play (`PlayerSender::menu_extra_rev`). A vanilla client is never asked. A client
//! that does not answer before it acknowledges `FinishConfiguration`, or answers another
//! revision, is sent the legacy layout (a menu of a block, never one without).
//! An answer the server did not ask for, or a second one, closes the connection.
use horizon_protocol::{menu_extra, packets::configuration as cfg};

/// One configuration phase's exchange.
#[derive(Default)]
pub struct Capability {
    asked: bool,
    answered: bool,
    revision: u8,
}

impl Capability {
    /// The query for a Horizon-branded client.
    pub fn query(&mut self) -> cfg::CustomPayload {
        self.asked = true;
        cfg::CustomPayload {
            channel: menu_extra::CHANNEL.into(),
            data: menu_extra::query(),
        }
    }

    /// The client's answer, `data` after the channel.
    pub fn receive(&mut self, data: &[u8]) -> Result<(), String> {
        if !self.asked {
            return Err("menu extra revision: unasked answer".into());
        }
        if self.answered {
            return Err("menu extra revision: second answer".into());
        }
        self.answered = true;
        self.revision = menu_extra::read_answer(data).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// The payload revision the client reads: 0 until it answers.
    pub fn revision(&self) -> u8 {
        self.revision
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_is_kept_only_when_asked_and_once() {
        let mut cap = Capability::default();
        assert!(cap.receive(&menu_extra::answer()).is_err(), "unasked");
        let _ = cap.query();
        assert_eq!(cap.receive(&menu_extra::answer()).unwrap(), ());
        assert_eq!(cap.revision(), menu_extra::REVISION);
        assert!(cap.receive(&menu_extra::answer()).is_err(), "second");
    }

    #[test]
    fn another_revisions_answer_keeps_the_legacy_layout() {
        let mut cap = Capability::default();
        let _ = cap.query();
        let older = [1, menu_extra::REVISION + 1];
        assert_eq!(cap.receive(&older).unwrap(), ());
        assert_eq!(cap.revision(), 0);
    }

    /// What a client saw and what the server kept for play.
    #[derive(Debug, PartialEq)]
    struct Joined {
        /// The `horizon:menu_extra` revision query it was sent.
        asked: Option<Vec<u8>>,
        /// The revision kept for its play payloads, or the reason it was closed.
        kept: Result<u8, String>,
    }

    fn payload(channel: &str, data: Vec<u8>) -> Vec<u8> {
        cfg::Serverbound::CustomPayload(cfg::CustomPayload {
            channel: channel.into(),
            data,
        })
        .encode()
    }

    /// A client with `brand` joins a server and answers the revision query once it is asked.
    fn join(brand: &str, answer: Option<&[u8]>) -> Joined {
        use crate::lan::{Lan, NetConfig, NetEvent};
        use horizon_protocol::{
            buf::Writer,
            connection::Connection,
            crypto::offline_uuid,
            packets::{Empty, handshake, login},
            payload_negotiation::{self as negotiation, Control, Manifest},
        };
        use std::{
            sync::Arc,
            time::{Duration, Instant},
        };

        let config = NetConfig {
            announce: false,
            listen: false,
            ..NetConfig::lan(0, "Menu extra".into(), 8)
        };
        let lan = Lan::start_with(Arc::new(crate::lan::data::SyncData::empty()), config).unwrap();
        let mut c = Connection::local(lan.connect_local().unwrap());
        c.reader
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        for packet in [
            handshake::Serverbound::Intention(handshake::Intention {
                protocol: horizon_protocol::PROTOCOL_VERSION,
                host: "127.0.0.1".into(),
                port: 0,
                intent: handshake::LOGIN,
            })
            .encode(),
            login::Serverbound::Hello(login::ServerHello {
                name: "Guest".into(),
                uuid: offline_uuid("Guest"),
            })
            .encode(),
        ] {
            c.send(&packet).unwrap();
        }
        loop {
            let (id, body) = c.recv().unwrap();
            match login::Clientbound::decode(id, &body).unwrap() {
                login::Clientbound::Compression(p) => c.set_compression(p.threshold),
                login::Clientbound::LoginFinished(_) => break,
                other => panic!("unexpected login packet {other:?}"),
            }
        }
        c.send(&login::Serverbound::LoginAcknowledged(Empty).encode())
            .unwrap();
        let mut name = Writer::new();
        name.string(brand);
        c.send(&payload("minecraft:brand", name.data)).unwrap();
        if brand == "horizon" {
            // As a Horizon client's net thread: the raw payload protocol, with no mod payloads.
            let offer = Control::Offer(Manifest::builder().finish().unwrap());
            for control in [Control::Query, offer] {
                c.send(&payload(negotiation::CHANNEL, control.encode().unwrap()))
                    .unwrap();
            }
        }
        let reason =
            |d: cfg::Disconnect| horizon_protocol::text::plain(&d.reason, &Default::default());
        let mut asked = None;
        loop {
            let (id, body) = c.recv().unwrap();
            match cfg::Clientbound::decode(id, &body).unwrap() {
                cfg::Clientbound::CustomPayload(p) if p.channel == menu_extra::CHANNEL => {
                    asked = Some(p.data);
                    if let Some(data) = answer {
                        c.send(&payload(menu_extra::CHANNEL, data.to_vec()))
                            .unwrap();
                    }
                }
                cfg::Clientbound::SelectKnownPacks(p) => {
                    let _ = c.send(&cfg::Serverbound::SelectKnownPacks(p).encode());
                }
                cfg::Clientbound::Ping(p) => {
                    let _ = c.send(&cfg::Serverbound::Pong(p).encode());
                }
                cfg::Clientbound::Disconnect(d) => {
                    return Joined {
                        asked,
                        kept: Err(reason(d)),
                    };
                }
                cfg::Clientbound::FinishConfiguration(_) => {
                    let _ = c.send(&cfg::Serverbound::FinishConfiguration(Empty).encode());
                    break;
                }
                _ => {}
            }
        }
        // The server may still refuse the player after its acknowledgement.
        c.reader
            .set_read_timeout(Some(Duration::from_millis(20)))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            for receipt in lan.poll() {
                if let (NetEvent::Joined(j), _) = receipt.into_parts() {
                    return Joined {
                        asked,
                        kept: Ok(j.sender.menu_extra_rev),
                    };
                }
            }
            if let Ok((id, body)) = c.recv()
                && let Ok(cfg::Clientbound::Disconnect(d)) = cfg::Clientbound::decode(id, &body)
            {
                return Joined {
                    asked,
                    kept: Err(reason(d)),
                };
            }
        }
        panic!("the server neither refused the player nor handed it to play");
    }

    #[test]
    fn a_horizon_client_is_asked_and_its_revision_is_kept() {
        assert_eq!(
            join("horizon", Some(&[1, menu_extra::REVISION])),
            Joined {
                asked: Some(menu_extra::query()),
                kept: Ok(menu_extra::REVISION),
            }
        );
    }

    #[test]
    fn a_silent_horizon_client_keeps_the_legacy_layout() {
        assert_eq!(
            join("horizon", None),
            Joined {
                asked: Some(menu_extra::query()),
                kept: Ok(0),
            }
        );
    }

    #[test]
    fn a_vanilla_client_is_never_asked() {
        assert_eq!(
            join("vanilla", None),
            Joined {
                asked: None,
                kept: Ok(0),
            }
        );
    }
}
