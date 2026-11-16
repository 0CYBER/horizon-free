//! One listener source, built into the wasm fixture and into a native test, at the real point of
//! `horizon_client::game::server_lifecycle::remember_shutdown(failure: &mut Option<String>,
//! message: String)`: the same record and the same answers on both targets.

/// "stop" cancels the body after writing the failure itself; any other message is shouted, and
/// the body then remembers the shouted text.
pub fn on_shutdown(args: &mut mod_points::GameServerLifecycleRememberShutdownArgs) {
    if args.message == "stop" {
        args.failure = Some("stopped by a listener".into());
        args.cancel = true;
    } else {
        args.message = args.message.to_uppercase();
    }
}

/// Each call (the place's value before, the message) and the place's value after.
pub const CASES: [(Option<&str>, &str, Option<&str>); 3] = [
    (None, "disk full", Some("DISK FULL")),
    (None, "stop", Some("stopped by a listener")),
    (Some("earlier"), "late", Some("earlier")),
];
