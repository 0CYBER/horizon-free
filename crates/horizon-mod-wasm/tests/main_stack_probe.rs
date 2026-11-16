//! A fresh executable image tests the actual main stack and production linker policy.
use mod_api::{
    Hooks,
    wire::{Reader, Writer},
};
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;
#[path = "support/no_hooks.rs"]
mod no_hooks;

struct Log(Mutex<Vec<String>>);
static LOG: Log = Log(Mutex::new(Vec::new()));
impl log::Log for Log {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        let line = record.args().to_string();
        if line.contains("wasm mod fixture stopped its group") {
            eprintln!(
                "probe, not in game: wasm mod fixture trapped: {}",
                if line.contains("stack") {
                    "guest stack overflow"
                } else {
                    "other fault"
                }
            );
        } else {
            eprintln!("{line}");
        }
        self.0.lock().unwrap().push(line);
    }
    fn flush(&self) {}
}
struct Host;
impl mod_api::Host for Host {}
mod_api::impl_host_areas!(Host);
impl mod_api::ClientHost for Host {}
mod_api::impl_client_host_areas!(Host);
impl mod_api::ServerHost for Host {}
mod_api::impl_server_host_areas!(Host);

fn module(reply: &Writer, depth: i32) -> horizon_mod_wasm::WasmMod {
    let stores: String = reply
        .bytes()
        .iter()
        .enumerate()
        .map(|(i, byte)| format!("i32.const {} i32.const {byte} i32.store8 ", 2048 + i))
        .collect();
    let bytes = wat::parse_str(format!(
        r#"(module
        (@custom "dylink.0" "\01\04\00\00\00\00")
        (import "env" "memory" (memory 1))
        (import "env" "__indirect_function_table" (table 1 funcref))
        (func (export "horizon_layout") (result i64) i64.const {})
        (func (export "horizon_in") (param i32) (result i32) i32.const 512)
        (func $recurse (param i32) (result i32)
            local.get 0 i32.eqz if (result i32) i32.const 0 else
            local.get 0 i32.const 1 i32.sub call $recurse i32.const 1 i32.add end)
        (func (export "horizon_hook") (param i32) (result i64)
            i32.const {depth} call $recurse i32.const 60000 i32.ne if unreachable end
            {stores} i64.const {}))"#,
        *mod_api::wire::LAYOUT,
        (2048u64 << 32) | reply.bytes().len() as u64
    ))
    .unwrap();
    horizon_mod_wasm::fixture::module(&bytes).unwrap()
}
fn memory(label: &str) {
    #[cfg(target_os = "linux")]
    let kib: u64 = std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find(|line| line.starts_with("VmRSS:"))
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    #[cfg(not(target_os = "linux"))]
    let kib =
        horizon_threads::process_memory::resident_bytes().expect("resident memory counter") / 1024;
    let metric = if cfg!(target_os = "linux") {
        "VmRSS"
    } else if cfg!(windows) {
        "working set"
    } else {
        "resident memory"
    };
    println!("probe, not in game: {label} {metric} KiB={kib}");
}

fn proof() {
    if let Ok(value) = std::env::var("PROBE_PRESERVED") {
        assert_eq!(value, "yes");
    }
    log::set_logger(&LOG).unwrap();
    log::set_max_level(log::LevelFilter::Warn);
    memory("unloaded");
    let mut host = Host;
    let mut reply = Writer::default();
    reply.put(&Duration::from_secs(60000));
    reply.put(&0.25f32);
    reply.put(&0.5f32);
    let client = module(&reply, 60000);
    memory("one module loaded");
    let mut render = mod_api::RenderFramePreEvent {
        elapsed: Duration::from_secs(1),
        world_partial_tick: 0.25,
        partial_tick: 0.5,
    };
    client.render_frame_pre(&mut host, &mut render);
    assert_eq!(render.elapsed, Duration::from_secs(60000));
    assert_eq!(
        (render.world_partial_tick, render.partial_tick),
        (0.25, 0.5)
    );
    let mut reply = Writer::default();
    reply.put(&mod_api::ServerTickPhase::ServerTickEnd {
        world_running: true,
    });
    let server = module(&reply, 60000);
    let mut context = Writer::default();
    context.put(&mod_api::Authority::DedicatedServer);
    context.put(&mod_api::WorldEpoch::new(1));
    context.put(&mod_api::TickId::new(1));
    let context = Reader::new(context.bytes())
        .take::<mod_api::LifecycleContext>()
        .unwrap();
    let mut tick = mod_api::ServerTickEvent {
        context: &context,
        phase: mod_api::ServerTickPhase::ServerTickStart {
            world_running: true,
        },
    };
    server.server_tick(&mut host, &mut tick);
    assert!(matches!(
        tick.phase,
        mod_api::ServerTickPhase::ServerTickEnd {
            world_running: true
        }
    ));
    memory("after deep render and server calls");
    let runaway = module(&reply, -1);
    assert!(horizon_mod_wasm::fixture::call_load(&runaway).is_none());
    assert!(
        LOG.0
            .lock()
            .unwrap()
            .iter()
            .any(|line| line.contains("wasm mod fixture stopped its group")
                && (line.contains("stack overflow") || line.contains("call stack exhausted")))
    );
    println!(
        "main-thread proof passed: render=60000 server=60000 overflow=fixture trap; reexec={:?}",
        std::env::var_os("HORIZON_STACK_REEXEC")
    );
}
fn child() -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .arg("--child")
        .env_remove("HORIZON_STACK_REEXEC")
        .env_remove("RUST_MIN_STACK");
    command
}
fn run(mut command: Command, success: bool, text: &str) {
    let output = command.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    print!("{stdout}");
    eprint!("{stderr}");
    assert_eq!(output.status.success(), success, "{stdout}\n{stderr}");
    assert!(format!("{stdout}{stderr}").contains(text), "missing {text}");
}
#[cfg(target_os = "linux")]
fn limited(soft: libc::rlim_t, hard: libc::rlim_t) -> Command {
    use std::os::unix::process::CommandExt;
    let mut command = child();
    // SAFETY: this closure only invokes async-signal-safe setrlimit in the child.
    unsafe {
        command.pre_exec(move || {
            let limit = libc::rlimit {
                rlim_cur: soft,
                rlim_max: hard,
            };
            if libc::setrlimit(libc::RLIMIT_STACK, &limit) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command
}
fn main() {
    // This is the same function re-exported as horizon_platform::prepare_main_stack,
    // the first call in horizon-server/src/main.rs. No test worker or hand-off.
    horizon_guest_stack::prepare_main();
    if std::env::args().any(|arg| arg == "--child") {
        proof();
        return;
    }
    run(
        child(),
        true,
        "render=60000 server=60000 overflow=fixture trap",
    );
    #[cfg(target_os = "linux")]
    {
        let mut soft = limited(8 * 1024 * 1024, libc::RLIM_INFINITY);
        soft.env("PROBE_PRESERVED", "yes");
        run(soft, true, "reexec=Some(\"1\")");
        run(
            limited(8 * 1024 * 1024, 8 * 1024 * 1024),
            false,
            "GuestStackUnavailable: stack hard limit is too low (soft=8192 KiB, hard=8192 KiB)",
        );
        let mut marker = limited(8 * 1024 * 1024, libc::RLIM_INFINITY);
        marker.env("HORIZON_STACK_REEXEC", "1");
        run(
            marker,
            false,
            "stack re-exec marker already set but the stack is insufficient (soft=8192 KiB, hard=unlimited)",
        );
    }
}
