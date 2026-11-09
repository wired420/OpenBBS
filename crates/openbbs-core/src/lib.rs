//! Core library for OpenBBS: configuration, storage and shared types.

pub mod config;
pub mod cp437;
pub mod db;
pub mod doors;
pub mod files;
pub mod mail;
pub mod messages;
pub mod text;
pub mod users;

/// The OpenBBS version, taken from the workspace `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
