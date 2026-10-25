//! `horizon-bots`: load-test bots for any Minecraft Java Edition 26.3 server, vanilla, Paper or Horizon
//! (docs/server.md, section 9). Each bot logs in offline, goes through configuration as vanilla's client
//! does, answers keep-alives and pings, accepts teleports, acknowledges chunk batches, and moves at 20 Hz.
//!
//! Bots fly a little above the ground, so the server must give them creative mode (`gamemode=creative`
//! and `force-gamemode=true`): they never touch blocks, so they need no world of their own, and no server
//! can refuse their moves for walking into terrain. Every behaviour is deterministic from `--seed`.
//!
//!   horizon-bots --server 127.0.0.1:25565 --bots 25 --behaviour mixed --seconds 360 --report bots.json
//!
//! It prints one JSON line a second (bots online, chunks received) and writes a summary with `--report`.
//! A bot moves only while it holds the chunk it stands in: vanilla's client stops ticking its player in a chunk
//! it hasn't received, so a player can't outrun the server's chunk sending, on any server.
//! Each bot also keeps the chunks it holds, as a client's chunk cache does, so the report tells chunks that
//! entered the view apart from chunks sent again while the bot still held them (`resent`), and counts the
//! times the server pulled a bot back (`pulled_back`: a teleport after the first, as for "moved too quickly").

use horizon_protocol::{
    PROTOCOL_VERSION,
    buf::{Reader, Writer},
    connection::{ConnWriter, Connection},
    crypto::offline_uuid,
    ids,
    packets::{
        Empty, configuration,
        handshake::{self, Intention},
        login::{self, ServerHello},
        play,
    },
};
use serde_json::json;
use std::{
    collections::HashSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const HELP: &str = "\
horizon-bots: load-test bots for a Minecraft Java Edition 26.3 server (offline mode, creative)

  --server HOST:PORT     default 127.0.0.1:25565
  --bots N               default 10
  --behaviour B          idle, walk, explore or mixed (60 % walk, 20 % explore, 20 % idle); default mixed
  --join-every SECONDS   default 1
  --seconds N            how long to run after the first bot joins; default 60
  --view-distance N      what the bots ask for; default 10
  --seed N               default 12345
  --explore-speed B      blocks a second an explore bot flies; default 22 (as fast as chunks come)
  --name PREFIX          bot names: PREFIX0, PREFIX1...; default Bot
  --report FILE          a JSON summary at the end (- prints it after the per-second lines)
";

#[derive(Clone, Copy, Debug, PartialEq)]
enum Behaviour {
    Idle,
    Walk,
    Explore,
}

impl Behaviour {
    fn name(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Walk => "walk",
            Self::Explore => "explore",
        }
    }
}

struct Options {
    host: String,
    port: u16,
    bots: usize,
    behaviour: String,
    join_every: f64,
    seconds: u64,
    view_distance: i8,
    seed: u64,
    explore_speed: f64,
    name: String,
    report: Option<String>,
}

fn options() -> Result<Options, String> {
    let mut o = Options {
        host: "127.0.0.1".into(),
        port: 25565,
        bots: 10,
        behaviour: "mixed".into(),
        join_every: 1.0,
        seconds: 60,
        view_distance: 10,
        seed: 12345,
        explore_speed: 22.0,
        name: "Bot".into(),
        report: None,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "-h" || arg == "--help" {
            print!("{HELP}");
            std::process::exit(0);
        }
        let value = args.next().ok_or(format!("{arg} needs a value"))?;
        let number = |what: &str| format!("{arg} needs {what}, not {value:?}");
        match arg.as_str() {
            "--server" => {
                let (host, port) = value.rsplit_once(':').unwrap_or((&value, "25565"));
                o.host = host.into();
                o.port = port.parse().map_err(|_| number("a port"))?;
            }
            "--bots" => o.bots = value.parse().map_err(|_| number("a number"))?,
            "--behaviour" => o.behaviour = value,
            "--join-every" => o.join_every = value.parse().map_err(|_| number("seconds"))?,
            "--seconds" => o.seconds = value.parse().map_err(|_| number("seconds"))?,
            "--view-distance" => o.view_distance = value.parse().map_err(|_| number("2 to 32"))?,
            "--seed" => o.seed = value.parse().map_err(|_| number("a number"))?,
            "--explore-speed" => {
                o.explore_speed = value.parse().map_err(|_| number("blocks a second"))?;
            }
            "--name" => o.name = value,
            "--report" => o.report = Some(value),
            _ => return Err(format!("unknown option {arg}; --help lists them")),
        }
    }
    if !["idle", "walk", "explore", "mixed"].contains(&o.behaviour.as_str()) {
        return Err(format!(
            "--behaviour {}: use idle, walk, explore or mixed",
            o.behaviour
        ));
    }
    Ok(o)
}

/// What one bot is doing, shared between its reader and its ticks.
#[derive(Default)]
struct Bot {
    online: AtomicBool,
    chunks: AtomicU64,
    /// Chunks received while the bot already held them (no ForgetLevelChunk in between).
    resent: AtomicU64,
    /// Different chunks received over the run.
    unique: AtomicU64,
    forgotten: AtomicU64,
    /// Teleports after the spawn one: the server refused a move.
    pulled_back: AtomicU64,
    /// Ticks the bot couldn't move because it didn't hold its chunk yet.
    waited: AtomicU64,
    /// The chunks the bot holds (ClientChunkCache).
    held: Mutex<HashSet<(i32, i32)>>,
    /// Where the server last put the bot (a teleport), to start from.
    teleport: Mutex<Option<[f64; 3]>>,
    error: Mutex<Option<String>>,
    joined_after_ms: AtomicU64,
}

/// A small deterministic generator (SplitMix64), so every run moves the bots the same way.
struct Random(u64);
impl Random {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// One bot thread that panics poisons the locks it held; the others and the report go on.
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn raw(id: i32, write: impl FnOnce(&mut Writer)) -> Vec<u8> {
    let mut w = Writer::new();
    w.varint(id);
    write(&mut w);
    w.data
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Handshake, offline login and configuration; returns the connection in the play state.
fn join(o: &Options, name: &str) -> Result<Connection, String> {
    let mut c = Connection::connect(&o.host, o.port, Duration::from_secs(10)).map_err(err)?;
    c.send(
        &handshake::Serverbound::Intention(Intention {
            protocol: PROTOCOL_VERSION,
            host: o.host.clone(),
            port: o.port,
            intent: handshake::LOGIN,
        })
        .encode(),
    )
    .map_err(err)?;
    c.send(
        &login::Serverbound::Hello(ServerHello {
            name: name.into(),
            uuid: offline_uuid(name),
        })
        .encode(),
    )
    .map_err(err)?;
    loop {
        let (id, body) = c.recv().map_err(err)?;
        match login::Clientbound::decode(id, &body).map_err(err)? {
            login::Clientbound::Compression(p) => c.set_compression(p.threshold),
            login::Clientbound::LoginFinished(_) => {
                c.send(&login::Serverbound::LoginAcknowledged(Empty).encode())
                    .map_err(err)?;
                break;
            }
            login::Clientbound::Disconnect(p) => return Err(format!("refused: {}", p.reason_json)),
            login::Clientbound::Hello(_) => {
                return Err("the server is in online mode; bots need online-mode=false".into());
            }
            login::Clientbound::CustomQuery(p) => c
                .send(
                    &login::Serverbound::CustomQueryAnswer(login::CustomQueryAnswer {
                        transaction: p.transaction,
                        payload: None,
                    })
                    .encode(),
                )
                .map_err(err)?,
            login::Clientbound::CookieRequest(p) => c
                .send(
                    &login::Serverbound::CookieResponse(login::CookieResponse {
                        key: p.key,
                        payload: None,
                    })
                    .encode(),
                )
                .map_err(err)?,
            login::Clientbound::Unknown(..) => {}
        }
    }
    use ids::configuration::clientbound as cb;
    c.send(
        &configuration::Serverbound::ClientInformation(configuration::ClientInformation::new(
            o.view_distance,
        ))
        .encode(),
    )
    .map_err(err)?;
    loop {
        let (id, body) = c.recv().map_err(err)?;
        let reply = match id {
            cb::SELECT_KNOWN_PACKS
            | cb::KEEP_ALIVE
            | cb::PING
            | cb::CODE_OF_CONDUCT
            | cb::COOKIE_REQUEST
            | cb::DISCONNECT
            | cb::FINISH_CONFIGURATION => {
                match configuration::Clientbound::decode(id, &body).map_err(err)? {
                    configuration::Clientbound::SelectKnownPacks(p) => {
                        configuration::Serverbound::SelectKnownPacks(p)
                    }
                    configuration::Clientbound::KeepAlive(p) => {
                        configuration::Serverbound::KeepAlive(p)
                    }
                    configuration::Clientbound::Ping(p) => configuration::Serverbound::Pong(p),
                    configuration::Clientbound::CodeOfConduct(_) => {
                        configuration::Serverbound::AcceptCodeOfConduct(Empty)
                    }
                    configuration::Clientbound::CookieRequest(p) => {
                        configuration::Serverbound::CookieResponse(configuration::CookieResponse {
                            key: p.key,
                            payload: None,
                        })
                    }
                    configuration::Clientbound::Disconnect(_) => {
                        return Err("disconnected during configuration".into());
                    }
                    configuration::Clientbound::FinishConfiguration(_) => {
                        c.send(&configuration::Serverbound::FinishConfiguration(Empty).encode())
                            .map_err(err)?;
                        return Ok(c);
                    }
                    _ => continue,
                }
            }
            _ => continue,
        };
        c.send(&reply.encode()).map_err(err)?;
    }
}

/// ChunkBatchSizeCalculator: the chunks per tick a vanilla client asks for, from how long its batches take
/// to arrive.
struct BatchSize {
    nanos_per_chunk: f64,
    old_samples_weight: u32,
    started: Instant,
}

impl BatchSize {
    fn new() -> Self {
        Self {
            nanos_per_chunk: 2_000_000.0,
            old_samples_weight: 1,
            started: Instant::now(),
        }
    }
    fn finished(&mut self, size: i32) {
        if size > 0 {
            let per_chunk = self.started.elapsed().as_nanos() as f64 / f64::from(size);
            let clamped = per_chunk.clamp(self.nanos_per_chunk / 3.0, self.nanos_per_chunk * 3.0);
            let weight = f64::from(self.old_samples_weight);
            self.nanos_per_chunk = (self.nanos_per_chunk * weight + clamped) / (weight + 1.0);
            self.old_samples_weight = (self.old_samples_weight + 1).min(49);
        }
    }
    fn desired_chunks_per_tick(&self) -> f32 {
        (7_000_000.0 / self.nanos_per_chunk) as f32
    }
}

/// Reads the play state: answers what a client must answer, counts chunks, records teleports.
fn read(
    mut reader: horizon_protocol::connection::ConnReader,
    writer: Arc<Mutex<ConnWriter>>,
    bot: Arc<Bot>,
) {
    use ids::play::clientbound as cb;
    use ids::play::serverbound as sb;
    let send = |body: Vec<u8>| lock(&writer).send(&body);
    let mut seen = HashSet::new();
    let mut batch = BatchSize::new();
    let mut teleports = 0u64;
    loop {
        let (id, body) = match reader.recv() {
            Ok(packet) => packet,
            Err(e) => {
                if bot.online.swap(false, Ordering::SeqCst) {
                    *lock(&bot.error) = Some(e.to_string());
                }
                return;
            }
        };
        let mut r = Reader::new(&body);
        let sent = match id {
            cb::KEEP_ALIVE => match r.i64() {
                Ok(v) => send(raw(sb::KEEP_ALIVE, |w| w.i64(v))),
                Err(_) => Ok(()),
            },
            cb::PING => match r.i32() {
                Ok(v) => send(raw(sb::PONG, |w| w.i32(v))),
                Err(_) => Ok(()),
            },
            cb::LEVEL_CHUNK_WITH_LIGHT => {
                bot.chunks.fetch_add(1, Ordering::Relaxed);
                if let (Ok(x), Ok(z)) = (r.i32(), r.i32()) {
                    if !lock(&bot.held).insert((x, z)) {
                        bot.resent.fetch_add(1, Ordering::Relaxed);
                    }
                    if seen.insert((x, z)) {
                        bot.unique.fetch_add(1, Ordering::Relaxed);
                    }
                }
                Ok(())
            }
            cb::FORGET_LEVEL_CHUNK => {
                if let Ok(play::Clientbound::ForgetLevelChunk(p)) =
                    play::Clientbound::decode(id, &body)
                    && lock(&bot.held).remove(&(p.pos[0], p.pos[1]))
                {
                    bot.forgotten.fetch_add(1, Ordering::Relaxed);
                }
                Ok(())
            }
            cb::CHUNK_BATCH_START => {
                batch.started = Instant::now();
                Ok(())
            }
            cb::CHUNK_BATCH_FINISHED => {
                batch.finished(r.varint().unwrap_or(0));
                send(
                    play::Serverbound::ChunkBatchReceived(play::ChunkBatchReceived {
                        chunks_per_tick: batch.desired_chunks_per_tick(),
                    })
                    .encode(),
                )
            }
            cb::PLAYER_POSITION => match play::Clientbound::decode(id, &body) {
                Ok(play::Clientbound::PlayerPosition(p)) => {
                    let mut at = lock(&bot.teleport);
                    let previous = at.unwrap_or([0.0; 3]);
                    // Relative flags: X 1, Y 2, Z 4.
                    let pos: [f64; 3] = std::array::from_fn(|i| {
                        if p.relatives & (1 << i) != 0 {
                            previous[i] + p.position[i]
                        } else {
                            p.position[i]
                        }
                    });
                    *at = Some(pos);
                    teleports += 1;
                    if teleports > 1 {
                        bot.pulled_back.fetch_add(1, Ordering::Relaxed);
                    }
                    send(
                        play::Serverbound::AcceptTeleportation(play::AcceptTeleportation {
                            id: p.teleport,
                            x: pos[0],
                            y: pos[1],
                            z: pos[2],
                            yaw: p.yaw,
                            pitch: p.pitch,
                        })
                        .encode(),
                    )
                }
                _ => Ok(()),
            },
            cb::DISCONNECT | cb::START_CONFIGURATION => {
                bot.online.store(false, Ordering::SeqCst);
                *lock(&bot.error) = Some(if id == cb::DISCONNECT {
                    format!(
                        "disconnected: {}",
                        String::from_utf8_lossy(&body)
                            .chars()
                            .filter(|c| !c.is_control())
                            .collect::<String>()
                    )
                } else {
                    "sent back to configuration".into()
                });
                return;
            }
            _ => Ok(()),
        };
        if sent.is_err() {
            bot.online.store(false, Ordering::SeqCst);
            return;
        }
    }
}

/// One bot: join, then move every 50 ms until `until`.
fn run_bot(
    o: Arc<Options>,
    index: usize,
    behaviour: Behaviour,
    bot: Arc<Bot>,
    until: Arc<Mutex<Option<Instant>>>,
) {
    let name = format!("{}{index}", o.name);
    let asked = Instant::now();
    let connection = match join(&o, &name) {
        Ok(c) => c,
        Err(e) => {
            *lock(&bot.error) = Some(e);
            return;
        }
    };
    let (reader, writer) = connection.split();
    let writer = Arc::new(Mutex::new(writer));
    bot.online.store(true, Ordering::SeqCst);
    {
        let (writer, bot) = (writer.clone(), bot.clone());
        thread::spawn(move || read(reader, writer, bot));
    }
    use ids::play::serverbound as sb;
    // Wait for the spawn position.
    let spawn = loop {
        if let Some(at) = *lock(&bot.teleport) {
            break at;
        }
        if !bot.online.load(Ordering::SeqCst) || asked.elapsed() > Duration::from_secs(60) {
            if lock(&bot.error).is_none() {
                *lock(&bot.error) = Some("no spawn position within 60 s".into());
            }
            return;
        }
        thread::sleep(Duration::from_millis(20));
    };
    bot.joined_after_ms
        .store(asked.elapsed().as_millis() as u64, Ordering::SeqCst);
    let send = |body: Vec<u8>| lock(&writer).send(&body).is_ok();
    send(raw(sb::PLAYER_LOADED, |_| {}));
    let mut random = Random(o.seed ^ (index as u64).wrapping_mul(0x2545_F491_4F6C_DD1D));
    let mut pos = spawn;
    let cruise = spawn[1] + 24.0;
    let heading = random.unit() * std::f64::consts::TAU;
    let mut target = pos;
    let mut next = Instant::now();
    loop {
        if lock(&until).is_some_and(|u| Instant::now() >= u) || !bot.online.load(Ordering::SeqCst) {
            break;
        }
        // A teleport from the server wins over where the bot thought it was.
        if let Some(at) = lock(&bot.teleport).take() {
            pos = at;
        }
        let here = ((pos[0].floor() as i32) >> 4, (pos[2].floor() as i32) >> 4);
        if !lock(&bot.held).contains(&here) {
            bot.waited.fetch_add(1, Ordering::Relaxed);
        } else if pos[1] < cruise {
            pos[1] = (pos[1] + 0.5).min(cruise);
        } else {
            match behaviour {
                Behaviour::Idle => {}
                Behaviour::Walk => {
                    let (dx, dz) = (target[0] - pos[0], target[2] - pos[2]);
                    let distance = dx.hypot(dz);
                    if distance < 1.0 {
                        let angle = random.unit() * std::f64::consts::TAU;
                        let reach = 8.0 + random.unit() * 40.0;
                        target = [
                            spawn[0] + angle.cos() * reach,
                            cruise,
                            spawn[2] + angle.sin() * reach,
                        ];
                    } else {
                        // Sprinting: 5.6 blocks a second.
                        let step = 0.28_f64.min(distance);
                        pos[0] += dx / distance * step;
                        pos[2] += dz / distance * step;
                    }
                }
                Behaviour::Explore => {
                    // Flying in creative, by default about 22 blocks a second, which keeps new
                    // chunks coming as fast as the server sends them.
                    let step = o.explore_speed / 20.0;
                    pos[0] += heading.cos() * step;
                    pos[2] += heading.sin() * step;
                }
            }
        }
        let ok = send(
            play::Serverbound::MovePlayerPos(play::MovePlayerPos {
                x: pos[0],
                y: pos[1],
                z: pos[2],
                flags: 0,
            })
            .encode(),
        ) && send(raw(sb::CLIENT_TICK_END, |_| {}));
        if !ok {
            bot.online.store(false, Ordering::SeqCst);
            break;
        }
        next += Duration::from_millis(50);
        let now = Instant::now();
        if next > now {
            thread::sleep(next - now);
        } else if now - next > Duration::from_secs(1) {
            next = now;
        }
    }
    // Offline first, so the reader doesn't count the closed connection as a failure.
    bot.online.store(false, Ordering::SeqCst);
    lock(&writer).shutdown();
}

fn behaviour_of(mode: &str, index: usize) -> Behaviour {
    match mode {
        "idle" => Behaviour::Idle,
        "walk" => Behaviour::Walk,
        "explore" => Behaviour::Explore,
        // mixed: of every five bots, three walk, one explores, one stands.
        _ => match index % 5 {
            0 | 2 | 4 => Behaviour::Walk,
            1 => Behaviour::Explore,
            _ => Behaviour::Idle,
        },
    }
}

fn main() {
    let o = match options() {
        Ok(o) => Arc::new(o),
        Err(e) => {
            eprintln!("horizon-bots: {e}");
            std::process::exit(2);
        }
    };
    let start = Instant::now();
    let until: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
    let bots: Vec<Arc<Bot>> = (0..o.bots).map(|_| Arc::new(Bot::default())).collect();
    let launcher = {
        let (o, bots, until) = (o.clone(), bots.clone(), until.clone());
        thread::spawn(move || {
            let mut handles = Vec::new();
            for (i, bot) in bots.iter().enumerate() {
                let (o2, bot, until) = (o.clone(), bot.clone(), until.clone());
                let behaviour = behaviour_of(&o.behaviour, i);
                handles.push(thread::spawn(move || run_bot(o2, i, behaviour, bot, until)));
                thread::sleep(Duration::from_secs_f64(o.join_every));
            }
            handles
        })
    };
    // The run lasts --seconds after the first bot is in.
    let mut previous_chunks = 0;
    let mut first = None;
    loop {
        thread::sleep(Duration::from_secs(1));
        let online = bots
            .iter()
            .filter(|b| b.online.load(Ordering::SeqCst))
            .count();
        let chunks: u64 = bots.iter().map(|b| b.chunks.load(Ordering::Relaxed)).sum();
        let total = |f: fn(&Bot) -> &AtomicU64| -> u64 {
            bots.iter().map(|b| f(b).load(Ordering::Relaxed)).sum()
        };
        let failed = bots.iter().filter(|b| lock(&b.error).is_some()).count();
        if first.is_none() && online > 0 {
            first = Some(Instant::now());
            *lock(&until) = Some(Instant::now() + Duration::from_secs(o.seconds));
        }
        println!(
            "{}",
            json!({
                "t": start.elapsed().as_secs(),
                "online": online,
                "failed": failed,
                "chunks": chunks,
                "chunks_per_second": chunks - previous_chunks,
                "unique": total(|b| &b.unique),
                "forgotten": total(|b| &b.forgotten),
                "resent": total(|b| &b.resent),
                "pulled_back": total(|b| &b.pulled_back),
                "waited_ticks": total(|b| &b.waited),
            })
        );
        previous_chunks = chunks;
        let over = lock(&until).is_some_and(|u| Instant::now() >= u);
        let all_failed = failed == o.bots;
        if over || all_failed || (first.is_none() && start.elapsed() > Duration::from_secs(120)) {
            *lock(&until) = Some(Instant::now());
            break;
        }
    }
    for handle in launcher.join().unwrap_or_default() {
        let _ = handle.join();
    }
    let summary = json!({
        "server": format!("{}:{}", o.host, o.port),
        "bots": o.bots,
        "behaviour": o.behaviour,
        "seed": o.seed,
        "explore_speed": o.explore_speed,
        "view_distance": o.view_distance,
        "seconds": o.seconds,
        "joined": bots.iter().filter(|b| b.joined_after_ms.load(Ordering::SeqCst) > 0).count(),
        "chunks": bots.iter().map(|b| b.chunks.load(Ordering::Relaxed)).sum::<u64>(),
        "resent": bots.iter().map(|b| b.resent.load(Ordering::Relaxed)).sum::<u64>(),
        "pulled_back": bots.iter().map(|b| b.pulled_back.load(Ordering::Relaxed)).sum::<u64>(),
        "waited_ticks": bots.iter().map(|b| b.waited.load(Ordering::Relaxed)).sum::<u64>(),
        "per_bot": bots.iter().enumerate().map(|(i, b)| json!({
            "name": format!("{}{i}", o.name),
            "behaviour": behaviour_of(&o.behaviour, i).name(),
            "joined_after_ms": b.joined_after_ms.load(Ordering::SeqCst),
            "chunks": b.chunks.load(Ordering::Relaxed),
            "unique_chunks": b.unique.load(Ordering::Relaxed),
            "resent": b.resent.load(Ordering::Relaxed),
            "forgotten": b.forgotten.load(Ordering::Relaxed),
            "pulled_back": b.pulled_back.load(Ordering::Relaxed),
            "waited_ticks": b.waited.load(Ordering::Relaxed),
            "error": *lock(&b.error),
        })).collect::<Vec<_>>(),
    });
    if o.report.as_deref() == Some("-") {
        println!("{summary:#}");
    } else if let Some(path) = &o.report {
        if let Err(e) = std::fs::write(path, format!("{summary:#}\n")) {
            eprintln!("horizon-bots: {path}: {e}");
        }
    }
    let errors: Vec<String> = bots
        .iter()
        .enumerate()
        .filter_map(|(i, b)| {
            b.error
                .lock()
                .unwrap()
                .clone()
                .map(|e| format!("{}{i}: {e}", o.name))
        })
        .collect();
    for e in errors.iter().take(10) {
        eprintln!("horizon-bots: {e}");
    }
    std::process::exit(if errors.is_empty() { 0 } else { 1 });
}
