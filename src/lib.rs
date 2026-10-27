pub mod app;
pub mod auth;
pub mod auto_update;
pub mod cli;
pub mod config;
pub mod core;
pub mod download;
pub mod mirrors;
pub mod osu_db;
#[expect(
    clippy::clone_on_copy,
    reason = "clippy 1.99 lints the derive(Clone) expansion in this cxx bridge: its per-field Clone::clone hits Copy fields; measured 2026-10-27, clippy 1.99.0. cxx rejects lint attributes inside the bridge, so this sits here. drop when the expansion stops cloning Copy fields"
)]
pub mod realm_bridge;
pub mod test_env;
pub mod tui;
pub mod utils;
#[cfg(windows)]
pub mod windows_init;
