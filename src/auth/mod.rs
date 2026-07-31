use crate::config::constants::CONFIG_SUBDIR;
use crate::utils::{AppError, Result};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, info, warn};

const AUTH_FILE: &str = "auth.json";
const REFRESH_MARGIN_SECS: u64 = 60;
const OSU_TOKEN_URL: &str = "https://osu.ppy.sh/oauth/token";

/// Base URL for osu! API v2.
pub const OSU_API_BASE: &str = "https://osu.ppy.sh/api/v2";

/// osu!lazer's first-party OAuth client id. The id/secret are public in the
/// open-source osu!lazer client (`ppy/osu`, `ProductionEndpointConfiguration.cs`).
/// Only this first-party client may request the `*` (lazer-tier) scope that
/// carries beatmap-download privilege, and only via the password (ROPC) grant —
/// a self-service third-party OAuth app gets `invalid_scope`.
pub const LAZER_CLIENT_ID: &str = "5";
/// Public client secret for osu!lazer's first-party client. See [`LAZER_CLIENT_ID`].
pub const LAZER_CLIENT_SECRET: &str = "FGc9GAtyHzeQDshWP5Ah7dega8hJACAJpQtw6OXk";
/// Scope requested by the lazer password grant. `*` carries every privilege,
/// including the beatmap download endpoint.
pub const LAZER_SCOPE: &str = "*";
/// `x-api-version` header value sent on every api v2 request. Any recent
/// `YYYYMMDD` integer works; this mirrors a known-good osu!lazer build and is
/// **mandatory** — osu! rejects api v2 calls that omit it.
pub const X_API_VERSION: &str = "20250115";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: u64,
    pub token_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredAuth {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: u64,
    pub scopes: Vec<String>,
    /// Cached `/me` `is_supporter` flag. `None` means unknown — never probed,
    /// or the last probe failed — and reads as **not** supporter via
    /// [`is_supporter`](Self::is_supporter): this gates supporter-only
    /// features, so an unconfirmed account must never unlock them.
    /// `#[serde(default)]` so an `auth.json` written before this field
    /// existed still loads.
    #[serde(default)]
    pub supporter: Option<bool>,
}

impl StoredAuth {
    pub fn is_expired(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        now + REFRESH_MARGIN_SECS >= self.expires_at
    }

    pub fn bearer_token(&self) -> &str {
        &self.access_token
    }

    /// Whether this token carries the `*` (lazer-tier) scope the official-mirror
    /// download endpoint requires. The client-side gate that stops an old
    /// narrow-scope token from being attached to `MirrorKind::OsuApi`.
    pub fn has_lazer_scope(&self) -> bool {
        self.scopes.iter().any(|scope| scope == LAZER_SCOPE)
    }

    /// Whether the account carries osu!supporter, per the last successful
    /// `/me` probe. `None` (never probed, or the probe failed) reads as
    /// `false` — the conservative direction, since this gates supporter-only
    /// features.
    pub fn is_supporter(&self) -> bool {
        self.supporter == Some(true)
    }
}

fn auth_path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|d| d.join(CONFIG_SUBDIR).join(AUTH_FILE))
}

pub fn load() -> Option<StoredAuth> {
    let path = auth_path()?;
    let contents = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&contents).ok()
}

pub fn save(auth: &StoredAuth) -> Result<()> {
    let path = auth_path().ok_or_else(|| AppError::config("cannot determine config dir"))?;

    if let Some(parent) = path.parent()
        && !parent.exists()
    {
        std::fs::create_dir_all(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(parent)?.permissions();
            perms.set_mode(0o700);
            std::fs::set_permissions(parent, perms)?;
        }
    }

    let json = serde_json::to_string_pretty(auth)
        .map_err(|e| AppError::other_dynamic(format!("auth serialize: {e}").into_boxed_str()))?;

    // Create the temp file 0600 *before* the token is written, so it is never
    // briefly world-readable under the process umask; the rename preserves the
    // mode onto the final file.
    let tmp = path.with_extension("json.tmp");
    {
        use std::io::Write as _;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut file = opts.open(&tmp)?;
        file.write_all(json.as_bytes())?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, &path)?;

    #[cfg(unix)]
    {
        // Belt-and-suspenders: a stale pre-existing temp would have kept its old
        // mode (OpenOptions::mode only applies on creation), so re-assert 0600.
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path)?.permissions();
        perms.set_mode(0o600);
        std::fs::set_permissions(&path, perms)?;
    }

    debug!("auth saved to {}", path.display());
    Ok(())
}

pub fn delete() -> Result<()> {
    if let Some(path) = auth_path()
        && path.exists()
    {
        std::fs::remove_file(&path)?;
        info!("auth tokens removed");
    }
    Ok(())
}

pub fn token_request_failed(operation: &str, status: reqwest::StatusCode) -> AppError {
    AppError::other_dynamic(format!("{operation} failed ({status})").into_boxed_str())
}

/// Form body for the lazer password (ROPC) grant.
pub fn password_grant_params<'a>(username: &'a str, password: &'a str) -> [(&'a str, &'a str); 6] {
    [
        ("grant_type", "password"),
        ("client_id", LAZER_CLIENT_ID),
        ("client_secret", LAZER_CLIENT_SECRET),
        ("username", username),
        ("password", password),
        ("scope", LAZER_SCOPE),
    ]
}

/// Exchange an osu! username + password for a `*`-scope (lazer-tier) token via
/// the ROPC grant, and persist it.
///
/// The raw password is sent to `osu.ppy.sh` over TLS but **never stored** — only
/// the returned access/refresh tokens are saved to `auth.json`.
pub async fn password_grant(
    client: &reqwest::Client,
    username: &str,
    password: &str,
) -> Result<StoredAuth> {
    let params = password_grant_params(username, password);

    let resp = client
        .post(OSU_TOKEN_URL)
        .form(&params)
        .send()
        .await
        .map_err(|e| AppError::other_dynamic(format!("login request: {e}").into_boxed_str()))?;

    if !resp.status().is_success() {
        // The body is intentionally dropped — it may echo the submitted credentials.
        return Err(token_request_failed("login", resp.status()));
    }

    let token = resp
        .json::<TokenResponse>()
        .await
        .map_err(|e| AppError::other_dynamic(format!("token parse: {e}").into_boxed_str()))?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let auth = StoredAuth {
        client_id: LAZER_CLIENT_ID.to_string(),
        client_secret: LAZER_CLIENT_SECRET.to_string(),
        redirect_uri: String::new(),
        access_token: token.access_token,
        refresh_token: token.refresh_token,
        expires_at: now + token.expires_in,
        scopes: vec![LAZER_SCOPE.to_string()],
        // Not yet known — [`lazer_login`] fills this in from the same `/me`
        // probe it uses for the verification check.
        supporter: None,
    };

    save(&auth)?;
    Ok(auth)
}

/// Outcome of a full [`lazer_login`] attempt.
#[derive(Debug, Clone, Copy)]
pub struct LazerLoginOutcome {
    /// osu! requires device (new-IP / 2FA) verification before this token can
    /// download.
    pub needs_verification: bool,
    /// Confirmed osu!supporter status. Only ever `true` when
    /// `needs_verification` is `false` — a verification-pending `/me` probe
    /// 401s and carries no supporter data; see [`refresh_supporter_status`].
    pub supporter: bool,
}

/// Full lazer login: [`password_grant`] followed by a session-verification
/// probe.
///
/// The same `/me` probe response also carries `is_supporter`: when
/// verification is *not* required, the response is trusted and cached on the
/// stored token, so one request covers both signals rather than a second
/// round-trip just for supporter status.
pub async fn lazer_login(
    client: &reqwest::Client,
    username: &str,
    password: &str,
) -> Result<LazerLoginOutcome> {
    let mut auth = password_grant(client, username, password).await?;
    let probe = fetch_me(client, auth.bearer_token()).await;
    let needs_verification = session_verification_required_from(&probe);
    if !needs_verification {
        apply_supporter_probe(&mut auth, &probe);
        if let Err(err) = save(&auth) {
            warn!(error = %err, "failed to persist supporter status");
        }
    }
    Ok(LazerLoginOutcome {
        needs_verification,
        supporter: auth.is_supporter(),
    })
}

/// Parsed subset of the osu! `/me` response this module reads.
#[derive(Debug, Deserialize)]
struct MeResponse {
    #[serde(default)]
    is_supporter: bool,
    #[serde(default)]
    session_verified: Option<bool>,
    #[serde(default)]
    session_verification_method: Option<serde_json::Value>,
}

/// Outcome of a `/me` GET. `Unauthorized` mirrors a 401 (pending device
/// verification); `Unreachable` covers a network error or an unparseable
/// body — every reader of this type treats both as "no usable data".
enum MeProbe {
    Ok(MeResponse),
    Unauthorized,
    Unreachable,
}

/// Single `/me` GET, shared by [`session_verification_required`] and
/// [`lazer_login`] / [`refresh_supporter_status`] so a login makes one probe
/// request rather than a separate one per signal.
async fn fetch_me(client: &reqwest::Client, access_token: &str) -> MeProbe {
    let resp = match client
        .get(format!("{OSU_API_BASE}/me"))
        .bearer_auth(access_token)
        .header("x-api-version", X_API_VERSION)
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(err) => {
            warn!(error = %err, "/me probe failed; assuming no usable data");
            return MeProbe::Unreachable;
        }
    };

    // A pending device verification makes osu! reject `/me` with 401.
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return MeProbe::Unauthorized;
    }

    match resp.json::<MeResponse>().await {
        Ok(body) => MeProbe::Ok(body),
        Err(_) => MeProbe::Unreachable,
    }
}

/// Probe whether osu! requires session (device) verification before this token
/// can download.
///
/// **UNVERIFIED against osu-web source.** osu!lazer (`ppy/osu`) is the only
/// reference and it does not implement ROPC, so the exact `/me` shape for this
/// gate could not be confirmed offline. This parses the documented signals
/// defensively — a `401` response, `session_verified == false`, or a non-null
/// `session_verification_method` — and treats any network/parse error or
/// unrecognised body as **not required**, so a false positive never blocks an
/// otherwise-working login. Confirm and adjust against a real account.
pub async fn session_verification_required(client: &reqwest::Client, access_token: &str) -> bool {
    session_verification_required_from(&fetch_me(client, access_token).await)
}

fn session_verification_required_from(probe: &MeProbe) -> bool {
    match probe {
        MeProbe::Unauthorized => true,
        MeProbe::Unreachable => false,
        MeProbe::Ok(body) => {
            body.session_verified == Some(false) || body.session_verification_method.is_some()
        }
    }
}

/// Applies a `/me` probe's `is_supporter` to `auth` when the probe carries a
/// usable body. A failed or unauthorized probe leaves `auth.supporter`
/// untouched — fail-soft, never claims a status it couldn't confirm.
fn apply_supporter_probe(auth: &mut StoredAuth, probe: &MeProbe) {
    if let MeProbe::Ok(body) = probe {
        auth.supporter = Some(body.is_supporter);
    }
}

/// Re-probes `/me` and persists `is_supporter` on `auth`, returning the
/// resulting [`StoredAuth::is_supporter`] value. Used after the
/// device-verification step succeeds, where the original login's `/me` probe
/// 401'd and never learned the account's supporter status.
///
/// Fail-soft: a network error or unparseable body just logs and leaves the
/// cached value as-is — this must never fail an otherwise-successful
/// verification.
pub async fn refresh_supporter_status(client: &reqwest::Client, auth: &mut StoredAuth) -> bool {
    let probe = fetch_me(client, auth.bearer_token()).await;
    apply_supporter_probe(auth, &probe);
    if matches!(probe, MeProbe::Ok(_))
        && let Err(err) = save(auth)
    {
        warn!(error = %err, "failed to persist supporter status");
    }
    auth.is_supporter()
}

/// Submit the emailed / TOTP session-verification code for the given token.
pub async fn submit_session_verification(
    client: &reqwest::Client,
    access_token: &str,
    code: &str,
) -> Result<()> {
    let resp = client
        .post(format!("{OSU_API_BASE}/session/verify"))
        .bearer_auth(access_token)
        .header("x-api-version", X_API_VERSION)
        .form(&[("verification_key", code.trim())])
        .send()
        .await
        .map_err(|e| AppError::other_dynamic(format!("verify request: {e}").into_boxed_str()))?;

    if !resp.status().is_success() {
        return Err(token_request_failed("verification", resp.status()));
    }
    Ok(())
}

/// Ask osu! to re-send (reissue) the session-verification code.
pub async fn reissue_session_verification(
    client: &reqwest::Client,
    access_token: &str,
) -> Result<()> {
    let resp = client
        .post(format!("{OSU_API_BASE}/session/verify/reissue"))
        .bearer_auth(access_token)
        .header("x-api-version", X_API_VERSION)
        .send()
        .await
        .map_err(|e| AppError::other_dynamic(format!("reissue request: {e}").into_boxed_str()))?;

    if !resp.status().is_success() {
        return Err(token_request_failed("code reissue", resp.status()));
    }
    Ok(())
}

pub fn refresh_params<'a>(
    client_id: &'a str,
    client_secret: &'a str,
    refresh_token: &'a str,
    scope: &'a str,
) -> [(&'a str, &'a str); 5] {
    [
        ("client_id", client_id),
        ("client_secret", client_secret),
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("scope", scope),
    ]
}

/// Refresh an access token. `scope` must match the stored token's scope so a
/// `*` (lazer) token refreshes with `*` rather than a narrower default.
pub async fn refresh(
    client: &reqwest::Client,
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
    scope: &str,
) -> Result<TokenResponse> {
    let params = refresh_params(client_id, client_secret, refresh_token, scope);

    let resp = client
        .post(OSU_TOKEN_URL)
        .form(&params)
        .send()
        .await
        .map_err(|e| AppError::other_dynamic(format!("refresh request: {e}").into_boxed_str()))?;

    if !resp.status().is_success() {
        return Err(token_request_failed("token refresh", resp.status()));
    }

    resp.json::<TokenResponse>()
        .await
        .map_err(|e| AppError::other_dynamic(format!("refresh parse: {e}").into_boxed_str()))
}

pub fn client_credentials_params<'a>(
    client_id: &'a str,
    client_secret: &'a str,
) -> [(&'a str, &'a str); 4] {
    [
        ("client_id", client_id),
        ("client_secret", client_secret),
        ("grant_type", "client_credentials"),
        ("scope", "public"),
    ]
}

pub async fn client_credentials(
    client: &reqwest::Client,
    client_id: &str,
    client_secret: &str,
) -> Result<TokenResponse> {
    let params = client_credentials_params(client_id, client_secret);
    request_token(client, OSU_TOKEN_URL, &params).await
}

async fn request_token(
    client: &reqwest::Client,
    token_url: &str,
    params: &[(&str, &str)],
) -> Result<TokenResponse> {
    let resp = client
        .post(token_url)
        .form(params)
        .send()
        .await
        .map_err(|e| AppError::other_dynamic(format!("token request: {e}").into_boxed_str()))?;

    if !resp.status().is_success() {
        return Err(token_request_failed("token request", resp.status()));
    }

    resp.json::<TokenResponse>()
        .await
        .map_err(|e| AppError::other_dynamic(format!("token parse: {e}").into_boxed_str()))
}

pub async fn ensure_valid(client: &reqwest::Client, auth: &mut StoredAuth) -> Result<()> {
    if !auth.is_expired() {
        return Ok(());
    }

    let token_resp = if let Some(refresh_token) = auth.refresh_token.as_deref() {
        info!("refreshing OAuth token");
        // Refresh with the token's own scope so a `*` (lazer) token keeps `*`.
        let scope = auth.scopes.join(" ");
        refresh(
            client,
            &auth.client_id,
            &auth.client_secret,
            refresh_token,
            &scope,
        )
        .await?
    } else {
        info!("refreshing OAuth token with client credentials");
        client_credentials(client, &auth.client_id, &auth.client_secret).await?
    };
    apply_token_response(auth, token_resp);
    let snap = auth.clone();
    tokio::task::spawn_blocking(move || save(&snap))
        .await
        .map_err(|e| {
            AppError::other_dynamic(format!("save task panicked: {e}").into_boxed_str())
        })??;
    Ok(())
}

fn apply_token_response(auth: &mut StoredAuth, resp: TokenResponse) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    auth.access_token = resp.access_token;
    if let Some(rt) = resp.refresh_token {
        auth.refresh_token = Some(rt);
    }
    auth.expires_at = now + resp.expires_in;
}

#[cfg(test)]
#[path = "../../tests/unit/auth_mod.rs"]
mod tests;

#[cfg(test)]
#[path = "../../tests/unit/auth.rs"]
mod integration;
