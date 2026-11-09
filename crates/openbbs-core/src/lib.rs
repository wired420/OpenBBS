//! Core library for OpenBBS: configuration, storage and shared types.

pub mod config;
pub mod db;

/// The OpenBBS version, taken from the workspace `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
