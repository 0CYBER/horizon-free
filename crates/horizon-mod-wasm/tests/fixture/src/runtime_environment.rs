//! Opt-in proof of registration snapshot transport and generated Host string replies.
use mod_api::{RegistrationContext, RuntimeEnvironment, ServerHost, Text};
use std::sync::OnceLock;

static REGISTERED: OnceLock<RuntimeEnvironment> = OnceLock::new();

pub fn register(cx: &RegistrationContext) {
    if cx.mods().get("runtime_probe").is_none() {
        return;
    }
    let value = RuntimeEnvironment::new(
        cx.dist(),
        cx.game_version().to_owned(),
        cx.horizon_version().to_owned(),
        cx.game_tests_enabled(),
    );
    assert_eq!(REGISTERED.get_or_init(|| value.clone()), &value);
}

pub fn started(host: &dyn ServerHost) {
    let Some(registered) = REGISTERED.get() else {
        return;
    };
    // Repeat both reads to exercise the generated static-string acknowledgement path,
    // retaining the first reply across a second imported call.
    let game = host.game_version();
    let horizon = host.horizon_version();
    let data_version = host.server_data_version();
    assert_eq!(data_version, host.server_data_version());
    assert_eq!(game, host.game_version());
    assert_eq!(horizon, host.horizon_version());
    assert_eq!(registered.dist(), host.dist());
    assert_eq!(registered.game_version(), game);
    assert_eq!(registered.horizon_version(), horizon);
    assert_eq!(registered.game_tests_enabled(), host.game_tests_enabled());
    host.messages()
        .broadcast(Text::literal(format!(
            "runtime_environment|{:?}|{}|{}|{}|{}",
            registered.dist(),
            game,
            horizon,
            registered.game_tests_enabled(),
            data_version,
        )))
        .unwrap();
}
