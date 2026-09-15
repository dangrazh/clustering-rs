//! Configured sign-in establishes identity; review JSON never establishes authorship.
use crate::storage::{digest, id, now, AppError, Store, User};
use anyhow::{ensure, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use jsonwebtoken::{decode, decode_header, jwk::JwkSet, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use turso::params;

#[derive(Clone)]
pub struct AllowlistConfig {
    pub path: std::path::PathBuf,
    pub session_seconds: i64,
    pub secure_cookie: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AllowedUser {
    email: String,
    name: String,
}
impl AllowlistConfig {
    async fn users(&self) -> Result<HashMap<String, String>> {
        use tokio::io::AsyncReadExt;
        let file = tokio::fs::File::open(&self.path)
            .await
            .context("Cannot read email allowlist")?;
        let mut bytes = Vec::new();
        file.take(1024 * 1024 + 1).read_to_end(&mut bytes).await?;
        ensure!(bytes.len() <= 1024 * 1024, "Email allowlist exceeds 1 MiB");
        let entries: Vec<AllowedUser> =
            serde_json::from_slice(&bytes).context("Invalid email allowlist JSON")?;
        let mut users = HashMap::new();
        for entry in entries {
            let email = entry.email.trim().to_lowercase();
            let name = entry.name.trim().to_owned();
            ensure!(
                crate::workflow::valid_email(&email),
                "Invalid allowlist email"
            );
            ensure!(
                !name.is_empty() && name.len() <= 200 && !name.chars().any(char::is_control),
                "Invalid allowlist display name"
            );
            ensure!(
                users.insert(email, name).is_none(),
                "Duplicate allowlist email (case-insensitive)"
            );
        }
        Ok(users)
    }
}

#[derive(Clone)]
pub struct EntraConfig {
    pub tenant: String,
    pub client: String,
    pub secret: String,
    pub redirect: String,
    pub allow_guests: bool,
    pub session_seconds: i64,
}
impl EntraConfig {
    pub fn from_env() -> Result<Option<Self>> {
        let Some(tenant) = std::env::var("ENTRA_TENANT_ID").ok() else {
            return Ok(None);
        };
        uuid::Uuid::parse_str(&tenant)?;
        let get = |k| std::env::var(k).with_context(|| format!("Missing {k}"));
        let redirect = get("ENTRA_REDIRECT_URL")?;
        let url = url::Url::parse(&redirect)?;
        ensure!(
            url.scheme() == "https" && url.path() == "/auth/callback",
            "ENTRA_REDIRECT_URL must be HTTPS with /auth/callback path"
        );
        let session_seconds = get("APP_SESSION_SECONDS")?.parse::<i64>()?;
        ensure!(
            (300..=86400).contains(&session_seconds),
            "APP_SESSION_SECONDS must be 300–86400"
        );
        Ok(Some(Self {
            tenant,
            client: get("ENTRA_CLIENT_ID")?,
            secret: get("ENTRA_CLIENT_SECRET")?,
            redirect,
            allow_guests: get("ENTRA_ALLOW_GUESTS")?.parse()?,
            session_seconds,
        }))
    }
}
struct Pending {
    verifier: String,
    nonce: String,
    expires: i64,
}
pub struct Auth {
    pub config: Option<EntraConfig>,
    pub allowlist: Option<AllowlistConfig>,
    store: Arc<Store>,
    client: reqwest::Client,
    pending: Mutex<HashMap<String, Pending>>,
    keys: tokio::sync::RwLock<Option<(i64, JwkSet)>>,
}
#[derive(Clone, Serialize)]
pub struct Session {
    pub user: User,
    pub csrf: String,
    pub expires: i64,
}
#[derive(Clone, Deserialize)]
struct Claims {
    tid: String,
    oid: String,
    nonce: String,
    name: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    preferred_username: String,
    #[serde(default)]
    acct: Option<i64>,
}
#[derive(Deserialize)]
struct TokenResponse {
    id_token: String,
}
impl Auth {
    pub async fn from_env(store: Arc<Store>) -> Result<Self> {
        match std::env::var("APP_AUTH_MODE")
            .unwrap_or_else(|_| "entra".into())
            .as_str()
        {
            "entra" => Self::new(store, EntraConfig::from_env()?),
            "allowlist" => {
                Self::with_allowlist(
                    store,
                    AllowlistConfig {
                        path: std::env::var("APP_EMAIL_ALLOWLIST")
                            .context("APP_EMAIL_ALLOWLIST is required for allowlist sign-in")?
                            .into(),
                        session_seconds: crate::config::number(
                            "APP_SESSION_SECONDS",
                            28800,
                            300,
                            86400,
                        )? as i64,
                        secure_cookie: std::env::var("APP_COOKIE_SECURE")
                            .unwrap_or_else(|_| "true".into())
                            .parse()
                            .context("APP_COOKIE_SECURE must be true or false")?,
                    },
                )
                .await
            }
            _ => anyhow::bail!("APP_AUTH_MODE must be entra or allowlist"),
        }
    }
    pub async fn with_allowlist(store: Arc<Store>, config: AllowlistConfig) -> Result<Self> {
        ensure!(
            (300..=86400).contains(&config.session_seconds),
            "Invalid session lifetime"
        );
        config.users().await?;
        let mut auth = Self::new(store, None)?;
        auth.allowlist = Some(config);
        Ok(auth)
    }
    pub fn session_cookie(&self, token: &str, lifetime: i64) -> String {
        let secure = if self.allowlist.as_ref().is_none_or(|c| c.secure_cookie) {
            "; Secure"
        } else {
            ""
        };
        format!("app_session={token}; Path=/; HttpOnly{secure}; SameSite=Lax; Max-Age={lifetime}")
    }
    pub async fn email_login(&self, email: &str) -> Result<String> {
        let config = self.allowlist.as_ref().ok_or(AppError::Forbidden)?;
        let email = email.trim().to_lowercase();
        let users = config.users().await?;
        let name = users.get(&email).ok_or(AppError::Forbidden)?;
        let user = self
            .store
            .user(&format!("allowlist:{email}"), name, &email)
            .await?;
        let token = self.issue(&user, config.session_seconds).await?;
        Ok(self.session_cookie(&token, config.session_seconds))
    }
    pub fn new(store: Arc<Store>, config: Option<EntraConfig>) -> Result<Self> {
        Ok(Self {
            config,
            allowlist: None,
            store,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            pending: Mutex::new(HashMap::new()),
            keys: tokio::sync::RwLock::new(None),
        })
    }
    pub fn login(&self) -> Result<(String, String)> {
        let cfg = self
            .config
            .as_ref()
            .context("Entra sign-in is not configured. Contact the application operator.")?;
        let state = id();
        let verifier = format!("{}{}", id().replace('-', ""), id().replace('-', ""));
        let nonce = id();
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let mut pending = self.pending.lock().unwrap();
        pending.retain(|_, p| p.expires > now());
        ensure!(
            pending.len() < 1000,
            "Too many pending sign-ins. Retry shortly."
        );
        pending.insert(
            state.clone(),
            Pending {
                verifier,
                nonce: nonce.clone(),
                expires: now() + 600,
            },
        );
        let mut url = url::Url::parse(&format!(
            "https://login.microsoftonline.com/{}/oauth2/v2.0/authorize",
            cfg.tenant
        ))?;
        url.query_pairs_mut().extend_pairs([
            ("client_id", cfg.client.as_str()),
            ("response_type", "code"),
            ("redirect_uri", cfg.redirect.as_str()),
            ("scope", "openid profile email"),
            ("state", state.as_str()),
            ("nonce", nonce.as_str()),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
        ]);
        Ok((
            url.into(),
            format!("login_state={state}; Path=/auth; HttpOnly; Secure; SameSite=Lax; Max-Age=600"),
        ))
    }
    pub async fn callback(&self, state: &str, code: &str, cookie_state: &str) -> Result<String> {
        ensure!(
            !state.is_empty() && state == cookie_state,
            "Invalid login state"
        );
        let pending = self
            .pending
            .lock()
            .unwrap()
            .remove(state)
            .context("Login expired or already used")?;
        ensure!(pending.expires > now(), "Login expired");
        let cfg = self.config.as_ref().context("Entra not configured")?;
        let token: TokenResponse = self
            .client
            .post(format!(
                "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
                cfg.tenant
            ))
            .form(&[
                ("client_id", cfg.client.as_str()),
                ("client_secret", cfg.secret.as_str()),
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", cfg.redirect.as_str()),
                ("code_verifier", pending.verifier.as_str()),
            ])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let header = decode_header(&token.id_token)?;
        ensure!(
            header.alg == Algorithm::RS256,
            "Unsupported identity signature"
        );
        let kid = header.kid.context("Identity key missing")?;
        let mut key = None;
        for force in [false, true] {
            if force
                || self
                    .keys
                    .read()
                    .await
                    .as_ref()
                    .is_none_or(|(expires, _)| *expires < now())
            {
                let keys: JwkSet = self
                    .client
                    .get(format!(
                        "https://login.microsoftonline.com/{}/discovery/v2.0/keys",
                        cfg.tenant
                    ))
                    .send()
                    .await?
                    .error_for_status()?
                    .json()
                    .await?;
                *self.keys.write().await = Some((now() + 3600, keys));
            }
            key = self
                .keys
                .read()
                .await
                .as_ref()
                .and_then(|(_, keys)| keys.find(&kid))
                .cloned();
            if key.is_some() {
                break;
            }
        }
        let key = DecodingKey::from_jwk(&key.context("Unknown identity key")?)?;
        let (claims, email) = validate_identity(&token.id_token, &key, cfg, &pending.nonce)?;
        let user = self
            .store
            .user(
                &format!("entra:{}:{}", claims.tid, claims.oid),
                &claims.name,
                &email,
            )
            .await?;
        let token = self.issue(&user, cfg.session_seconds).await?;
        Ok(format!(
            "app_session={token}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age={}",
            cfg.session_seconds
        ))
    }
    pub async fn issue(&self, user: &User, lifetime: i64) -> Result<String> {
        let token = format!("{}{}", id(), id());
        let _m = self.store.maintenance.read().await;
        let _p = self.store.writers.acquire().await?;
        let c = self.store.connect().await?;
        c.execute(
            "INSERT INTO sessions VALUES(?,?,?,?)",
            params![
                digest(token.as_bytes()),
                user.id.clone(),
                id(),
                now() + lifetime
            ],
        )
        .await?;
        Ok(token)
    }
    pub async fn session(&self, cookies: &str) -> Result<Session> {
        let token = cookie(cookies, "app_session").ok_or(AppError::Unauthorized)?;
        let c = self.store.connect().await?;
        let row=c.query("SELECT u.id,u.name,u.email,s.csrf,s.expires,u.external_id FROM sessions s JOIN users u ON u.id=s.user_id WHERE s.token=? AND s.expires>? AND u.active=1",params![digest(token.as_bytes()),now()]).await?.next().await?.ok_or(AppError::Unauthorized)?;
        let mut session = Session {
            user: User {
                id: row.get(0)?,
                name: row.get(1)?,
                email: row.get(2)?,
            },
            csrf: row.get(3)?,
            expires: row.get(4)?,
        };
        if let Some(config) = &self.allowlist {
            ensure!(
                row.get::<String>(5)? == format!("allowlist:{}", session.user.email),
                AppError::Unauthorized
            );
            session.user.name = config
                .users()
                .await?
                .get(&session.user.email)
                .cloned()
                .ok_or(AppError::Unauthorized)?;
        } else if self.config.is_some() && !row.get::<String>(5)?.starts_with("entra:") {
            return Err(AppError::Unauthorized.into());
        }
        Ok(session)
    }
    pub async fn logout(&self, cookies: &str) -> Result<()> {
        let _m = self.store.maintenance.read().await;
        let _p = self.store.writers.acquire().await?;
        let c = self.store.connect().await?;
        if let Some(token) = cookie(cookies, "app_session") {
            c.execute(
                "DELETE FROM sessions WHERE token=?",
                [digest(token.as_bytes())],
            )
            .await?;
        }
        Ok(())
    }
}
pub fn cookie<'a>(header: &'a str, name: &str) -> Option<&'a str> {
    header.split(';').find_map(|s| {
        s.trim()
            .split_once('=')
            .filter(|(k, _)| *k == name)
            .map(|(_, v)| v)
    })
}

fn validate_identity(
    token: &str,
    key: &DecodingKey,
    cfg: &EntraConfig,
    nonce: &str,
) -> Result<(Claims, String)> {
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_audience(&[&cfg.client]);
    validation.set_issuer(&[format!(
        "https://login.microsoftonline.com/{}/v2.0",
        cfg.tenant
    )]);
    validation.validate_nbf = true;
    validation.set_required_spec_claims(&["exp", "iss", "aud", "nbf"]);
    let claims = decode::<Claims>(token, key, &validation)?.claims;
    ensure!(
        claims.tid == cfg.tenant && claims.nonce == nonce,
        "Identity tenant or nonce mismatch"
    );
    uuid::Uuid::parse_str(&claims.oid)?;
    ensure!(
        cfg.allow_guests || claims.acct == Some(0),
        "Company-member identity is required. Configure the Entra acct optional claim."
    );
    let email = if claims.email.is_empty() {
        claims.preferred_username.clone()
    } else {
        claims.email.clone()
    };
    ensure!(
        crate::workflow::valid_email(&email),
        "Entra must provide a valid email or email-form username"
    );
    ensure!(
        !claims.name.trim().is_empty(),
        "Entra must provide a display name"
    );
    Ok((claims, email))
}

#[cfg(test)]
#[path = "auth_tests.rs"]
mod tests;
