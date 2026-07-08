//! State for the Get Maps `Filter` source: a flat AND attribute filter over
//! nzbasic's batch-beatmap-downloader database (per-diff ranges, status/mode/
//! special chips, text rows) plus its results browse. The wire contract lives
//! in `osu-downloader/src/filter.rs`; this module holds the form state, the
//! preset seed-macros, and the lazy `beatmapDetails` pager.
//!
//! Like search, the fetch is CTA-triggered ([`AppCommand::RunFilter`]); presets
//! SEED the editable fields (no hidden query state), so what the form shows is
//! always exactly what is sent.
//!
//! [`AppCommand::RunFilter`]: crate::app::AppCommand::RunFilter

use super::home::InputField;
use super::search_source::{SetBrowse, cycle_idx};
use osu_downloader::filter::{
    FilterDirection, FilterMode, FilterQuery, FilterRange, FilterSort, FilterSpecial, FilterStatus,
};
use std::collections::HashMap;

/// Diff ids per `beatmapDetails` request. The first page fetches automatically
/// when results land; `m` in the browse loads the next (a free solo-dev
/// instance — never sweep every page unprompted).
pub const DETAILS_PAGE: usize = 250;

/// Status of the current filter fetch, shown inline on the form.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum FilterStatusMsg {
    /// No fetch run yet.
    #[default]
    Idle,
    /// A query is in flight.
    Loading,
    /// Results are in: the deduped set count and the summed `SizeMap` bytes.
    Ready { sets: usize, total_bytes: u64 },
    /// The query matched nothing.
    Empty,
    /// The fetch failed; the string is user-facing.
    Error(String),
}

/// A curated sort preset for the sort chip: label plus the nzbasic column +
/// direction it maps to.
struct SortPreset {
    label: &'static str,
    sort: FilterSort,
    direction: FilterDirection,
}

const SORT_PRESETS: &[SortPreset] = &[
    SortPreset {
        label: "ranked ↓",
        sort: FilterSort::ApprovedDate,
        direction: FilterDirection::Desc,
    },
    SortPreset {
        label: "stars ↓",
        sort: FilterSort::Stars,
        direction: FilterDirection::Desc,
    },
    SortPreset {
        label: "stars ↑",
        sort: FilterSort::Stars,
        direction: FilterDirection::Asc,
    },
    SortPreset {
        label: "plays ↓",
        sort: FilterSort::PlayCount,
        direction: FilterDirection::Desc,
    },
    SortPreset {
        label: "favourites ↓",
        sort: FilterSort::FavouriteCount,
        direction: FilterDirection::Desc,
    },
    SortPreset {
        label: "updated ↓",
        sort: FilterSort::LastUpdate,
        direction: FilterDirection::Desc,
    },
    SortPreset {
        label: "bpm ↓",
        sort: FilterSort::Bpm,
        direction: FilterDirection::Desc,
    },
    SortPreset {
        label: "length ↑",
        sort: FilterSort::TotalLength,
        direction: FilterDirection::Asc,
    },
];

/// Mode chip labels; `mode_idx` indexes both this and [`MODE_VALUES`].
const MODE_LABELS: &[&str] = &["any", "osu", "taiko", "catch", "mania"];
const MODE_VALUES: &[Option<FilterMode>] = &[
    None,
    Some(FilterMode::Osu),
    Some(FilterMode::Taiko),
    Some(FilterMode::Catch),
    Some(FilterMode::Mania),
];

const _: () = assert!(MODE_LABELS.len() == MODE_VALUES.len());

/// Status chip labels; `status_idx` indexes both this and [`STATUS_VALUES`].
const STATUS_LABELS: &[&str] = &[
    "any",
    "leaderboard",
    "ranked",
    "loved",
    "approved",
    "pending",
    "wip",
    "graveyard",
    "unranked",
];
const STATUS_VALUES: &[Option<FilterStatus>] = &[
    None,
    Some(FilterStatus::Leaderboard),
    Some(FilterStatus::Ranked),
    Some(FilterStatus::Loved),
    Some(FilterStatus::Approved),
    Some(FilterStatus::Pending),
    Some(FilterStatus::Wip),
    Some(FilterStatus::Graveyard),
    Some(FilterStatus::Unranked),
];

const _: () = assert!(STATUS_LABELS.len() == STATUS_VALUES.len());

/// The default status chip position (`leaderboard`), matching the osu! search
/// default so an untouched form never sweeps graveyard noise.
const STATUS_DEFAULT_IDX: usize = 1;

/// Special-tag chip labels; `special_idx` indexes both this and [`SPECIAL_VALUES`].
/// These flags exist only in nzbasic's database (not in osu! api v2).
const SPECIAL_LABELS: &[&str] = &["none", "farm", "stream", "ranked mapper"];
const SPECIAL_VALUES: &[Option<FilterSpecial>] = &[
    None,
    Some(FilterSpecial::Farm),
    Some(FilterSpecial::Stream),
    Some(FilterSpecial::RankedMapper),
];

const _: () = assert!(SPECIAL_LABELS.len() == SPECIAL_VALUES.len());

/// Preset chip labels. A preset is a seed-macro: selecting one RESETS the form
/// to defaults and seeds the fields below — every value stays visible and
/// editable, so there is no hidden query state. `none` is the plain reset.
const PRESET_LABELS: &[&str] = &["none", "all ranked", "loved", "farm", "stream", "7★+"];

/// The Get Maps `Filter` source: the flat form plus its results browse. Kept on
/// `HomeTab` so its state survives source-strip switches (keep-both).
pub struct FilterSource {
    preset_idx: usize,
    special_idx: usize,
    mode_idx: usize,
    status_idx: usize,
    sort_idx: usize,
    pub stars: InputField,
    pub ar: InputField,
    pub cs: InputField,
    pub od: InputField,
    pub hp: InputField,
    pub bpm: InputField,
    pub length: InputField,
    pub artist: InputField,
    pub creator: InputField,
    pub title: InputField,
    pub limit: InputField,
    pub status_msg: FilterStatusMsg,
    /// Every matching diff id, in server order — the `beatmapDetails` pager
    /// walks this via [`details_cursor`](Self::details_cursor).
    pub diff_ids: Vec<u32>,
    /// Offset into [`diff_ids`](Self::diff_ids) of the next unfetched details
    /// page.
    details_cursor: usize,
    /// Bytes per set id, from the fetch response's `SizeMap`.
    pub size_map: HashMap<u32, u64>,
    /// Snapshot of the canonical inputs that produced the loaded rows, so the
    /// `view N maps` button can tell fresh results from stale ones.
    results_inputs: Option<String>,
    pub browse: SetBrowse,
}

impl FilterSource {
    pub fn new() -> Self {
        Self {
            preset_idx: 0,
            special_idx: 0,
            mode_idx: 0,
            status_idx: STATUS_DEFAULT_IDX,
            sort_idx: 0,
            stars: InputField::new("stars", "", "min-max, e.g. 5.5-7"),
            ar: InputField::new("ar", "", "min-max"),
            cs: InputField::new("cs", "", "min-max"),
            od: InputField::new("od", "", "min-max"),
            hp: InputField::new("hp", "", "min-max"),
            bpm: InputField::new("bpm", "", "min-max, e.g. 180-"),
            length: InputField::new("length", "", "seconds, e.g. 90-300"),
            artist: InputField::new("artist", "", "contains…"),
            creator: InputField::new("mapper", "", "contains…"),
            title: InputField::new("title", "", "contains…"),
            limit: InputField::new("limit", "", "500"),
            status_msg: FilterStatusMsg::Idle,
            diff_ids: Vec::new(),
            details_cursor: 0,
            size_map: HashMap::new(),
            results_inputs: None,
            browse: SetBrowse::new(),
        }
    }

    // ── presets ───────────────────────────────────────────────────────────────

    /// Cycle the preset chip and apply its seed: reset the form to defaults,
    /// then set the preset's fields. Sort/limit are left untouched (they shape
    /// the result order/size, not the criteria).
    pub fn cycle_preset(&mut self, forward: bool) {
        self.preset_idx = cycle_idx(self.preset_idx, PRESET_LABELS.len(), forward);
        self.apply_preset(self.preset_idx);
    }

    fn apply_preset(&mut self, idx: usize) {
        // Reset the criteria fields (not sort/limit/results).
        self.special_idx = 0;
        self.mode_idx = 0;
        self.status_idx = STATUS_DEFAULT_IDX;
        for field in [
            &mut self.stars,
            &mut self.ar,
            &mut self.cs,
            &mut self.od,
            &mut self.hp,
            &mut self.bpm,
            &mut self.length,
            &mut self.artist,
            &mut self.creator,
            &mut self.title,
        ] {
            field.set_value("");
        }

        match PRESET_LABELS[idx] {
            "all ranked" => self.status_idx = position_of_status(FilterStatus::Ranked),
            "loved" => self.status_idx = position_of_status(FilterStatus::Loved),
            // BBD parity: its farm/stream presets pin mode to osu!standard.
            "farm" => {
                self.mode_idx = position_of_mode(FilterMode::Osu);
                self.special_idx = position_of_special(FilterSpecial::Farm);
            }
            "stream" => {
                self.mode_idx = position_of_mode(FilterMode::Osu);
                self.special_idx = position_of_special(FilterSpecial::Stream);
            }
            "7★+" => self.stars.set_value("7-"),
            _ => {} // "none" = the plain reset above
        }
    }

    // ── chips ─────────────────────────────────────────────────────────────────

    pub fn cycle_special(&mut self, forward: bool) {
        self.special_idx = cycle_idx(self.special_idx, SPECIAL_VALUES.len(), forward);
    }

    pub fn cycle_mode(&mut self, forward: bool) {
        self.mode_idx = cycle_idx(self.mode_idx, MODE_VALUES.len(), forward);
    }

    pub fn cycle_status(&mut self, forward: bool) {
        self.status_idx = cycle_idx(self.status_idx, STATUS_VALUES.len(), forward);
    }

    pub fn cycle_sort(&mut self, forward: bool) {
        self.sort_idx = cycle_idx(self.sort_idx, SORT_PRESETS.len(), forward);
    }

    pub fn preset_label(&self) -> &'static str {
        PRESET_LABELS[self.preset_idx]
    }

    pub fn special_label(&self) -> &'static str {
        SPECIAL_LABELS[self.special_idx]
    }

    pub fn mode_label(&self) -> &'static str {
        MODE_LABELS[self.mode_idx]
    }

    pub fn status_label(&self) -> &'static str {
        STATUS_LABELS[self.status_idx]
    }

    pub fn sort_label(&self) -> &'static str {
        SORT_PRESETS[self.sort_idx].label
    }

    /// Every option label per chip, in cycle order — for full cloudy cycle rows
    /// that show all options with the active one bracketed.
    pub fn preset_labels(&self) -> &'static [&'static str] {
        PRESET_LABELS
    }

    pub fn special_labels(&self) -> &'static [&'static str] {
        SPECIAL_LABELS
    }

    pub fn mode_labels(&self) -> &'static [&'static str] {
        MODE_LABELS
    }

    pub fn status_labels(&self) -> &'static [&'static str] {
        STATUS_LABELS
    }

    pub fn sort_labels(&self) -> Vec<&'static str> {
        SORT_PRESETS.iter().map(|preset| preset.label).collect()
    }

    // ── query ─────────────────────────────────────────────────────────────────

    /// The nzbasic query for the current form. `Err` carries a user-facing
    /// message naming the first invalid field.
    pub fn build_query(&self) -> Result<FilterQuery, String> {
        let preset = &SORT_PRESETS[self.sort_idx];
        Ok(FilterQuery {
            mode: MODE_VALUES[self.mode_idx],
            status: STATUS_VALUES[self.status_idx],
            special: SPECIAL_VALUES[self.special_idx],
            stars: parse_range("stars", &self.stars.value)?,
            ar: parse_range("ar", &self.ar.value)?,
            cs: parse_range("cs", &self.cs.value)?,
            od: parse_range("od", &self.od.value)?,
            hp: parse_range("hp", &self.hp.value)?,
            bpm: parse_range("bpm", &self.bpm.value)?,
            length: parse_range("length", &self.length.value)?,
            artist: self.artist.value.trim().to_string(),
            creator: self.creator.value.trim().to_string(),
            title: self.title.value.trim().to_string(),
            sort: Some((preset.sort, preset.direction)),
            limit: Some(parse_limit(&self.limit.value)?),
        })
    }

    /// Canonical string of the CRITERIA fields only (no sort/limit): two runs
    /// filtering the same maps share a folder tag even if ordered differently.
    fn criteria_string(query: &FilterQuery) -> String {
        format!(
            "mode={:?}|status={:?}|special={:?}|stars={:?}|ar={:?}|cs={:?}|od={:?}|hp={:?}|bpm={:?}|len={:?}|artist={}|creator={}|title={}",
            query.mode,
            query.status,
            query.special,
            query.stars,
            query.ar,
            query.cs,
            query.od,
            query.hp,
            query.bpm,
            query.length,
            query.artist,
            query.creator,
            query.title,
        )
    }

    /// Canonical string of ALL inputs (criteria + sort + limit) — the staleness
    /// key for the `view N maps` button.
    fn inputs_string(&self) -> String {
        let sort = &SORT_PRESETS[self.sort_idx];
        format!(
            "{}|sort={:?}/{:?}|limit={}",
            self.build_query()
                .map(|q| Self::criteria_string(&q))
                .unwrap_or_else(|err| format!("invalid:{err}")),
            sort.sort,
            sort.direction,
            self.limit.value.trim(),
        )
    }

    /// The per-run subdir tag: the preset label when the current criteria still
    /// match that preset's seed exactly, else an 8-hex FNV-1a hash of the
    /// canonical criteria — deterministic, so re-running the same filter lands
    /// in the same dir and different filters never collide on the per-dir lock.
    pub fn folder_tag(&self) -> String {
        let Ok(query) = self.build_query() else {
            return "results".to_string();
        };
        if self.preset_idx != 0 {
            let mut seeded = Self::new();
            seeded.apply_preset(self.preset_idx);
            if let Ok(preset_query) = seeded.build_query()
                && Self::criteria_string(&preset_query) == Self::criteria_string(&query)
            {
                return PRESET_LABELS[self.preset_idx].to_string();
            }
        }
        format!("{:08x}", fnv1a_32(&Self::criteria_string(&query)))
    }

    /// The per-run display label: the matching preset, else the first text
    /// criterion, else the special/stars/status descriptor. Never empty.
    pub fn run_label(&self) -> String {
        if self.preset_idx != 0 && self.folder_tag() == PRESET_LABELS[self.preset_idx] {
            return PRESET_LABELS[self.preset_idx].to_string();
        }
        for field in [&self.title, &self.artist, &self.creator] {
            let value = field.value.trim();
            if !value.is_empty() {
                return value.to_string();
            }
        }
        if self.special_idx != 0 {
            return SPECIAL_LABELS[self.special_idx].to_string();
        }
        let stars = self.stars.value.trim();
        if !stars.is_empty() {
            return format!("stars {stars}");
        }
        if self.status_idx != STATUS_DEFAULT_IDX {
            return STATUS_LABELS[self.status_idx].to_string();
        }
        "results".to_string()
    }

    // ── results / staleness ───────────────────────────────────────────────────

    /// Whether the loaded results still match the current inputs (so the
    /// `view N maps` button offers the right results, not stale ones).
    pub fn results_current(&self) -> bool {
        self.results_inputs.as_ref() == Some(&self.inputs_string())
    }

    pub fn mark_results_current(&mut self) {
        self.results_inputs = Some(self.inputs_string());
    }

    pub fn clear_results_snapshot(&mut self) {
        self.results_inputs = None;
    }

    /// Adopt a fetch response: remember the diff ids + sizes and rewind the
    /// details pager. Row population happens caller-side (the runtime handler
    /// owns the `BrowseRow` mapping).
    pub fn set_results(&mut self, diff_ids: Vec<u32>, size_map: HashMap<u32, u64>) {
        self.diff_ids = diff_ids;
        self.size_map = size_map;
        self.details_cursor = 0;
    }

    /// The next unfetched `beatmapDetails` page, advancing the pager. `None`
    /// once every diff id has been requested.
    pub fn next_details_page(&mut self) -> Option<Vec<u32>> {
        if self.details_cursor >= self.diff_ids.len() {
            return None;
        }
        let end = (self.details_cursor + DETAILS_PAGE).min(self.diff_ids.len());
        let page = self.diff_ids[self.details_cursor..end].to_vec();
        self.details_cursor = end;
        Some(page)
    }

    /// Rewind the pager to `cursor` (a failed page retries on the next `m`).
    pub fn rewind_details(&mut self, cursor: usize) {
        self.details_cursor = cursor.min(self.diff_ids.len());
    }

    /// The pager offset (captured before a fetch so a failure can rewind).
    pub fn details_cursor(&self) -> usize {
        self.details_cursor
    }

    /// Whether `m` still has details pages to load.
    pub fn has_more_details(&self) -> bool {
        self.details_cursor < self.diff_ids.len()
    }
}

impl Default for FilterSource {
    fn default() -> Self {
        Self::new()
    }
}

/// Chip position of a status value. Compile-time-adjacent: the arrays are
/// const-asserted equal-length, and every enum value appears in them.
fn position_of_status(status: FilterStatus) -> usize {
    STATUS_VALUES
        .iter()
        .position(|&v| v == Some(status))
        .unwrap_or(0)
}

fn position_of_mode(mode: FilterMode) -> usize {
    MODE_VALUES
        .iter()
        .position(|&v| v == Some(mode))
        .unwrap_or(0)
}

fn position_of_special(special: FilterSpecial) -> usize {
    SPECIAL_VALUES
        .iter()
        .position(|&v| v == Some(special))
        .unwrap_or(0)
}

/// Parse a min-max pair: empty = unconstrained, `a-b` = both bounds,
/// `a-` = min only, `-b` = max only, a bare `a` = exactly that value.
fn parse_range(label: &str, value: &str) -> Result<FilterRange, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(FilterRange::default());
    }
    let parse = |part: &str| -> Result<f64, String> {
        part.trim()
            .parse::<f64>()
            .ok()
            // `f64::parse` accepts "nan"/"inf"; neither is a usable bound and
            // NaN would slip past the min>max guard, so reject at the boundary.
            .filter(|value| value.is_finite())
            .ok_or_else(|| format!("{label}: \"{part}\" is not a number"))
    };
    let range = match value.split_once('-') {
        Some((min, max)) => FilterRange {
            min: (!min.trim().is_empty()).then(|| parse(min)).transpose()?,
            max: (!max.trim().is_empty()).then(|| parse(max)).transpose()?,
        },
        None => {
            let exact = parse(value)?;
            FilterRange {
                min: Some(exact),
                max: Some(exact),
            }
        }
    };
    if let (Some(min), Some(max)) = (range.min, range.max)
        && min > max
    {
        return Err(format!("{label}: min {min} is greater than max {max}"));
    }
    Ok(range)
}

/// Default 500; the cap protects the free instance from megaqueries.
fn parse_limit(value: &str) -> Result<u32, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(500);
    }
    let limit: u32 = value
        .parse()
        .map_err(|_| format!("limit: \"{value}\" is not a number"))?;
    if !(1..=10_000).contains(&limit) {
        return Err("limit: must be between 1 and 10000".to_string());
    }
    Ok(limit)
}

/// FNV-1a 32-bit — a stable, dependency-free hash for the folder tag.
fn fnv1a_32(s: &str) -> u32 {
    s.bytes().fold(0x811c_9dc5_u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
    })
}

#[cfg(test)]
#[path = "../../tests/unit/app_filter_source.rs"]
mod tests;
