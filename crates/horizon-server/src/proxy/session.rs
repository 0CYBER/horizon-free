//! One player through the proxy: logged in with the client, logged in to the game, then every packet relayed
//! both ways, as it is, while the client's own settings are kept for a later login to the game (replay.rs).

use super::{
    Deadline, Inner,
    auth::Profile,
    backend::{self, Backend, Identity},
    replay::{Phase, Replay, body},
    wire::{Closer, ConnReader, ConnWriter, Connection, ids, login},
};
use std::{
    net::IpAddr,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

/// How long either side may be silent: the game and the client both keep each other alive every 15 s.
const SILENCE: Duration = Duration::from_secs(30);

/// Takes a client whose name is verified through the rest of its login and its whole play.
pub(super) fn run(
    inner: &Arc<Inner>,
    mut client: Connection,
    who: Profile,
    peer: IpAddr,
    intent: i32,
    deadline: Deadline,
) {
    let identity = Identity {
        address: peer,
        uuid: who.uuid,
        name: &who.name,
        intent,
        properties: &who.properties,
    };
    let game = match backend::login(inner.game, &inner.secret, &identity) {
        Ok(game) => game,
        Err(backend::Failure::Refused(json)) => return refuse(&mut client, json),
        Err(e) => {
            inner.note(
                "WARN",
                &format!("{} can't join: the game gave no login ({e})", who.name),
            );
            let json = super::wire::translated_json(
                "horizon.proxy.unavailable",
                "The server is not available right now. Try again in a moment.",
            );
            return refuse(&mut client, json);
        }
    };
    let finished = (|| {
        if inner.compression >= 0 {
            client.send(
                &login::Clientbound::Compression(login::Compression {
                    threshold: inner.compression,
                })
                .encode(),
            )?;
            client.set_compression(inner.compression);
        }
        client.send(
            &login::Clientbound::LoginFinished(login::LoginFinished {
                profile: login::GameProfile {
                    uuid: who.uuid,
                    name: who.name.clone(),
                    properties: who.properties.clone(),
                },
                session: who.uuid,
            })
            .encode(),
        )?;
        let (id, payload) = client.recv()?;
        match login::Serverbound::decode(id, &payload)? {
            login::Serverbound::LoginAcknowledged(_) => Ok(()),
            _ => Err(super::wire::Error::new("unexpected packet during login")),
        }
    })();
    if finished.is_err() {
        game.writer.shutdown();
        return;
    }
    // The client acknowledged login. The relay now has its own silence timeout.
    drop(deadline);
    let Backend {
        reader: game_reader,
        writer: mut game_writer,
        ..
    } = game;
    if backend::acknowledge(&mut game_writer).is_err() {
        client.writer.shutdown();
        return;
    }
    inner.note(
        "INFO",
        &format!(
            "{} joined through the proxy (UUID {:032x})",
            who.name, who.uuid
        ),
    );
    let (client_reader, client_writer) = client.split();
    let _ = client_reader.set_read_timeout(Some(SILENCE));
    let (Ok(client_closer), Ok(game_closer)) = (client_reader.closer(), game_reader.closer())
    else {
        return;
    };
    let both = Arc::new(Pair {
        client: Mutex::new(client_writer),
        game: Mutex::new(game_writer),
        closers: [client_closer, game_closer],
    });
    let down = {
        let both = both.clone();
        thread::Builder::new()
            .name("Proxy game reader".into())
            .spawn(move || game_to_client(game_reader, &both))
    };
    client_to_game(client_reader, &both);
    both.close();
    if let Ok(down) = down {
        let _ = down.join();
    }
    inner.note("INFO", &format!("{} left through the proxy", who.name));
}

fn refuse(client: &mut Connection, reason_json: String) {
    let _ =
        client.send(&login::Clientbound::Disconnect(login::Disconnect { reason_json }).encode());
}

/// The two writers, each behind its own lock so a packet is never written in two pieces.
struct Pair {
    client: Mutex<ConnWriter>,
    game: Mutex<ConnWriter>,
    /// Shut the sockets without the writers' locks: a thread blocked sending to a client that stopped reading
    /// holds its lock, and closing has to be what frees it.
    closers: [Closer; 2],
}

impl Pair {
    fn close(&self) {
        for c in &self.closers {
            c.close();
        }
    }
}

fn game_to_client(mut game: ConnReader, both: &Pair) {
    set_read_timeout(&game);
    while let Ok((id, payload)) = game.recv() {
        let sent = both
            .client
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .send(&body(id, &payload));
        if sent.is_err() {
            break;
        }
    }
    both.close();
}

fn client_to_game(mut client: ConnReader, both: &Pair) {
    let (mut phase, mut replay) = (Phase::Config, Replay::default());
    while let Ok((id, payload)) = client.recv() {
        replay.note(phase, id, &payload);
        phase = next_phase(phase, id);
        let sent = both
            .game
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .send(&body(id, &payload));
        if sent.is_err() {
            break;
        }
    }
}

fn set_read_timeout(reader: &ConnReader) {
    let _ = reader.set_read_timeout(Some(SILENCE));
}

/// The state the client is in after it sent packet `id`: FinishConfiguration starts play, and
/// ConfigurationAcknowledged (the answer to the game's StartConfiguration) a new configuration.
pub fn next_phase(phase: Phase, id: i32) -> Phase {
    match phase {
        Phase::Config if id == ids::configuration::serverbound::FINISH_CONFIGURATION => Phase::Play,
        Phase::Play if id == ids::play::serverbound::CONFIGURATION_ACKNOWLEDGED => Phase::Config,
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_client_follows_the_game_through_configuration_and_back() {
        use ids::{configuration::serverbound as cfg, play::serverbound as play};
        let mut phase = Phase::Config;
        for (id, expected) in [
            (cfg::CLIENT_INFORMATION, Phase::Config),
            (cfg::FINISH_CONFIGURATION, Phase::Play),
            (play::KEEP_ALIVE, Phase::Play),
            // 3 means FinishConfiguration only in configuration: in play it is another packet.
            (cfg::FINISH_CONFIGURATION, Phase::Play),
            (play::CONFIGURATION_ACKNOWLEDGED, Phase::Config),
            // 16 is ConfigurationAcknowledged only in play.
            (play::CONFIGURATION_ACKNOWLEDGED, Phase::Config),
        ] {
            phase = next_phase(phase, id);
            assert_eq!(phase, expected, "after packet {id}");
        }
    }
}
