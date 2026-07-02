pub(crate) mod pool;

pub(crate) use pool::{Acquire, MirrorPool};

use crate::error::{Error, Result};
use reqwest::header::HeaderMap;
use std::fmt::Write as _;
use std::time::Duration;

/// Starting request spacing for the osu! official API ([`MirrorKind::OsuApi`]).
///
/// osu!'s API v2 guidance is roughly 60 requests/minute, so one request per
/// second is the floor the scheduler's AIMD spacing decays back toward for this
/// mirror; a 429 only ever widens it. Independent of the *reactive* per-mirror
/// [`MirrorKind::rate_limit_backoff`] cooldown applied after a 429.
///
/// Note: this does **not** keep downloads under osu!'s separate hourly download
/// quota (~10–20/hour). Once that cap is hit osu! returns 429 and the cooldown
/// takes over.
#[cfg(not(test))]
pub(crate) const OSU_API_BASE_INTERVAL: Duration = Duration::from_secs(1);
#[cfg(test)]
pub(crate) const OSU_API_BASE_INTERVAL: Duration = Duration::from_millis(1);

/// Starting request spacing for every mirror except [`MirrorKind::OsuApi`].
///
/// The per-slot AIMD floor the scheduler decays back toward after a run of
/// successes; a 429 doubles the live spacing up to the ceiling. Keyed per slot,
/// so two custom mirrors sharing [`MirrorKind::Custom`] pace independently.
#[cfg(not(test))]
pub(crate) const BASE_INTERVAL: Duration = Duration::from_millis(250);
#[cfg(test)]
pub(crate) const BASE_INTERVAL: Duration = Duration::from_millis(1);

/// Mirror type identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MirrorKind {
    /// Nerinyan mirror.
    Nerinyan,
    /// osu.direct mirror.
    OsuDirect,
    /// Sayobot mirror.
    Sayobot,
    /// Nekoha mirror.
    Nekoha,
    /// Beatconnect mirror (anonymous direct downloads).
    Beatconnect,
    /// osu!dl mirror (anonymous Cloudflare R2; ranked/approved/loved only).
    Osudl,
    /// catboy.best mirror (anonymous direct downloads).
    Catboy,
    /// Hinamizawa mirror (cascades across the other mirrors server-side).
    Hinamizawa,
    /// Official osu! API v2 download endpoint.
    ///
    /// Unlike the other built-ins this requires an `Authorization: Bearer`
    /// header carrying a user token with the `*` (lazer-tier) scope **and** an
    /// `x-api-version` header; the caller must attach both via
    /// [`Mirror::with_headers`]. See [`MirrorKind::requires_auth`].
    OsuApi,
    /// Custom mirror with user-provided URL template.
    Custom,
}

struct ProviderMeta {
    label: &'static str,
    #[cfg_attr(test, allow(dead_code))]
    backoff_secs: u64,
    template: &'static str,
    template_no_video: &'static str,
}

impl MirrorKind {
    fn meta(&self) -> Option<ProviderMeta> {
        match self {
            MirrorKind::Nerinyan => Some(ProviderMeta {
                label: "Nerinyan",
                backoff_secs: 45,
                template: "https://api.nerinyan.moe/d/{id}",
                template_no_video: "https://api.nerinyan.moe/d/{id}?nv=1",
            }),
            MirrorKind::OsuDirect => Some(ProviderMeta {
                label: "osu.direct",
                backoff_secs: 75,
                template: "https://osu.direct/d/{id}",
                template_no_video: "https://osu.direct/d/{id}n",
            }),
            MirrorKind::Sayobot => Some(ProviderMeta {
                label: "Sayobot",
                backoff_secs: 60,
                template: "https://dl.sayobot.cn/beatmaps/download/full/{id}",
                template_no_video: "https://dl.sayobot.cn/beatmaps/download/novideo/{id}",
            }),
            MirrorKind::Nekoha => Some(ProviderMeta {
                label: "Nekoha",
                backoff_secs: 45,
                template: "https://mirror.nekoha.moe/api4/download/{id}",
                template_no_video: "https://mirror.nekoha.moe/api4/download/{id}",
            }),
            MirrorKind::Beatconnect => Some(ProviderMeta {
                label: "Beatconnect",
                backoff_secs: 60,
                template: "https://beatconnect.io/b/{id}/",
                template_no_video: "https://beatconnect.io/b/{id}/?novideo=1",
            }),
            MirrorKind::Osudl => Some(ProviderMeta {
                // No published rate limit; Cloudflare may throttle at the edge.
                // Match the other anonymous mirrors' penalty backoff.
                label: "osu!dl",
                backoff_secs: 45,
                template: "https://osudl.org/s/{id}",
                template_no_video: "https://osudl.org/s/{id}?video=false",
            }),
            MirrorKind::Catboy => Some(ProviderMeta {
                // No published rate limit; a fast anonymous mirror. Match the
                // other anonymous mirrors' penalty backoff.
                label: "catboy.best",
                backoff_secs: 45,
                template: "https://catboy.best/d/{id}",
                // catboy.best strips video with the trailing `n` suffix
                // (verified live: full set vs `/d/{id}n` differ in size).
                template_no_video: "https://catboy.best/d/{id}n",
            }),
            MirrorKind::Hinamizawa => Some(ProviderMeta {
                label: "Hinamizawa",
                backoff_secs: 45,
                template: "https://mirror.hinamizawa.ai/api/v1/hinai/d/{id}",
                template_no_video: "https://mirror.hinamizawa.ai/api/v1/hinai/d/{id}?no_video=true",
            }),
            MirrorKind::OsuApi => Some(ProviderMeta {
                // Hard hourly download quota (10 free / 20 supporter) means a 429
                // takes the mirror out for a long time — back off aggressively.
                label: "osu! official",
                backoff_secs: 300,
                template: "https://osu.ppy.sh/api/v2/beatmapsets/{id}/download",
                // `?noVideo=1` is the verified no-video query param (osu-web).
                template_no_video: "https://osu.ppy.sh/api/v2/beatmapsets/{id}/download?noVideo=1",
            }),
            MirrorKind::Custom => None,
        }
    }

    /// All built-in mirror kinds, in default registration order. Excludes
    /// [`MirrorKind::Custom`].
    pub const BUILTINS: &'static [MirrorKind] = &[
        MirrorKind::OsuDirect,
        MirrorKind::Nerinyan,
        MirrorKind::Sayobot,
        MirrorKind::Nekoha,
        MirrorKind::Beatconnect,
        MirrorKind::Osudl,
        MirrorKind::Catboy,
        MirrorKind::Hinamizawa,
        MirrorKind::OsuApi,
    ];

    /// Whether this mirror needs a caller-supplied `Authorization` header to
    /// download. Only [`MirrorKind::OsuApi`] does; every other mirror downloads
    /// anonymously. Use this to skip auth-gated mirrors in anonymous contexts
    /// (e.g. availability probes).
    #[inline]
    pub fn requires_auth(&self) -> bool {
        matches!(self, MirrorKind::OsuApi)
    }

    /// Display label for this mirror.
    #[inline]
    pub fn label(&self) -> &'static str {
        match self {
            MirrorKind::Custom => "Custom",
            other => other.meta().expect("builtin mirror has meta").label,
        }
    }

    /// Display host for this mirror (e.g. `"osu.direct"`). Returns `"custom"`
    /// for [`MirrorKind::Custom`]. Suitable for UI labels.
    #[inline]
    pub fn host(&self) -> &'static str {
        match self {
            MirrorKind::Nerinyan => "api.nerinyan.moe",
            MirrorKind::OsuDirect => "osu.direct",
            MirrorKind::Sayobot => "dl.sayobot.cn",
            MirrorKind::Nekoha => "mirror.nekoha.moe",
            MirrorKind::Beatconnect => "beatconnect.io",
            MirrorKind::Osudl => "osudl.org",
            MirrorKind::Catboy => "catboy.best",
            MirrorKind::Hinamizawa => "mirror.hinamizawa.ai",
            MirrorKind::OsuApi => "osu.ppy.sh",
            MirrorKind::Custom => "custom",
        }
    }

    /// Per-slot base request spacing for the scheduler's AIMD limiter: the floor
    /// a slot's interval decays back toward. [`MirrorKind::OsuApi`] uses
    /// [`OSU_API_BASE_INTERVAL`]; every other kind uses [`BASE_INTERVAL`].
    #[inline]
    pub(crate) fn base_request_interval(&self) -> Duration {
        match self {
            MirrorKind::OsuApi => OSU_API_BASE_INTERVAL,
            _ => BASE_INTERVAL,
        }
    }

    pub(crate) fn rate_limit_backoff(&self) -> Duration {
        if cfg!(test) {
            return Duration::from_millis(10);
        }
        match self {
            MirrorKind::Custom => Duration::from_secs(60),
            other => {
                Duration::from_secs(other.meta().expect("builtin mirror has meta").backoff_secs)
            }
        }
    }

    fn download_template(&self, no_video: bool) -> Option<&'static str> {
        match self {
            MirrorKind::Custom => None,
            other => {
                let meta = other.meta().expect("builtin mirror has meta");
                Some(if no_video {
                    meta.template_no_video
                } else {
                    meta.template
                })
            }
        }
    }
}

/// URL template split into the parts before and after `{id}`, parsed once at
/// construction so `url_for` can build the URL with a single allocation.
#[derive(Debug, Clone)]
struct SplitTemplate {
    prefix: Box<str>,
    suffix: Box<str>,
}

impl SplitTemplate {
    fn new(template: &str) -> Self {
        // validate_template already guarantees `{id}` is present.
        let (prefix, suffix) = template.split_once("{id}").unwrap_or((template, ""));
        Self {
            prefix: prefix.into(),
            suffix: suffix.into(),
        }
    }
}

/// Identifies the mirror an [`Event`](crate::Event) refers to, pairing the
/// [`MirrorKind`] tag with a display host.
///
/// Built-in mirrors share one host per kind, but every [`MirrorKind::Custom`]
/// mirror has its own URL, so the kind alone cannot tell two custom mirrors
/// apart. This carries the parsed host (e.g. `"example.com"`) so consumers can
/// distinguish and label custom mirrors individually.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirrorRef {
    /// Provider kind.
    pub kind: MirrorKind,
    /// Display host: the static host for built-ins, the parsed URL host for
    /// custom mirrors.
    pub host: Box<str>,
}

impl MirrorRef {
    /// Display label: the provider label for built-ins ([`MirrorKind::label`]),
    /// the per-mirror host for [`MirrorKind::Custom`].
    #[inline]
    pub fn label(&self) -> &str {
        match self.kind {
            MirrorKind::Custom => &self.host,
            other => other.label(),
        }
    }
}

/// Parse the host of a mirror URL template for display.
///
/// Strips the scheme, any `userinfo@`, the port, and the path/query, leaving the
/// bare host. Falls back to the trimmed input when no host can be isolated.
fn parse_host(template: &str) -> Box<str> {
    let after_scheme = template
        .strip_prefix("https://")
        .or_else(|| template.strip_prefix("http://"))
        .unwrap_or(template);
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(after_scheme);
    let host = authority.rsplit('@').next().unwrap_or(authority);
    // IPv6 literals are bracketed (`[2001:db8::1]:8080`); keep the bracketed
    // host and drop the port. Other hosts split on the first `:` (port sep).
    let host = match host.strip_prefix('[').and_then(|rest| rest.find(']')) {
        Some(close) => &host[..close + 2],
        None => host.split(':').next().unwrap_or(host),
    };
    let host = if host.is_empty() { template } else { host };
    host.into()
}

/// Mirror endpoint for downloading beatmapsets.
#[derive(Debug, Clone)]
pub struct Mirror {
    pub(crate) kind: MirrorKind,
    pub(crate) template: Box<str>,
    split: SplitTemplate,
    /// Display host, parsed once at construction. See [`MirrorRef`].
    host: Box<str>,
    pub(crate) headers: Option<HeaderMap>,
    pub(crate) no_video: bool,
}

impl Mirror {
    /// Custom mirror with a URL template.
    ///
    /// Template must contain `{id}` and start with `http://` or `https://`.
    /// Use [`Mirror::validate_template`] to check a template without allocating.
    pub fn custom(template: impl Into<String>) -> Result<Self> {
        let template = template.into();
        Self::validate_template(&template)?;
        let split = SplitTemplate::new(&template);
        let host = parse_host(&template);
        Ok(Self {
            kind: MirrorKind::Custom,
            template: template.into_boxed_str(),
            split,
            host,
            headers: None,
            no_video: false,
        })
    }

    /// Validate a custom mirror URL template without constructing a [`Mirror`].
    ///
    /// The template must contain exactly one `{id}` placeholder and start with
    /// `http://` or `https://`. Prefer this over [`Mirror::custom`] when only
    /// checking validity (e.g. live input validation in a UI).
    pub fn validate_template(template: &str) -> Result<()> {
        match template.matches("{id}").count() {
            0 => return Err(Error::mirror("Mirror URL must contain {id} placeholder")),
            1 => {}
            _ => {
                return Err(Error::mirror(
                    "Mirror URL must contain exactly one {id} placeholder",
                ));
            }
        }

        if !template.starts_with("http://") && !template.starts_with("https://") {
            return Err(Error::mirror(
                "Mirror URL must start with http:// or https://",
            ));
        }

        Ok(())
    }

    /// Construct a built-in mirror from its [`MirrorKind`].
    ///
    /// Returns `None` for [`MirrorKind::Custom`] since custom mirrors have no
    /// predefined template — use [`Mirror::custom`] for those.
    pub fn builtin(kind: MirrorKind) -> Option<Self> {
        match kind {
            MirrorKind::Custom => None,
            other => Some(Self::new_builtin(other)),
        }
    }

    /// Nerinyan mirror (<https://api.nerinyan.moe>).
    pub fn nerinyan() -> Self {
        Self::new_builtin(MirrorKind::Nerinyan)
    }

    /// osu.direct mirror.
    pub fn osu_direct() -> Self {
        Self::new_builtin(MirrorKind::OsuDirect)
    }

    /// Sayobot mirror.
    pub fn sayobot() -> Self {
        Self::new_builtin(MirrorKind::Sayobot)
    }

    /// Nekoha mirror.
    pub fn nekoha() -> Self {
        Self::new_builtin(MirrorKind::Nekoha)
    }

    /// Beatconnect mirror (anonymous direct downloads).
    pub fn beatconnect() -> Self {
        Self::new_builtin(MirrorKind::Beatconnect)
    }

    /// osu!dl mirror (anonymous Cloudflare R2 storage).
    ///
    /// Coverage is ranked/approved/loved only; a graveyard/pending set returns
    /// `404`, which the download loop treats as a miss and rotates past.
    pub fn osudl() -> Self {
        Self::new_builtin(MirrorKind::Osudl)
    }

    /// catboy.best mirror (anonymous direct downloads).
    pub fn catboy() -> Self {
        Self::new_builtin(MirrorKind::Catboy)
    }

    /// Hinamizawa mirror (server-side cascade across the other mirrors).
    pub fn hinamizawa() -> Self {
        Self::new_builtin(MirrorKind::Hinamizawa)
    }

    /// Official osu! API v2 download mirror.
    ///
    /// The returned mirror has **no** auth header; downloads will fail with
    /// `401`/`403` until the caller attaches a `*` (lazer-tier) bearer token
    /// plus an `x-api-version` header via [`Mirror::with_headers`]. See
    /// [`MirrorKind::requires_auth`].
    pub fn osu_api() -> Self {
        Self::new_builtin(MirrorKind::OsuApi)
    }

    /// Every built-in mirror, in the library's default preference order.
    ///
    /// Includes [`MirrorKind::OsuApi`], which requires a caller-supplied auth
    /// header — filter with [`MirrorKind::requires_auth`] in anonymous contexts.
    pub fn builtins() -> Vec<Mirror> {
        vec![
            Mirror::nerinyan(),
            Mirror::osu_direct(),
            Mirror::sayobot(),
            Mirror::nekoha(),
            Mirror::beatconnect(),
            Mirror::osudl(),
            Mirror::catboy(),
            Mirror::hinamizawa(),
            Mirror::osu_api(),
        ]
    }

    fn new_builtin(kind: MirrorKind) -> Self {
        let template = kind
            .download_template(false)
            .expect("builtin mirror has template");
        let split = SplitTemplate::new(template);
        Self {
            kind,
            template: template.into(),
            split,
            host: kind.host().into(),
            headers: None,
            no_video: false,
        }
    }

    /// Attach HTTP headers to requests sent through this mirror.
    #[must_use]
    pub fn with_headers(mut self, headers: HeaderMap) -> Self {
        self.headers = Some(headers);
        self
    }

    /// Strip video from the served archive where the mirror supports it.
    /// No-op for custom mirrors and for mirrors without a no-video variant.
    #[must_use]
    pub fn no_video(mut self) -> Self {
        self.no_video = true;
        if let Some(template) = self.kind.download_template(true) {
            self.split = SplitTemplate::new(template);
            self.template = template.into();
        }
        self
    }

    /// Mirror kind.
    pub fn kind(&self) -> MirrorKind {
        self.kind
    }

    /// URL template used by this mirror. Contains `{id}` for substitution.
    pub fn template(&self) -> &str {
        &self.template
    }

    /// Display host for this mirror: the static host for built-ins, the parsed
    /// URL host for [`MirrorKind::Custom`]. Unlike [`MirrorKind::host`] this
    /// distinguishes individual custom mirrors.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Lightweight identity ([`MirrorRef`]) for this mirror, pairing its kind
    /// with its display host. Used to label custom mirrors individually in
    /// [`Event`](crate::Event)s.
    pub fn mirror_ref(&self) -> MirrorRef {
        MirrorRef {
            kind: self.kind,
            host: self.host.clone(),
        }
    }

    pub(crate) fn headers(&self) -> Option<&HeaderMap> {
        self.headers.as_ref()
    }

    #[inline]
    pub(crate) fn url_for(&self, beatmapset_id: u32) -> String {
        let id_digits = beatmapset_id.checked_ilog10().unwrap_or(0) as usize + 1;
        let cap = self.split.prefix.len() + id_digits + self.split.suffix.len();
        let mut url = String::with_capacity(cap);
        url.push_str(&self.split.prefix);
        write!(url, "{beatmapset_id}").expect("write to String is infallible");
        url.push_str(&self.split.suffix);
        url
    }

    #[cfg(test)]
    pub(crate) fn with_kind_and_template(kind: MirrorKind, template: impl Into<String>) -> Self {
        let template = template.into();
        let split = SplitTemplate::new(&template);
        let host = parse_host(&template);
        Self {
            kind,
            template: template.into_boxed_str(),
            split,
            host,
            headers: None,
            no_video: false,
        }
    }
}

#[cfg(test)]
#[path = "../../tests/mirrors.rs"]
mod tests;
