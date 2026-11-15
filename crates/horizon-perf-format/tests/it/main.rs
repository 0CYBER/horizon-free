//! The integration tests of this crate, linked into one binary: a binary per file linked the crate and its
//! dependencies again for every file. A new test file goes in this folder with a `mod` line here.

#[path = "../common/mod.rs"]
mod common;

mod counter_registry;
mod descriptor_lint;
mod files;
mod fixture_v1;
mod health_recorder;
mod live_follow;
mod merge;
mod slow_ring;
mod writer_thread;
