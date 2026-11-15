//! The integration tests of this crate, linked into one binary: a binary per file linked the crate and its
//! dependencies again for every file. A new test file goes in this folder with a `mod` line here.

mod bridge_slots;
mod global_unknown;
mod hosts_per_address;
mod open_per_address;
mod ping_schedule;
mod proxy_from;
mod tunnel;
