use crate::{Error, Mirror, MirrorKind};

#[test]
fn builder_requires_at_least_one_mirror() {
    assert!(crate::Downloader::builder().build().is_err());
}

#[test]
fn builder_rejects_zero_concurrency() {
    let result = crate::Downloader::builder()
        .mirror(Mirror::nerinyan())
        .concurrent_downloads(0)
        .build();
    assert!(matches!(result, Err(Error::Config(_))));
}

#[test]
fn default_mirrors_include_every_builtin_mirror() {
    let downloader = crate::Downloader::builder().builtins().build().unwrap();

    let mirror_kinds: Vec<_> = downloader.mirrors().iter().map(Mirror::kind).collect();
    assert_eq!(
        mirror_kinds,
        vec![
            MirrorKind::Nerinyan,
            MirrorKind::OsuDirect,
            MirrorKind::Sayobot,
            MirrorKind::Nekoha,
            MirrorKind::Beatconnect,
            MirrorKind::Osudl,
            MirrorKind::Catboy,
            MirrorKind::Hinamizawa,
            MirrorKind::OsuApi,
            MirrorKind::Nzbasic,
        ]
    );
}

#[test]
fn per_mirror_no_video_switches_template() {
    let downloader = crate::Downloader::builder()
        .mirror(Mirror::nerinyan().no_video())
        .mirror(
            Mirror::custom("https://example.com/d/{id}")
                .unwrap()
                .no_video(),
        )
        .build()
        .unwrap();

    let mirrors = downloader.mirrors();
    assert_eq!(
        mirrors[0].url_for(123),
        "https://api.nerinyan.moe/d/123?nv=1"
    );
    // Custom mirrors don't have a no-video variant; their template stays as-is.
    assert_eq!(mirrors[1].url_for(123), "https://example.com/d/123");
}
