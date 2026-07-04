pub mod terminal;
pub mod theme;

pub(crate) mod banner;
mod config;
mod download;
mod footer;
mod header;
mod home;
mod login;
pub(crate) mod modal;
mod toast;
mod updates;
mod widgets;

pub use theme::{Theme, apply_theme, theme};

use crate::app::{App, system_banners};
use crate::config::constants::{CONFIG_TAB_INDEX, HOME_TAB_INDEX, UPDATES_TAB_INDEX};
use osu_downloader::MirrorKind;
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Color, Style},
    widgets::Block,
};
use std::rc::Rc;
use std::sync::LazyLock;

/// App-side display label for a mirror. Distinct from `MirrorKind::label()`
/// (which the lib uses for its own defaults) — this carries the TUI's
/// lowercase + branded styling (`osu!direct`).
pub fn mirror_label(kind: MirrorKind) -> &'static str {
    match kind {
        MirrorKind::OsuDirect => "osu!direct",
        MirrorKind::Nerinyan => "nerinyan",
        MirrorKind::Sayobot => "sayobot",
        MirrorKind::Nekoha => "nekoha",
        MirrorKind::Beatconnect => "beatconnect",
        MirrorKind::Osudl => "osu!dl",
        MirrorKind::Catboy => "catboy.best",
        MirrorKind::Hinamizawa => "hinamizawa",
        MirrorKind::OsuApi => "osu! official",
        MirrorKind::Custom => "custom",
    }
}

/// Minimum per-tab content-area height before a view switches to its compact
/// layout. Density rule: ≥ 14 rows = normal, < 14 = compact (chrome
/// stripped + the `TooSmall` banner surfaces).
pub(crate) const COMPACT_HEIGHT: u16 = 14;

pub(crate) const GLYPH_BLOCK: &str = "█";
pub(crate) const GLYPH_SHADE: &str = "░";
pub(crate) const GLYPH_SPACE: &str = " ";

const MAX_FILL_WIDTH: usize = 256;

pub(crate) static FILL_BLOCK: LazyLock<String> =
    LazyLock::new(|| GLYPH_BLOCK.repeat(MAX_FILL_WIDTH));
pub(crate) static FILL_SHADE: LazyLock<String> =
    LazyLock::new(|| GLYPH_SHADE.repeat(MAX_FILL_WIDTH));
pub(crate) static FILL_SPACE: LazyLock<String> =
    LazyLock::new(|| GLYPH_SPACE.repeat(MAX_FILL_WIDTH));

/// Returns a `&str` slice of `n` repetitions of `glyph` backed by `buf`.
///
/// When `n ≤ MAX_FILL_WIDTH` this is a zero-alloc slice into the pre-built
/// static buffer.  For wider terminals it falls back to `glyph.repeat(n)` so
/// behaviour is always correct.
pub(crate) fn glyph_fill<'a>(
    buf: &'a LazyLock<String>,
    glyph: &'static str,
    n: usize,
) -> std::borrow::Cow<'a, str> {
    if n == 0 {
        return std::borrow::Cow::Borrowed("");
    }
    let end = n * glyph.len();
    if n <= MAX_FILL_WIDTH {
        let s = buf.as_str();
        debug_assert!(
            s.is_char_boundary(end),
            "glyph_fill: {end} is not a char boundary"
        );
        std::borrow::Cow::Borrowed(&s[..end])
    } else {
        std::borrow::Cow::Owned(glyph.repeat(n))
    }
}

// Palette accessors — always go through the process-wide theme so that the
// selected variant (full / compatible) is respected.
// Internal modules import these functions; external callers use `theme()`.
pub(crate) fn accent() -> Color {
    theme().accent
}
pub(crate) fn accent_alt() -> Color {
    theme().accent_alt
}
pub(crate) fn info() -> Color {
    theme().info
}
pub(crate) fn success() -> Color {
    theme().success
}
pub(crate) fn warning() -> Color {
    theme().warning
}
pub(crate) fn danger() -> Color {
    theme().danger
}
pub(crate) fn text() -> Color {
    theme().text
}
pub(crate) fn text_dim() -> Color {
    theme().text_dim
}
pub(crate) fn text_faint() -> Color {
    theme().text_faint
}
pub(crate) fn line() -> Color {
    theme().line
}
pub(crate) fn line_strong() -> Color {
    theme().line_strong
}
pub(crate) fn bg() -> Color {
    theme().bg
}
pub(crate) fn bg_raised() -> Color {
    theme().bg_raised
}
pub(crate) fn bg_hover() -> Color {
    theme().bg_hover
}
pub(crate) fn bg_sunken() -> Color {
    theme().bg_sunken
}

/// Spinner frames pre-padded with a leading and trailing space.
///
/// Use `spinner_str` when the frame is the only content in a span — no
/// `format!` needed, zero allocation.  When embedding in a larger `format!`
/// string, use `spinner_str(tick).trim()` to strip the padding.
pub const SPINNER_FRAMES_PADDED: [&str; 10] = [
    " ⠋ ", " ⠙ ", " ⠹ ", " ⠸ ", " ⠼ ", " ⠴ ", " ⠦ ", " ⠧ ", " ⠇ ", " ⠏ ",
];

/// Returns the current spinner frame pre-padded as `&'static str`.
///
/// Use this when the frame is the only content in a span to avoid allocating.
/// When embedding in a larger `format!` string, call `.trim()` on the result.
pub fn spinner_str(tick: u64) -> &'static str {
    SPINNER_FRAMES_PADDED[tick as usize % SPINNER_FRAMES_PADDED.len()]
}

/// Format free bytes as `"1.5 TiB free"`, `"45.1 GiB free"`, `"234.5 MiB free"`, etc.
///
/// Storage uses IEC binary units (1024-based).
pub(crate) fn format_free_space(bytes: u64) -> String {
    const TIB: f64 = 1024.0 * 1024.0 * 1024.0 * 1024.0;
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    const KIB: f64 = 1024.0;
    let f = bytes as f64;
    if f >= TIB {
        format!("{:.1} TiB free", f / TIB)
    } else if f >= GIB {
        format!("{:.1} GiB free", f / GIB)
    } else if f >= MIB {
        format!("{:.1} MiB free", f / MIB)
    } else if f >= KIB {
        format!("{:.0} KiB free", f / KIB)
    } else {
        format!("{bytes} B free")
    }
}

pub const HELP_CUSTOM_MIRROR: &str = "must contain {id}";
/// Focus tooltip / locked-row reason for the osu! official mirror when logged out.
pub const HELP_OSU_OFFICIAL_LOCKED: &str = "log in to enable the osu! official mirror";

pub fn eyebrow() -> Style {
    // Eyebrow / column-header style: TEXT_DIM (sanctioned bold variant),
    // never the faint placeholder tier.
    Style::default().fg(text_dim()).bold()
}

pub fn focused_label(focused: bool) -> Style {
    if focused {
        Style::default().fg(text()).bold()
    } else {
        // Blurred form-row label sits in TEXT_DIM (one of the three text tiers).
        Style::default().fg(text_dim())
    }
}

/// Render one full frame.
///
/// A focused text field positions the terminal caret via
/// [`Frame::set_cursor_position`] from inside the focused/editing render fn;
/// ratatui 0.30 applies it after the buffer flush, so there is no flash. The
/// caret is suppressed while an overlay is open by skipping the body view's
/// cursor request (overlays don't carry a text caret).
pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    if area.width == 0 || area.height == 0 {
        return;
    }

    frame.render_widget(Block::default().style(Style::default().bg(bg())), area);

    // Borderless shell: header row, body, footer row — no `─` divider rules
    // between regions; spacing + the body panel border separate them.
    let chunks: Rc<[_]> = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(area);

    let (header_area, content_area, footer_area) = (chunks[0], chunks[1], chunks[2]);

    let tabs = app.tab_titles();
    header::render(
        frame,
        header::RenderParams {
            area: header_area,
            tabs: &tabs,
            active: app.active_tab(),
            tick: app.tick_count,
            downloading: app.is_downloading(),
            brand_ramp: app.brand_ramp(),
            update_phase: app.update_phase,
            client: app.updates.path.client_type,
        },
    );

    // System-wide banners (disk low/full + compact "terminal too small") render
    // at the top of the body on EVERY tab, then the active view fills the rest.
    let banners = system_banners(
        app.disk_free_bytes(),
        content_area.height,
        &app.banner_recency,
        app.tick_count,
    );
    let (banner_area, body_area) = banner::split_banner_area(content_area, &banners);
    banner::render_banners(frame, banner_area, &banners);

    // A focused text field positions the caret via `Frame::set_cursor_position`
    // inside the view render fn — but never under an overlay, so suppress the
    // caret by clearing `editing` when a modal/help overlay is open (overlays
    // carry no text caret).
    let overlay_open = app.confirm_retry_on_start.is_some()
        || app.confirm_retry.is_some()
        || app.help_open
        || app.update_modal.is_some();
    let editing = app.editing && !overlay_open;
    match app.active_tab() {
        HOME_TAB_INDEX => home::render(frame, body_area, &app.home, editing),
        UPDATES_TAB_INDEX => updates::render(frame, body_area, &app.updates, editing),
        CONFIG_TAB_INDEX => {
            let osu_dir = app.updates.osu_path();
            let library_db_hint = crate::app::library_cache::db_file_path(
                app.updates.path.client_type,
                std::path::Path::new(&osu_dir),
            )
            .display()
            .to_string();
            config::render(
                frame,
                body_area,
                &app.config,
                editing,
                &library_db_hint,
                &app.home.mirror_latency,
            );
        }
        tab if app.is_login_tab(tab) => {
            if let Some(login_tab) = app.login.as_ref() {
                login::render(
                    frame,
                    body_area,
                    login_tab,
                    &app.config.login_state,
                    editing,
                );
            }
        }
        tab => match app.download_for_tab(tab) {
            Some(page) => download::render(frame, body_area, page, app.tick_count),
            None => home::render(frame, body_area, &app.home, editing),
        },
    }

    footer::render(frame, footer_area, app);

    if let Some(modal) = &app.confirm_retry_on_start {
        modal::render_retry_on_start_modal(frame, area, modal.failed_count, modal.focus);
    } else if let Some(modal) = &app.confirm_retry {
        modal::render_confirm_retry_modal(frame, area, modal.retryable_count, modal.focus);
    } else if let Some(modal) = &app.update_modal {
        if let Some(info) = app.available_update.as_ref() {
            let clamped = modal::render_update_modal(
                frame,
                area,
                &info.version,
                &info.changelog,
                modal.scroll.get(),
                modal.focus,
            );
            modal.scroll.set(clamped);
        }
    } else if app.help_open {
        // Clamp the requested scroll to the real viewport and store it back so
        // the next ↑/↓ starts from the on-screen position (no dead presses).
        let clamped = modal::render_help_overlay(
            frame,
            area,
            app.help_scroll.get(),
            app.active_tab(),
            app.is_login_tab(app.active_tab()),
            app.config.vim_keys,
        );
        app.help_scroll.set(clamped);
    }

    // Toasts float over everything else — rendered last so the
    // buffer beneath is final for the 75 % blend.
    toast::render_toasts(frame, area, &app.toasts);
}

#[cfg(test)]
#[path = "../../tests/unit/tui.rs"]
mod tests;
