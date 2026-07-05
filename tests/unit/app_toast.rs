use super::{Toast, ToastLevel, ToastLife, ToastTag, Toasts};
use std::time::{Duration, Instant};

#[test]
fn push_drops_oldest_past_cap() {
    let mut toasts = Toasts::default();
    toasts.push(Toast::info("one"));
    toasts.push(Toast::info("two"));
    toasts.push(Toast::info("three"));
    toasts.push(Toast::info("four"));

    let titles: Vec<&str> = toasts.iter().map(Toast::title).collect();
    assert_eq!(
        titles,
        vec!["two", "three", "four"],
        "oldest must drop at cap"
    );
}

#[test]
fn dismiss_top_pops_newest() {
    let mut toasts = Toasts::default();
    toasts.push(Toast::info("old"));
    toasts.push(Toast::info("new"));

    assert!(toasts.dismiss_top());
    let titles: Vec<&str> = toasts.iter().map(Toast::title).collect();
    assert_eq!(titles, vec!["old"]);

    assert!(toasts.dismiss_top());
    assert!(!toasts.dismiss_top(), "empty stack returns false");
}

#[test]
fn clear_expired_drops_only_aged_auto_toasts() {
    let mut toasts = Toasts::default();
    toasts.push(Toast::success("fresh"));
    toasts.push(Toast::success("stale"));
    // Age the second toast past the default dwell.
    if let Some(stale) = toasts.items.get_mut(1) {
        stale.created_at = Instant::now() - Duration::from_secs(6);
    }

    toasts.clear_expired();
    let titles: Vec<&str> = toasts.iter().map(Toast::title).collect();
    assert_eq!(titles, vec!["fresh"], "only the aged auto toast clears");
}

#[test]
fn danger_outlives_default_dwell() {
    let mut danger = Toast::danger("boom");
    // 5.5 s: past the 5 s default dwell but shy of danger's 6 s.
    danger.created_at = Instant::now() - Duration::from_millis(5500);
    assert!(!danger.is_expired(), "danger dwell is 6 s, not 5 s");

    danger.created_at = Instant::now() - Duration::from_secs(7);
    assert!(danger.is_expired());
}

#[test]
fn sticky_toasts_never_auto_expire() {
    for life in [ToastLife::UntilResolved, ToastLife::UntilDismissed] {
        let mut toast = Toast::info("sticky");
        toast.life = life;
        toast.created_at = Instant::now() - Duration::from_secs(600);
        assert!(!toast.is_expired(), "{life:?} must not auto-expire");
    }
}

#[test]
fn replace_tagged_swaps_in_place() {
    let mut toasts = Toasts::default();
    toasts.push(Toast::info("a"));
    toasts.push(Toast::info("downloading").tagged(ToastTag::Update));
    toasts.push(Toast::info("c"));

    toasts.replace_tagged(ToastTag::Update, Toast::success("installed"));

    let titles: Vec<&str> = toasts.iter().map(Toast::title).collect();
    assert_eq!(
        titles,
        vec!["a", "installed", "c"],
        "tagged toast swaps in place, order preserved"
    );
    assert_eq!(toasts.items[1].level(), ToastLevel::Success);
}

#[test]
fn replace_tagged_pushes_when_absent() {
    let mut toasts = Toasts::default();
    toasts.replace_tagged(ToastTag::Update, Toast::danger("update failed"));

    let titles: Vec<&str> = toasts.iter().map(Toast::title).collect();
    assert_eq!(titles, vec!["update failed"], "absent tag pushes instead");
}
