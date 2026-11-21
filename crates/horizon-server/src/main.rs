//! Thin executable for the shared server CLI and supervisor library.
fn main() -> std::process::ExitCode {
    horizon_platform::run_main(horizon_server::main)
}
