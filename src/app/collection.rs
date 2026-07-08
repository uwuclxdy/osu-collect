use crate::download::{
    BeatmapStage, DownloadConfig, DownloadId, DownloadStage, DownloadSummary, FailedMap,
};
use ratatui::style::Color;
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

use crate::config::constants::{SPEED_STALE_AFTER, SPEED_UPDATE_INTERVAL};

/// minimum time between text updates on a single active-download slot.
const STATUS_DEBOUNCE: Duration = Duration::from_millis(50);

/// Rows moved per page-scroll keystroke (`Ctrl+d` / `Ctrl+u`) on a download page.
const PAGE_STEP: usize = 10;

#[derive(Debug, Clone, Default)]
struct DisplayedStatus {
    message: String,
    rate_limited: bool,
}

fn non_empty_status(stage: BeatmapStage, beatmapset_id: u32, message: &str) -> String {
    if !message.trim().is_empty() {
        return message.to_string();
    }

    match stage {
        BeatmapStage::Pending => format!("queued #{beatmapset_id}"),
        BeatmapStage::Downloading => format!("downloading #{beatmapset_id}"),
        BeatmapStage::Verifying => format!("verifying #{beatmapset_id}"),
        BeatmapStage::Success => format!("done #{beatmapset_id}"),
        BeatmapStage::Skipped => format!("skipped #{beatmapset_id}"),
        BeatmapStage::Failed => format!("failed #{beatmapset_id}"),
        BeatmapStage::Aborted => format!("aborted #{beatmapset_id}"),
    }
}

#[derive(Debug, Default, Clone)]
pub struct DownloadStats {
    pub downloaded: u32,
    pub skipped: u32,
    pub failed: u32,
    pub unverified: u32,
    pub bytes_downloaded: u64,
    pub total_collection_bytes: Option<u64>,
    pub verified_bytes: u64,
    pub verify_total_count: u32,
    pub verify_total_us: u64,
}

/// Why a beatmapset failed. Categorized from the library's `Error` at the app boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureReason {
    /// Not found on any mirror (HTTP 404 or `UnavailableOnMirrors`).
    NotFound,
    /// Rate-limited on all mirrors with retries exhausted.
    RateLimited,
    /// Connection / timeout / DNS failure.
    NetworkError,
    /// Hash mismatch, bad ZIP, or EOCD validation failure.
    ValidationFailed,
    /// Any error not covered by the above categories.
    Unknown,
}

impl FailureReason {
    /// Short human-readable label shown in the failed list.
    pub fn label(self) -> &'static str {
        match self {
            Self::NotFound => "not found",
            Self::RateLimited => "rate-limited",
            Self::NetworkError => "network error",
            Self::ValidationFailed => "archive invalid",
            Self::Unknown => "unknown error",
        }
    }
}

/// One line in the "active downloads" panel. Keyed by `beatmapset_id`.
#[derive(Debug, Clone)]
pub struct ActiveDownloadLine {
    pub beatmapset_id: u32,
    /// drives bar color and slot reuse; updated immediately on every status event so
    /// `first_free_slot` / `bar_color` see reality without lag.
    pub stage: BeatmapStage,
    pending: RefCell<DisplayedStatus>,
    displayed: RefCell<DisplayedStatus>,
    /// when `pending` is allowed to flip into `displayed`. `None` means `displayed` is current.
    debounce_at: Cell<Option<Instant>>,
    /// last time `displayed` was written. enforces a 50ms min gap between text updates.
    last_applied: Cell<Option<Instant>>,
    pub downloaded: u64,
    pub total: u64,
    bytes_for_speed: u64,
    last_update: Option<Instant>,
    speed_bytes_per_sec: f64,
    /// Instant at which the rate-limit cooldown expires. Kept outside the debounce
    /// machinery so the render can compute remaining seconds without waiting for the
    /// debounce window to elapse.
    cooldown_until: Option<Instant>,
}

impl ActiveDownloadLine {
    fn new(beatmapset_id: u32) -> Self {
        Self {
            beatmapset_id,
            stage: BeatmapStage::Downloading,
            pending: RefCell::default(),
            displayed: RefCell::default(),
            debounce_at: Cell::new(None),
            last_applied: Cell::new(None),
            downloaded: 0,
            total: 0,
            bytes_for_speed: 0,
            last_update: None,
            speed_bytes_per_sec: 0.0,
            cooldown_until: None,
        }
    }

    pub fn speed_bytes_per_sec(&self) -> f64 {
        self.last_update
            .filter(|last| last.elapsed() <= SPEED_STALE_AFTER)
            .map_or(0.0, |_| self.speed_bytes_per_sec)
    }

    /// bar fill color for the current stage. rate_limited overrides downloading color.
    pub fn bar_color(&self) -> Color {
        use crate::tui::{accent, danger, info, line, success, text_dim, text_faint, warning};
        if matches!(self.stage, BeatmapStage::Downloading) && self.displayed_rate_limited() {
            return warning();
        }
        match self.stage {
            BeatmapStage::Pending => text_faint(),
            BeatmapStage::Downloading => accent(),
            BeatmapStage::Verifying => info(),
            BeatmapStage::Success => success(),
            BeatmapStage::Skipped => line(),
            BeatmapStage::Failed => danger(),
            BeatmapStage::Aborted => text_dim(),
        }
    }

    fn apply_status(
        &mut self,
        stage: BeatmapStage,
        message: &str,
        rate_limited: bool,
        cooldown_until: Option<Instant>,
    ) {
        // stage is structural (bar / slot reuse) and updates immediately. the *text* shown to
        // the user is rate-limited to one write per STATUS_DEBOUNCE for all stages — rapid
        // changes (download → verify → success in <50ms) coalesce to the last write.
        self.stage = stage;
        // cooldown_until is stored outside the debounce window — the render derives remaining
        // seconds directly, so it must not wait for the text debounce to elapse.
        self.cooldown_until = cooldown_until;
        let next = DisplayedStatus {
            message: non_empty_status(stage, self.beatmapset_id, message),
            rate_limited,
        };
        *self.pending.borrow_mut() = next.clone();

        let now = Instant::now();
        let elapsed = self
            .last_applied
            .get()
            .map_or(Duration::MAX, |t| now.saturating_duration_since(t));
        if elapsed >= STATUS_DEBOUNCE {
            *self.displayed.borrow_mut() = next;
            self.last_applied.set(Some(now));
            self.debounce_at.set(None);
        } else {
            let last = self.last_applied.get().unwrap();
            self.debounce_at.set(Some(last + STATUS_DEBOUNCE));
        }
    }

    pub fn progress_ratio(&self) -> Option<f32> {
        (self.total > 0).then(|| (self.downloaded as f32 / self.total as f32).clamp(0.0, 1.0))
    }

    pub fn displayed_message(&self) -> String {
        self.resolve_pending();
        self.displayed.borrow().message.clone()
    }

    pub fn displayed_rate_limited(&self) -> bool {
        self.resolve_pending();
        self.displayed.borrow().rate_limited
    }

    /// Remaining cooldown seconds for this rate-limited row.
    ///
    /// Updated immediately on every `RateLimited` status (outside the debounce window) so
    /// the render always shows the freshest countdown without waiting 50ms.
    /// Returns `None` when the row is not rate-limited or the deadline has already passed.
    pub fn cooldown_secs_remaining(&self) -> Option<u64> {
        let until = self.cooldown_until?;
        let remaining = until.saturating_duration_since(Instant::now());
        // keep showing 0 until the lib emits the next status (clearing rate_limited)
        Some(remaining.as_secs())
    }

    fn resolve_pending(&self) {
        let Some(at) = self.debounce_at.get() else {
            return;
        };
        let now = Instant::now();
        if now >= at {
            *self.displayed.borrow_mut() = self.pending.borrow().clone();
            self.last_applied.set(Some(now));
            self.debounce_at.set(None);
        }
    }
}

pub struct CollectionPage {
    pub id: DownloadId,
    pub title: String,
    title_lower: String,
    pub stage: DownloadStage,
    pub total_maps: usize,
    pub download_target: usize,
    pub stats: DownloadStats,
    pub output_dir: Option<String>,
    pub uploader: Option<String>,
    /// Config snapshot taken at download start, used to build retry requests.
    pub download_config: Option<DownloadConfig>,
    /// Fixed-size slot list — one slot per worker thread. Free slots are `None`.
    /// Only actively-downloading beatmapsets occupy a slot: reaching a terminal
    /// stage (success / skipped / failed / aborted) frees the slot immediately,
    /// so the row stops rendering rather than lingering with a final message.
    /// A new download then claims the lowest free slot.
    pub active_downloads: Vec<Option<ActiveDownloadLine>>,
    /// Beatmapsets the library deferred (requeued after a long rate-limit) that
    /// have not yet re-entered with a fresh status. Not counted in any tally —
    /// they are still "queued". Gates the defer/drop keys + footer hint and the
    /// stalled OVERVIEW status; cleared when a map re-attempts or the run ends.
    deferred_ids: HashSet<u32>,
    pub concurrent: usize,
    /// Last-seen downloaded bytes per beatmapset id; used by `update_progress`
    /// to compute the delta added to `stats.bytes_downloaded` each call.
    prev_bytes: HashMap<u32, u64>,
    pub summary: Option<DownloadSummary>,
    pub failed_maps: Vec<FailedMap>,
    /// Whether the FAILED collapsible section is currently expanded.
    pub failed_section_expanded: bool,
    /// Row cursor inside the expanded failed section. `None` means the section
    /// header itself is focused; `Some(i)` points at `failed_maps[i]`.
    /// Cleared automatically when the section collapses.
    pub failed_focus: Option<usize>,
    pub low_disk_space: Option<u64>,
    pub resolve_progress: Option<(u32, u32)>,
    pub thread_scroll: usize,
    pub thread_visible_items: Cell<usize>,
    pub thread_total_items: Cell<usize>,
    cached_cumulative_speed: Cell<f64>,
    last_speed_update: Cell<Option<Instant>>,
    /// Instant when the first `DownloadStage::Downloading` entry occurred.
    /// Set once on `CollectionReady`; never overwritten so retries don't reset the clock.
    pub session_start: Option<Instant>,
}

impl CollectionPage {
    pub fn new(id: DownloadId, title: String, concurrent: usize) -> Self {
        let slot_count = concurrent.max(1);
        let title_lower = title.to_lowercase();
        Self {
            id,
            title,
            title_lower,
            stage: DownloadStage::Pending,
            total_maps: 0,
            download_target: 0,
            stats: DownloadStats::default(),
            output_dir: None,
            uploader: None,
            download_config: None,
            active_downloads: (0..slot_count).map(|_| None).collect(),
            deferred_ids: HashSet::new(),
            concurrent,
            prev_bytes: HashMap::new(),
            summary: None,
            failed_maps: Vec::new(),
            failed_section_expanded: false,
            failed_focus: None,
            low_disk_space: None,
            resolve_progress: None,
            thread_scroll: 0,
            thread_visible_items: Cell::new(0),
            thread_total_items: Cell::new(0),
            cached_cumulative_speed: Cell::new(0.0),
            last_speed_update: Cell::new(None),
            session_start: None,
        }
    }

    pub fn set_title(&mut self, title: String) {
        self.title_lower = title.to_lowercase();
        self.title = title;
    }

    /// Whether the run reached a terminal stage (completed or failed). Settled
    /// pages are retained for the session so the Downloads tab can still
    /// preview them; they evict to history records on cap or exit.
    pub fn is_settled(&self) -> bool {
        matches!(self.stage, DownloadStage::Completed | DownloadStage::Failed)
    }

    pub fn title_lower(&self) -> &str {
        &self.title_lower
    }

    pub fn active_lines(&self) -> impl Iterator<Item = &ActiveDownloadLine> {
        self.active_downloads
            .iter()
            .flatten()
            .filter(|line| !line.stage.is_terminal())
    }

    pub fn clear_active_downloads(&mut self) {
        self.active_downloads.fill(None);
        self.deferred_ids.clear();
    }

    /// Free the active slot for a deferred map and remember it as
    /// deferred-pending. The map is not counted terminal (the library will retry
    /// it); the id is dropped again the moment it re-enters with a fresh status.
    pub fn mark_deferred(&mut self, beatmapset_id: u32) {
        if let Some(slot) = self.active_downloads.iter_mut().find(|slot| {
            slot.as_ref()
                .is_some_and(|l| l.beatmapset_id == beatmapset_id)
        }) {
            *slot = None;
        }
        self.deferred_ids.insert(beatmapset_id);
    }

    pub fn deferred_count(&self) -> usize {
        self.deferred_ids.len()
    }

    /// True when the run is fully stalled on rate limits: every active row is
    /// parked on a cooldown, or all remaining maps have deferred (no active row
    /// is downloading). Drives the OVERVIEW `rate limited` status.
    pub fn fully_rate_limited(&self) -> bool {
        let has_active = self.active_lines().next().is_some();
        let all_active_parked = self.active_lines().all(|l| l.displayed_rate_limited());
        (has_active || !self.deferred_ids.is_empty()) && all_active_parked
    }

    pub fn any_active_rate_limited(&self) -> bool {
        self.active_lines().any(|l| l.displayed_rate_limited())
    }

    /// Whether the manual defer (`s`) / drop (`S`) keys apply: some active row is
    /// parked on a cooldown, or some map is waiting to retry after a defer.
    pub fn rate_limited_or_deferred(&self) -> bool {
        self.any_active_rate_limited() || !self.deferred_ids.is_empty()
    }

    pub fn update_progress(&mut self, beatmapset_id: u32, downloaded: u64, total: u64) {
        let prev = self.prev_bytes.get(&beatmapset_id).copied().unwrap_or(0);
        self.stats.bytes_downloaded = self.stats.bytes_downloaded.saturating_sub(prev) + downloaded;
        self.prev_bytes.insert(beatmapset_id, downloaded);
        // `total` is unused here; the active-download slot tracks it for the progress bar.
        let _ = total;
    }

    pub fn update_active_status(
        &mut self,
        beatmapset_id: u32,
        stage: BeatmapStage,
        message: &str,
        rate_limited: bool,
        cooldown_until: Option<Instant>,
    ) {
        // Any fresh status means a previously-deferred map is re-attempting (or
        // reaching a terminal state), so it is no longer waiting in the defer
        // queue — clear it to keep the gating / stalled counters accurate.
        self.deferred_ids.remove(&beatmapset_id);

        // Terminal stages drop the line immediately — only actively-downloading
        // rows belong in the ACTIVE panel, so a finished / failed / skipped slot
        // is freed at once and stops rendering (rows that aren't downloading are
        // removed, not kept around with a final message).
        if stage.is_terminal() {
            if let Some(slot) = self.active_downloads.iter_mut().find(|slot| {
                slot.as_ref()
                    .is_some_and(|l| l.beatmapset_id == beatmapset_id)
            }) {
                *slot = None;
            }
            return;
        }

        if let Some(line) = self.find_active_line_mut(beatmapset_id) {
            line.apply_status(stage, message, rate_limited, cooldown_until);
            return;
        }

        // Only an in-flight download stage may claim a free slot — precheck transitions
        // (`Pending`) update the underlying beatmap row but must not consume an active
        // slot, otherwise the panel grows past the worker count.
        if !matches!(stage, BeatmapStage::Downloading) {
            return;
        }
        let Some(slot_idx) = self.first_free_slot() else {
            return;
        };
        let mut line = ActiveDownloadLine::new(beatmapset_id);
        line.apply_status(stage, message, rate_limited, cooldown_until);
        self.active_downloads[slot_idx] = Some(line);
    }

    pub fn update_active_progress(&mut self, beatmapset_id: u32, downloaded: u64, total: u64) {
        // Slot allocation is the status path's job — by the time progress arrives the lib
        // has already emitted `Contacting`/`Downloading` and the slot exists with a real
        // message. Allocating here would render a slot with an empty `displayed_message`.
        let Some(line) = self.find_active_line_mut(beatmapset_id) else {
            return;
        };
        line.downloaded = downloaded;
        if total > 0 {
            line.total = total;
        }
        let now = Instant::now();
        match line.last_update {
            Some(last) if now.duration_since(last) > SPEED_UPDATE_INTERVAL => {
                let elapsed = now.duration_since(last).as_secs_f64();
                let delta = downloaded.saturating_sub(line.bytes_for_speed) as f64;
                line.speed_bytes_per_sec = line.speed_bytes_per_sec * 0.7 + delta / elapsed * 0.3;
                line.bytes_for_speed = downloaded;
                line.last_update = Some(now);
            }
            None => {
                line.bytes_for_speed = downloaded;
                line.last_update = Some(now);
            }
            _ => {}
        }
    }

    fn find_active_line_mut(&mut self, beatmapset_id: u32) -> Option<&mut ActiveDownloadLine> {
        self.active_downloads
            .iter_mut()
            .flatten()
            .find(|line| line.beatmapset_id == beatmapset_id)
    }

    fn first_free_slot(&self) -> Option<usize> {
        // Terminal rows are freed the moment they finish (see `update_active_status`),
        // so a free slot is simply an empty one — the lowest keeps positions stable.
        self.active_downloads.iter().position(Option::is_none)
    }

    pub fn cumulative_speed(&self) -> f64 {
        let now = Instant::now();
        let stale = self
            .last_speed_update
            .get()
            .is_none_or(|last| now.duration_since(last) >= SPEED_UPDATE_INTERVAL);
        if stale {
            let speed = self.active_lines().map(|l| l.speed_bytes_per_sec()).sum();
            self.cached_cumulative_speed.set(speed);
            self.last_speed_update.set(Some(now));
        }
        self.cached_cumulative_speed.get()
    }

    /// Override the cached cumulative speed for testing purposes.
    #[cfg(test)]
    pub fn set_cached_speed_for_test(&self, speed: f64) {
        self.cached_cumulative_speed.set(speed);
        self.last_speed_update.set(Some(Instant::now()));
    }

    /// Store the failure list shown in the FAILED collapsible section,
    /// sorted ascending by `beatmapset_id` so the order is stable across runs.
    pub fn set_failed_maps(&mut self, failures: Vec<FailedMap>) {
        self.failed_maps = failures;
        self.failed_maps.sort_by_key(|f| f.beatmapset_id);
    }

    /// Toggle the failed-maps section expanded/collapsed. No-op when empty.
    /// Clears row focus when collapsing.
    pub fn toggle_failed_section(&mut self) {
        if !self.failed_maps.is_empty() {
            self.failed_section_expanded = !self.failed_section_expanded;
            if !self.failed_section_expanded {
                self.failed_focus = None;
            }
        }
    }

    /// Move the failed-row cursor down by one. When past the last row, wraps
    /// back to the header (`None`). No-op if the section is collapsed or empty.
    pub fn failed_focus_next(&mut self) {
        if !self.failed_section_expanded || self.failed_maps.is_empty() {
            return;
        }
        self.failed_focus = match self.failed_focus {
            None => Some(0),
            Some(i) if i + 1 < self.failed_maps.len() => Some(i + 1),
            Some(_) => None,
        };
    }

    /// Move the failed-row cursor up by one. When already at the header (`None`)
    /// wraps to the last row. No-op if the section is collapsed or empty.
    pub fn failed_focus_prev(&mut self) {
        if !self.failed_section_expanded || self.failed_maps.is_empty() {
            return;
        }
        self.failed_focus = match self.failed_focus {
            None => Some(self.failed_maps.len().saturating_sub(1)),
            Some(0) => None,
            Some(i) => Some(i - 1),
        };
    }

    /// IDs that are eligible for retry: excludes `NotFound` (404s are not
    /// fixable by retrying). Optionally restricted to a single row index.
    pub fn retryable_ids(&self, row: Option<usize>) -> Vec<u32> {
        let maps: &[FailedMap] = match row {
            Some(i) => self.failed_maps.get(i..=i).unwrap_or_default(),
            None => &self.failed_maps,
        };
        maps.iter()
            .filter(|f| f.reason != FailureReason::NotFound)
            .map(|f| f.beatmapset_id)
            .collect()
    }

    /// Remove a single failed-map entry from the list and adjust the focus
    /// cursor so it stays valid.
    pub fn remove_failed_map(&mut self, beatmapset_id: u32) {
        let Some(pos) = self
            .failed_maps
            .iter()
            .position(|f| f.beatmapset_id == beatmapset_id)
        else {
            return;
        };
        self.failed_maps.remove(pos);
        // clamp focus so it doesn't point past the end
        if let Some(focused) = self.failed_focus {
            if self.failed_maps.is_empty() {
                self.failed_focus = None;
                self.failed_section_expanded = false;
            } else if focused >= self.failed_maps.len() {
                self.failed_focus = Some(self.failed_maps.len() - 1);
            }
        }
    }

    pub fn scroll_threads_up(&mut self) {
        // Re-clamp first: a taller viewport (resize) shrinks the max offset while
        // the render only clamps the drawn window, leaving `thread_scroll` stale
        // and the first few ↑ presses dead. Snap to the last full page, then step.
        self.thread_scroll = self
            .thread_scroll
            .min(self.max_thread_scroll())
            .saturating_sub(1);
    }

    pub fn scroll_threads_down(&mut self) {
        let total = self.thread_total_items.get();
        let visible = self.thread_visible_items.get();
        let max_scroll = total.saturating_sub(visible);
        if self.thread_scroll < max_scroll {
            self.thread_scroll += 1;
        }
    }

    /// Largest valid thread-scroll offset (bottom of the list).
    fn max_thread_scroll(&self) -> usize {
        self.thread_total_items
            .get()
            .saturating_sub(self.thread_visible_items.get())
    }

    /// Whether the failed-row cursor (not the thread list) owns vertical motion:
    /// the failed section is expanded and non-empty.
    fn failed_rows_active(&self) -> bool {
        self.failed_section_expanded && !self.failed_maps.is_empty()
    }

    /// Jump the active list (failed rows when expanded, else the thread list) to
    /// the top (`gg` / Home).
    pub fn jump_top(&mut self) {
        if self.failed_rows_active() {
            self.failed_focus = Some(0);
        } else {
            self.thread_scroll = 0;
        }
    }

    /// Jump the active list to the bottom (`G` / End).
    pub fn jump_bottom(&mut self) {
        if self.failed_rows_active() {
            self.failed_focus = Some(self.failed_maps.len() - 1);
        } else {
            self.thread_scroll = self.max_thread_scroll();
        }
    }

    /// Page the active list up by [`PAGE_STEP`] rows (`Ctrl+u`), clamped (no wrap).
    pub fn page_up(&mut self) {
        if self.failed_rows_active() {
            let cur = self.failed_focus.unwrap_or(0);
            self.failed_focus = Some(cur.saturating_sub(PAGE_STEP));
        } else {
            self.thread_scroll = self
                .thread_scroll
                .min(self.max_thread_scroll())
                .saturating_sub(PAGE_STEP);
        }
    }

    /// Page the active list down by [`PAGE_STEP`] rows (`Ctrl+d`), clamped.
    pub fn page_down(&mut self) {
        if self.failed_rows_active() {
            let last = self.failed_maps.len() - 1;
            let cur = self.failed_focus.unwrap_or(0);
            self.failed_focus = Some((cur + PAGE_STEP).min(last));
        } else {
            self.thread_scroll = (self.thread_scroll + PAGE_STEP).min(self.max_thread_scroll());
        }
    }

    pub fn total_downloaded_bytes(&self) -> u64 {
        self.stats
            .bytes_downloaded
            .saturating_add(self.stats.verified_bytes)
    }

    pub fn avg_verify_us(&self) -> Option<u64> {
        if self.stats.verify_total_count == 0 {
            return None;
        }
        let avg = self.stats.verify_total_us / self.stats.verify_total_count as u64;
        (avg > 0).then_some(avg)
    }
}

#[cfg(test)]
#[path = "../../tests/unit/active_download_line.rs"]
mod tests;

#[cfg(test)]
#[path = "../../tests/unit/collection_page.rs"]
mod collection_page_tests;
