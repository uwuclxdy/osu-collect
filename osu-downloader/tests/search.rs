use super::{
    BeatmapSetMeta, SearchMode, SearchQuery, SearchResults, SearchStatus, SortField, SortOrder,
    build_query_params, encode_query_string,
};

#[test]
fn empty_query_still_sends_q() {
    let params = build_query_params(&SearchQuery::default());
    assert_eq!(params, vec![("q", String::new())]);
}

#[test]
fn full_query_encodes_every_param() {
    let query = SearchQuery {
        text: "tekno".to_string(),
        mode: Some(SearchMode::Mania),
        status: Some(SearchStatus::Loved),
        sort: Some((SortField::Plays, SortOrder::Asc)),
        cursor: Some("abc123".to_string()),
    };
    assert_eq!(
        build_query_params(&query),
        vec![
            ("q", "tekno".to_string()),
            ("m", "3".to_string()),
            ("s", "loved".to_string()),
            ("sort", "plays_asc".to_string()),
            ("cursor_string", "abc123".to_string()),
        ]
    );
}

#[test]
fn sort_encodes_field_then_order() {
    let query = SearchQuery {
        sort: Some((SortField::Difficulty, SortOrder::Desc)),
        ..SearchQuery::default()
    };
    let params = build_query_params(&query);
    assert!(params.contains(&("sort", "difficulty_desc".to_string())));
}

#[test]
fn mode_params_map_to_osu_ints() {
    for (mode, expected) in [
        (SearchMode::Osu, "0"),
        (SearchMode::Taiko, "1"),
        (SearchMode::Fruits, "2"),
        (SearchMode::Mania, "3"),
    ] {
        let query = SearchQuery {
            mode: Some(mode),
            ..SearchQuery::default()
        };
        assert!(build_query_params(&query).contains(&("m", expected.to_string())));
    }
}

#[test]
fn query_string_percent_encodes_free_text() {
    let query = SearchQuery {
        text: "a b&c".to_string(),
        ..SearchQuery::default()
    };
    // space -> %20, ampersand -> %26, so the literal text never breaks param
    // boundaries.
    assert_eq!(
        encode_query_string(&build_query_params(&query)),
        "q=a%20b%26c"
    );
}

#[test]
fn deserializes_search_response_fields() {
    // A compact envelope shaped like a real osu! API v2 response. The test fails
    // if a consumed field is renamed away.
    let json = r#"{
        "beatmapsets": [
            {
                "id": 42,
                "title": "Title",
                "artist": "Artist",
                "creator": "Mapper",
                "status": "ranked",
                "favourite_count": 1200,
                "play_count": 9000000,
                "nsfw": false,
                "video": true
            }
        ],
        "total": 137,
        "cursor_string": "next-page"
    }"#;

    let results: SearchResults = serde_json::from_str(json).expect("parse search response");
    assert_eq!(results.total, 137);
    assert_eq!(results.cursor_string.as_deref(), Some("next-page"));
    assert_eq!(results.beatmapsets.len(), 1);
    let set: &BeatmapSetMeta = &results.beatmapsets[0];
    assert_eq!(set.id, 42);
    assert_eq!(set.title, "Title");
    assert_eq!(set.artist, "Artist");
    assert_eq!(set.creator, "Mapper");
    assert_eq!(set.status, "ranked");
    assert_eq!(set.favourite_count, 1200);
    assert_eq!(set.play_count, 9_000_000);
    assert!(!set.nsfw);
    assert!(set.video);
}

#[test]
fn null_cursor_marks_last_page() {
    let json = r#"{ "beatmapsets": [], "total": 0, "cursor_string": null }"#;
    let results: SearchResults = serde_json::from_str(json).expect("parse empty response");
    assert!(results.beatmapsets.is_empty());
    assert_eq!(results.cursor_string, None);
}
