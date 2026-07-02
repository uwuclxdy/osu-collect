use super::{Acquire, INTERVAL_DECAY, MirrorPool, RETRY_AFTER_MAX, RETRY_AFTER_MIN};
use crate::{Mirror, MirrorKind};
use std::time::{Duration, Instant};

fn granted(a: Acquire) -> usize {
    match a {
        Acquire::Granted(idx) => idx,
        Acquire::Wait { .. } => panic!("expected a grant, got wait"),
    }
}

fn waited(a: Acquire) -> (Instant, bool) {
    match a {
        Acquire::Wait { until, cooling } => (until, cooling),
        Acquire::Granted(idx) => panic!("expected a wait, got grant of {idx}"),
    }
}

fn base_cooldown() -> Duration {
    MirrorKind::Nerinyan.rate_limit_backoff()
}

#[test]
fn first_acquire_grants_first_admissible_in_order() {
    let pool = MirrorPool::new(vec![
        Mirror::nerinyan(),
        Mirror::osu_direct(),
        Mirror::sayobot(),
    ]);
    let now = Instant::now();
    assert_eq!(granted(pool.acquire_at(&[0, 1, 2], now)), 0);
}

#[test]
fn grant_consumes_token_for_the_interval_window() {
    let pool = MirrorPool::new(vec![Mirror::nerinyan()]);
    let base = MirrorKind::Nerinyan.base_request_interval();
    let now = Instant::now();
    assert_eq!(granted(pool.acquire_at(&[0], now)), 0);
    let (until, cooling) = waited(pool.acquire_at(&[0], now));
    assert!(!cooling, "a spent token is a spacing wait, not a cooldown");
    assert_eq!(until, now + base);
}

#[test]
fn concurrent_acquire_at_one_instant_grants_one_token() {
    // Simulated wake-together crowd: only a single grant is handed out for the
    // slot per interval window; the rest are told to wait.
    let pool = MirrorPool::new(vec![Mirror::nerinyan()]);
    let now = Instant::now();
    let grants = (0..16)
        .filter(|_| matches!(pool.acquire_at(&[0], now), Acquire::Granted(_)))
        .count();
    assert_eq!(grants, 1);
}

#[test]
fn acquire_spills_to_next_slot_when_first_token_is_spent() {
    let pool = MirrorPool::new(vec![Mirror::nerinyan(), Mirror::osu_direct()]);
    let now = Instant::now();
    assert_eq!(granted(pool.acquire_at(&[0, 1], now)), 0);
    assert_eq!(granted(pool.acquire_at(&[0, 1], now)), 1);
    // Both tokens spent at this instant.
    let (_until, cooling) = waited(pool.acquire_at(&[0, 1], now));
    assert!(!cooling);
}

#[test]
fn cooldown_blocks_admission_and_reports_cooling() {
    let pool = MirrorPool::new(vec![Mirror::nerinyan()]);
    let now = Instant::now();
    pool.mark_rate_limited_at(0, None, now);
    let (until, cooling) = waited(pool.acquire_at(&[0], now));
    assert!(cooling);
    assert_eq!(until, now + base_cooldown());
}

#[test]
fn cooldown_expires_then_grants() {
    let pool = MirrorPool::new(vec![Mirror::nerinyan()]);
    let now = Instant::now();
    pool.mark_rate_limited_at(0, None, now);
    let after = now + base_cooldown() + Duration::from_millis(1);
    assert_eq!(granted(pool.acquire_at(&[0], after)), 0);
}

#[test]
fn remark_during_live_cooldown_is_a_noop() {
    let pool = MirrorPool::new(vec![Mirror::nerinyan()]);
    let now = Instant::now();
    pool.mark_rate_limited_at(0, None, now);
    // A second 429 mid-cooldown must not extend the original deadline.
    pool.mark_rate_limited_at(0, None, now + base_cooldown() / 2);
    let (until, _) = waited(pool.acquire_at(&[0], now + base_cooldown() / 2));
    assert_eq!(until, now + base_cooldown());
}

#[test]
fn escalation_within_window_doubles_the_cooldown() {
    let pool = MirrorPool::new(vec![Mirror::nerinyan()]);
    let t0 = Instant::now();
    pool.mark_rate_limited_at(0, None, t0);
    // Cooldown has expired but the next 429 lands inside the escalation window.
    let t1 = t0 + base_cooldown() + Duration::from_secs(1);
    pool.mark_rate_limited_at(0, None, t1);
    let (until, _) = waited(pool.acquire_at(&[0], t1));
    assert_eq!(until, t1 + base_cooldown() * 2);
}

#[test]
fn escalation_resets_outside_window() {
    let pool = MirrorPool::new(vec![Mirror::nerinyan()]);
    let t0 = Instant::now();
    pool.mark_rate_limited_at(0, None, t0);
    // Well past the escalation window: back to the per-kind base.
    let t1 = t0 + base_cooldown() + Duration::from_secs(31);
    pool.mark_rate_limited_at(0, None, t1);
    let (until, _) = waited(pool.acquire_at(&[0], t1));
    assert_eq!(until, t1 + base_cooldown());
}

#[test]
fn retry_after_is_clamped_to_the_ceiling() {
    let pool = MirrorPool::new(vec![Mirror::nerinyan()]);
    let now = Instant::now();
    pool.mark_rate_limited_at(0, Some(Duration::from_secs(3600)), now);
    let (until, _) = waited(pool.acquire_at(&[0], now));
    assert_eq!(until, now + RETRY_AFTER_MAX);
}

#[test]
fn retry_after_is_clamped_to_the_floor() {
    let pool = MirrorPool::new(vec![Mirror::nerinyan()]);
    let now = Instant::now();
    pool.mark_rate_limited_at(0, Some(Duration::ZERO), now);
    let (until, _) = waited(pool.acquire_at(&[0], now));
    assert_eq!(until, now + RETRY_AFTER_MIN);
}

#[test]
fn retry_after_floors_but_does_not_cap_escalation() {
    // A repeat offender answering 429 + a tiny Retry-After must still escalate:
    // the server hint is a floor, so the escalated cooldown wins over it.
    let pool = MirrorPool::new(vec![Mirror::nerinyan()]);
    let t0 = Instant::now();
    pool.mark_rate_limited_at(0, None, t0);
    let t1 = t0 + base_cooldown() + Duration::from_secs(1);
    pool.mark_rate_limited_at(0, Some(Duration::from_millis(1)), t1);
    let (until, _) = waited(pool.acquire_at(&[0], t1));
    assert_eq!(
        until,
        t1 + base_cooldown() * 2,
        "escalation must win over a short Retry-After inside the window"
    );
}

#[test]
fn cooling_is_reported_when_cooldown_is_out_sat_by_spacing() {
    // A live cooldown whose tail is shorter than a grown send-token spacing must
    // still report a cooling wait, so the countdown / auto-defer budget / skip
    // path stays reachable for that slot.
    let base = Duration::from_millis(1500);
    let pool = MirrorPool::new_with_base_interval(
        vec![Mirror::custom("https://a.example/d/{id}").unwrap()],
        base,
    );
    let now = Instant::now();
    // The grant pushes next_free out to now + 1.5 s (the spacing token).
    assert_eq!(granted(pool.acquire_at(&[0], now)), 0);
    // A 0.5 s cooldown expires before that spacing token frees.
    pool.mark_rate_limited_at(0, Some(Duration::from_millis(500)), now);
    let (until, cooling) = waited(pool.acquire_at(&[0], now));
    assert!(
        cooling,
        "a live cooldown makes the wait cooling even when spacing is the later bound"
    );
    assert_eq!(
        until,
        now + base,
        "the later of the cooldown tail and the spacing token binds"
    );
}

#[test]
fn interval_doubles_on_429_and_decays_on_success() {
    let pool = MirrorPool::new(vec![Mirror::nerinyan()]);
    let base = MirrorKind::Nerinyan.base_request_interval();
    let t0 = Instant::now();
    assert_eq!(pool.interval(0), base);
    pool.mark_rate_limited_at(0, None, t0);
    assert_eq!(pool.interval(0), base * 2);
    // A second 429 after the cooldown expired doubles again.
    pool.mark_rate_limited_at(0, None, t0 + base_cooldown() + Duration::from_millis(1));
    assert_eq!(pool.interval(0), base * 4);
    pool.on_success(0);
    // Decay never dips below the per-kind base. With the shrunken test base
    // (1 ms) this exercises only the underflow-to-floor branch; the stepwise
    // AIMD decay is pinned separately below with a larger base.
    assert_eq!(pool.interval(0), base);
}

#[test]
fn success_decays_interval_by_exactly_one_step() {
    // Base chosen so base * 4 - INTERVAL_DECAY > base: one success must shave
    // exactly INTERVAL_DECAY off the interval, not hard-reset it to the base.
    let base = Duration::from_millis(40);
    let pool = MirrorPool::new_with_base_interval(vec![Mirror::nerinyan()], base);
    let t0 = Instant::now();
    pool.mark_rate_limited_at(0, None, t0);
    pool.mark_rate_limited_at(0, None, t0 + base_cooldown() + Duration::from_millis(1));
    assert_eq!(pool.interval(0), base * 4);
    pool.on_success(0);
    assert_eq!(pool.interval(0), base * 4 - INTERVAL_DECAY);
    // Repeated successes keep stepping down and only floor-clamp at the base.
    for _ in 0..16 {
        pool.on_success(0);
    }
    assert_eq!(pool.interval(0), base);
}

#[test]
fn cooldowns_are_independent_across_custom_mirrors() {
    // Two custom mirrors share `MirrorKind::Custom`; per-slot keying keeps their
    // cooldowns apart so a 429 on one does not sideline the other.
    let pool = MirrorPool::new(vec![
        Mirror::custom("https://a.example/d/{id}").unwrap(),
        Mirror::custom("https://b.example/d/{id}").unwrap(),
    ]);
    let now = Instant::now();
    pool.mark_rate_limited_at(0, None, now);
    assert_eq!(granted(pool.acquire_at(&[0, 1], now)), 1);
}

#[test]
fn out_of_range_index_is_ignored() {
    let pool = MirrorPool::new(vec![Mirror::nerinyan()]);
    let now = Instant::now();
    pool.mark_rate_limited_at(5, None, now);
    // Candidate 5 does not exist and is skipped; slot 0 is still granted.
    assert_eq!(granted(pool.acquire_at(&[5, 0], now)), 0);
}

#[test]
fn mirrors_preserves_order_and_duplicates() {
    let pool = MirrorPool::new(vec![
        Mirror::nerinyan(),
        Mirror::osu_direct(),
        Mirror::nerinyan(),
    ]);
    let kinds: Vec<MirrorKind> = pool.mirrors().iter().map(Mirror::kind).collect();
    assert_eq!(
        kinds,
        vec![
            MirrorKind::Nerinyan,
            MirrorKind::OsuDirect,
            MirrorKind::Nerinyan,
        ]
    );
}
