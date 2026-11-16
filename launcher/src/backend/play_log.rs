use std::process::{Child, Command};

pub(super) fn spawn(cmd: &mut Command) -> Result<Child, String> {
    crate::run_leases::spawn(cmd, true).map_err(|error| {
        crate::i18n::trf(
            "Could not complete game launch: {error}",
            &[("error", &error.to_string())],
        )
    })
}
