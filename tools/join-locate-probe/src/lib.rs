//! Focused production-source tests and timings without the renderer dependency tree.
#[path = "../../../crates/horizon-client/src/join_seq.rs"]
pub mod join_seq;

// The residency tests read the diagnostics those sources log. `horizon-client` serves
// `crate::test_log` to the same files (crates/horizon-client/src/client_lib.rs):
// the probe imports the production sources, so it carries the same shim.
#[cfg(test)]
mod test_log;
