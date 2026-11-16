use mod_api::{Flow, Hooks, ServerHost};
use serde_json::json;
use std::{
    fs::File,
    io::Write,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

static JOIN: AtomicU64 = AtomicU64::new(u64::MAX);
static PLAYER: Mutex<Option<u128>> = Mutex::new(None);
static CONFIG: OnceLock<serde_json::Value> = OnceLock::new();
fn config() -> &'static serde_json::Value {
    CONFIG.get_or_init(|| {
        serde_json::from_slice(
            &std::fs::read(std::env::var_os("HORIZON_PAPER_TRACE_CONFIG").expect("trace config"))
                .expect("read config"),
        )
        .expect("parse config")
    })
}

static TICK: AtomicU64 = AtomicU64::new(0);
static OUTPUT: OnceLock<Mutex<File>> = OnceLock::new();

fn emit(name: &str, fields: serde_json::Value) {
    record(name, fields, false);
}
fn record(name: &str, fields: serde_json::Value, cancelled: bool) {
    let output = OUTPUT.get_or_init(|| {
        Mutex::new(
            File::create(std::env::var_os("HORIZON_PAPER_TRACE").expect("HORIZON_PAPER_TRACE"))
                .expect("create trace"),
        )
    });
    let mut file = output.lock().unwrap();
    writeln!(
        file,
        "{}",
        json!({"tick": TICK.load(Ordering::Relaxed), "name": name,
        "fields": fields, "cancelled": cancelled})
    )
    .expect("write trace");
    file.flush().expect("flush trace");
}

#[derive(Default)]
struct Mod;

#[mod_api::horizon_mod]
impl Hooks for Mod {
    fn server_tick(&self, host: &mut dyn ServerHost, e: &mut mod_api::ServerTickEvent<'_>) {
        if !matches!(e.phase, mod_api::ServerTickPhase::ServerTickStart { .. }) {
            return;
        }
        let tick = e.context.tick.get();
        TICK.store(tick, Ordering::Relaxed);
        if let Some(uuid) = PLAYER.lock().unwrap().take() {
            let slot = host.player(uuid).and_then(|p| p.selected_slot(host));
            emit("_slot", json!({"slot": slot}));
        }
        if let Some(window) = config()["window_ticks"].as_u64() {
            let elapsed = tick
                .checked_sub(JOIN.load(Ordering::Relaxed))
                .unwrap_or(u64::MAX);
            if elapsed == 40 || elapsed == 40 + window {
                let (speed, marker) = if elapsed == 40 {
                    (1024, "_window_start")
                } else {
                    (0, "_window_end")
                };
                assert!(
                    host.commands()
                        .run_command(&format!("gamerule random_tick_speed {speed}"))
                );
                emit(marker, json!({}));
            }
        }
    }
    fn chunk_load(&self, _: &mut dyn ServerHost, e: &mut mod_api::ChunkLoadEvent<'_>) {
        emit(
            "chunk_load",
            json!({"dimension": e.dimension, "chunk": e.chunk, "new": e.new}),
        );
    }
    fn player_switch_hotbar_slot_pre(
        &self,
        _: &mut dyn ServerHost,
        e: &mut mod_api::PlayerSwitchHotbarSlotPreEvent,
    ) -> Flow {
        let cancel = config()["player_switch_hotbar_slot_pre"]["new"].as_i64()
            == Some(i64::from(e.new_slot));
        *PLAYER.lock().unwrap() = Some(e.player);
        record(
            "player_switch_hotbar_slot_pre",
            json!({"old_slot": e.old_slot, "new_slot": e.new_slot}),
            cancel,
        );
        if cancel { Flow::Cancel } else { Flow::Continue }
    }
    fn player_switch_hotbar_slot_post(
        &self,
        _: &mut dyn ServerHost,
        e: &mut mod_api::PlayerSwitchHotbarSlotPostEvent,
    ) {
        emit(
            "player_switch_hotbar_slot_post",
            json!({"old_slot": e.old_slot, "new_slot": e.new_slot}),
        );
    }
    fn moisture_change(
        &self,
        _: &dyn mod_api::LevelHost,
        e: &mut mod_api::MoistureChangeEvent,
    ) -> Flow {
        let pos = json!([e.pos.x, e.pos.y, e.pos.z]);
        let cancel = config()["moisture_change"]["pos"] == pos;
        record(
            "moisture_change",
            json!({"pos": pos, "from": e.from, "to": e.to}),
            cancel,
        );
        if cancel { Flow::Cancel } else { Flow::Continue }
    }
    fn player_logged_in(&self, _: &mut dyn ServerHost, _: &mut mod_api::PlayerLoggedInEvent<'_>) {
        JOIN.store(TICK.load(Ordering::Relaxed), Ordering::Relaxed);
        emit("_join", json!({}));
    }
    fn player_logged_out(&self, _: &mut dyn ServerHost, _: &mut mod_api::PlayerLoggedOutEvent<'_>) {
        emit("_quit", json!({}));
    }
}
