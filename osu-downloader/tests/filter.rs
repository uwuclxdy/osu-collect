use super::{
    BeatmapDetails, FilterClient, FilterDirection, FilterMode, FilterQuery, FilterRange,
    FilterResults, FilterSort, FilterSpecial, FilterStatus, build_request,
};
use serde_json::json;

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
    assert_eq!(row.approved, "ranked");
    assert_eq!(row.mode, "osu");
    assert_eq!(row.total_length, 148);
    assert_eq!(row.favourite_count, 234);
    assert_eq!(row.play_count, 1_040_818);
    assert_eq!(row.size, 7_980_194);
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
