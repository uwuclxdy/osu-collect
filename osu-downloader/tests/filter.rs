#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::{
    BeatmapDetails, BeatmapStatus, FilterClient, FilterDirection, FilterMode, FilterQuery,
    FilterRange, FilterResults, FilterSort, FilterSpecial, FilterStatus, build_request,
};
use serde_json::json;
use std::collections::HashMap;

#[test]
fn empty_query_builds_bare_envelope() {
    let body = build_request(&FilterQuery::default());
    assert_eq!(
        body,
        json!({
            "node": {
                "id": "root",
                "group": { "connector": { "type": "AND", "not": [] }, "children": [] }
            },
            "clientId": "osu-collect",
        })
    );
}

#[test]
fn full_query_encodes_every_rule_in_canonical_order() {
    let query = FilterQuery {
        mode: Some(FilterMode::Osu),
        status: Some(FilterStatus::Ranked),
        special: Some(FilterSpecial::Farm),
        stars: FilterRange {
            min: Some(6.5),
            max: Some(9.0),
        },
        bpm: FilterRange {
            min: Some(180.0),
            max: None,
        },
        length: FilterRange {
            min: None,
            max: Some(300.0),
        },
        artist: "camellia".to_string(),
        creator: "sotarks".to_string(),
        title: "ghost".to_string(),
        sort: Some((FilterSort::Stars, FilterDirection::Desc)),
        limit: Some(500),
        ..FilterQuery::default()
    };
    let body = build_request(&query);
    assert_eq!(body["clientId"], "osu-collect");
    assert_eq!(body["limit"], 500);
    assert_eq!(body["by"], "stars");
    assert_eq!(body["direction"], "desc");

    let children = body["node"]["group"]["children"]
        .as_array()
        .expect("children array");
    let rules: Vec<_> = children
        .iter()
        .map(|child| {
            (
                child["rule"]["field"].as_str().expect("field"),
                child["rule"]["operator"].as_str().expect("operator"),
                child["rule"]["value"].as_str().expect("value"),
                child["rule"]["type"].as_str().expect("type"),
            )
        })
        .collect();
    assert_eq!(
        rules,
        vec![
            ("Mode", "=", "osu!", "Text"),
            ("Approved", "=", "ranked", "Text"),
            ("Farm", "=", "1", "Numeric"),
            ("Stars", ">=", "6.5", "Numeric"),
            ("Stars", "<=", "9", "Numeric"),
            ("Bpm", ">=", "180", "Numeric"),
            ("TotalLength", "<=", "300", "Numeric"),
            ("Artist", "like", "camellia", "Text"),
            ("Creator", "like", "sotarks", "Text"),
            ("Title", "like", "ghost", "Text"),
        ]
    );

    let ids: Vec<_> = children
        .iter()
        .map(|child| child["id"].as_str().expect("id").to_string())
        .collect();
    assert_eq!(ids, (1..=10).map(|i| i.to_string()).collect::<Vec<_>>());
}

#[test]
fn special_rewrites_to_flag_column() {
    // The hosted server has no `Special` column; BBD rewrites the rule to the
    // flag column with value "1" before sending. Verified live 2026-07-08.
    for (special, field) in [
        (FilterSpecial::Farm, "Farm"),
        (FilterSpecial::Stream, "Stream"),
        (FilterSpecial::RankedMapper, "RankedMapper"),
    ] {
        let query = FilterQuery {
            special: Some(special),
            ..FilterQuery::default()
        };
        let body = build_request(&query);
        let rule = &body["node"]["group"]["children"][0]["rule"];
        assert_eq!(rule["field"], field);
        assert_eq!(rule["operator"], "=");
        assert_eq!(rule["value"], "1");
        assert_eq!(rule["type"], "Numeric");
    }
}

#[test]
fn status_macros_pass_through_server_keywords() {
    for (status, expected) in [
        (FilterStatus::Leaderboard, "HasLeaderboard"),
        (FilterStatus::Unranked, "unranked"),
        (FilterStatus::Wip, "WIP"),
        (FilterStatus::Ranked, "ranked"),
        (FilterStatus::Loved, "loved"),
        (FilterStatus::Approved, "approved"),
        (FilterStatus::Pending, "pending"),
        (FilterStatus::Graveyard, "graveyard"),
    ] {
        let query = FilterQuery {
            status: Some(status),
            ..FilterQuery::default()
        };
        let rule = &build_request(&query)["node"]["group"]["children"][0]["rule"];
        assert_eq!(rule["field"], "Approved");
        assert_eq!(rule["value"], expected);
    }
}

#[test]
fn mode_values_match_bbd_labels() {
    for (mode, expected) in [
        (FilterMode::Osu, "osu!"),
        (FilterMode::Taiko, "Taiko"),
        (FilterMode::Catch, "Catch the Beat"),
        (FilterMode::Mania, "osu!mania"),
    ] {
        let query = FilterQuery {
            mode: Some(mode),
            ..FilterQuery::default()
        };
        let rule = &build_request(&query)["node"]["group"]["children"][0]["rule"];
        assert_eq!(rule["field"], "Mode");
        assert_eq!(rule["value"], expected);
    }
}

#[test]
fn text_rules_send_raw_value_for_server_side_wrapping() {
    // The server wraps `like` values in %...% itself (filter.go); sending a
    // pre-wrapped value would double-wrap.
    let query = FilterQuery {
        artist: "a b&c".to_string(),
        ..FilterQuery::default()
    };
    let rule = &build_request(&query)["node"]["group"]["children"][0]["rule"];
    assert_eq!(rule["operator"], "like");
    assert_eq!(rule["value"], "a b&c");
}

#[test]
fn deserializes_filter_response() {
    // SizeMap arrives keyed by set id as JSON-object string keys.
    let json = r#"{
        "Ids": [61843, 61995],
        "SetIds": [17331],
        "SizeMap": { "17331": 7980194 },
        "Hashes": ["415a9a62daea25ff866807aec7426246"],
        "Id": "metrics-uuid-ignored"
    }"#;
    let results: FilterResults = serde_json::from_str(json).expect("parse filter response");
    assert_eq!(results.ids, vec![61843, 61995]);
    assert_eq!(results.set_ids, vec![17331]);
    assert_eq!(results.size_map.get(&17331), Some(&7_980_194));
    assert_eq!(results.hashes, vec!["415a9a62daea25ff866807aec7426246"]);
}

#[test]
fn deserializes_response_missing_size_map_and_hashes() {
    // Defensive default: a response omitting the (empty) map/array shapes
    // still parses instead of failing the whole run.
    let json = r#"{ "Ids": [], "SetIds": [] }"#;
    let results: FilterResults = serde_json::from_str(json).expect("parse sparse response");
    assert!(results.ids.is_empty());
    assert!(results.set_ids.is_empty());
    assert!(results.size_map.is_empty());
    assert!(results.hashes.is_empty());
}

#[test]
fn deserializes_beatmap_details_row() {
    // Captured live 2026-07-08, trimmed to the consumed fields; the real
    // response carries extras (TimingPoints, Tags, ...) that serde must ignore.
    let json = r#"[{
        "Title": "I Did It For Love", "Artist": "BoA", "Creator": "Lena",
        "Version": "Hard", "Hp": 8, "Cs": 4, "Od": 8, "Ar": 8,
        "ApprovedDate": 1277585173000, "Approved": "ranked", "Bpm": 130.6,
        "Id": 61843, "SetId": 17331, "Stars": 3.79, "FavouriteCount": 234,
        "Mode": "osu", "TotalLength": 148, "PlayCount": 1040818,
        "Size": 7980194, "TimingPoints": ""
    }]"#;
    let rows: Vec<BeatmapDetails> = serde_json::from_str(json).expect("parse details response");
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.id, 61843);
    assert_eq!(row.set_id, 17331);
    assert_eq!(row.title, "I Did It For Love");
    assert_eq!(row.artist, "BoA");
    assert_eq!(row.creator, "Lena");
    assert_eq!(row.version, "Hard");
    assert_eq!(row.stars, 3.79);
    assert_eq!(row.bpm, 130.6);
    assert_eq!(row.ar, 8.0);
    assert_eq!(row.cs, 4.0);
    assert_eq!(row.od, 8.0);
    assert_eq!(row.hp, 8.0);
    assert_eq!(row.status, Some(BeatmapStatus::Ranked));
    assert_eq!(row.mode, Some(FilterMode::Osu));
    assert_eq!(row.total_length, 148);
    assert_eq!(row.favourite_count, 234);
    assert_eq!(row.play_count, 1_040_818);
    assert_eq!(row.size, 7_980_194);
    assert_eq!(row.approved_date, 1_277_585_173_000);
}

#[test]
fn deserializes_every_field_of_a_live_fixture_row_exactly() {
    // Full 30-column row captured live 2026-07-14 (osu-downloader/tests/fixtures/beatmap_details.json).
    // This is the drift alarm: a server column rename/retype must fail here.
    let raw = include_str!("fixtures/beatmap_details.json");
    let rows: Vec<BeatmapDetails> = serde_json::from_str(raw).expect("parse fixture");
    assert_eq!(rows.len(), 13, "fixture row count changed");

    let row = rows
        .iter()
        .find(|row| row.id == 24_722)
        .expect("fixture row 24722 present");
    assert_eq!(row.set_id, 4_651);
    assert_eq!(row.title, "BARUSA of MIKOSU");
    assert_eq!(row.artist, "Nico Nico Douga");
    assert_eq!(row.creator, "DJPop");
    assert_eq!(row.version, "TAG4");
    assert_eq!(row.stars, 9.36);
    assert_eq!(row.bpm, 180.0);
    assert_eq!(row.ar, 8.0);
    assert_eq!(row.cs, 5.0);
    assert_eq!(row.od, 8.0);
    assert_eq!(row.hp, 3.0);
    assert_eq!(row.status, Some(BeatmapStatus::Approved));
    assert_eq!(row.mode, Some(FilterMode::Osu));
    assert_eq!(row.total_length, 181);
    assert_eq!(row.favourite_count, 1_627);
    assert_eq!(row.play_count, 1_088_174);
    assert_eq!(row.size, 20_282_348);
    assert_eq!(row.hash, "15f8cc75cee3f3752c562ef069fe1270");
    assert_eq!(
        row.tags,
        "barusa of mikosu nico douga lucky star hiiragi tsukasa mad night of nights beatmario"
    );
    assert_eq!(row.source, "Lucky Star");
    assert_eq!(row.genre, "novelty");
    assert_eq!(row.language, "Japanese");
    assert_eq!(row.max_combo, 1_246);
    assert_eq!(row.hit_length, 171);
    assert_eq!(row.pass_count, 191_891);
    // ApprovedDate is epoch MILLIseconds; LastUpdate is epoch SECONDS (a
    // 10-digit value here, not 13) — confirmed against every fixture row.
    assert_eq!(row.approved_date, 1_234_105_108_000);
    assert_eq!(row.last_update, 1_234_107_731);
}

#[test]
fn all_eight_live_mode_spellings_map_to_the_right_filter_mode() {
    let raw = include_str!("fixtures/beatmap_details.json");
    let rows: Vec<BeatmapDetails> = serde_json::from_str(raw).expect("parse fixture");

    // One exemplar id per distinct wire spelling in the fixture.
    let expected: HashMap<u32, FilterMode> = HashMap::from([
        (24_722, FilterMode::Osu),    // "osu"
        (107_434, FilterMode::Osu),   // "osu!"
        (51_025, FilterMode::Taiko),  // "taiko"
        (108_914, FilterMode::Taiko), // "Taiko"
        (74_845, FilterMode::Catch),  // "fruits"
        (449_201, FilterMode::Catch), // "Catch the Beat"
        (210_180, FilterMode::Mania), // "mania"
        (703_125, FilterMode::Mania), // "osu!mania"
    ]);
    assert_eq!(expected.len(), 8, "test must cover all 8 live spellings");

    for (id, want) in &expected {
        let row = rows
            .iter()
            .find(|row| row.id == *id)
            .unwrap_or_else(|| panic!("fixture missing id {id}"));
        assert_eq!(row.mode, Some(*want), "id {id} mode mismatch");
    }
}

#[test]
fn all_six_live_status_values_map_to_the_right_beatmap_status() {
    let raw = include_str!("fixtures/beatmap_details.json");
    let rows: Vec<BeatmapDetails> = serde_json::from_str(raw).expect("parse fixture");

    // One exemplar id per distinct `Approved` value in the fixture.
    let expected: HashMap<u32, BeatmapStatus> = HashMap::from([
        (24_722, BeatmapStatus::Approved),
        (29_757, BeatmapStatus::Graveyard),
        (39_076, BeatmapStatus::Ranked),
        (47_359, BeatmapStatus::Loved),
        (188_836, BeatmapStatus::Pending),
        (694_305, BeatmapStatus::Wip),
    ]);
    assert_eq!(expected.len(), 6, "test must cover all 6 live statuses");

    for (id, want) in &expected {
        let row = rows
            .iter()
            .find(|row| row.id == *id)
            .unwrap_or_else(|| panic!("fixture missing id {id}"));
        assert_eq!(row.status, Some(*want), "id {id} status mismatch");
    }
}

#[test]
fn unrecognized_mode_and_status_spellings_deserialize_to_none_without_erroring() {
    // A hard enum here would let one unfamiliar row poison the whole batch
    // parse — verify the row still parses with `None` in both fields.
    let json = r#"[{
        "Id": 1, "SetId": 2, "Title": "t", "Artist": "a", "Creator": "c",
        "Version": "v", "Stars": 1.0, "Bpm": 1.0, "Ar": 1.0, "Cs": 1.0,
        "Od": 1.0, "Hp": 1.0, "Approved": "onion", "Mode": "quaver",
        "TotalLength": 1, "FavouriteCount": 0, "PlayCount": 0, "Size": 0
    }]"#;
    let rows: Vec<BeatmapDetails> =
        serde_json::from_str(json).expect("unrecognized spellings must not error the whole row");
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.mode, None);
    assert_eq!(row.status, None);
    // The rest of the row still lands despite the unrecognized enum values.
    assert_eq!(row.id, 1);
    assert_eq!(row.title, "t");
    assert_eq!(row.total_length, 1);
}

#[tokio::test]
async fn details_with_empty_slice_returns_empty_without_a_request() {
    // No network call happens here: the empty-slice guard returns before
    // `post_json` is reached, so this is safe to run unignored.
    let client = FilterClient::new();
    let details = client
        .details(&[])
        .await
        .expect("empty slice must not error");
    assert!(details.is_empty());
}

#[tokio::test]
#[ignore = "network: hits v2.nzbasic.com"]
async fn live_farm_filter_returns_sets_and_details() {
    let client = FilterClient::new();
    let query = FilterQuery {
        special: Some(FilterSpecial::Farm),
        limit: Some(10),
        ..FilterQuery::default()
    };
    let results = client.fetch(&query).await.expect("filter fetch");
    assert!(!results.set_ids.is_empty(), "farm filter returned no sets");
    assert!(!results.size_map.is_empty(), "size map empty");

    let slice = &results.ids[..results.ids.len().min(3)];
    let details = client.details(slice).await.expect("details fetch");
    assert_eq!(details.len(), slice.len());
    assert!(details.iter().all(|row| row.set_id > 0));
}
