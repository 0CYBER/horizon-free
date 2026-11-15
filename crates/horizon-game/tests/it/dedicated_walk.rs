//! The real dedicated server (`horizon-client --dedicated`) against a protocol bot that walks through
//! freshly generated terrain. Needs the prepared resources; skipped without them.
use horizon_protocol::{
    PROTOCOL_VERSION,
    connection::Connection,
    crypto::offline_uuid,
    packets::{
        Empty,
        configuration::{self as cfg, ClientInformation},
        handshake, login,
        play::{self, Clientbound as C, Serverbound as S},
    },
};
use std::{
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

/// Reap the server on assertion failures as well as successful test completion.
struct Server(std::process::Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn resources() -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    root.join("items.json").exists().then_some(root)
}

/// Log in offline and finish configuration as a vanilla client that knows the core pack.
fn join(port: u16, name: &str) -> horizon_protocol::Result<Connection> {
    let mut c = Connection::connect("127.0.0.1", port, Duration::from_secs(5))?;
    c.reader.set_read_timeout(Some(Duration::from_secs(60)))?;
    c.send(
        &handshake::Serverbound::Intention(handshake::Intention {
            protocol: PROTOCOL_VERSION,
            host: "127.0.0.1".into(),
            port,
            intent: handshake::LOGIN,
        })
        .encode(),
    )?;
    c.send(
        &login::Serverbound::Hello(login::ServerHello {
            name: name.into(),
            uuid: offline_uuid(name),
        })
        .encode(),
    )?;
    loop {
        let (id, payload) = c.recv()?;
        match login::Clientbound::decode(id, &payload)? {
            login::Clientbound::Compression(p) => c.set_compression(p.threshold),
            login::Clientbound::LoginFinished(_) => break,
            other => return Err(horizon_protocol::Error::new(format!("{other:?}"))),
        }
    }
    c.send(&login::Serverbound::LoginAcknowledged(Empty).encode())?;
    c.send(&cfg::Serverbound::ClientInformation(ClientInformation::new(3)).encode())?;
    loop {
        let (id, payload) = c.recv()?;
        match cfg::Clientbound::decode(id, &payload)? {
            cfg::Clientbound::SelectKnownPacks(p) => c.send(
                &cfg::Serverbound::SelectKnownPacks(cfg::SelectKnownPacks { packs: p.packs })
                    .encode(),
            )?,
            cfg::Clientbound::FinishConfiguration(_) => {
                c.send(&cfg::Serverbound::FinishConfiguration(Empty).encode())?;
                return Ok(c);
            }
            _ => {}
        }
    }
}

/// The dedicated server keeps running while a player walks through freshly generated terrain and
/// sand falls and lands near it:
/// the whole level tick (entities, falling blocks, the entity residency pass) runs every tick
/// against real worldgen, not a flat test world. #2845: a pass left unbalanced stopped the
/// server about 30 level ticks after a walking bot joined.
#[test]
#[ignore = "daily"]
fn a_dedicated_server_keeps_ticking_while_a_player_walks() {
    use std::{
        io::{BufRead, Write},
        process::Stdio,
    };
    let Some(resources) = resources() else {
        return;
    };
    let scratch = tempfile::tempdir().unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let game = scratch.path().join("server");
    std::fs::create_dir(&game).unwrap();
    std::fs::write(
        game.join("server.properties"),
        format!(
            "server-ip=127.0.0.1\nserver-port={port}\nonline-mode=false\nwhite-list=false\nlevel-seed=12345\n\
             view-distance=3\nsimulation-distance=3\ngamemode=creative\nforce-gamemode=true\n\
             spawn-protection=0\npause-when-empty-seconds=-1\n"
        ),
    )
    .unwrap();
    std::fs::write(game.join("eula.txt"), "eula=true\n").unwrap();
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_horizon-client"));
    command
        .args(["--resources".as_ref(), resources.as_os_str()])
        .args(["--dedicated", "--nogui"])
        .current_dir(&game)
        .env("HORIZON_NO_AUDIO", "1")
        .env("HORIZON_LAUNCHER_NO_SELF_UPDATE", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, folder) in [
        ("HORIZON_HOME", "home"),
        ("HORIZON_LAUNCHER_HOME", "launcher"),
        ("XDG_DATA_HOME", "data"),
        ("XDG_CONFIG_HOME", "config"),
    ] {
        command.env(key, scratch.path().join(folder));
    }
    let mut server = Server(command.spawn().unwrap());
    let (lines, read) = std::sync::mpsc::channel::<String>();
    for stream in [
        Box::new(server.0.stdout.take().unwrap()) as Box<dyn std::io::Read + Send>,
        Box::new(server.0.stderr.take().unwrap()),
    ] {
        let lines = lines.clone();
        thread::spawn(move || {
            for line in std::io::BufReader::new(stream)
                .lines()
                .map_while(Result::ok)
            {
                let _ = lines.send(line);
            }
        });
    }
    drop(lines);
    let mut log = Vec::new();
    let started = Instant::now();
    while !log.iter().any(|line: &String| line.contains("Done (")) {
        let line = read
            .recv_timeout(Duration::from_secs(300).saturating_sub(started.elapsed()))
            .unwrap_or_else(|_| panic!("the server never finished starting:\n{}", log.join("\n")));
        log.push(line);
    }
    let mut console = server.0.stdin.take().unwrap();
    let walked = (|| {
        let mut bot = join(port, "Walker").map_err(|e| e.to_string())?;
        bot.reader
            .set_read_timeout(Some(Duration::from_millis(50)))
            .map_err(|e| e.to_string())?;
        // A failed send shows up as the next read's lost connection.
        let send = |bot: &mut Connection, p: S| {
            let _ = bot.send(&p.encode());
        };
        let mut at: Option<[f64; 3]> = None;
        let mut moves = 0;
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            let tick = Instant::now();
            loop {
                match bot.recv() {
                    Ok((id, payload)) => match C::decode(id, &payload) {
                        Ok(C::PlayerPosition(p)) => {
                            send(
                                &mut bot,
                                S::AcceptTeleportation(play::AcceptTeleportation {
                                    id: p.teleport,
                                    x: p.position[0],
                                    y: p.position[1],
                                    z: p.position[2],
                                    yaw: 0.0,
                                    pitch: 0.0,
                                }),
                            );
                            if at.is_none() {
                                send(&mut bot, S::PlayerLoaded(Empty));
                            }
                            at = Some(p.position);
                        }
                        Ok(C::KeepAlive(k)) => send(&mut bot, S::KeepAlive(k)),
                        Ok(C::ChunkBatchFinished(_)) => send(
                            &mut bot,
                            S::ChunkBatchReceived(play::ChunkBatchReceived {
                                chunks_per_tick: 20.0,
                            }),
                        ),
                        Ok(C::Disconnect(d)) => return Err(format!("disconnected: {d:?}")),
                        _ => {}
                    },
                    Err(e) if e.is_timed_out() => {}
                    Err(e) => return Err(format!("connection lost after {moves} moves: {e}")),
                }
                if tick.elapsed() >= Duration::from_millis(50) {
                    break;
                }
            }
            // Once the player is in, the console summons six falling sand blocks just behind it; each
            // falls and lands, as the gravity blocks the player disturbed did in #2845.
            if moves == 20
                && let Some([x, y, z]) = at
            {
                let (x, y, z) = (x.floor() as i32 - 4, y.floor() as i32 + 6, z.floor() as i32);
                for (dx, dz) in [(0, 0), (1, 0), (2, 0), (0, 1), (1, 1), (2, 1)] {
                    writeln!(
                        console,
                        "summon falling_block {} {y} {} {{BlockState:{{Name:\"minecraft:sand\"}},Time:1}}",
                        x + dx,
                        z + dz
                    )
                    .map_err(|e| e.to_string())?;
                }
            }
            // Fly east at sprint speed, two blocks above the spawn: new chunks keep entering the view.
            if let Some([x, y, z]) = at.as_mut().map(|p| {
                p[0] += 0.28;
                *p
            }) {
                send(
                    &mut bot,
                    S::MovePlayerPos(play::MovePlayerPos {
                        x,
                        y: y + 2.0,
                        z,
                        flags: 0,
                    }),
                );
                send(&mut bot, S::ClientTickEnd(Empty));
                moves += 1;
            }
        }
        if moves < 200 {
            return Err(format!("only {moves} moves"));
        }
        Ok(())
    })();
    let exited = server.0.try_wait().unwrap();
    let _ = server.0.kill();
    let _ = server.0.wait();
    log.extend(read.try_iter());
    let unfinished = log
        .iter()
        .any(|line| line.contains("residency") && line.contains("did not finish"));
    assert!(
        walked.is_ok() && exited.is_none() && !unfinished,
        "walk: {walked:?}, server exit: {exited:?}\n{}",
        log.join("\n")
    );
}
