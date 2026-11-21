//! The cloud client, and — behind the `cloud-server` feature — its server.
//!
//! The client is what every installation carries: the HTTP client that talks to
//! a peer, the background loop that drives it, and the persisted configuration.
//! The server half is a separate binary someone deploys, so it is compiled only
//! under `cloud-server`; that keeps PostgreSQL, the HTTP framework and the
//! signing crates out of a build that never hosts anything.
#[cfg(feature = "cloud-server")]
pub mod auth;
pub mod autosync;
pub mod client;
#[cfg(feature = "cloud-server")]
pub mod cloudserver;
#[cfg(feature = "cloud-server")]
pub mod cloudstore;
pub mod config;
pub mod remote;
#[cfg(all(test, feature = "cloud-server"))]
mod test_support;

#[cfg(feature = "cloud-server")]
pub use auth::{AuthService, ManagedToken, ManagedTokenHasher, Principal};
pub use autosync::{Autosync, AutosyncConfig, AutosyncStatus};
pub use client::ClientConfig;
#[cfg(feature = "cloud-server")]
pub use cloudserver::CloudServer;
#[cfg(feature = "cloud-server")]
pub use cloudstore::CloudStore;
pub use config::CloudConfig;
pub use remote::RemoteClient;

pub const MAX_MUTATION_BATCH_SIZE: usize = 100;

pub const CLOUD_SYNC_TARGET: &str = "cloud";
