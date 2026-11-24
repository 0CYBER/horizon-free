//! Exercise the shared production durable-publication boundary in the existing Windows suite.
#![cfg(windows)]

// The whole file is mounted for its tests; the server's list locking in it (`lock`,
// `write_in_place`) has no caller here.
#[allow(dead_code)]
#[path = "../../crates/horizon-server-files/src/durable.rs"]
mod durable;
