pub mod auth;
pub mod autosync;
pub mod client;
pub mod cloudserver;
pub mod cloudstore;
pub mod config;
pub mod remote;

pub use auth::{AuthService, ManagedToken, ManagedTokenHasher, Principal};
pub use autosync::{Autosync, AutosyncConfig, AutosyncStatus};
pub use client::ClientConfig;
pub use cloudserver::CloudServer;
pub use cloudstore::CloudStore;
pub use config::CloudConfig;
pub use remote::RemoteClient;

pub const MAX_MUTATION_BATCH_SIZE: usize = 100;

pub const CLOUD_SYNC_TARGET: &str = "cloud";
