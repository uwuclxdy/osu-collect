use crate::config::ThemeMode;
use ratatui::style::Color;
use std::sync::{OnceLock, PoisonError, RwLock};

/// Capability tier reported by a resolved [`Theme`].
///
/// Widgets that need tier-dependent rendering (sub-cell progress glyphs
/// `▏▎▍▌▋▊▉` vs full blocks, slide-switch vs `[on]`/`[off]` toggle) can
/// branch on this value — all other glyphs are shared across tiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// 24-bit RGB truecolor.
    Full,
    /// xterm-256 compatible colors.
    Compatible,
}

/// All semantic color slots used by the TUI.
///
/// Obtain the active instance via [`theme()`].
#[derive(Debug, Clone)]
pub struct Theme {
    pub accent: Color,
    pub accent_alt: Color,
    pub info: Color,
    pub success: Color,
    pub warning: Color,
    pub danger: Color,
    pub text: Color,
    pub text_dim: Color,
    pub text_faint: Color,
    /// Blurred panel borders, soft rules, scrollbar track (LINE: 49,50,68 / 238).
    pub line: Color,
    /// Focused panel borders (LINE_STRONG: 69,71,90 / 240).
    pub line_strong: Color,
    pub bg: Color,
    pub bg_raised: Color,
    /// Selected row tint (BG_HOVER: 40,40,56 / 236).
    pub bg_hover: Color,
    /// Toast / sunken surface (BG_SUNKEN: 17,17,27 / 233).
    pub bg_sunken: Color,
    tier: Tier,
}

impl Theme {
    /// Returns the capability tier this theme was built for.
    pub fn tier(&self) -> Tier {
        self.tier
    }
}

/// The osu! star-rating difficulty tier color for a given star value. Each band
/// maps to a fixed RGB triple; the active theme's [`Tier`] decides whether it
/// reaches the terminal as [`Color::Rgb`] (full truecolor) or
/// [`Color::Indexed`] (nearest xterm-256, for compatible terminals).
///
/// The band edges mirror the in-game difficulty-tier colours so a familiar
/// reader picks out the tier at a glance.
pub fn stars_color(stars: f64) -> Color {
    let (r, g, b) = if stars < 1.5 {
        (99, 195, 255)
    } else if stars < 2.0 {
        (102, 204, 255)
    } else if stars < 2.7 {
        (135, 213, 126)
    } else if stars < 4.0 {
        (178, 217, 109)
    } else if stars < 5.3 {
        (250, 219, 142)
    } else if stars < 6.5 {
        (252, 165, 87)
    } else if stars < 7.5 {
        (243, 139, 168)
    } else {
        (201, 160, 255)
    };
    match theme().tier() {
        Tier::Full => Color::Rgb(r, g, b),
        Tier::Compatible => Color::Indexed(nearest_xterm256(r, g, b)),
    }
}

/// Blend two colors at ratio `t` (0.0 = all `b`, 1.0 = all `a`).
///
/// On the [`Tier::Full`] theme, channels are mixed in RGB and returned as
/// `Color::Rgb`.  On [`Tier::Compatible`] the same RGB blend is performed and
/// the result is snapped to the nearest xterm-256 indexed color so that
/// terminals without truecolor support still get a meaningful approximation.
///
/// Both `a` and `b` must be `Color::Rgb` or `Color::Indexed`; any other
/// variant (e.g. `Color::Reset`) is treated as black (0, 0, 0).
pub(crate) fn blend(a: Color, b: Color, t: f32) -> Color {
    let (ar, ag, ab) = color_to_rgb(a);
    let (br, bg, bb) = color_to_rgb(b);
    let r = (t * ar as f32 + (1.0 - t) * br as f32).round() as u8;
    let g = (t * ag as f32 + (1.0 - t) * bg as f32).round() as u8;
    let b_ch = (t * ab as f32 + (1.0 - t) * bb as f32).round() as u8;
    match theme().tier() {
        Tier::Full => Color::Rgb(r, g, b_ch),
        Tier::Compatible => Color::Indexed(nearest_xterm256(r, g, b_ch)),
    }
}

/// Extract (r, g, b) from a `Color::Rgb` or `Color::Indexed`; everything
/// else maps to (0, 0, 0).
fn color_to_rgb(c: Color) -> (u8, u8, u8) {
    match c {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Indexed(i) => xterm256_to_rgb(i),
        _ => (0, 0, 0),
    }
}

/// Convert an xterm-256 index to approximate (r, g, b).
fn xterm256_to_rgb(n: u8) -> (u8, u8, u8) {
    if n < 16 {
        // 4-bit ANSI approximation
        let table: [(u8, u8, u8); 16] = [
            (0, 0, 0),
            (128, 0, 0),
            (0, 128, 0),
            (128, 128, 0),
            (0, 0, 128),
            (128, 0, 128),
            (0, 128, 128),
            (192, 192, 192),
            (128, 128, 128),
            (255, 0, 0),
            (0, 255, 0),
            (255, 255, 0),
            (0, 0, 255),
            (255, 0, 255),
            (0, 255, 255),
            (255, 255, 255),
        ];
        table[n as usize]
    } else if n >= 232 {
        // grayscale ramp 232–255
        let v = 8u8.saturating_add((n - 232).saturating_mul(10));
        (v, v, v)
    } else {
        // 6×6×6 color cube 16–231
        let idx = n - 16;
        let bi = idx % 6;
        let gi = (idx / 6) % 6;
        let ri = idx / 36;
        const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
        (
            LEVELS[ri as usize],
            LEVELS[gi as usize],
            LEVELS[bi as usize],
        )
    }
}

/// Find the nearest xterm-256 index (16–255) for an RGB triple.
///
/// Only the 6×6×6 cube and grayscale ramp are searched (indices 16–255);
/// the 4-bit ANSI colors (0–15) are skipped because their actual rendering
/// is terminal-dependent and unreliable as blend targets.
fn nearest_xterm256(r: u8, g: u8, b: u8) -> u8 {
    let mut best_idx = 16u8;
    let mut best_dist = u32::MAX;
    for i in 16u16..=255 {
        let (cr, cg, cb) = xterm256_to_rgb(i as u8);
        let dr = r as i32 - cr as i32;
        let dg = g as i32 - cg as i32;
        let db = b as i32 - cb as i32;
        let dist = (dr * dr + dg * dg + db * db) as u32;
        if dist < best_dist {
            best_dist = dist;
            best_idx = i as u8;
        }
    }
    best_idx
}

/// The two canonical palettes, each leaked once so `theme()` can hand out a
/// `&'static Theme` while [`apply_theme`] swaps which one is active at runtime.
fn full_theme() -> &'static Theme {
    static FULL: OnceLock<Theme> = OnceLock::new();
    FULL.get_or_init(Theme::full)
}

fn compatible_theme() -> &'static Theme {
    static COMPATIBLE: OnceLock<Theme> = OnceLock::new();
    COMPATIBLE.get_or_init(Theme::compatible)
}

/// Active palette pointer. `None` until first set; [`theme`] lazily installs the
/// truecolor default so a call before [`apply_theme`] stays safe (tests).
static ACTIVE: RwLock<Option<&'static Theme>> = RwLock::new(None);

/// Return the process-wide [`Theme`] reference.
///
/// Reflects the most recent [`apply_theme`] call. If none has
/// run yet the truecolor default is installed and returned (safe for tests).
pub fn theme() -> &'static Theme {
    if let Some(active) = *ACTIVE.read().unwrap_or_else(PoisonError::into_inner) {
        return active;
    }
    // First access before init: install the truecolor default and return it.
    // A race here just re-stores the same leaked static — harmless.
    let fallback = full_theme();
    *ACTIVE.write().unwrap_or_else(PoisonError::into_inner) = Some(fallback);
    fallback
}

/// Set the process-wide theme from config. The startup entry point and the
/// live config-tab change both call this.
///
/// `Some(mode)` forces that palette. `None` (config key absent — first run, or
/// a config that failed to parse and fell back to defaults) selects the full
/// truecolor palette. There is no terminal auto-detection.
pub fn apply_theme(mode: Option<ThemeMode>) {
    let resolved = match mode {
        Some(ThemeMode::Compatible) => compatible_theme(),
        Some(ThemeMode::Full) | None => full_theme(),
    };
    *ACTIVE.write().unwrap_or_else(PoisonError::into_inner) = Some(resolved);
}

impl Default for Theme {
    fn default() -> Self {
        Theme::full()
    }
}

impl Theme {
    /// Full Catppuccin Mocha truecolor (RGB) palette.
    pub fn full() -> Self {
        Self {
            accent: Color::Rgb(67, 171, 229),
            accent_alt: Color::Rgb(217, 119, 87),
            info: Color::Rgb(116, 199, 236),
            success: Color::Rgb(166, 227, 161),
            warning: Color::Rgb(249, 226, 175),
            danger: Color::Rgb(243, 139, 168),
            text: Color::Rgb(205, 214, 244),
            text_dim: Color::Rgb(166, 173, 200),
            text_faint: Color::Rgb(127, 132, 156),
            line: Color::Rgb(49, 50, 68),
            line_strong: Color::Rgb(69, 71, 90),
            bg: Color::Rgb(30, 30, 46),
            bg_raised: Color::Rgb(24, 24, 37),
            bg_hover: Color::Rgb(40, 40, 56),
            bg_sunken: Color::Rgb(17, 17, 27),
            tier: Tier::Full,
        }
    }

    /// xterm-256 compatible palette — maps every slot to the nearest indexed color.
    ///
    /// `bg` and `bg_raised` both collapse to xterm 235 / 234 which are close
    /// enough; panels rely on borders (not fill) for visual separation.
    pub fn compatible() -> Self {
        Self {
            accent: Color::Indexed(75),
            accent_alt: Color::Indexed(173),
            info: Color::Indexed(117),
            success: Color::Indexed(151),
            warning: Color::Indexed(223),
            danger: Color::Indexed(211),
            text: Color::Indexed(189),
            text_dim: Color::Indexed(145),
            text_faint: Color::Indexed(102),
            line: Color::Indexed(238),
            line_strong: Color::Indexed(240),
            bg: Color::Indexed(235),
            bg_raised: Color::Indexed(234),
            bg_hover: Color::Indexed(236),
            bg_sunken: Color::Indexed(233),
            tier: Tier::Compatible,
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/tui_theme.rs"]
mod tests;
