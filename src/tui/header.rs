use std::borrow::Cow;

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::app::UpdateIndicator;

use super::theme::blend;
use super::{accent, accent_alt, spinner_str, text_dim, warning};

const BRAND: &str = " osu!collect";
const VERSION: &str = concat!(" v", env!("CARGO_PKG_VERSION"), " ");
/// Running version, no leading `v` / padding — the base of the update cue.
const CUR_VERSION: &str = env!("CARGO_PKG_VERSION");
const WHITE: Color = Color::Rgb(255, 255, 255);

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
    /// Self-update indicator phase; when set, the right-side version label gains
    /// a trailing glyph after the current version (shimmering `↑` available,
    /// spinner downloading, static `↻` restart-pending). `None` renders the plain
    /// dim version.
    pub update_phase: Option<UpdateIndicator>,
}

pub fn render(frame: &mut Frame, params: RenderParams<'_, '_>) {
    let RenderParams {
        area,
        tabs,
        active,
        tick,
        downloading,
        brand_ramp,
        update_phase,
    } = params;

    if area.width == 0 || area.height == 0 {
        return;
    }

    let (version_line, version_width) = version_indicator(update_phase, tick);
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

    frame.render_widget(
        Paragraph::new(Line::from(version_line)).alignment(Alignment::Right),
        layout[2],
    );
}

/// Right-side version label + its rendered width. Idle (`None`): the static dim
/// version. With an update phase, the current version keeps a trailing cue: a
/// white shimmer `↑` when available (same motion as the downloading brand), an
/// accent spinner while the update downloads, and a static warning `↻` once it's
/// installed and waiting on a restart.
fn version_indicator(phase: Option<UpdateIndicator>, tick: u64) -> (Vec<Span<'static>>, u16) {
    let width = |s: &str| s.chars().count() as u16;
    match phase {
        None => (
            vec![Span::styled(VERSION, Style::default().fg(text_dim()))],
            width(VERSION),
        ),
        Some(UpdateIndicator::Available) => {
            // A white pulse sweeps across the cue, then it rests at plain accent
            // for `UPDATE_REST_TICKS` before the next one — a periodic nudge, not
            // a continuous shimmer. A half-sine envelope fades each pulse in and
            // out (crest only tints ~0.55 toward white) so it never pops or
            // saturates.
            let text = format!(" v{CUR_VERSION} ↑ ");
            let cycle = UPDATE_SWEEP_TICKS + UPDATE_REST_TICKS;
            let t = tick % cycle;
            let depth = if t < UPDATE_SWEEP_TICKS {
                let progress = t as f32 / UPDATE_SWEEP_TICKS as f32;
                0.55 * (std::f32::consts::PI * progress).sin()
            } else {
                0.0
            };
            let spans = wave_spans(
                &text,
                t,
                UPDATE_SWEEP_TICKS as f32,
                accent(),
                WHITE,
                depth,
                WHITE,
                0.0,
            );
            (spans, width(&text))
        }
        Some(UpdateIndicator::Downloading) => {
            // Spinner replaces the arrow; the spin is the motion, so no shimmer.
            let text = format!(" v{CUR_VERSION} {} ", spinner_str(tick).trim());
            let w = width(&text);
            (
                vec![Span::styled(text, Style::default().fg(accent()).bold())],
                w,
            )
        }
        Some(UpdateIndicator::RestartPending) => {
            // Static reload glyph in warning-amber, nudging the restart.
            let text = format!(" v{CUR_VERSION} ↻ ");
            let w = width(&text);
            (
                vec![Span::styled(text, Style::default().fg(warning()).bold())],
                w,
            )
        }
    }
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

    // `WAVE_PERIOD` is the tick count for one full left-to-right sweep; `MAX_MIX`
    // caps how far each letter leans toward the accent so it pulses instead of
    // strobing; `MAX_WHITE` caps the white sparkle riding the crest tip. `ramp`
    // eases the whole effect in and out so it materializes and dissolves gently
    // rather than snapping.
    const WAVE_PERIOD: f32 = 16.0;
    const MAX_MIX: f32 = 0.65;
    const MAX_WHITE: f32 = 0.4;
    wave_spans(
        BRAND,
        tick,
        WAVE_PERIOD,
        base,
        accent(),
        MAX_MIX * ramp,
        WHITE,
        MAX_WHITE * ramp,
    )
}

/// Ticks for one white pulse to travel across the update-available cue.
const UPDATE_SWEEP_TICKS: u64 = 18;
/// Flat-accent ticks between pulses — the gap that keeps it a periodic nudge
/// (~1.7s at the 50ms tick) rather than a continuous shimmer.
const UPDATE_REST_TICKS: u64 = 34;

/// One left-to-right shimmer sweep across `text`, per-char. A cosine crest
/// travels over the string at `period` ticks/sweep: each column leans `base`
/// toward `crest` by up to `depth` (0..1), with a sharp `glint` (crest^6) riding
/// only the tip up to `glint_max`. Shared by the brand wordmark and the
/// update-available version cue so both animate with the same motion.
///
/// Constant-velocity crest: a looping phase fed straight to `cos`, so the sweep
/// wraps seamlessly with no velocity break.
#[allow(clippy::too_many_arguments)]
fn wave_spans(
    text: &str,
    tick: u64,
    period: f32,
    base: Color,
    crest: Color,
    depth: f32,
    glint: Color,
    glint_max: f32,
) -> Vec<Span<'static>> {
    let head = (tick as f32 / period).fract() * std::f32::consts::TAU;
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len().max(1) as f32;

    chars
        .into_iter()
        .enumerate()
        .map(|(i, ch)| {
            // Distance of this column from the moving crest, wrapped over the
            // string width so the sweep loops seamlessly.
            let col = (i as f32 / len) * std::f32::consts::TAU;
            // 0..1 brightness: 1 at the crest, easing to 0 between sweeps.
            let crest_factor = (col - head).cos() * 0.5 + 0.5;
            let lit = blend(crest, base, crest_factor * depth);
            let glint_amt = crest_factor.powi(6) * glint_max;
            let fg = blend(glint, lit, glint_amt);
            Span::styled(ch.to_string(), Style::default().fg(fg).bold())
        })
        .collect()
}

#[cfg(test)]
#[path = "../../tests/unit/tui_header.rs"]
mod tests;
