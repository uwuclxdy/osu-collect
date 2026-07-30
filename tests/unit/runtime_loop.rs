//! The batch the runtime loop applies between two frames.
//!
//! `run()` renders once per BATCH rather than once per event, so the drain that
//! collects the batch has to reach every queued event exactly once and keep each
//! channel's own order. The render half needs a real terminal (`DefaultTerminal`
//! is backend-concrete), so it is not covered here.

use super::*;

/// A short label per event, so a drained batch compares as a sequence.
fn tag(event: &LoopEvent) -> String {
    match event {
        LoopEvent::Input(InputEvent::Key(key)) => format!("key {:?}", key.code),
        LoopEvent::Input(InputEvent::Paste(text)) => format!("paste {text}"),
        LoopEvent::Input(InputEvent::Resize) => "resize".to_string(),
        LoopEvent::Input(InputEvent::Tick) => "tick".to_string(),
        LoopEvent::HomeCover(HomeCoverEvent::Missing { set_id }) => format!("cover {set_id}"),
        LoopEvent::HomeCover(HomeCoverEvent::Loaded { set_id, .. }) => {
            format!("cover-loaded {set_id}")
        }
        _ => "other".to_string(),
    }
}

fn press(code: KeyCode) -> InputEvent {
    InputEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

#[test]
fn the_batch_keeps_every_queued_input_in_order() {
    let (tx, mut input_rx) = mpsc::unbounded_channel::<InputEvent>();
    for event in [
        press(KeyCode::Down),
        press(KeyCode::Down),
        InputEvent::Tick,
        press(KeyCode::Up),
        InputEvent::Resize,
    ] {
        tx.send(event).expect("receiver is live");
    }

    let mut drained = Vec::new();
    while let Some(event) = next_queued!(input_rx => Input) {
        drained.push(tag(&event));
    }

    assert_eq!(
        drained,
        [
            "key Down".to_string(),
            "key Down".to_string(),
            "tick".to_string(),
            "key Up".to_string(),
            "resize".to_string(),
        ],
        "a coalesced batch drops nothing and reorders nothing"
    );
}

#[test]
fn the_batch_reaches_every_channel() {
    let (input_tx, mut input_rx) = mpsc::unbounded_channel::<InputEvent>();
    let (cover_tx, mut cover_rx) = mpsc::unbounded_channel::<HomeCoverEvent>();
    input_tx.send(press(KeyCode::Down)).expect("live");
    cover_tx
        .send(HomeCoverEvent::Missing { set_id: 7 })
        .expect("live");
    input_tx.send(InputEvent::Tick).expect("live");

    let mut drained = Vec::new();
    while let Some(event) = next_queued!(input_rx => Input, cover_rx => HomeCover) {
        drained.push(tag(&event));
    }
    drained.sort();

    assert_eq!(
        drained,
        [
            "cover 7".to_string(),
            "key Down".to_string(),
            "tick".to_string()
        ],
        "a later channel's event must not consume-and-discard an earlier one's"
    );
}

#[test]
fn an_empty_set_of_channels_ends_the_batch() {
    let (_input_tx, mut input_rx) = mpsc::unbounded_channel::<InputEvent>();
    let (_cover_tx, mut cover_rx) = mpsc::unbounded_channel::<HomeCoverEvent>();
    assert!(
        next_queued!(input_rx => Input, cover_rx => HomeCover).is_none(),
        "nothing queued is what returns the loop to the render"
    );
}
