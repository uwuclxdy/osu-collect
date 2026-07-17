//! Cover-picker protocol tests: which graphics protocol a session is forced to
//! when `ratatui-image`'s env-keyed blacklist strands detection on halfblocks.
//! Pure — `classify_host` / `resolve_outer` / `protocol_override` are the
//! decisions split out of `query_cover_picker` so they can be exercised without
//! a terminal to query, and without mutating the process environment.

use super::*;

/// What konsole reports; it is indistinguishable from a plain xterm by TERM.
const KONSOLE_TERM: &str = "xterm-256color";

#[test]
fn konsole_on_the_halfblocks_floor_promotes_to_iterm2() {
    let outer = resolve_outer(Some(KONSOLE_TERM), true, Host::Direct);
    assert_eq!(outer, Outer::Iterm2Only);
    assert_eq!(
        protocol_override(ProtocolType::Halfblocks, outer),
        Some(ProtocolType::Iterm2),
        "konsole speaks iTerm2 inline images; ratatui-image's blacklist strands \
         it on halfblocks, so the floor must be raised"
    );
}

#[test]
fn a_detected_protocol_is_never_overridden() {
    // The override is a floor-raise: anything the terminal actually answered
    // for outranks it, so lifting the upstream blacklist retires this silently.
    for detected in [
        ProtocolType::Sixel,
        ProtocolType::Kitty,
        ProtocolType::Iterm2,
    ] {
        for outer in [Outer::KittyPlaceholders, Outer::Iterm2Only, Outer::Unknown] {
            assert_eq!(
                protocol_override(detected, outer),
                None,
                "{detected:?} was detected and must survive the {outer:?} override"
            );
        }
    }
}

#[test]
fn other_terminals_keep_their_halfblocks_fallback() {
    let outer = resolve_outer(Some("xterm-256color"), false, Host::Direct);
    assert_eq!(outer, Outer::Unknown);
    assert_eq!(
        protocol_override(ProtocolType::Halfblocks, outer),
        None,
        "a non-konsole terminal on halfblocks genuinely has no graphics protocol"
    );
}

#[test]
fn konsole_inside_a_multiplexer_keeps_halfblocks() {
    // The reported bug: KONSOLE_VERSION is inherited by every tmux pane, so the
    // promotion used to fire there and hand out iTerm2 — which has no cell
    // anchoring, so tmux repaints over it and the cover vanishes entirely.
    // Halfblocks is a mosaic, but it is a mosaic the user can see.
    let outer = resolve_outer(Some(KONSOLE_TERM), true, Host::Passthrough);
    assert_eq!(outer, Outer::Unknown);
    assert_eq!(
        protocol_override(ProtocolType::Halfblocks, outer),
        None,
        "an inherited KONSOLE_VERSION says nothing about the attached client"
    );
}

#[test]
fn a_kitty_client_outranks_a_stale_konsole_env() {
    // tmux reports the real client, so a server started from konsole and
    // attached from kitty still gets the protocol kitty can actually paint.
    for host in [Host::Direct, Host::Passthrough] {
        let outer = resolve_outer(Some("xterm-kitty"), true, host);
        assert_eq!(outer, Outer::KittyPlaceholders, "{host:?}");
        assert_eq!(
            protocol_override(ProtocolType::Halfblocks, outer),
            Some(ProtocolType::Kitty),
            "{host:?}: kitty placeholders anchor to text cells and survive a redraw"
        );
    }
}

#[test]
fn ghostty_counts_as_kitty_placeholders() {
    for termname in ["xterm-ghostty", "ghostty"] {
        assert_eq!(
            resolve_outer(Some(termname), false, Host::Passthrough),
            Outer::KittyPlaceholders,
            "{termname} implements kitty graphics with unicode placeholders"
        );
    }
}

#[test]
fn an_opaque_multiplexer_never_gets_a_pixel_protocol() {
    // Screen, or a tmux ratatui-image won't wrap for: the escapes never reach
    // the outer terminal, so even a known-capable client must stay on text.
    for konsole_env in [true, false] {
        let outer = resolve_outer(Some("xterm-kitty"), konsole_env, Host::Opaque);
        assert_eq!(outer, Outer::Unknown);
        assert_eq!(protocol_override(ProtocolType::Halfblocks, outer), None);
    }
}

#[test]
fn tmux_is_passthrough_only_when_ratatui_image_will_wrap_for_it() {
    // ratatui-image keys passthrough off TERM/TERM_PROGRAM, never $TMUX.
    assert_eq!(
        classify_host(true, false, Some("tmux-256color"), Some("tmux")),
        Host::Passthrough,
        "tmux's own default TERM"
    );
    assert_eq!(
        classify_host(true, false, Some("xterm-256color"), Some("tmux")),
        Host::Passthrough,
        "a default-terminal override is still wrapped via TERM_PROGRAM"
    );
    assert_eq!(
        classify_host(true, false, Some("xterm-256color"), None),
        Host::Opaque,
        "override + a tmux too old for TERM_PROGRAM: escapes go out raw and die"
    );
}

#[test]
fn screen_nested_with_tmux_is_still_opaque() {
    // Whichever of the two nests inside the other, both env vars are set and the
    // escapes still have to cross screen, which eats them. tmux's passthrough
    // only carries them as far as its own client.
    assert_eq!(
        classify_host(true, true, Some("tmux-256color"), Some("tmux")),
        Host::Opaque,
        "tmux passthrough hands the escapes to screen, which drops them"
    );
}

#[test]
fn screen_and_scrubbed_multiplexers_are_opaque() {
    assert_eq!(
        classify_host(false, true, Some("screen-256color"), None),
        Host::Opaque
    );
    assert_eq!(
        classify_host(false, false, Some("screen-256color"), None),
        Host::Opaque,
        "$STY scrubbed, but TERM still names the multiplexer"
    );
    assert_eq!(
        classify_host(false, false, Some("tmux-256color"), None),
        Host::Opaque,
        "$TMUX scrubbed, but TERM still names the multiplexer"
    );
}

#[test]
fn a_bare_terminal_is_direct() {
    assert_eq!(
        classify_host(false, false, Some("xterm-256color"), None),
        Host::Direct
    );
    assert_eq!(
        classify_host(false, false, Some("xterm-kitty"), None),
        Host::Direct
    );
    assert_eq!(classify_host(false, false, None, None), Host::Direct);
}
