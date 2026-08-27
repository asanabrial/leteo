use std::collections::BTreeSet;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use hmac::{Hmac, KeyInit, Mac};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use thiserror::Error;

use super::cloudstore::{CloudStore, CloudStoreError};

type HmacSha256 = Hmac<Sha256>;

const MANAGED_TOKEN_HASH_PREFIX: &str = "hmac-sha256:v1:";
const MANAGED_TOKEN_DOMAIN: &[u8] = b"leteo-cloud-token:v1:";
const DASHBOARD_SESSION_DOMAIN: &[u8] = b"leteo-dashboard-session:v1:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedToken {
    pub raw: String,
    pub prefix: String,
}

impl ManagedToken {
    pub fn generate(environment: &str) -> Self {
        let environment = normalize_environment(environment);
        let mut prefix_bytes = [0_u8; 4];
        let mut secret_bytes = [0_u8; 32];
        let mut rng = rand::rng();
        rng.fill_bytes(&mut prefix_bytes);
        rng.fill_bytes(&mut secret_bytes);
        let prefix = format!("ltc_{environment}_{}", hex::encode(prefix_bytes));
        let secret = URL_SAFE_NO_PAD.encode(secret_bytes).replace('_', "-");
        Self {
            raw: format!("{prefix}_{secret}"),
            prefix,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ManagedTokenHasher {
    pepper: Vec<u8>,
}

impl ManagedTokenHasher {
    pub fn new(pepper: impl AsRef<[u8]>) -> Result<Self, AuthError> {
        let pepper = pepper.as_ref();
        if pepper.len() < 32 {
            return Err(AuthError::TokenPepperTooShort);
        }
        Ok(Self {
            pepper: pepper.to_vec(),
        })
    }

    pub fn hash(&self, raw_token: &str) -> Result<String, AuthError> {
        let raw_token = raw_token.trim();
        if raw_token.is_empty() {
            return Err(AuthError::TokenRequired);
        }
        Ok(format!(
            "{MANAGED_TOKEN_HASH_PREFIX}{}",
            URL_SAFE_NO_PAD.encode(self.sum(raw_token))
        ))
    }

    pub fn verify(&self, raw_token: &str, verifier: &str) -> bool {
        let Some(encoded) = verifier.trim().strip_prefix(MANAGED_TOKEN_HASH_PREFIX) else {
            return false;
        };
        let Ok(provided) = URL_SAFE_NO_PAD.decode(encoded) else {
            return false;
        };
        let expected = self.sum(raw_token.trim());
        expected.as_slice().ct_eq(provided.as_slice()).into()
    }

    fn sum(&self, raw_token: &str) -> Vec<u8> {
        let mut mac = HmacSha256::new_from_slice(&self.pepper).expect("HMAC accepts any key size");
        mac.update(MANAGED_TOKEN_DOMAIN);
        mac.update(raw_token.as_bytes());
        mac.finalize().into_bytes().to_vec()
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalKind {
    Human,
    ServiceAccount,
    Legacy,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalRole {
    Admin,
    Member,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalSource {
    ManagedToken,
    LegacySync,
    LegacyAdmin,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Principal {
    pub id: String,
    pub kind: PrincipalKind,
    pub display_name: String,
    pub role: PrincipalRole,
    pub enabled: bool,
    pub source: PrincipalSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_id: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct AuthService {
    hasher: Option<ManagedTokenHasher>,
    legacy_sync_token: String,
    legacy_admin_token: String,
    dashboard_secret: Vec<u8>,
    allowed_projects: BTreeSet<String>,
    allow_all_projects: bool,
}

impl AuthService {
    pub(crate) fn new(
        dashboard_secret: impl AsRef<[u8]>,
        token_pepper: Option<&str>,
        legacy_sync_token: impl Into<String>,
        legacy_admin_token: impl Into<String>,
        allowed_projects: &[String],
    ) -> Result<Self, AuthError> {
        let dashboard_secret = dashboard_secret.as_ref();
        if dashboard_secret.len() < 32 {
            return Err(AuthError::DashboardSecretTooShort);
        }
        let hasher = token_pepper
            .filter(|pepper| !pepper.trim().is_empty())
            .map(|pepper| ManagedTokenHasher::new(pepper.as_bytes()))
            .transpose()?;
        let allow_all_projects = allowed_projects.iter().any(|project| project.trim() == "*");
        let allowed_projects = allowed_projects
            .iter()
            .map(|project| crate::memory::normalize::project(project))
            .filter(|project| !project.is_empty() && project != "*")
            .collect();
        Ok(Self {
            hasher,
            legacy_sync_token: legacy_sync_token.into().trim().to_owned(),
            legacy_admin_token: legacy_admin_token.into().trim().to_owned(),
            dashboard_secret: dashboard_secret.to_vec(),
            allowed_projects,
            allow_all_projects,
        })
    }

    pub async fn resolve_bearer(
        &self,
        store: &CloudStore,
        token: &str,
    ) -> Result<Principal, AuthError> {
        let token = validate_bearer_shape(token)?;
        if constant_time_token_eq(token, &self.legacy_sync_token) {
            return Ok(legacy_principal(false));
        }
        if constant_time_token_eq(token, &self.legacy_admin_token) {
            return Ok(legacy_principal(true));
        }
        let hasher = self.hasher.as_ref().ok_or(AuthError::UnknownToken)?;
        let verifier = hasher.hash(token)?;
        let identity = store
            .find_managed_token(&verifier)
            .await
            .map_err(AuthError::Store)?
            .ok_or(AuthError::UnknownToken)?;
        if identity.revoked {
            return Err(AuthError::TokenRevoked);
        }
        if !identity.enabled {
            return Err(AuthError::PrincipalDisabled);
        }
        let kind = match identity.kind.as_str() {
            "human" => PrincipalKind::Human,
            "service_account" => PrincipalKind::ServiceAccount,
            _ => return Err(AuthError::InvalidPrincipal),
        };
        let role = match identity.role.as_str() {
            "admin" => PrincipalRole::Admin,
            "member" => PrincipalRole::Member,
            _ => return Err(AuthError::InvalidPrincipal),
        };
        store
            .touch_managed_token(identity.token_id)
            .await
            .map_err(AuthError::Store)?;
        Ok(Principal {
            id: identity.principal_id.to_string(),
            kind,
            display_name: identity.display_name,
            role,
            enabled: true,
            source: PrincipalSource::ManagedToken,
            token_id: Some(identity.token_id),
        })
    }

    pub async fn authorize_project(
        &self,
        store: &CloudStore,
        principal: &Principal,
        project: &str,
    ) -> Result<String, AuthError> {
        let project = required_project(project)?;
        let allowed = match principal.source {
            PrincipalSource::ManagedToken => {
                let principal_id = principal
                    .id
                    .parse()
                    .map_err(|_| AuthError::InvalidPrincipal)?;
                store
                    .principal_has_project_grant(principal_id, &project)
                    .await
                    .map_err(AuthError::Store)?
            }
            PrincipalSource::LegacySync | PrincipalSource::LegacyAdmin => {
                self.allow_all_projects || self.allowed_projects.contains(&project)
            }
        };
        if allowed {
            Ok(project)
        } else {
            Err(AuthError::ProjectForbidden)
        }
    }

    pub async fn enrolled_projects(
        &self,
        store: &CloudStore,
        principal: &Principal,
    ) -> Result<Option<Vec<String>>, AuthError> {
        match principal.source {
            PrincipalSource::ManagedToken => {
                let principal_id = principal
                    .id
                    .parse()
                    .map_err(|_| AuthError::InvalidPrincipal)?;
                let projects = store
                    .list_principal_project_grants(principal_id)
                    .await
                    .map_err(AuthError::Store)?;
                if projects.iter().any(|project| project == "*") {
                    Ok(None)
                } else {
                    Ok(Some(projects))
                }
            }
            PrincipalSource::LegacySync | PrincipalSource::LegacyAdmin => {
                if self.allow_all_projects {
                    Ok(None)
                } else {
                    Ok(Some(self.allowed_projects.iter().cloned().collect()))
                }
            }
        }
    }

    pub fn mint_dashboard_session(&self, principal: &Principal) -> Result<String, AuthError> {
        self.mint_dashboard_session_at(principal, Utc::now())
    }

    pub fn parse_dashboard_session(&self, token: &str) -> Result<Principal, AuthError> {
        self.parse_dashboard_session_at(token, Utc::now())
    }

    fn mint_dashboard_session_at(
        &self,
        principal: &Principal,
        now: DateTime<Utc>,
    ) -> Result<String, AuthError> {
        if !principal.enabled || principal.role != PrincipalRole::Admin {
            return Err(AuthError::DashboardAdminRequired);
        }
        let claims = DashboardClaims {
            principal: principal.clone(),
            issued_at: now.timestamp(),
            expires_at: (now + ChronoDuration::hours(8)).timestamp(),
            version: 1,
        };
        let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims)?);
        let signature = URL_SAFE_NO_PAD.encode(self.sign_dashboard(&payload));
        Ok(format!("{payload}.{signature}"))
    }

    fn parse_dashboard_session_at(
        &self,
        token: &str,
        now: DateTime<Utc>,
    ) -> Result<Principal, AuthError> {
        let (payload, signature) = token
            .trim()
            .split_once('.')
            .ok_or(AuthError::InvalidDashboardSession)?;
        if payload.is_empty() || signature.is_empty() || signature.contains('.') {
            return Err(AuthError::InvalidDashboardSession);
        }
        let provided = URL_SAFE_NO_PAD
            .decode(signature)
            .map_err(|_| AuthError::InvalidDashboardSession)?;
        let expected = self.sign_dashboard(payload);
        if !bool::from(expected.as_slice().ct_eq(provided.as_slice())) {
            return Err(AuthError::InvalidDashboardSession);
        }
        let claims: DashboardClaims = serde_json::from_slice(
            &URL_SAFE_NO_PAD
                .decode(payload)
                .map_err(|_| AuthError::InvalidDashboardSession)?,
        )?;
        if claims.version != 1
            || claims.expires_at <= now.timestamp()
            || claims.issued_at > now.timestamp() + 60
            || !claims.principal.enabled
            || claims.principal.role != PrincipalRole::Admin
        {
            return Err(AuthError::InvalidDashboardSession);
        }
        Ok(claims.principal)
    }

    fn sign_dashboard(&self, payload: &str) -> Vec<u8> {
        let mut mac =
            HmacSha256::new_from_slice(&self.dashboard_secret).expect("HMAC accepts any key size");
        mac.update(DASHBOARD_SESSION_DOMAIN);
        mac.update(payload.as_bytes());
        mac.finalize().into_bytes().to_vec()
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct DashboardClaims {
    principal: Principal,
    issued_at: i64,
    expires_at: i64,
    version: u8,
}

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("managed token pepper must be at least 32 bytes")]
    TokenPepperTooShort,
    #[error("managed token is required")]
    TokenRequired,
    #[error("dashboard signing secret must be at least 32 bytes")]
    DashboardSecretTooShort,
    #[error("invalid bearer token")]
    InvalidBearer,
    #[error("unknown bearer token")]
    UnknownToken,
    #[error("managed token is revoked")]
    TokenRevoked,
    #[error("principal is disabled")]
    PrincipalDisabled,
    #[error("invalid principal")]
    InvalidPrincipal,
    #[error("project is required")]
    ProjectRequired,
    #[error("project is not allowed")]
    ProjectForbidden,
    #[error("dashboard requires an admin principal")]
    DashboardAdminRequired,
    #[error("invalid dashboard session")]
    InvalidDashboardSession,
    #[error("cloud auth store: {0}")]
    Store(#[source] CloudStoreError),
    #[error("dashboard session encoding: {0}")]
    Json(#[from] serde_json::Error),
}

fn legacy_principal(admin: bool) -> Principal {
    Principal {
        id: if admin { "legacy:admin" } else { "legacy:sync" }.to_owned(),
        kind: PrincipalKind::Legacy,
        display_name: if admin { "OPERATOR" } else { "LEGACY_SYNC" }.to_owned(),
        role: if admin {
            PrincipalRole::Admin
        } else {
            PrincipalRole::Member
        },
        enabled: true,
        source: if admin {
            PrincipalSource::LegacyAdmin
        } else {
            PrincipalSource::LegacySync
        },
        token_id: None,
    }
}

/// Rejects bearer values that cannot be a token before any comparison or
/// database lookup happens.
///
/// Interior whitespace means the header was split wrong or carries more than
/// one value, and an empty token must never reach the managed-token path where
/// an empty verifier could be compared against stored rows.
fn validate_bearer_shape(token: &str) -> Result<&str, AuthError> {
    let token = token.trim();
    if token.is_empty() || token.bytes().any(|byte| byte.is_ascii_whitespace()) {
        return Err(AuthError::InvalidBearer);
    }
    Ok(token)
}

fn constant_time_token_eq(presented: &str, expected: &str) -> bool {
    if presented.is_empty() || expected.is_empty() {
        return false;
    }
    let presented = Sha256::digest(presented.as_bytes());
    let expected = Sha256::digest(expected.as_bytes());
    bool::from(presented.as_slice().ct_eq(expected.as_slice()))
}

fn normalize_environment(environment: &str) -> String {
    let mut normalized = String::new();
    let mut previous_separator = false;
    for character in environment.trim().to_ascii_lowercase().chars() {
        if character.is_ascii_alphanumeric() {
            normalized.push(character);
            previous_separator = false;
        } else if !previous_separator && !normalized.is_empty() {
            normalized.push('-');
            previous_separator = true;
        }
    }
    while normalized.ends_with('-') {
        normalized.pop();
    }
    if normalized.is_empty() {
        "live".to_owned()
    } else {
        normalized
    }
}

/// The project a request names, normalised, or the reason it names none.
///
/// Split out of `authorize_project` because it is the half that needs no
/// database. Inside that method it sat behind a `&CloudStore`, so the only
/// tests that could reach it were among the eleven carrying
/// `#[ignore = "requires TEST_DATABASE_URL"]` — and none of those reaches
/// this check. One of the eleven does assert the refusal, at the store's own
/// entry points rather than here; what none of them can do is reach the auth
/// layer without a database. A mutation deleting the check survived the
/// entire suite.
///
/// Normalising here rather than at the caller is what makes the check mean
/// something: `"   "` and `"--"` are not empty as strings and are empty as
/// project names, and the normalised value is what the grant is looked up by
/// and what the row is written under.
fn required_project(raw: &str) -> Result<String, AuthError> {
    let project = crate::memory::normalize::project(raw);
    if project.is_empty() {
        return Err(AuthError::ProjectRequired);
    }
    Ok(project)
}

#[cfg(test)]
mod tests {

    /// A request that names no project is refused before any grant is read.
    ///
    /// This check used to sit behind a `&CloudStore`, which put it out of
    /// reach of every test that runs without PostgreSQL — and none of the
    /// eleven that need one reaches it here. Deleting it survived the whole
    /// suite.
    ///
    /// The cases that matter are the ones that are not empty as strings:
    /// whitespace only, which trims away to nothing.
    #[test]
    fn a_request_that_names_no_project_is_refused_before_any_grant_is_read() {
        for empty in [
            "", " ", "   ", "	
", " 
	 ",
        ] {
            assert!(
                matches!(required_project(empty), Err(AuthError::ProjectRequired)),
                "{empty:?} is not a project name"
            );
        }

        assert_eq!(required_project("  My--Project  ").unwrap(), "my-project");

        assert_eq!(required_project("--").unwrap(), "-");
        assert_eq!(required_project("__").unwrap(), "_");
    }

    use super::*;

    fn service() -> AuthService {
        AuthService::new(
            "dashboard-signing-secret-at-least-32-bytes",
            Some("managed-token-pepper-at-least-32-bytes"),
            "sync-secret",
            "admin-secret",
            &["proj-a".to_owned()],
        )
        .unwrap()
    }

    #[test]
    fn managed_token_has_domain_separated_hmac_format() {
        let hasher = ManagedTokenHasher::new("managed-token-pepper-at-least-32-bytes").unwrap();
        let verifier = hasher.hash("ltc_live_01234567_secret").unwrap();
        assert!(verifier.starts_with(MANAGED_TOKEN_HASH_PREFIX));
        assert!(hasher.verify("ltc_live_01234567_secret", &verifier));
        assert!(!hasher.verify("ltc_live_01234567_secre", &verifier));
        assert!(!hasher.verify("ltc_live_01234567_secret-extra", &verifier));
    }

    #[test]
    fn the_token_verifier_is_the_exact_value_a_deployment_already_stores() {
        let hasher = ManagedTokenHasher::new("managed-token-pepper-at-least-32-bytes").unwrap();
        assert_eq!(
            hasher.hash("ltc_live_01234567_secret").unwrap(),
            "hmac-sha256:v1:j1H0EGrOPNawCt5CL4FROibwKEMJsO5laoMWsVGSg-s"
        );
    }

    #[test]
    fn generated_token_uses_managed_format() {
        let token = ManagedToken::generate("Prod US-East");
        assert!(token.raw.starts_with("ltc_prod-us-east_"));
        assert_eq!(token.raw.split('_').count(), 4);
        assert!(token.raw.len() >= token.prefix.len() + 44);
    }

    #[test]
    fn legacy_comparison_rejects_prefixes_and_empty_values() {
        assert!(constant_time_token_eq("secret", "secret"));
        assert!(!constant_time_token_eq("secre", "secret"));
        assert!(!constant_time_token_eq("secret-extra", "secret"));
        assert!(!constant_time_token_eq("", ""));
    }

    #[test]
    fn dashboard_session_is_signed_opaque_and_expires() {
        let service = service();
        let principal = legacy_principal(true);
        let issued = DateTime::parse_from_rfc3339("2026-07-27T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let token = service
            .mint_dashboard_session_at(&principal, issued)
            .unwrap();
        assert!(!token.contains("admin-secret"));
        assert_eq!(
            service
                .parse_dashboard_session_at(&token, issued + ChronoDuration::hours(1))
                .unwrap(),
            principal
        );
        assert!(matches!(
            service.parse_dashboard_session_at(&token, issued + ChronoDuration::hours(9)),
            Err(AuthError::InvalidDashboardSession)
        ));
        assert!(matches!(
            service.parse_dashboard_session_at(&(token + "x"), issued),
            Err(AuthError::InvalidDashboardSession)
        ));
    }

    #[test]
    fn a_forged_dashboard_session_is_never_accepted() {
        let service = service();
        let issued = DateTime::parse_from_rfc3339("2026-07-27T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let token = service
            .mint_dashboard_session_at(&legacy_principal(true), issued)
            .unwrap();
        let (payload, signature) = token.split_once('.').expect("a signed session");
        let rejected = |token: String| {
            matches!(
                service.parse_dashboard_session_at(&token, issued),
                Err(AuthError::InvalidDashboardSession)
            )
        };

        let elevated = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&DashboardClaims {
                principal: Principal {
                    display_name: "intruder".to_owned(),
                    ..legacy_principal(true)
                },
                issued_at: issued.timestamp(),
                expires_at: (issued + ChronoDuration::hours(8)).timestamp(),
                version: 1,
            })
            .unwrap(),
        );
        assert!(rejected(format!("{elevated}.{signature}")));

        let other = AuthService::new(
            "another-dashboard-secret-of-at-least-32-bytes",
            None,
            "",
            "",
            &[],
        )
        .unwrap();
        let foreign = other
            .mint_dashboard_session_at(&legacy_principal(true), issued)
            .unwrap();
        assert!(rejected(foreign));

        assert!(rejected(payload.to_owned()));
        assert!(rejected(format!(".{signature}")));
        assert!(rejected(format!("{payload}.")));
        assert!(rejected(format!("{payload}.{signature}.{signature}")));
        assert!(rejected(format!("{payload}.not-base64!!")));
        assert!(rejected(String::new()));

        let ahead = service
            .mint_dashboard_session_at(&legacy_principal(true), issued + ChronoDuration::hours(2))
            .unwrap();
        assert!(rejected(ahead));
    }

    #[test]
    fn only_enabled_administrators_receive_a_dashboard_session() {
        let service = service();
        let issued = DateTime::parse_from_rfc3339("2026-07-27T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        assert!(matches!(
            service.mint_dashboard_session_at(&legacy_principal(false), issued),
            Err(AuthError::DashboardAdminRequired)
        ));
        let disabled = Principal {
            enabled: false,
            ..legacy_principal(true)
        };
        assert!(matches!(
            service.mint_dashboard_session_at(&disabled, issued),
            Err(AuthError::DashboardAdminRequired)
        ));
    }

    #[test]
    fn malformed_bearer_values_are_rejected_before_any_lookup() {
        for token in ["", "   ", "two tokens", "tab\tseparated", "line\nbreak"] {
            assert!(
                matches!(validate_bearer_shape(token), Err(AuthError::InvalidBearer)),
                "{token:?} must be refused"
            );
        }
        assert_eq!(
            validate_bearer_shape("  ltc_prod_abc_secret  ").unwrap(),
            "ltc_prod_abc_secret"
        );
    }

    #[test]
    fn short_secrets_are_rejected() {
        assert!(matches!(
            AuthService::new("short", None, "", "", &[]),
            Err(AuthError::DashboardSecretTooShort)
        ));
        assert!(matches!(
            ManagedTokenHasher::new("short"),
            Err(AuthError::TokenPepperTooShort)
        ));
    }
}
