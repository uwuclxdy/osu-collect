use std::borrow::Cow;

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use super::theme::blend;
use super::{accent, accent_alt, text_dim};

const BRAND: &str = " osu!collect";
const VERSION: &str = concat!(" v", env!("CARGO_PKG_VERSION"), " ");

/// Header inputs other than the frame. Grouped into a struct so the brand
/// animation inputs (`tick`, `downloading`) ride along without a long argument
/// list. The `frame` is a separate `render` argument to keep its borrow tight.
pub struct RenderParams<'a, 't> {
    pub area: Rect,
    pub tabs: &'a [Cow<'t, str>],
    pub active: usize,
    /// Global frame tick; drives the brand shimmer phase.
    pub tick: u64,
    /// True while any download is in a non-terminal stage; idle renders the
    /// brand statically.
    pub downloading: bool,
    /// Ease-in ramp (0..1) for the shimmer. Rises from 0 when downloading
    /// begins so the animation fades in instead of cutting in.
    pub brand_ramp: f32,
    /// Newer release version (semver, no leading `v`) when one is available in
    /// notify-only mode; renders as an accent `↑ v<version>` on the right,
    /// replacing the dim current-version label.
    pub update_version: Option<&'a str>,
}

pub fn render(frame: &mut Frame, params: RenderParams<'_, '_>) {
    let RenderParams {
        area,
        tabs,
        active,
        tick,
        downloading,
        brand_ramp,
        update_version,
    } = params;

    if area.width == 0 || area.height == 0 {
        return;
    }

    // Borrow the static label when idle (zero alloc, per the version bench); an
    // available update needs a formatted `↑ v<new>` string.
    let version_text: Cow<'_, str> = match update_version {
        Some(v) => format!(" ↑ v{v} ").into(),
        None => VERSION.into(),
    };
    let version_width = version_text.chars().count() as u16;
    let brand_width = BRAND.chars().count() as u16;

    let layout = Layout::horizontal([
        Constraint::Length(brand_width),
        Constraint::Min(0),
        Constraint::Length(version_width),
    ])
    .split(area);

    frame.render_widget(
        Paragraph::new(Line::from(brand_spans(tick, downloading, brand_ramp))),
        layout[0],
    );

    let mut spans: Vec<Span<'_>> = Vec::with_capacity(tabs.len() * 3);
    // bullet separator between brand and first tab
    spans.push(Span::styled("  •  ", Style::default().fg(text_dim())));
    for (index, title) in tabs.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw("   "));
        }
        let style = if index == active {
            Style::default().fg(accent()).bold().underlined()
        } else {
            Style::default().fg(text_dim())
        };
        spans.push(Span::styled(title.clone(), style));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).alignment(Alignment::Left),
        layout[1],
    );

    let version_style = if update_version.is_some() {
        Style::default().fg(accent()).bold()
    } else {
        Style::default().fg(text_dim())
    };
    frame.render_widget(
        Paragraph::new(version_text.as_ref())
            .style(version_style)
            .alignment(Alignment::Right),
        layout[2],
    );
}

/// Build the brand line. Idle: a static bold `accent_alt` wordmark. Downloading:
/// a subtle accent shimmer sweeps left-to-right across the letters, so the brand
/// reads as a quiet live-status cue rather than a loud spinner.
///
/// `ramp` (0..1) scales the shimmer depth, easing in when a download starts and
/// back out when it finishes — at `ramp == 0` every letter sits on the base
/// color (indistinguishable from idle), so the wave keeps rendering through the
/// fade-out even though `downloading` is already false.
fn brand_spans(tick: u64, downloading: bool, ramp: f32) -> Vec<Span<'static>> {
    let base = accent_alt();
    let ramp = ramp.clamp(0.0, 1.0);
    if !downloading && ramp <= 0.0 {
        return vec![Span::styled(BRAND, Style::default().fg(base).bold())];
    }

    // A cosine wave crest travels across the brand. `WAVE_PERIOD` is the tick
    // count for one full left-to-right sweep; `MAX_MIX` caps how far each letter
    // leans toward the accent so it pulses instead of strobing; `MAX_WHITE` caps
    // the white sparkle riding the crest tip. `ramp` eases the whole effect in
    // and out so it materializes and dissolves gently rather than snapping.
    const WAVE_PERIOD: f32 = 16.0;
    const MAX_MIX: f32 = 0.65;
    const MAX_WHITE: f32 = 0.4;
    let depth = MAX_MIX * ramp;
    let highlight = accent();
    let white = Color::Rgb(255, 255, 255);

    // Constant-velocity crest: a looping phase fed straight to `cos`, so the
    // sweep wraps seamlessly with no velocity break. (The old ease-out stalled
    // the crest at each pass end then snapped it back to full speed, which read
    // as the wave resetting.)
    let crest = (tick as f32 / WAVE_PERIOD).fract() * std::f32::consts::TAU;
    let chars: Vec<char> = BRAND.chars().collect();
    let len = chars.len().max(1) as f32;

    chars
        .into_iter()
        .enumerate()
        .map(|(i, ch)| {
            // Distance of this column from the moving crest, wrapped over the
            // wordmark width so the sweep loops seamlessly.
            let phase = (i as f32 / len) * std::f32::consts::TAU;
            // 0..1 brightness: 1 at the crest, easing to 0 between sweeps.
            let crest_factor = (phase - crest).cos() * 0.5 + 0.5;
            let lit = blend(highlight, base, crest_factor * depth);
            // A sharp white glint sits only on the very tip of the crest — the
            // high power keeps it to the leading letter or two, not the whole
            // sweep. Scaled by `ramp` so it fades with the rest.
            let white_amt = crest_factor.powi(6) * MAX_WHITE * ramp;
            let fg = blend(white, lit, white_amt);
            Span::styled(ch.to_string(), Style::default().fg(fg).bold())
        })
        .collect()
}

#[cfg(test)]
#[path = "../../tests/unit/tui_header.rs"]
mod tests;
