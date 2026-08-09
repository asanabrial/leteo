use std::{collections::BTreeSet, env};

use thiserror::Error;

pub const DEFAULT_MAX_PUSH_BODY_BYTES: usize = 8 * 1024 * 1024;
const MIN_SECRET_BYTES: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudConfig {
    pub database_url: String,
    pub dashboard_secret: String,
    pub token_pepper: String,
    pub bind_host: String,
    pub port: u16,
    pub max_pool: u32,
    pub sync_token: String,
    pub admin_token: String,
    pub allowed_projects: Vec<String>,
    pub max_push_body_bytes: usize,
}

impl Default for CloudConfig {
    fn default() -> Self {
        Self {
            database_url: String::new(),
            dashboard_secret: String::new(),
            token_pepper: String::new(),
            bind_host: "127.0.0.1".to_owned(),
            port: 8080,
            max_pool: 10,
            sync_token: String::new(),
            admin_token: String::new(),
            allowed_projects: Vec::new(),
            max_push_body_bytes: DEFAULT_MAX_PUSH_BODY_BYTES,
        }
    }
}

impl CloudConfig {
    pub fn from_env() -> Self {
        Self::from_getter(|key| env::var(key).ok())
    }

    fn from_getter(mut get: impl FnMut(&str) -> Option<String>) -> Self {
        let mut config = Self::default();
        set_nonempty(&mut config.database_url, get("LETEO_DATABASE_URL"));
        set_nonempty(&mut config.dashboard_secret, get("LETEO_DASHBOARD_SECRET"));
        set_nonempty(&mut config.token_pepper, get("LETEO_CLOUD_TOKEN_PEPPER"));
        set_nonempty(&mut config.bind_host, get("LETEO_CLOUD_HOST"));
        set_nonempty(&mut config.sync_token, get("LETEO_CLOUD_TOKEN"));
        set_nonempty(&mut config.admin_token, get("LETEO_CLOUD_ADMIN"));
        if let Some(value) = positive_number::<u16>(get("LETEO_CLOUD_PORT")) {
            config.port = value;
        }
        if let Some(value) = positive_number::<u32>(get("LETEO_CLOUD_MAX_POOL")) {
            config.max_pool = value;
        }
        if let Some(value) = positive_number::<usize>(get("LETEO_CLOUD_MAX_PUSH_BYTES")) {
            config.max_push_body_bytes = value;
        }
        if let Some(value) = get("LETEO_CLOUD_ALLOWED_PROJECTS") {
            config.allowed_projects = parse_projects(&value);
        }
        config
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.database_url.trim().is_empty() {
            return Err(ConfigError::MissingDatabaseUrl);
        }
        if self.dashboard_secret.is_empty() {
            return Err(ConfigError::MissingDashboardSecret);
        }
        if self.dashboard_secret.len() < MIN_SECRET_BYTES {
            return Err(ConfigError::DashboardSecretTooShort);
        }
        if !self.token_pepper.is_empty() && self.token_pepper.len() < MIN_SECRET_BYTES {
            return Err(ConfigError::TokenPepperTooShort);
        }
        if !self.sync_token.is_empty() && self.sync_token.len() < MIN_SECRET_BYTES {
            return Err(ConfigError::SyncTokenTooShort);
        }
        if !self.admin_token.is_empty() && self.admin_token.len() < MIN_SECRET_BYTES {
            return Err(ConfigError::AdminTokenTooShort);
        }
        if self.token_pepper.is_empty() && self.sync_token.is_empty() && self.admin_token.is_empty()
        {
            return Err(ConfigError::MissingAuthentication);
        }
        if (!self.sync_token.is_empty() || !self.admin_token.is_empty())
            && self.allowed_projects.is_empty()
        {
            return Err(ConfigError::MissingAllowedProjects);
        }
        if (!self.sync_token.is_empty() && self.sync_token == self.admin_token)
            || (!self.token_pepper.is_empty()
                && (self.token_pepper == self.dashboard_secret
                    || self.token_pepper == self.sync_token
                    || self.token_pepper == self.admin_token))
            || self.dashboard_secret == self.sync_token
            || self.dashboard_secret == self.admin_token
        {
            return Err(ConfigError::SecretsMustDiffer);
        }
        if self.bind_host.trim().is_empty() {
            return Err(ConfigError::MissingBindHost);
        }
        if self.max_pool == 0 || self.max_push_body_bytes == 0 {
            return Err(ConfigError::InvalidLimit);
        }
        Ok(())
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("cloud database URL is required")]
    MissingDatabaseUrl,
    #[error("dashboard signing secret is required; set LETEO_DASHBOARD_SECRET")]
    MissingDashboardSecret,
    #[error("dashboard signing secret must be at least 32 bytes")]
    DashboardSecretTooShort,
    #[error("managed token pepper must be at least 32 bytes")]
    TokenPepperTooShort,
    #[error("legacy sync token must be at least 32 bytes")]
    SyncTokenTooShort,
    #[error("legacy admin token must be at least 32 bytes")]
    AdminTokenTooShort,
    #[error("cloud authentication is required; configure a managed token pepper or legacy token")]
    MissingAuthentication,
    #[error("legacy cloud tokens require LETEO_CLOUD_ALLOWED_PROJECTS")]
    MissingAllowedProjects,
    #[error("cloud signing, pepper, and bearer secrets must be distinct")]
    SecretsMustDiffer,
    #[error("cloud bind host is required")]
    MissingBindHost,
    #[error("cloud limits must be positive")]
    InvalidLimit,
}

fn set_nonempty(target: &mut String, value: Option<String>) {
    if let Some(value) = value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
    {
        *target = value;
    }
}

fn positive_number<T>(value: Option<String>) -> Option<T>
where
    T: std::str::FromStr + PartialOrd + Default,
{
    value
        .as_deref()
        .map(str::trim)
        .and_then(|value| value.parse::<T>().ok())
        .filter(|value| value > &T::default())
}

fn parse_projects(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(crate::memory::normalize::project)
        .filter(|project| !project.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[test]
    fn defaults_are_loopback_bounded_and_fail_closed() {
        let config = CloudConfig::default();
        assert_eq!(config.bind_host, "127.0.0.1");
        assert_eq!(config.max_push_body_bytes, 8 * 1024 * 1024);
        assert_eq!(config.validate(), Err(ConfigError::MissingDatabaseUrl));
    }

    #[test]
    fn environment_values_are_trimmed_and_projects_deduplicated() {
        let values = BTreeMap::from([
            ("LETEO_CLOUD_HOST", " 0.0.0.0 "),
            ("LETEO_CLOUD_PORT", "9090"),
            ("LETEO_CLOUD_ALLOWED_PROJECTS", "proj-b, PROJ-A,proj-b"),
            ("LETEO_CLOUD_MAX_PUSH_BYTES", "1024"),
        ]);
        let config = CloudConfig::from_getter(|key| values.get(key).map(ToString::to_string));
        assert_eq!(config.bind_host, "0.0.0.0");
        assert_eq!(config.port, 9090);
        assert_eq!(config.allowed_projects, ["proj-a", "proj-b"]);
        assert_eq!(config.max_push_body_bytes, 1024);
    }

    #[test]
    fn invalid_environment_numbers_keep_defaults() {
        let config = CloudConfig::from_getter(|key| match key {
            "LETEO_CLOUD_PORT" => Some("0".to_owned()),
            "LETEO_CLOUD_MAX_POOL" => Some("bad".to_owned()),
            _ => None,
        });
        assert_eq!(config.port, 8080);
        assert_eq!(config.max_pool, 10);
    }

    #[test]
    fn secrets_are_validated_independently() {
        let mut config = CloudConfig {
            database_url: "postgres://localhost/leteo-test".to_owned(),
            dashboard_secret: "short".to_owned(),
            token_pepper: "p".repeat(32),
            ..CloudConfig::default()
        };
        assert_eq!(config.validate(), Err(ConfigError::DashboardSecretTooShort));
        config.dashboard_secret = "x".repeat(32);
        config.token_pepper = "short".to_owned();
        assert_eq!(config.validate(), Err(ConfigError::TokenPepperTooShort));
    }

    #[test]
    fn valid_managed_auth_uses_distinct_explicit_secrets() {
        let config = CloudConfig {
            database_url: "postgres://localhost/leteo-test".to_owned(),
            dashboard_secret: "d".repeat(32),
            token_pepper: "p".repeat(32),
            ..CloudConfig::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn legacy_auth_requires_allowlist_and_distinct_secrets() {
        let mut config = CloudConfig {
            database_url: "postgres://localhost/leteo-test".to_owned(),
            dashboard_secret: "d".repeat(32),
            sync_token: "s".repeat(32),
            ..CloudConfig::default()
        };
        assert_eq!(config.validate(), Err(ConfigError::MissingAllowedProjects));

        config.allowed_projects = vec!["project-a".to_owned()];
        assert!(config.validate().is_ok());

        config.admin_token = config.sync_token.clone();
        assert_eq!(config.validate(), Err(ConfigError::SecretsMustDiffer));
    }
}
