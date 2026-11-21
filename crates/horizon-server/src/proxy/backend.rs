//! The proxy's login to the game as a player: the game listens on 127.0.0.1 with Velocity modern forwarding
//! (docs/velocity-forwarding.md), so the proxy says who the player is and signs it with the shared secret.

use super::wire::{
    ConnReader, ConnWriter, Connection, PROTOCOL_VERSION, Writer, handshake, login, status,
};
use sha2::{Digest, Sha256};
use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};

const CHANNEL: &str = "velocity:player_info";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// How long the game may take to answer during the login (it checks its lists and mods' pre-login first).
const LOGIN_TIMEOUT: Duration = Duration::from_secs(25);

/// Why a game could not be logged into.
#[derive(Debug)]
pub enum Failure {
    /// Nothing answered: the game is down or not listening yet.
    Down(String),
    /// The game turned the player away (banned, not whitelisted, full): its reason, a JSON component.
    Refused(String),
    /// The game said something the login doesn't expect.
    Protocol(String),
}

/// A game's connection after its login: ready to be acknowledged.
pub struct Backend {
    pub reader: ConnReader,
    pub writer: ConnWriter,
}

/// What the proxy vouches for to the game.
pub struct Identity<'a> {
    pub address: IpAddr,
    pub uuid: u128,
    pub name: &'a str,
    /// The handshake intent the client used (login or transfer): the game checks it itself.
    pub intent: i32,
    pub properties: &'a [login::Property],
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Down(why) | Self::Protocol(why) => f.write_str(why),
            Self::Refused(json) => write!(f, "refused: {json}"),
        }
    }
}

fn down(e: impl std::fmt::Display) -> Failure {
    Failure::Down(e.to_string())
}

/// Logs into the game at `game` as `who`, up to the game's LoginFinished.
pub fn login(game: SocketAddr, secret: &[u8], who: &Identity) -> Result<Backend, Failure> {
    let mut c =
        Connection::connect(&game.ip().to_string(), game.port(), CONNECT_TIMEOUT).map_err(down)?;
    c.reader
        .set_read_timeout(Some(LOGIN_TIMEOUT))
        .map_err(down)?;
    c.send(&intention(game, who.intent)).map_err(down)?;
    c.send(
        &login::Serverbound::Hello(login::ServerHello {
            name: who.name.to_owned(),
            uuid: who.uuid,
        })
        .encode(),
    )
    .map_err(down)?;
    loop {
        let (id, payload) = c.recv().map_err(down)?;
        match login::Clientbound::decode(id, &payload)
            .map_err(|e| Failure::Protocol(e.to_string()))?
        {
            login::Clientbound::CustomQuery(q) if q.channel == CHANNEL => {
                let answer = login::Serverbound::CustomQueryAnswer(login::CustomQueryAnswer {
                    transaction: q.transaction,
                    payload: Some(forwarded(secret, who)),
                })
                .encode();
                c.send(&answer).map_err(down)?;
            }
            login::Clientbound::Compression(t) => c.set_compression(t.threshold),
            login::Clientbound::LoginFinished(_) => {
                let (reader, writer) = c.split();
                return Ok(Backend { reader, writer });
            }
            login::Clientbound::Disconnect(d) => return Err(Failure::Refused(d.reason_json)),
            other => {
                return Err(Failure::Protocol(format!(
                    "unexpected login packet {other:?}"
                )));
            }
        }
    }
}

/// The handshake a game at `game` is greeted with.
pub fn intention(game: SocketAddr, intent: i32) -> Vec<u8> {
    handshake::Serverbound::Intention(handshake::Intention {
        protocol: PROTOCOL_VERSION,
        host: game.ip().to_string(),
        port: game.port(),
        intent,
    })
    .encode()
}

/// The game's server list answer, as JSON, or why there is none.
pub fn status_json(game: SocketAddr) -> Result<String, Failure> {
    let mut c =
        Connection::connect(&game.ip().to_string(), game.port(), CONNECT_TIMEOUT).map_err(down)?;
    c.reader
        .set_read_timeout(Some(CONNECT_TIMEOUT))
        .map_err(down)?;
    c.send(&intention(game, handshake::STATUS)).map_err(down)?;
    c.send(&status::Serverbound::StatusRequest(super::wire::Empty).encode())
        .map_err(down)?;
    let (id, payload) = c.recv().map_err(down)?;
    match status::Clientbound::decode(id, &payload).map_err(|e| Failure::Protocol(e.to_string()))? {
        status::Clientbound::StatusResponse(r) => Ok(r.json),
        other => Err(Failure::Protocol(format!(
            "unexpected status packet {other:?}"
        ))),
    }
}

/// LoginAcknowledged: the game starts its configuration.
pub fn acknowledge(writer: &mut ConnWriter) -> Result<(), super::wire::Error> {
    writer.send(&login::Serverbound::LoginAcknowledged(super::wire::Empty).encode())
}

/// The login query answer: the signature over the data, then the data (version 1: address, UUID, name,
/// properties). The same layout the game checks in lan/forwarding.rs.
fn forwarded(secret: &[u8], who: &Identity) -> Vec<u8> {
    let mut w = Writer::new();
    w.varint(1);
    w.string(&who.address.to_string());
    w.uuid(who.uuid);
    w.string(who.name);
    w.varint(who.properties.len() as i32);
    for p in who.properties {
        w.string(&p.name);
        w.string(&p.value);
        w.bool(p.signature.is_some());
        if let Some(s) = &p.signature {
            w.string(s);
        }
    }
    let mut out = hmac_sha256(secret, &w.data).to_vec();
    out.extend_from_slice(&w.data);
    out
}

/// HMAC-SHA256 (RFC 2104).
pub(super) fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut padded = [0u8; BLOCK];
    if key.len() > BLOCK {
        padded[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        padded[..key.len()].copy_from_slice(key);
    }
    let inner = Sha256::new()
        .chain_update(padded.map(|b| b ^ 0x36))
        .chain_update(data)
        .finalize();
    Sha256::new()
        .chain_update(padded.map(|b| b ^ 0x5c))
        .chain_update(inner)
        .finalize()
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::wire::Reader;

    #[test]
    fn hmac_matches_rfc_4231_case_1() {
        let mac = hmac_sha256(&[0x0b; 20], b"Hi There");
        let hex: String = mac.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex,
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }

    #[test]
    fn the_forwarded_data_is_what_the_game_reads() {
        let who = Identity {
            address: "203.0.113.9".parse().unwrap(),
            uuid: 0x069a79f444e94726a5befca90e38aaf5,
            name: "Steve",
            intent: handshake::LOGIN,
            properties: &[login::Property {
                name: "textures".into(),
                value: "e30=".into(),
                signature: Some("sig".into()),
            }],
        };
        let answer = forwarded(b"secret", &who);
        let (signature, data) = answer.split_at(32);
        assert_eq!(signature, hmac_sha256(b"secret", data));
        let mut r = Reader::new(data);
        assert_eq!(r.varint().unwrap(), 1);
        assert_eq!(r.string(64).unwrap(), "203.0.113.9");
        assert_eq!(r.uuid().unwrap(), who.uuid);
        assert_eq!(r.string(16).unwrap(), "Steve");
        assert_eq!(r.varint().unwrap(), 1);
        assert_eq!(r.string(64).unwrap(), "textures");
        assert_eq!(r.string(32767).unwrap(), "e30=");
        assert!(r.bool().unwrap());
        assert_eq!(r.string(1024).unwrap(), "sig");
        assert_eq!(r.remaining(), 0);
    }
}
