//! The integration tests of this crate, linked into one binary: a binary per file linked the crate and its
//! dependencies again for every file. A new test file goes in this folder with a `mod` line here.

#[path = "../common/mod.rs"]
mod common;

mod bench;
mod cli;
mod diff;
mod diff_calls;
mod export;
mod files;
mod follow_reader;
mod golden;
mod mcp;
mod pacing_fixture;
mod part_cut;
mod query_cli;
mod query_gpu;
mod query_samples;
mod query_sections;
mod query_series_stats;
mod query_spans;
mod query_tick_samples;
mod read_capture;
mod summary;
mod summary_client;
mod summary_samples;
mod summary_sections;
