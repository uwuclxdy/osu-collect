//! Cover-picker protocol tests: the konsole halfblocks-floor promotion. Pure —
//! `konsole_promotion` is the decision split out of `query_cover_picker` so it
//! can be exercised without a terminal to query.

use super::*;

#[test]
fn konsole_on_the_halfblocks_floor_promotes_to_iterm2() {
    assert_eq!(
        konsole_promotion(ProtocolType::Halfblocks, true),
        Some(ProtocolType::Iterm2),
        "konsole speaks iTerm2 inline images; ratatui-image's blacklist strands \
         it on halfblocks, so the floor must be raised"
    );
}

#[test]
fn a_detected_protocol_is_never_overridden() {
    // The promotion is a floor-raise: anything the terminal actually answered
    // for outranks it, so lifting the upstream blacklist retires this silently.
    for detected in [
        ProtocolType::Sixel,
        ProtocolType::Kitty,
        ProtocolType::Iterm2,
    ] {
        assert_eq!(
            konsole_promotion(detected, true),
            None,
            "{detected:?} was detected and must survive the konsole promotion"
        );
    }
}

#[test]
fn other_terminals_keep_their_halfblocks_fallback() {
    assert_eq!(
        konsole_promotion(ProtocolType::Halfblocks, false),
        None,
        "a non-konsole terminal on halfblocks genuinely has no graphics protocol"
    );
}
