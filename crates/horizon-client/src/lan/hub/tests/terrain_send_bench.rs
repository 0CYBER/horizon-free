//! Synthetic shared-spawn workload. The ordinary case can also be run on the parent revision.
use super::*;
use horizon_world::registry::BlockStateId;
use std::time::{Duration, Instant};

pub(super) fn burst(hub: &mut Hub) -> (f64, f64) {
    const PLAYERS: u32 = 8;
    hub.encoder = ChunkEncoder::new(
        ["minecraft:air", "minecraft:stone", "minecraft:diamond_ore"]
            .map(|state| (Arc::<str>::from(state), false)),
        &horizon_protocol::registry::Builtin::default(),
        &[],
        &[],
    );
    let mut world = World::new(BlockStateId(0), 0);
    let mut template = Chunk::empty(0, 0);
    template.lit = true;
    for y in 64..80 {
        for z in 0..16 {
            for x in 0..16 {
                template.set(x, y, z, if (x + 3 * y + 5 * z) % 17 == 0 { 2 } else { 1 });
            }
        }
    }
    for x in -5..=5 {
        for z in -5..=5 {
            let mut column = template.clone();
            (column.x, column.z) = (x, z);
            world.chunks.insert((x, z), Arc::new(column));
        }
    }

    let mut receivers = Vec::new();
    for conn in 2..=PLAYERS {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut guest = Guest::new(
            crate::lan::Joined {
                conn,
                uuid: u128::from(conn),
                name: format!("Spawn{conn}"),
                properties: Vec::new(),
                info: horizon_protocol::packets::configuration::ClientInformation::new(8),
                brand: "vanilla".into(),
                sender: crate::lan::PlayerSender::new(tx, Arc::default()),
                latency: Arc::default(),
                address: "127.0.0.1:25565".parse().unwrap(),
                expected: false,
            },
            conn as i32,
            [0.5, 64.0, 0.5],
            8,
        );
        guest.placed = true;
        guest.dimension = "minecraft:overworld".into();
        hub.guests.push(guest);
        receivers.push(rx);
    }
    let paced = std::env::var_os("HORIZON_TERRAIN_BENCH_PACED").is_some();
    let warm = std::env::var_os("HORIZON_TERRAIN_BENCH_WARM").is_some();
    let cadence = Duration::from_millis(if paced { 50 } else { 1 });
    for i in 0..PLAYERS as usize {
        hub.guests
            .update(i, |g| g.chunks_per_tick = if paced { 9.0 } else { 128.0 });
    }
    let columns = world.chunks.len();
    let mut next_entity = 100;
    let border = crate::worldborder::WorldBorder::default();
    let ctx = TickCtx {
        sea_level: 63,
        world: &mut world,
        entities: &[],
        next_entity: &mut next_entity,
        spawn: [0.5, 64.0, 0.5],
        spawn_angle: [0.0; 2],
        spawn_dimension: "minecraft:overworld",
        difficulty: 2,
        difficulty_locked: false,
        clock_rate: 1.0,
        tick_rate: 20.0,
        tick_frozen: false,
        tick_steps: Default::default(),
        collision: None,
        border: &border,
        streaming: None,
    };
    let wave = |hub: &mut Hub| {
        let start = Instant::now();
        let mut on_tick = Duration::ZERO;
        let mut longest = Duration::ZERO;
        let mut passes = 0;
        while hub.guests.iter().any(|g| g.sent_chunks.len() < columns) {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "spawn send stalled"
            );
            hub.ticks += 2;
            for i in 0..PLAYERS as usize {
                hub.guests.update(i, |g| g.unacknowledged = 0);
            }
            let tick = Instant::now();
            hub.update_chunks(&ctx);
            let spent = tick.elapsed();
            on_tick += spent;
            longest = longest.max(spent);
            passes += 1;
            for rx in &receivers {
                rx.try_iter().for_each(drop);
            }
            if hub.guests.iter().any(|g| g.sent_chunks.len() < columns) {
                std::thread::sleep(cadence.saturating_sub(tick.elapsed()));
            }
        }
        let elapsed = start.elapsed();
        let sent = f64::from(PLAYERS) * columns as f64;
        println!(
            "sent {sent:.0} columns in {passes} passes; longest {:.3} ms",
            longest.as_secs_f64() * 1e3
        );
        (sent / elapsed.as_secs_f64(), on_tick.as_secs_f64() * 1e3)
    };
    if warm {
        wave(hub);
        // New deliveries to the same recipients, under the same rules and resident sections.
        for i in 0..PLAYERS as usize {
            hub.guests.update(i, |g| {
                g.center = None;
                g.sent_chunks.clear();
            });
        }
    }
    wave(hub)
}

pub(super) fn median(mut measure: impl FnMut() -> (f64, f64)) -> (f64, f64) {
    let _ = measure();
    let (mut rate, mut tick): (Vec<_>, Vec<_>) =
        (0..if std::env::var_os("HORIZON_TERRAIN_BENCH_PACED").is_some() {
            3
        } else {
            9
        })
            .map(|_| measure())
            .unzip();
    rate.sort_by(f64::total_cmp);
    tick.sort_by(f64::total_cmp);
    (rate[rate.len() / 2], tick[tick.len() / 2])
}

#[test]
fn ordinary_shared_spawn() {
    if std::env::var_os("HORIZON_TERRAIN_BENCH").is_none() {
        return;
    }
    let (rate, ms) = median(|| {
        let (mut hub, _rx) = hub_with_a_guest();
        burst(&mut hub)
    });
    println!(
        "ordinary shared spawn: {rate:.0} recipient chunks/s, {ms:.3} ms total in chunk passes"
    );
}
