use super::*;
use crate::app::{App, AppCommand, EnrichSink, EnrichTarget, FindStatusMsg};
use crate::config::Config;
use osu_downloader::filter::FilterResults;
use std::collections::HashMap;

fn app() -> App {
    App::new(Config::default())
}

fn results(set_ids: Vec<u32>, diff_ids: Vec<u32>) -> FilterResults {
    let size_map: HashMap<u32, u64> = set_ids.iter().map(|&id| (id, 1_000_000)).collect();
    FilterResults {
        ids: diff_ids,
        set_ids,
        size_map,
        hashes: Vec::new(),
    }
}

#[test]
fn loading_sets_status() {
    let mut app = app();
    let follow_up = handle_home_filter_event(HomeFilterEvent::Loading, &mut app);
    assert!(follow_up.is_none());
    assert_eq!(app.home.find.status_msg, FindStatusMsg::Loading);
}

#[test]
fn results_populate_descend_and_request_first_enrich_page() {
    let mut app = app();
    let follow_up = handle_home_filter_event(
        HomeFilterEvent::Results {
            results: results(vec![10, 20], vec![1, 2, 3]),
        },
        &mut app,
    );
    // The auto-fetch of the first enrichment page rides back as a command.
    assert!(matches!(
        follow_up,
        Some(AppCommand::LoadEnrichment {
            target: EnrichTarget::Find
        })
    ));
    assert_eq!(
        app.home.find.status_msg,
        FindStatusMsg::ReadyFilter {
            sets: 2,
            total_bytes: 2_000_000
        }
    );
    assert!(app.home.find.browse.is_browsing());
    assert_eq!(app.home.find.browse.rows.len(), 2);
    assert!(app.home.find.browse.rows.iter().all(|r| r.meta.is_none()));
    assert!(app.home.find.results_current());
    // The diff ids seeded the browse's enrichment pager, so `m` has more to load.
    assert!(app.home.find.browse.has_more_enrichment());
}

/// Part 1 of the size-fetch rework: nzbasic's per-set sizes are free and
/// exact, so they fold into the shared cache the download-size seed reads —
/// this is also what gives the nzbasic route its `· ~X` download-button
/// suffix (rendered off `checked_known_bytes`) for free.
#[test]
fn results_fold_nzbasic_sizes_into_size_cache() {
    let mut app = app();
    handle_home_filter_event(
        HomeFilterEvent::Results {
            results: results(vec![10, 20], vec![1, 2]),
        },
        &mut app,
    );
    app.home.find.browse.set_all_selected(true);
    // `results()` seeds 1_000_000 bytes per set.
    assert_eq!(app.home.find.checked_known_bytes(), 2_000_000);
    assert_eq!(
        app.home.find.known_sizes_for(&[10, 20, 30]),
        HashMap::from([(10, 1_000_000), (20, 1_000_000)])
    );
}

#[test]
fn empty_clears_rows_and_snapshot() {
    let mut app = app();
    handle_home_filter_event(
        HomeFilterEvent::Results {
            results: results(vec![10], vec![1]),
        },
        &mut app,
    );
    handle_home_filter_event(HomeFilterEvent::Empty, &mut app);
    assert_eq!(app.home.find.status_msg, FindStatusMsg::Empty);
    assert!(app.home.find.browse.rows.is_empty());
    assert!(!app.home.find.results_current());
    // `set_rows(empty)` cleared the enrichment pager along with the rows.
    assert!(!app.home.find.browse.has_more_enrichment());
}

#[test]
fn failure_reports_the_reason_and_stales_results() {
    let mut app = app();
    handle_home_filter_event(
        HomeFilterEvent::Failed {
            reason: "nzbasic unreachable".to_string(),
        },
        &mut app,
    );
    assert_eq!(
        app.home.find.status_msg,
        FindStatusMsg::Error("nzbasic unreachable".to_string())
    );
    assert!(!app.home.find.results_current());
}

/// Cross-routed end-to-end: a nzbasic-forcer routed the fetch — the handler
/// records the nzbasic backend and the download follows it into
/// `IdsRunSource::Filter` (the `filter-` subdir prefix), driven by the recorded
/// results backend, not the form's default.
#[test]
fn results_record_nzbasic_backend_and_download_routes_filter() {
    use crate::app::FindBackend;
    use crate::download::IdsRunSource;
    let mut app = app();
    // A nzbasic-forcer, as the run that produced these results would have.
    app.home.find.cycle_special(true); // → farm

    handle_home_filter_event(
        HomeFilterEvent::Results {
            results: results(vec![10, 20], vec![1, 2]),
        },
        &mut app,
    );
    assert_eq!(app.home.find.results_backend(), Some(FindBackend::Nzbasic));

    app.home.find.browse.set_all_selected(true);
    let (_, request) = app
        .request_find_download()
        .expect("default config has mirrors enabled");
    assert_eq!(request.source, IdsRunSource::Filter);
}
