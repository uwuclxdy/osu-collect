use super::Mirror;
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

/// Ceiling on a slot's adaptive request spacing. A run of 429s doubles the
/// interval up to this cap; it never grows past it.
const INTERVAL_CEILING: Duration = Duration::from_secs(5);

/// Amount a slot's spacing shrinks per successful attempt, decaying back toward
/// the per-kind [`base_request_interval`](super::MirrorKind::base_request_interval).
const INTERVAL_DECAY: Duration = Duration::from_millis(25);

/// A 429 landing within this window of the previous cooldown's expiry escalates
/// (doubles) the cooldown; a 429 arriving later resets it to the per-kind base.
const ESCALATION_WINDOW: Duration = Duration::from_secs(30);

/// Hard cap on an escalated cooldown.
const COOLDOWN_CAP: Duration = Duration::from_secs(10 * 60);

/// Lower clamp for a server-supplied `Retry-After`. A hostile or buggy value
/// below this is floored so the pool never busy-retries.
#[cfg(not(test))]
const RETRY_AFTER_MIN: Duration = Duration::from_secs(1);
#[cfg(test)]
const RETRY_AFTER_MIN: Duration = Duration::from_millis(1);

/// Upper clamp for a server-supplied `Retry-After`.
const RETRY_AFTER_MAX: Duration = Duration::from_secs(15 * 60);

/// Outcome of a [`MirrorPool::acquire`] call.
pub(crate) enum Acquire {
    /// Slot index granted a send token; the token has been consumed.
    Granted(usize),
    /// No candidate is admissible yet.
    Wait {
        /// Earliest instant any candidate becomes admissible.
        until: Instant,
        /// Whether that earliest candidate is gated by an active cooldown (a
        /// rate-limit penalty) rather than plain request spacing. Drives the
        /// caller's decision to surface a rate-limit status to the UI.
        cooling: bool,
    },
}

/// Per-slot scheduler state. Keyed by **mirror index** (position in `mirrors`),
/// never by [`MirrorKind`](super::MirrorKind): two custom mirrors share the
/// `Custom` kind but must be paced and penalized independently.
struct SlotState {
    /// AIMD floor this slot's `interval` decays back toward.
    base_interval: Duration,
    /// Live request spacing: doubled on a 429, decayed on success.
    interval: Duration,
    /// Send token: the slot may be granted again once `now >= next_free`.
    next_free: Instant,
    /// Active rate-limit cooldown end, if the slot is penalized.
    cooldown_until: Option<Instant>,
    /// Duration of the most recent cooldown; the escalation multiplier's input.
    last_cooldown: Duration,
    /// Expiry anchor of the most recent cooldown; a fresh 429 within
    /// [`ESCALATION_WINDOW`] of this instant escalates instead of resetting.
    cooldown_expired_at: Option<Instant>,
}

impl SlotState {
    fn new(base_interval: Duration, now: Instant) -> Self {
        Self {
            base_interval,
            interval: base_interval,
            next_free: now,
            cooldown_until: None,
            last_cooldown: Duration::ZERO,
            cooldown_expired_at: None,
        }
    }
}

/// Admission-control scheduler over the configured mirror list.
///
/// Workers do not free-fire: they [`acquire`](MirrorPool::acquire) a send token
/// for one of their candidate slots. Granting consumes the token
/// (`next_free = now + interval`), so concurrent acquirers self-space and spill
/// to the next mirror in the caller's configured order instead of bursting the
/// same slot. A 429 [`marks`](MirrorPool::mark_rate_limited) the slot with an
/// (escalating) cooldown and widens its spacing; a success narrows it.
pub(crate) struct MirrorPool {
    mirrors: Vec<Mirror>,
    slots: Mutex<Vec<SlotState>>,
}

impl MirrorPool {
    pub(crate) fn new(mirrors: Vec<Mirror>) -> Self {
        let now = Instant::now();
        let slots = mirrors
            .iter()
            .map(|m| SlotState::new(m.kind().base_request_interval(), now))
            .collect();
        Self {
            mirrors,
            slots: Mutex::new(slots),
        }
    }

    /// Test-only constructor with an explicit per-slot base interval, so decay
    /// arithmetic can be asserted with a base larger than the shrunken test
    /// constants (where `base * n - INTERVAL_DECAY` would underflow to the
    /// floor and mask a hard reset).
    #[cfg(test)]
    pub(crate) fn new_with_base_interval(mirrors: Vec<Mirror>, base: Duration) -> Self {
        let now = Instant::now();
        let slots = mirrors.iter().map(|_| SlotState::new(base, now)).collect();
        Self {
            mirrors,
            slots: Mutex::new(slots),
        }
    }

    /// Grant a send token for the first admissible slot in `candidates`
    /// (caller's configured order), or report when the earliest candidate frees.
    pub(crate) fn acquire(&self, candidates: &[usize]) -> Acquire {
        self.acquire_at(candidates, Instant::now())
    }

    /// [`acquire`](Self::acquire) with an injected clock for deterministic tests.
    pub(crate) fn acquire_at(&self, candidates: &[usize], now: Instant) -> Acquire {
        let mut slots = self.slots.lock().unwrap();
        let mut grant: Option<usize> = None;
        let mut best: Option<(Instant, bool)> = None;

        for &idx in candidates {
            let Some(s) = slots.get(idx) else { continue };
            let cd_active = s.cooldown_until.filter(|&c| c > now);
            if cd_active.is_none() && s.next_free <= now {
                grant = Some(idx);
                break;
            }
            let free_at = s.next_free.max(cd_active.unwrap_or(now));
            // A live cooldown makes this a rate-limit wait regardless of whether
            // the cooldown tail or a grown spacing token is the later bound; a
            // cooldown out-sat by spacing must not lose its countdown/budget/skip.
            let cooling = cd_active.is_some();
            if best.is_none_or(|(t, _)| free_at < t) {
                best = Some((free_at, cooling));
            }
        }

        if let Some(idx) = grant {
            let s = &mut slots[idx];
            s.next_free = now + s.interval;
            return Acquire::Granted(idx);
        }

        match best {
            Some((until, cooling)) => Acquire::Wait { until, cooling },
            None => Acquire::Wait {
                until: now,
                cooling: false,
            },
        }
    }

    /// Mark the slot at `idx` rate-limited. `retry_after` is the parsed 429
    /// `Retry-After` (clamped to `[1 s, 15 min]`); absent, the cooldown is the
    /// per-kind base. A 429 landing within [`ESCALATION_WINDOW`] of the last
    /// cooldown's expiry escalates (doubles) it; when `retry_after` is present the
    /// escalated value is a floor, so a short server hint cannot undercut a repeat
    /// offender's penalty. Widens the slot's spacing too. A no-op while a cooldown
    /// is live, so one burst of concurrent 429s is one penalty.
    pub(crate) fn mark_rate_limited(&self, idx: usize, retry_after: Option<Duration>) {
        self.mark_rate_limited_at(idx, retry_after, Instant::now());
    }

    /// [`mark_rate_limited`](Self::mark_rate_limited) with an injected clock.
    pub(crate) fn mark_rate_limited_at(
        &self,
        idx: usize,
        retry_after: Option<Duration>,
        now: Instant,
    ) {
        let base = match self.mirrors.get(idx) {
            Some(mirror) => mirror.kind().rate_limit_backoff(),
            None => return,
        };
        let mut slots = self.slots.lock().unwrap();
        let Some(s) = slots.get_mut(idx) else { return };
        if s.cooldown_until.is_some_and(|c| c > now) {
            return;
        }

        // A 429 landing inside the escalation window doubles the previous
        // cooldown; outside it (or on the first hit) there is no escalation.
        let escalating = s
            .cooldown_expired_at
            .is_some_and(|exp| now >= exp && now.duration_since(exp) <= ESCALATION_WINDOW);
        let escalated = escalating.then(|| {
            s.last_cooldown
                .checked_mul(2)
                .unwrap_or(COOLDOWN_CAP)
                .min(COOLDOWN_CAP)
        });

        let cooldown = match retry_after {
            // A repeat offender's server hint is a floor, never a ceiling: honor
            // Retry-After but never let a short one undercut the escalated penalty.
            Some(ra) => {
                let clamped = ra.clamp(RETRY_AFTER_MIN, RETRY_AFTER_MAX);
                escalated.map_or(clamped, |esc| clamped.max(esc))
            }
            None => escalated.unwrap_or(base),
        };

        s.last_cooldown = cooldown;
        let until = now + cooldown;
        s.cooldown_until = Some(until);
        s.cooldown_expired_at = Some(until);
        s.interval = s
            .interval
            .checked_mul(2)
            .unwrap_or(INTERVAL_CEILING)
            .min(INTERVAL_CEILING);
    }

    /// Narrow the slot's spacing after a successful attempt, decaying it toward
    /// the per-kind base.
    pub(crate) fn on_success(&self, idx: usize) {
        let mut slots = self.slots.lock().unwrap();
        if let Some(s) = slots.get_mut(idx) {
            s.interval = s
                .interval
                .checked_sub(INTERVAL_DECAY)
                .unwrap_or(s.base_interval)
                .max(s.base_interval);
        }
    }

    /// Current request spacing for the slot at `idx` (base if out of range).
    #[cfg_attr(not(feature = "instrument"), allow(dead_code))]
    pub(crate) fn interval(&self, idx: usize) -> Duration {
        let slots = self.slots.lock().unwrap();
        slots
            .get(idx)
            .map(|s| s.interval)
            .unwrap_or(super::BASE_INTERVAL)
    }

    pub(crate) fn mirrors(&self) -> &[Mirror] {
        &self.mirrors
    }
}

#[cfg(test)]
#[path = "../../tests/pool.rs"]
mod tests;
