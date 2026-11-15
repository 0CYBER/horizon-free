//! Thin executable for the shared server CLI and supervisor library.
fn main() -> std::process::ExitCode {
    horizon_platform::prepare_main_stack();
    horizon_server::main()
}
