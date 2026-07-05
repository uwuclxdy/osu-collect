//! Top-strip tab identity.
//!
//! The three static tabs render in declaration order; each active or settled
//! download run appends one [`Tab::Download`] after them. Adding or removing a
//! static tab is an edit here plus the [`Tab::to_index`]/[`Tab::from_index`]
//! arms, not a hunt for scattered magic indices.

use crate::config::constants::STATIC_TABS;

/// One tab in the top strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    /// The `home` / Get Maps tab.
    Home,
    /// The `updates` tab.
    Updates,
    /// The `config` tab.
    Config,
    /// A per-run download page; the payload indexes `App.downloads`.
    Download(usize),
}

impl Tab {
    /// Flat tab-strip position: static tabs occupy `0..STATIC_TABS`, download
    /// tabs follow in `downloads` order.
    pub fn to_index(self) -> usize {
        match self {
            Tab::Home => 0,
            Tab::Updates => 1,
            Tab::Config => 2,
            Tab::Download(slot) => STATIC_TABS + slot,
        }
    }

    /// Inverse of [`to_index`](Tab::to_index): any index at or past `STATIC_TABS`
    /// is a download tab. Callers keep the index in range for the live tab count.
    pub fn from_index(index: usize) -> Tab {
        match index {
            0 => Tab::Home,
            1 => Tab::Updates,
            2 => Tab::Config,
            n => Tab::Download(n - STATIC_TABS),
        }
    }
}
