use super::{
    Beatmap, BeatmapRow, BeatmapSetMeta, BeatmapsResponse, Error, Extra, ExtraSet, Genre, Language,
    MAX_BATCH_IDS, PlayedFilter, QueryRange, Rank, RankSet, SearchClient, SearchMode, SearchQuery,
    SearchResults, SearchStatus, SortField, SortOrder, build_query_params, encode_query_string,
};

/// The emitted `q` value for a query (always the first param).
fn q_of(query: &SearchQuery) -> String {
    build_query_params(query)[0].1.clone()
}

/// The emitted value for `key`, or `None` when the param is omitted entirely.
fn value_of(query: &SearchQuery, key: &str) -> Option<String> {
    build_query_params(query)
        .into_iter()
        .find(|(emitted, _)| *emitted == key)
        .map(|(_, value)| value)
}

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
        ..SearchQuery::default()
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

// ── typed q-DSL criteria (byte-for-byte pins) ───────────────────────────────

#[test]
fn range_emits_ge_le_pair() {
    let query = SearchQuery {
        stars: Some(QueryRange::between(5.0, 6.5)),
        ..SearchQuery::default()
    };
    assert_eq!(q_of(&query), "stars>=5 stars<=6.5");
}

#[test]
fn strict_bounds_emit_gt_lt() {
    let lower = SearchQuery {
        stars: Some(QueryRange::greater_than(5.0)),
        ..SearchQuery::default()
    };
    assert_eq!(q_of(&lower), "stars>5");

    let upper = SearchQuery {
        ar: Some(QueryRange::less_than(3.0)),
        ..SearchQuery::default()
    };
    assert_eq!(q_of(&upper), "ar<3");
}

#[test]
fn range_open_lower_and_upper_bounds() {
    let lower = SearchQuery {
        ar: Some(QueryRange::at_least(9.0)),
        ..SearchQuery::default()
    };
    assert_eq!(q_of(&lower), "ar>=9");

    let upper = SearchQuery {
        od: Some(QueryRange::at_most(4.0)),
        ..SearchQuery::default()
    };
    assert_eq!(q_of(&upper), "od<=4");
}

#[test]
fn hp_range_emits_dr_key() {
    let query = SearchQuery {
        hp: Some(QueryRange::between(3.0, 7.0)),
        ..SearchQuery::default()
    };
    // HP drain serializes under the canonical `dr` key, not `hp`.
    assert_eq!(q_of(&query), "dr>=3 dr<=7");
}

#[test]
fn length_range_uses_raw_seconds() {
    let query = SearchQuery {
        length: Some(QueryRange::between(60, 120)),
        ..SearchQuery::default()
    };
    assert_eq!(q_of(&query), "length>=60 length<=120");
}

#[test]
fn keys_and_favourites_ranges() {
    let query = SearchQuery {
        keys: Some(QueryRange::between(4, 7)),
        favourites: Some(QueryRange::at_least(10_000)),
        ..SearchQuery::default()
    };
    assert_eq!(q_of(&query), "keys>=4 keys<=7 favourites>=10000");
}

#[test]
fn ranked_date_range_accepts_partial_dates() {
    let query = SearchQuery {
        ranked: Some(QueryRange::between(
            "2020".to_string(),
            "2024-06".to_string(),
        )),
        ..SearchQuery::default()
    };
    assert_eq!(q_of(&query), "ranked>=2020 ranked<=2024-06");
}

#[test]
fn bare_exact_numeric_emits_equals() {
    let query = SearchQuery {
        stars: Some(QueryRange::Exact(5.0)),
        ..SearchQuery::default()
    };
    // An exact value emits `key=a`; the server applies its tolerance band.
    assert_eq!(q_of(&query), "stars=5");
}

#[test]
fn text_fields_are_always_double_quoted_with_inner_escape() {
    let plain = SearchQuery {
        creator: Some("mrekk".to_string()),
        ..SearchQuery::default()
    };
    assert_eq!(q_of(&plain), "creator=\"mrekk\"");

    let quoted = SearchQuery {
        title: Some("tri\"angle".to_string()),
        ..SearchQuery::default()
    };
    // Inner `"` escapes to `\"`.
    assert_eq!(q_of(&quoted), "title=\"tri\\\"angle\"");
}

#[test]
fn approved_status_emits_q_term_and_any_category() {
    let query = SearchQuery {
        status: Some(SearchStatus::Approved),
        ..SearchQuery::default()
    };
    let params = build_query_params(&query);
    assert_eq!(q_of(&query), "status=approved");
    // The category param is forced to `any` so it never masks the q term.
    assert!(params.contains(&("s", "any".to_string())));
}

#[test]
fn non_approved_status_stays_on_s_param_only() {
    let query = SearchQuery {
        status: Some(SearchStatus::Loved),
        ..SearchQuery::default()
    };
    let params = build_query_params(&query);
    // No status term leaks into `q` for any status other than `approved`.
    assert_eq!(q_of(&query), "");
    assert!(params.contains(&("s", "loved".to_string())));
}

#[test]
fn combined_query_pins_full_fixed_order() {
    let query = SearchQuery {
        text: "tekno".to_string(),
        status: Some(SearchStatus::Approved),
        stars: Some(QueryRange::between(5.0, 6.5)),
        ar: Some(QueryRange::at_least(9.0)),
        cs: Some(QueryRange::at_most(4.0)),
        od: Some(QueryRange::Exact(8.0)),
        hp: Some(QueryRange::between(3.0, 7.0)),
        bpm: Some(QueryRange::between(180.0, 200.0)),
        length: Some(QueryRange::between(60, 120)),
        keys: Some(QueryRange::Exact(7)),
        ranked: Some(QueryRange::between(
            "2020".to_string(),
            "2024-06".to_string(),
        )),
        favourites: Some(QueryRange::at_least(10_000)),
        creator: Some("mrekk".to_string()),
        artist: Some("Camellia".to_string()),
        title: Some("tri\"angle".to_string()),
        ..SearchQuery::default()
    };
    // Fixed order: free text, status=approved, then stars/ar/cs/od/dr/bpm/
    // length/keys/ranked/favourites, then creator/artist/title.
    assert_eq!(
        q_of(&query),
        "tekno status=approved stars>=5 stars<=6.5 ar>=9 cs<=4 od=8 dr>=3 dr<=7 \
         bpm>=180 bpm<=200 length>=60 length<=120 keys=7 ranked>=2020 ranked<=2024-06 \
         favourites>=10000 creator=\"mrekk\" artist=\"Camellia\" title=\"tri\\\"angle\""
    );
}

#[test]
fn all_none_range_emits_nothing() {
    let query = SearchQuery {
        stars: Some(QueryRange::Range {
            min: None,
            max: None,
        }),
        ..SearchQuery::default()
    };
    assert_eq!(q_of(&query), "");
}

#[test]
fn from_bounds_collapses_absent_bounds_to_none() {
    assert_eq!(QueryRange::<f64>::from_bounds(None, None), None);
    assert_eq!(
        QueryRange::from_bounds(Some(1.0), None),
        Some(QueryRange::at_least(1.0))
    );
}

// ── standalone params (g / l / e / nsfw / r / played) ───────────────────────

#[test]
fn genre_params_map_to_probed_ids() {
    for (genre, id) in [
        (Genre::Unspecified, "1"),
        (Genre::VideoGame, "2"),
        (Genre::Anime, "3"),
        (Genre::Rock, "4"),
        (Genre::Pop, "5"),
        (Genre::Other, "6"),
        (Genre::Novelty, "7"),
        (Genre::HipHop, "9"),
        (Genre::Electronic, "10"),
        (Genre::Metal, "11"),
        (Genre::Classical, "12"),
        (Genre::Folk, "13"),
        (Genre::Jazz, "14"),
    ] {
        let query = SearchQuery {
            genre: Some(genre),
            ..SearchQuery::default()
        };
        assert_eq!(value_of(&query, "g").as_deref(), Some(id), "{genre:?}");
    }
}

#[test]
fn genre_ids_skip_the_missing_eight() {
    // osu!'s own numbering has no id 8, so no variant may claim it. These ids are
    // an external fact (probed 2026-07-31) that a refactor could silently shift.
    let ids: Vec<u8> = Genre::ALL
        .iter()
        .map(|genre| genre.as_param().parse().expect("genre id is an integer"))
        .collect();
    assert_eq!(ids, vec![1, 2, 3, 4, 5, 6, 7, 9, 10, 11, 12, 13, 14]);
}

#[test]
fn language_params_map_to_probed_ids() {
    for (language, id) in [
        (Language::Unspecified, "1"),
        (Language::English, "2"),
        (Language::Japanese, "3"),
        (Language::Chinese, "4"),
        (Language::Instrumental, "5"),
        (Language::Korean, "6"),
        (Language::French, "7"),
        (Language::German, "8"),
        (Language::Swedish, "9"),
        (Language::Spanish, "10"),
        (Language::Italian, "11"),
        (Language::Russian, "12"),
        (Language::Polish, "13"),
        (Language::Other, "14"),
    ] {
        let query = SearchQuery {
            language: Some(language),
            ..SearchQuery::default()
        };
        assert_eq!(value_of(&query, "l").as_deref(), Some(id), "{language:?}");
    }
}

#[test]
fn language_ids_run_one_through_fourteen() {
    // Unlike genre, the language numbering is gapless (probed 2026-07-31).
    let ids: Vec<u8> = Language::ALL
        .iter()
        .map(|language| {
            language
                .as_param()
                .parse()
                .expect("language id is an integer")
        })
        .collect();
    assert_eq!(ids, (1..=14).collect::<Vec<u8>>());
}

#[test]
fn extra_set_emits_canonical_dot_joined_order() {
    let video = SearchQuery {
        extra: ExtraSet::new().with(Extra::Video),
        ..SearchQuery::default()
    };
    assert_eq!(value_of(&video, "e").as_deref(), Some("video"));

    let storyboard = SearchQuery {
        extra: ExtraSet::new().with(Extra::Storyboard),
        ..SearchQuery::default()
    };
    assert_eq!(value_of(&storyboard, "e").as_deref(), Some("storyboard"));

    // Insertion order does not leak into the parameter: built storyboard-first,
    // it still emits video first.
    let both = SearchQuery {
        extra: [Extra::Storyboard, Extra::Video].into_iter().collect(),
        ..SearchQuery::default()
    };
    assert_eq!(value_of(&both, "e").as_deref(), Some("video.storyboard"));
}

#[test]
fn extra_set_cannot_repeat_a_member() {
    let query = SearchQuery {
        extra: [Extra::Video, Extra::Video].into_iter().collect(),
        ..SearchQuery::default()
    };
    assert_eq!(value_of(&query, "e").as_deref(), Some("video"));
}

#[test]
fn rank_set_emits_canonical_dot_joined_order() {
    let single = SearchQuery {
        rank: RankSet::new().with(Rank::Sh),
        ..SearchQuery::default()
    };
    assert_eq!(value_of(&single, "r").as_deref(), Some("SH"));

    // Collected worst-rank-first; the emitted order is still best-rank-first.
    let every = SearchQuery {
        rank: [
            Rank::D,
            Rank::C,
            Rank::B,
            Rank::A,
            Rank::S,
            Rank::Sh,
            Rank::X,
            Rank::Xh,
        ]
        .into_iter()
        .collect(),
        ..SearchQuery::default()
    };
    assert_eq!(value_of(&every, "r").as_deref(), Some("XH.X.SH.S.A.B.C.D"));
}

#[test]
fn rank_set_cannot_repeat_a_member() {
    let query = SearchQuery {
        rank: [Rank::S, Rank::S].into_iter().collect(),
        ..SearchQuery::default()
    };
    assert_eq!(value_of(&query, "r").as_deref(), Some("S"));
}

#[test]
fn rank_set_membership_round_trips() {
    let set = RankSet::new().with(Rank::S).with(Rank::A);
    assert!(set.contains(Rank::S));
    assert!(!set.contains(Rank::Xh));
    assert!(!set.is_empty());

    let dropped = set.without(Rank::S);
    assert!(!dropped.contains(Rank::S));
    assert!(dropped.contains(Rank::A));
    assert!(dropped.without(Rank::A).is_empty());
    assert!(ExtraSet::new().is_empty());
}

#[test]
fn empty_sets_emit_no_param() {
    let query = SearchQuery {
        text: "tekno".to_string(),
        extra: ExtraSet::new(),
        rank: RankSet::new(),
        ..SearchQuery::default()
    };
    // The query itself still emits `q`, so an omitted `e`/`r` is the set being
    // empty rather than the builder never running.
    assert_eq!(q_of(&query), "tekno");
    assert_eq!(value_of(&query, "e"), None);
    assert_eq!(value_of(&query, "r"), None);
}

#[test]
fn nsfw_emits_bool_literals() {
    for (flag, expected) in [(true, "true"), (false, "false")] {
        let query = SearchQuery {
            nsfw: Some(flag),
            ..SearchQuery::default()
        };
        assert_eq!(value_of(&query, "nsfw").as_deref(), Some(expected));
    }
}

#[test]
fn played_params_map_to_api_tokens() {
    for (played, expected) in [
        (PlayedFilter::Any, "any"),
        (PlayedFilter::Played, "played"),
        (PlayedFilter::Unplayed, "unplayed"),
    ] {
        let query = SearchQuery {
            played: Some(played),
            ..SearchQuery::default()
        };
        assert_eq!(value_of(&query, "played").as_deref(), Some(expected));
    }
}

#[test]
fn unset_standalone_criteria_emit_no_params() {
    let query = SearchQuery {
        text: "tekno".to_string(),
        mode: Some(SearchMode::Osu),
        sort: Some((SortField::Plays, SortOrder::Desc)),
        ..SearchQuery::default()
    };
    // Other params still emit, so an absent key is the field being unset rather
    // than the builder bailing out early.
    assert_eq!(value_of(&query, "m").as_deref(), Some("0"));
    for key in ["g", "l", "e", "nsfw", "r", "played"] {
        assert_eq!(value_of(&query, key), None, "`{key}` must stay unset");
    }
}

#[test]
fn standalone_criteria_never_enter_q() {
    let query = SearchQuery {
        genre: Some(Genre::Anime),
        language: Some(Language::Japanese),
        extra: ExtraSet::new().with(Extra::Video),
        nsfw: Some(false),
        rank: RankSet::new().with(Rank::Xh),
        played: Some(PlayedFilter::Played),
        ..SearchQuery::default()
    };
    // All six are standalone url params, never q-DSL terms.
    assert_eq!(q_of(&query), "");
}

#[test]
fn standalone_criteria_pin_param_order_and_values() {
    let query = SearchQuery {
        text: "tekno".to_string(),
        mode: Some(SearchMode::Osu),
        status: Some(SearchStatus::Ranked),
        genre: Some(Genre::Anime),
        language: Some(Language::Japanese),
        extra: ExtraSet::new().with(Extra::Video).with(Extra::Storyboard),
        nsfw: Some(false),
        rank: RankSet::new().with(Rank::Xh).with(Rank::S),
        played: Some(PlayedFilter::Unplayed),
        sort: Some((SortField::Plays, SortOrder::Desc)),
        cursor: Some("abc123".to_string()),
        ..SearchQuery::default()
    };
    assert_eq!(
        build_query_params(&query),
        vec![
            ("q", "tekno".to_string()),
            ("m", "0".to_string()),
            ("s", "ranked".to_string()),
            ("g", "3".to_string()),
            ("l", "3".to_string()),
            ("e", "video.storyboard".to_string()),
            ("nsfw", "false".to_string()),
            ("r", "XH.S".to_string()),
            ("played", "unplayed".to_string()),
            ("sort", "plays_desc".to_string()),
            ("cursor_string", "abc123".to_string()),
        ]
    );
}

// ── response deserialization ────────────────────────────────────────────────

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

#[test]
fn deserializes_batch_beatmaps_envelope() {
    // Shape from the live probe (2026-07-11): a `beatmaps` array where each row
    // nests its parent `beatmapset`. `mode_int` is optional.
    let json = r#"{
        "beatmaps": [
            {
                "id": 75,
                "beatmapset_id": 1,
                "mode_int": 0,
                "beatmapset": {
                    "id": 1,
                    "title": "DISCO PRINCE",
                    "artist": "Kenji Ninuma",
                    "creator": "peppy",
                    "status": "ranked",
                    "favourite_count": 42,
                    "play_count": 123456,
                    "nsfw": false,
                    "video": false
                }
            },
            {
                "id": 129891,
                "beatmapset_id": 41823,
                "beatmapset": {
                    "id": 41823,
                    "title": "FREEDOM DiVE",
                    "artist": "xi",
                    "creator": "Nakagawa-Kanon",
                    "status": "ranked"
                }
            }
        ]
    }"#;

    let response: BeatmapsResponse = serde_json::from_str(json).expect("parse batch response");
    assert_eq!(response.beatmaps.len(), 2);

    let first: &BeatmapRow = &response.beatmaps[0];
    assert_eq!(first.beatmap.id, 75);
    assert_eq!(first.beatmap.beatmapset_id, 1);
    assert_eq!(first.beatmap.mode_int, 0);
    assert_eq!(first.beatmapset.title, "DISCO PRINCE");
    assert_eq!(first.beatmapset.creator, "peppy");
    assert_eq!(first.beatmapset.favourite_count, 42);

    // Second row omits `mode_int` and the set's optional count/flag fields; they
    // default rather than failing the parse.
    let second: &BeatmapRow = &response.beatmaps[1];
    assert_eq!(second.beatmap.mode_int, 0);
    assert_eq!(second.beatmapset.id, 41823);
    assert_eq!(second.beatmapset.play_count, 0);
    assert!(!second.beatmapset.video);
}

#[test]
fn deserializes_nested_beatmaps_and_renames_drain_accuracy() {
    // The osu API v2 search response nests a `beatmaps[]` array under each
    // `beatmapsets[]` element. HP drain arrives under the wire key `drain` and
    // overall difficulty under `accuracy` — both renamed onto the `hp`/`od`
    // fields — so this test fails if either rename is dropped or mistyped.
    let json = r#"{
        "id": 7,
        "title": "Song",
        "artist": "Artist",
        "creator": "Mapper",
        "status": "ranked",
        "beatmaps": [
            {
                "id": 100,
                "beatmapset_id": 7,
                "mode_int": 0,
                "version": "Expert",
                "difficulty_rating": 5.47,
                "bpm": 180.0,
                "ar": 9.0,
                "cs": 4.0,
                "drain": 6.0,
                "accuracy": 8.0,
                "total_length": 240,
                "hit_length": 200
            }
        ]
    }"#;

    let set: BeatmapSetMeta = serde_json::from_str(json).expect("parse set with beatmaps");
    assert_eq!(set.beatmaps.len(), 1);
    let diff: &Beatmap = &set.beatmaps[0];
    assert_eq!(diff.id, 100);
    assert_eq!(diff.version, "Expert");
    assert_eq!(diff.difficulty_rating, 5.47);
    assert_eq!(diff.bpm, 180.0);
    assert_eq!(diff.ar, 9.0);
    assert_eq!(diff.cs, 4.0);
    assert_eq!(diff.hp, 6.0, "`drain` wire key must map onto `hp`");
    assert_eq!(diff.od, 8.0, "`accuracy` wire key must map onto `od`");
    assert_eq!(diff.total_length, 240);
    assert_eq!(diff.hit_length, 200);
}

#[test]
fn beatmaps_default_to_empty_when_absent() {
    // A pre-`beatmaps[]` search response (the shape every existing fixture
    // uses) parses with an empty vec rather than failing.
    let json = r#"{ "id": 7, "title": "Song", "artist": "A", "creator": "M", "status": "ranked" }"#;
    let set: BeatmapSetMeta = serde_json::from_str(json).expect("parse set without beatmaps");
    assert!(set.beatmaps.is_empty());
}

// ── batch guards (return before any network I/O) ────────────────────────────

#[tokio::test]
async fn beatmaps_empty_ids_short_circuits() {
    let client = SearchClient::new();
    let rows = client
        .beatmaps("token", &[])
        .await
        .expect("empty ids is a no-op");
    assert!(rows.is_empty());
}

#[tokio::test]
async fn beatmaps_rejects_oversized_batch() {
    let client = SearchClient::new();
    let ids: Vec<u32> = (0..=MAX_BATCH_IDS as u32).collect(); // MAX_BATCH_IDS + 1
    let result = client.beatmaps("token", &ids).await;
    assert!(matches!(result, Err(Error::Config(_))));
}
