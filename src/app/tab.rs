//! Top-strip tab identity.
//!
//! All three tabs are static and render in declaration order. Per-run download
//! tabs are gone — every run (active or past) lives on the Downloads tab's
//! list. Adding or removing a tab is an edit here plus the
//! [`Tab::to_index`]/[`Tab::from_index`] arms, not a hunt for magic indices.

/// One tab in the top strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    /// The `get maps` tab (search / collection / update sources).
    Home,
    /// The `downloads` tab — active runs + past-run history.
    Downloads,
    /// The `config` tab.
    Config,
}

impl Tab {
    /// Flat tab-strip position, matching declaration order.
    pub fn to_index(self) -> usize {
        match self {
            Tab::Home => 0,
            Tab::Downloads => 1,
            Tab::Config => 2,
        }
    }

    /// Inverse of [`to_index`](Tab::to_index). Callers keep the index in range
    /// (`0..STATIC_TABS`); anything past the end clamps to the last tab.
    pub fn from_index(index: usize) -> Tab {
        match index {
            0 => Tab::Home,
            1 => Tab::Downloads,
            _ => Tab::Config,
        }
    }
}
