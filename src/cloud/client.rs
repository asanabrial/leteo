//! Persisted client configuration for cloud replication.
//!
//! The cloud server reads its settings from the environment because it runs as
//! a service. A workstation is different: a developer configures the endpoint
//! once and expects `leteo serve`, `leteo mcp`, and `leteo cloud sync` to keep
//! working across restarts, so the client settings live in a file next to the
//! database.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::memory::normalize;

/// File name inside the Leteo data directory.
pub const CLIENT_CONFIG_FILE: &str = "cloud.json";
/// Poll interval used when the configuration does not set one.
pub const DEFAULT_POLL_INTERVAL_SECONDS: u64 = 30;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClientConfig {
    /// Cloud endpoint, for example `https://memory.example.com`.
    pub server: String,
    /// Bearer or managed token presented to that endpoint.
    pub token: String,
    /// Projects replicated to the cloud.
    pub projects: Vec<String>,
    /// Seconds between background sync cycles.
    pub poll_interval_seconds: Option<u64>,
    /// Whether background sync may run. Configuration can be kept while sync
    /// is paused.
    pub enabled: bool,
}

impl ClientConfig {
    pub fn path_in(data_directory: impl AsRef<Path>) -> PathBuf {
        data_directory.as_ref().join(CLIENT_CONFIG_FILE)
    }

    /// Loads the configuration, falling back to the cloud environment variables
    /// so an existing environment-driven setup keeps working unchanged.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let mut config = match std::fs::read(path) {
            Ok(content) => serde_json::from_slice::<Self>(&content)
                .with_context(|| format!("parse {}", path.display()))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(error) => {
                return Err(error).with_context(|| format!("read {}", path.display()));
            }
        };
        config.apply_environment();
        config.normalize();
        Ok(config)
    }

    fn apply_environment(&mut self) {
        if self.server.trim().is_empty()
            && let Ok(server) = std::env::var("LETEO_CLOUD_SERVER")
        {
            self.server = server;
        }
        if self.token.trim().is_empty()
            && let Ok(token) = std::env::var("LETEO_CLOUD_TOKEN")
        {
            self.token = token;
        }
    }

    fn normalize(&mut self) {
        self.server = self.server.trim().to_owned();
        self.token = self.token.trim().to_owned();
        let mut projects: Vec<String> = self
            .projects
            .iter()
            .map(|project| normalize::project(project))
            .filter(|project| !project.is_empty())
            .collect();
        projects.sort();
        projects.dedup();
        self.projects = projects;
    }

    /// Writes the configuration, restricting permissions where the platform
    /// supports it because the file holds a token.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let mut body = serde_json::to_string_pretty(self).context("serialize cloud config")?;
        body.push('\n');
        // This holds the cloud token. Writing it and restricting it afterwards
        // put it on disk world-readable for as long as the two calls were
        // apart, so the mode goes on before the file has the name.
        crate::files::replace_private(path, body.as_bytes())
            .with_context(|| format!("write {}", path.display()))
    }

    /// Whether background sync should start.
    pub fn is_runnable(&self) -> bool {
        self.enabled
            && !self.server.is_empty()
            && !self.token.is_empty()
            && !self.projects.is_empty()
    }

    pub fn poll_interval(&self) -> Duration {
        Duration::from_secs(
            self.poll_interval_seconds
                .filter(|seconds| *seconds > 0)
                .unwrap_or(DEFAULT_POLL_INTERVAL_SECONDS),
        )
    }

    /// A view safe to print: the token becomes a presence flag.
    pub fn redacted(&self) -> serde_json::Value {
        serde_json::json!({
            "server": self.server,
            "token_configured": !self.token.is_empty(),
            "projects": self.projects,
            "poll_interval_seconds": self.poll_interval().as_secs(),
            "enabled": self.enabled,
            "runnable": self.is_runnable(),
        })
    }

    pub fn require_runnable(&self) -> Result<()> {
        if self.server.is_empty() {
            bail!("no cloud server configured; run: leteo cloud config set --server URL");
        }
        if self.token.is_empty() {
            bail!("no cloud token configured; run: leteo cloud config set --token TOKEN");
        }
        if self.projects.is_empty() {
            bail!("no cloud projects configured; run: leteo cloud enroll --project NAME");
        }
        if !self.enabled {
            bail!("cloud sync is disabled; run: leteo cloud config set --enable");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn missing_configuration_is_an_empty_default() {
        let temp = TempDir::new().unwrap();
        let config = ClientConfig::load(ClientConfig::path_in(temp.path())).unwrap();

        assert_eq!(config, ClientConfig::default());
        assert!(!config.is_runnable());
        assert_eq!(
            config.poll_interval(),
            Duration::from_secs(DEFAULT_POLL_INTERVAL_SECONDS)
        );
        let error = config.require_runnable().unwrap_err().to_string();
        assert!(error.contains("no cloud server configured"));
    }

    #[test]
    fn configuration_round_trips_and_normalizes_projects() {
        let temp = TempDir::new().unwrap();
        let path = ClientConfig::path_in(temp.path());
        let config = ClientConfig {
            server: "  https://memory.example.com  ".to_owned(),
            token: " token-value ".to_owned(),
            projects: vec![
                "Beta".to_owned(),
                "alpha--one".to_owned(),
                "beta".to_owned(),
                "   ".to_owned(),
            ],
            poll_interval_seconds: Some(15),
            enabled: true,
        };
        config.save(&path).unwrap();

        let loaded = ClientConfig::load(&path).unwrap();
        assert_eq!(loaded.server, "https://memory.example.com");
        assert_eq!(loaded.token, "token-value");
        assert_eq!(loaded.projects, ["alpha-one", "beta"]);
        assert_eq!(loaded.poll_interval(), Duration::from_secs(15));
        assert!(loaded.is_runnable());
        loaded.require_runnable().unwrap();

        let redacted = loaded.redacted();
        assert_eq!(redacted["token_configured"], true);
        assert_eq!(redacted["runnable"], true);
        assert!(!redacted.to_string().contains("token-value"));
    }

    #[test]
    fn a_disabled_or_incomplete_configuration_never_runs() {
        let complete = ClientConfig {
            server: "https://memory.example.com".to_owned(),
            token: "token".to_owned(),
            projects: vec!["leteo".to_owned()],
            poll_interval_seconds: None,
            enabled: false,
        };
        assert!(!complete.is_runnable());
        assert!(
            complete
                .require_runnable()
                .unwrap_err()
                .to_string()
                .contains("disabled")
        );

        let no_projects = ClientConfig {
            enabled: true,
            projects: Vec::new(),
            ..complete.clone()
        };
        assert!(!no_projects.is_runnable());
        assert!(
            no_projects
                .require_runnable()
                .unwrap_err()
                .to_string()
                .contains("no cloud projects")
        );

        let zero_interval = ClientConfig {
            poll_interval_seconds: Some(0),
            ..complete
        };
        assert_eq!(
            zero_interval.poll_interval(),
            Duration::from_secs(DEFAULT_POLL_INTERVAL_SECONDS),
            "a zero interval would spin"
        );
    }

    #[test]
    fn a_corrupt_configuration_is_reported_instead_of_ignored() {
        let temp = TempDir::new().unwrap();
        let path = ClientConfig::path_in(temp.path());
        std::fs::write(&path, b"{ not json").unwrap();

        let error = ClientConfig::load(&path).unwrap_err().to_string();

        assert!(error.contains("parse"), "{error}");
    }
}
