use crate::{Mirror, MirrorKind};

#[test]
fn mirror_templates() {
    assert_eq!(
        Mirror::nerinyan().url_for(123),
        "https://api.nerinyan.moe/d/123"
    );
    assert_eq!(
        Mirror::osu_direct().url_for(789),
        "https://osu.direct/d/789"
    );
    assert_eq!(
        Mirror::sayobot().url_for(42),
        "https://dl.sayobot.cn/beatmaps/download/full/42"
    );
    assert_eq!(
        Mirror::nekoha().url_for(1),
        "https://mirror.nekoha.moe/api4/download/1"
    );
    assert_eq!(
        Mirror::beatconnect().url_for(320118),
        "https://beatconnect.io/b/320118/"
    );
    assert_eq!(
        Mirror::osudl().url_for(320118),
        "https://osudl.org/s/320118"
    );
    assert_eq!(
        Mirror::catboy().url_for(320118),
        "https://catboy.best/d/320118"
    );
    assert_eq!(
        Mirror::hinamizawa().url_for(320118),
        "https://mirror.hinamizawa.ai/api/v1/hinai/d/320118"
    );
    assert_eq!(
        Mirror::osu_api().url_for(320118),
        "https://osu.ppy.sh/api/v2/beatmapsets/320118/download"
    );
    assert_eq!(
        Mirror::nzbasic().url_for(320118),
        "https://direct.nzbasic.com/320118.osz"
    );
}

#[test]
fn no_video_templates_for_new_mirrors() {
    assert_eq!(
        Mirror::beatconnect().no_video().url_for(42),
        "https://beatconnect.io/b/42/?novideo=1"
    );
    assert_eq!(
        Mirror::osudl().no_video().url_for(42),
        "https://osudl.org/s/42?video=false"
    );
    // catboy.best strips video with the trailing `n` suffix.
    assert_eq!(
        Mirror::catboy().no_video().url_for(42),
        "https://catboy.best/d/42n"
    );
    assert_eq!(
        Mirror::hinamizawa().no_video().url_for(42),
        "https://mirror.hinamizawa.ai/api/v1/hinai/d/42?no_video=true"
    );
    // osu! official no-video appends the verified `?noVideo=1` query param.
    assert_eq!(
        Mirror::osu_api().no_video().url_for(42),
        "https://osu.ppy.sh/api/v2/beatmapsets/42/download?noVideo=1"
    );
    // nzbasic's CDN has no no-video variant; both templates match.
    assert_eq!(
        Mirror::nzbasic().no_video().url_for(42),
        "https://direct.nzbasic.com/42.osz"
    );
}

#[test]
fn only_osu_api_requires_auth() {
    assert!(MirrorKind::OsuApi.requires_auth());
    for kind in [
        MirrorKind::Nerinyan,
        MirrorKind::OsuDirect,
        MirrorKind::Sayobot,
        MirrorKind::Nekoha,
        MirrorKind::Beatconnect,
        MirrorKind::Osudl,
        MirrorKind::Catboy,
        MirrorKind::Hinamizawa,
        MirrorKind::Nzbasic,
        MirrorKind::Custom,
    ] {
        assert!(!kind.requires_auth(), "{kind:?} must download anonymously");
    }
}

#[test]
fn osu_api_is_last_builtin_and_nzbasic_is_the_only_envelope_mirror() {
    assert_eq!(MirrorKind::BUILTINS.last(), Some(&MirrorKind::OsuApi));
    assert_eq!(MirrorKind::Nzbasic.host(), "direct.nzbasic.com");
    for kind in MirrorKind::BUILTINS.iter().copied() {
        assert_eq!(
            kind.multipart_envelope(),
            kind == MirrorKind::Nzbasic,
            "only nzbasic serves a multipart envelope: {kind:?}"
        );
    }
}

#[test]
fn url_for_edge_ids() {
    // id=0
    assert_eq!(
        Mirror::nerinyan().url_for(0),
        "https://api.nerinyan.moe/d/0"
    );
    assert_eq!(Mirror::osu_direct().url_for(0), "https://osu.direct/d/0");
    // id=u32::MAX
    assert_eq!(
        Mirror::nerinyan().url_for(u32::MAX),
        format!("https://api.nerinyan.moe/d/{}", u32::MAX)
    );
    assert_eq!(
        Mirror::osu_direct().url_for(u32::MAX),
        format!("https://osu.direct/d/{}", u32::MAX)
    );
    // all builtin kinds via Mirror::builtin
    for kind in MirrorKind::BUILTINS.iter().copied() {
        let mirror = Mirror::builtin(kind).expect("builtin mirror exists");
        let url = mirror.url_for(1);
        assert!(
            url.contains('1'),
            "url_for(1) must contain '1' for {kind:?}: {url}"
        );
        assert!(
            !url.contains("{id}"),
            "url must not contain literal '{{id}}' for {kind:?}"
        );
    }
}

#[test]
fn no_video_switches_template_when_supported() {
    assert_eq!(
        Mirror::nerinyan().no_video().url_for(42),
        "https://api.nerinyan.moe/d/42?nv=1"
    );
}

#[test]
fn no_video_is_noop_for_custom_mirrors() {
    let mirror = Mirror::custom("https://example.com/dl/{id}")
        .unwrap()
        .no_video();
    assert_eq!(mirror.url_for(123), "https://example.com/dl/123");
}

#[test]
fn custom_mirror() {
    let mirror = Mirror::custom("https://example.com/dl/{id}").unwrap();
    assert_eq!(mirror.url_for(123), "https://example.com/dl/123");
}

#[test]
fn invalid_custom_mirror() {
    assert!(Mirror::custom("https://example.com/dl/").is_err());
    assert!(Mirror::custom("ftp://example.com/{id}").is_err());
}

#[test]
fn custom_mirror_host_is_parsed_for_display() {
    let mirror = Mirror::custom("https://mirror.example.com:8443/d/{id}?nv=1").unwrap();
    assert_eq!(mirror.host(), "mirror.example.com");
    let mref = mirror.mirror_ref();
    assert_eq!(mref.kind, MirrorKind::Custom);
    // A custom mirror's label is its host, so two customs are distinguishable.
    assert_eq!(mref.label(), "mirror.example.com");
}

#[test]
fn custom_mirror_host_handles_ipv6_literal() {
    let mirror = Mirror::custom("https://[2001:db8::1]:8080/d/{id}").unwrap();
    assert_eq!(mirror.host(), "[2001:db8::1]");
}

#[test]
fn builtin_mirror_ref_uses_kind_label() {
    let mref = Mirror::osu_direct().mirror_ref();
    assert_eq!(mref.kind, MirrorKind::OsuDirect);
    assert_eq!(mref.label(), MirrorKind::OsuDirect.label());
    assert_eq!(mref.host.as_ref(), MirrorKind::OsuDirect.host());
}

#[test]
fn custom_mirror_rejects_multiple_id_placeholders() {
    // a template with two `{id}` placeholders would silently leave the second one
    // un-substituted (split_once stops at the first match); rejecting at construction
    // gives operators an early error instead of a malformed URL at request time.
    assert!(Mirror::custom("https://example.com/{id}/folder/{id}.osz").is_err());
}

#[test]
fn validate_template_accepts_valid() {
    assert!(Mirror::validate_template("https://example.com/dl/{id}").is_ok());
    assert!(Mirror::validate_template("http://example.com/{id}").is_ok());
}

#[test]
fn validate_template_rejects_missing_placeholder() {
    assert!(Mirror::validate_template("https://example.com/dl/").is_err());
}

#[test]
fn validate_template_rejects_multiple_placeholders() {
    assert!(Mirror::validate_template("https://example.com/{id}/folder/{id}.osz").is_err());
}

#[test]
fn validate_template_rejects_non_http_scheme() {
    assert!(Mirror::validate_template("ftp://example.com/{id}").is_err());
}

#[test]
fn validate_template_matches_custom_constructor() {
    // validate_template and Mirror::custom must agree on every case.
    let cases = [
        "https://example.com/dl/{id}",
        "http://example.com/{id}",
        "https://example.com/dl/",
        "ftp://example.com/{id}",
        "https://example.com/{id}/folder/{id}.osz",
        "",
    ];
    for template in cases {
        assert_eq!(
            Mirror::validate_template(template).is_ok(),
            Mirror::custom(template).is_ok(),
            "validate_template and Mirror::custom disagree for {template:?}"
        );
    }
}
