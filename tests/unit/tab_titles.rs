use crate::{
    app::{App, collection::CollectionPage},
    config::{Config, constants::STATIC_TABS},
    download::DownloadStage,
};

fn make_page(title: &str) -> CollectionPage {
    CollectionPage::new(1, title.to_string(), 1)
}

/// Return the download tab titles (strips the static tabs).
fn download_titles(app: &App) -> Vec<String> {
    app.tab_titles()
        .into_iter()
        .skip(STATIC_TABS)
        .map(|t| t.into_owned())
        .collect()
}

#[test]
fn static_tabs_unchanged() {
    let app = App::new(Config::default());
    let titles = app.tab_titles();
    assert_eq!(titles[0], "home");
    assert_eq!(titles[1], "config");
    assert_eq!(titles.len(), STATIC_TABS, "no download tabs by default");
}

#[test]
fn no_progress_shows_bare_name() {
    let mut app = App::new(Config::default());
    app.downloads.push(make_page("Top 100 of 2024"));
    let titles = download_titles(&app);
    assert_eq!(titles[0], "top 100 of 2024");
}

#[test]
fn in_progress_shows_percentage() {
    let mut app = App::new(Config::default());
    let mut page = make_page("Top 100 of 2024");
    page.download_target = 100;
    page.stats.downloaded = 43;
    page.stats.skipped = 0;
    app.downloads.push(page);
    let titles = download_titles(&app);
    assert!(
        titles[0].contains("(43%)"),
        "expected (43%) in {:?}",
        titles[0]
    );
}

#[test]
fn skipped_counts_toward_progress() {
    let mut app = App::new(Config::default());
    let mut page = make_page("col");
    page.download_target = 100;
    page.stats.downloaded = 30;
    page.stats.skipped = 13;
    app.downloads.push(page);
    let titles = download_titles(&app);
    assert!(
        titles[0].contains("(43%)"),
        "expected (43%) in {:?}",
        titles[0]
    );
}

#[test]
fn completed_stage_shows_checkmark() {
    let mut app = App::new(Config::default());
    let mut page = make_page("col");
    page.download_target = 100;
    page.stats.downloaded = 100;
    page.stage = DownloadStage::Completed;
    app.downloads.push(page);
    let titles = download_titles(&app);
    assert!(titles[0].contains("(✓)"), "expected (✓) in {:?}", titles[0]);
}

#[test]
fn failures_append_star() {
    let mut app = App::new(Config::default());
    let mut page = make_page("col");
    page.download_target = 100;
    page.stats.downloaded = 43;
    page.stats.failed = 2;
    app.downloads.push(page);
    let titles = download_titles(&app);
    assert!(
        titles[0].ends_with('*'),
        "expected title to end with * in {:?}",
        titles[0]
    );
    assert!(
        titles[0].contains("(43%)"),
        "expected (43%) in {:?}",
        titles[0]
    );
}

#[test]
fn completed_with_failures_shows_checkmark_and_star() {
    let mut app = App::new(Config::default());
    let mut page = make_page("col");
    page.download_target = 100;
    page.stats.downloaded = 98;
    page.stats.failed = 2;
    page.stage = DownloadStage::Completed;
    app.downloads.push(page);
    let titles = download_titles(&app);
    assert!(titles[0].contains("(✓)"), "expected (✓) in {:?}", titles[0]);
    assert!(
        titles[0].ends_with('*'),
        "expected * suffix in {:?}",
        titles[0]
    );
}

#[test]
fn zero_target_with_failures_shows_just_star() {
    let mut app = App::new(Config::default());
    let mut page = make_page("col");
    page.download_target = 0;
    page.stats.failed = 1;
    app.downloads.push(page);
    let titles = download_titles(&app);
    assert!(
        titles[0].ends_with('*'),
        "expected * suffix in {:?}",
        titles[0]
    );
    assert!(
        !titles[0].contains('%'),
        "no percent sign expected in {:?}",
        titles[0]
    );
}
