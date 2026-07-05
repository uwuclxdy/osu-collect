//! Transient top-right notifications (toast).
//!
//! Toasts carry fire-and-forget results and errors. They are ephemeral by
//! design — there is no history surface. Durable signals live elsewhere: a
//! sticky condition is a banner, a broken resource is inline `[ failed ]`
//! state, an in-progress operation is the footer loading line.

use std::time::{Duration, Instant};

/// Maximum simultaneously-stacked toasts. A later arrival drops the oldest.
const MAX_TOASTS: usize = 3;
/// Auto-dismiss dwell for non-error toasts.
const DWELL_DEFAULT: Duration = Duration::from_secs(5);
/// Auto-dismiss dwell for `Danger` toasts — a failure lingers long enough to read.
const DWELL_DANGER: Duration = Duration::from_secs(6);

/// Severity of a toast — drives the left-bar color and the auto-dismiss dwell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastLevel {
    Success,
    Info,
    Warning,
    Danger,
}

/// How long a toast stays on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToastLife {
    /// Auto-dismiss after the level's dwell (5 s; 6 s for `Danger`).
    Auto,
    /// Stays until [`Toasts::replace_tagged`] swaps it out — for an operation
    /// with no known end time (the self-update download).
    UntilResolved,
    /// Stays until the user presses `x` — for a notice that needs a follow-up
    /// action (restart after an update).
    UntilDismissed,
}

/// Identifies a toast later code needs to find again to swap or update it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastTag {
    /// The self-update toast: "downloading update" while in flight, swapped for
    /// the "restart to finish" notice on completion.
    Update,
}

/// A single notification.
///
/// Build via [`Toast::success`] / [`Toast::info`] / [`Toast::warning`] /
/// [`Toast::danger`], then chain [`with_detail`](Toast::with_detail),
/// [`until_resolved`](Toast::until_resolved),
/// [`until_dismissed`](Toast::until_dismissed) or [`tagged`](Toast::tagged).
pub struct Toast {
    level: ToastLevel,
    title: String,
    detail: Option<String>,
    life: ToastLife,
    tag: Option<ToastTag>,
    created_at: Instant,
}

impl Toast {
    pub fn success(title: impl Into<String>) -> Self {
        Self::new(ToastLevel::Success, title)
    }

    pub fn info(title: impl Into<String>) -> Self {
        Self::new(ToastLevel::Info, title)
    }

    pub fn warning(title: impl Into<String>) -> Self {
        Self::new(ToastLevel::Warning, title)
    }

    pub fn danger(title: impl Into<String>) -> Self {
        Self::new(ToastLevel::Danger, title)
    }

    fn new(level: ToastLevel, title: impl Into<String>) -> Self {
        Self {
            level,
            title: title.into(),
            detail: None,
            life: ToastLife::Auto,
            tag: None,
            created_at: Instant::now(),
        }
    }

    /// Add a second (dim) line carrying context — target, reason, amount.
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Keep the toast until [`Toasts::replace_tagged`] swaps it out.
    pub fn until_resolved(mut self) -> Self {
        self.life = ToastLife::UntilResolved;
        self
    }

    /// Keep the toast until the user dismisses it with `x`.
    pub fn until_dismissed(mut self) -> Self {
        self.life = ToastLife::UntilDismissed;
        self
    }

    /// Tag the toast so later code can [`Toasts::replace_tagged`] it.
    pub fn tagged(mut self, tag: ToastTag) -> Self {
        self.tag = Some(tag);
        self
    }

    pub fn level(&self) -> ToastLevel {
        self.level
    }

    /// First line — the toast title (rendered `TEXT + bold`).
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Optional context line (rendered `TEXT_DIM`).
    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }

    fn dwell(&self) -> Duration {
        match self.level {
            ToastLevel::Danger => DWELL_DANGER,
            _ => DWELL_DEFAULT,
        }
    }

    fn is_expired(&self) -> bool {
        matches!(self.life, ToastLife::Auto) && self.created_at.elapsed() >= self.dwell()
    }
}

/// The live toast stack. Newest renders at the top; capped at [`MAX_TOASTS`].
#[derive(Default)]
pub struct Toasts {
    items: Vec<Toast>,
}

impl Toasts {
    /// Push a new toast, dropping the oldest if the stack is at capacity.
    pub fn push(&mut self, toast: Toast) {
        if self.items.len() >= MAX_TOASTS {
            self.items.remove(0);
        }
        self.items.push(toast);
    }

    /// Replace the toast carrying `tag` in place, or push `toast` if none is
    /// present. Lets the update flow turn its "downloading" toast into the
    /// "restart" notice without reordering the stack.
    pub fn replace_tagged(&mut self, tag: ToastTag, toast: Toast) {
        match self.items.iter_mut().find(|t| t.tag == Some(tag)) {
            Some(slot) => *slot = toast,
            None => self.push(toast),
        }
    }

    /// Drop every auto-dismiss toast past its dwell. Driven by the tick.
    pub fn clear_expired(&mut self) {
        self.items.retain(|toast| !toast.is_expired());
    }

    /// Dismiss the newest (topmost) toast. Returns whether one was removed.
    pub fn dismiss_top(&mut self) -> bool {
        self.items.pop().is_some()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Toasts oldest-to-newest. The renderer reverses this so the newest sits
    /// at the top of the stack.
    pub fn iter(&self) -> std::slice::Iter<'_, Toast> {
        self.items.iter()
    }
}

#[cfg(test)]
#[path = "../../tests/unit/app_toast.rs"]
mod tests;
