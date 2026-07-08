//! Tab-strip title tests. All three tabs are static — per-run download tabs
//! folded into the Downloads tab (phase 7), so the strip never grows and
//! carries no per-run progress suffix.

use crate::{
    app::{App, collection::CollectionPage},
    config::{Config, constants::STATIC_TABS},
    download::DownloadStage,
};

fn make_app() -> App {
    App::new(Config::default())
}

#[test]
fn strip_is_exactly_the_three_static_tabs() {
    let app = make_app();
    let titles = app.tab_titles();
    assert_eq!(titles.len(), STATIC_TABS);
    assert_eq!(titles, vec!["home", "downloads", "config"]);
}

#[test]
fn strip_does_not_grow_with_runs() {
    let mut app = make_app();
    let mut page = CollectionPage::new(1, "Ranked Maps".to_string(), 2);
    page.stage = DownloadStage::Downloading;
    app.downloads.push(page);
    let mut settled = CollectionPage::new(2, "Old Run".to_string(), 2);
    settled.stage = DownloadStage::Completed;
    app.downloads.push(settled);

    let titles = app.tab_titles();
    assert_eq!(
        titles.len(),
        STATIC_TABS,
        "runs live on the Downloads tab, never as their own tabs"
    );
}
