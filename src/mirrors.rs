use crate::utils::{AppError, Result};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CatboyRegion {
    Central,
    Us,
    Asia,
}

impl CatboyRegion {
    pub fn label(&self) -> &'static str {
        match self {
            CatboyRegion::Central => "Catboy (Central)",
            CatboyRegion::Us => "Catboy (US)",
            CatboyRegion::Asia => "Catboy (Asia)",
        }
    }

    pub fn base_url(&self) -> &'static str {
        match self {
            CatboyRegion::Central => "https://catboy.best",
            CatboyRegion::Us => "https://us.catboy.best",
            CatboyRegion::Asia => "https://sg.catboy.best",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MirrorKind {
    Nerinyan,
    Catboy(CatboyRegion),
    OsuDirect,
    Sayobot,
    Nekoha,
    Custom,
}

impl MirrorKind {
    pub fn label(&self) -> &'static str {
        match self {
            MirrorKind::Nerinyan => "Nerinyan",
            MirrorKind::Catboy(region) => region.label(),
            MirrorKind::OsuDirect => "osu.direct",
            MirrorKind::Sayobot => "Sayobot",
            MirrorKind::Nekoha => "Nekoha",
            MirrorKind::Custom => "Custom",
        }
    }

    pub fn rate_limit_backoff(&self) -> Duration {
        match self {
            MirrorKind::Nerinyan => Duration::from_secs(45),
            MirrorKind::Catboy(_) => Duration::from_secs(30),
            MirrorKind::OsuDirect => Duration::from_secs(75),
            MirrorKind::Sayobot => Duration::from_secs(60),
            MirrorKind::Nekoha => Duration::from_secs(45),
            MirrorKind::Custom => Duration::from_secs(60),
        }
    }

    pub fn download_template(&self, no_video: bool) -> Option<String> {
        match self {
            MirrorKind::Nerinyan => {
                let template = if no_video {
                    "https://api.nerinyan.moe/d/{id}?nv=1"
                } else {
                    "https://api.nerinyan.moe/d/{id}"
                };
                Some(template.to_string())
            }
            MirrorKind::Catboy(region) => {
                let suffix = if no_video { "n" } else { "" };
                Some(format!("{}/d/{{id}}{}", region.base_url(), suffix))
            }
            MirrorKind::OsuDirect => {
                let suffix = if no_video { "n" } else { "" };
                Some(format!("https://osu.direct/d/{{id}}{}", suffix))
            }
            MirrorKind::Sayobot => {
                let template = if no_video {
                    "https://dl.sayobot.cn/beatmaps/download/novideo/{id}"
                } else {
                    "https://dl.sayobot.cn/beatmaps/download/full/{id}"
                };
                Some(template.to_string())
            }
            MirrorKind::Nekoha => {
                // Nekoha doesn't support no-video downloads
                Some("https://mirror.nekoha.moe/api4/download/{id}".to_string())
            }
            MirrorKind::Custom => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MirrorEndpoint {
    pub kind: MirrorKind,
    pub template: Box<str>,
}

impl MirrorEndpoint {
    pub fn builtin(kind: MirrorKind, no_video: bool) -> Option<Self> {
        kind.download_template(no_video).map(|template| Self {
            kind,
            template: template.into_boxed_str(),
        })
    }

    pub fn custom(template: &str) -> Result<Self> {
        validate_template(template)?;
        Ok(Self {
            kind: MirrorKind::Custom,
            template: template.into(),
        })
    }

    pub fn url_for(&self, beatmapset_id: u32) -> String {
        self.template.replace("{id}", &beatmapset_id.to_string())
    }

    pub fn display_name(&self) -> &'static str {
        self.kind.label()
    }
}

pub fn validate_template(template: &str) -> Result<()> {
    if !template.contains("{id}") {
        return Err(AppError::config("Mirror URL must contain {id} placeholder"));
    }

    if !template.starts_with("http://") && !template.starts_with("https://") {
        return Err(AppError::config(
            "Mirror URL must start with http:// or https://",
        ));
    }

    Ok(())
}
