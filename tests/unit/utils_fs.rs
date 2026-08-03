use crate::utils::{expand_tilde, pretty_path};
use std::path::Path;

// These tests call `dirs::home_dir()` and assert symmetry between the two
// helpers. They pass on any machine with a home directory set.

#[test]
fn pretty_path_collapses_home_subpath() {
    let Some(home) = dirs::home_dir() else {
        return; // no home configured in this environment — skip
    };
    let songs = home.join("Songs");
    // Display-only: `~` is followed by `/` on every platform — `pretty_path`
    // normalizes the Windows separator away (the filesystem never reads this).
    assert_eq!(pretty_path(&songs).as_ref(), "~/Songs");
}

#[test]
fn pretty_path_collapses_home_itself() {
    let Some(home) = dirs::home_dir() else {
        return;
    };
    assert_eq!(pretty_path(&home).as_ref(), "~");
}

#[test]
fn pretty_path_leaves_unrelated_path_unchanged() {
    assert_eq!(
        pretty_path(Path::new("/other/path")).as_ref(),
        "/other/path"
    );
}

#[test]
fn expand_tilde_expands_tilde_slash_prefix() {
    let Some(home) = dirs::home_dir() else {
        return;
    };
    let expected = format!(
        "{}{}Songs",
        home.to_string_lossy(),
        std::path::MAIN_SEPARATOR
    );
    assert_eq!(expand_tilde("~/Songs"), expected);
}

#[cfg(windows)]
#[test]
fn expand_tilde_expands_tilde_backslash_on_windows() {
    let Some(home) = dirs::home_dir() else {
        return;
    };
    let expected = format!(r"{}\Songs", home.to_string_lossy());
    assert_eq!(expand_tilde(r"~\Songs"), expected);
}

#[test]
fn expand_tilde_expands_bare_tilde() {
    let Some(home) = dirs::home_dir() else {
        return;
    };
    assert_eq!(expand_tilde("~"), home.to_string_lossy().as_ref());
}

#[test]
fn expand_tilde_leaves_absolute_path_unchanged() {
    assert_eq!(expand_tilde("/abs/path"), "/abs/path");
}

#[test]
fn expand_tilde_leaves_relative_path_unchanged() {
    assert_eq!(expand_tilde("relative/path"), "relative/path");
}

#[test]
fn pretty_and_expand_are_inverse() {
    let Some(home) = dirs::home_dir() else {
        return;
    };
    let original = home.join("Songs");
    let prettied = pretty_path(&original);
    let expanded = expand_tilde(prettied.as_ref());
    assert_eq!(
        Path::new(&expanded),
        original,
        "expand_tilde(pretty_path(p)) should round-trip to the original path"
    );
}

#[test]
fn pretty_path_does_not_collapse_sibling_user_dir() {
    // /home/userx/Songs must NOT collapse to ~x/Songs when home is /home/user.
    // Path::strip_prefix uses component-wise matching so /home/userx is NOT
    // a prefix of /home/user — this test guards against string-prefix bugs.
    let Some(home) = dirs::home_dir() else {
        return;
    };
    // Construct a sibling directory by appending "x" to the last component.
    let mut sibling = home.clone();
    let last = sibling
        .file_name()
        .map(|n| {
            let mut s = n.to_os_string();
            s.push("x");
            s
        })
        .unwrap_or_else(|| std::ffi::OsString::from("xuser"));
    sibling.pop();
    sibling.push(last);
    let path = sibling.join("Songs");
    let result = pretty_path(&path);
    // Must not start with "~" since it is not under our home.
    assert!(
        !result.starts_with('~'),
        "pretty_path({path:?}) = {result:?} — must not collapse sibling user dir"
    );
}

#[test]
fn expand_tilde_leaves_tilde_followed_by_non_slash_unchanged() {
    // `~root` / `~otheruser` must never be expanded — only `~` and `~/…`.
    assert_eq!(expand_tilde("~root"), "~root");
    assert_eq!(expand_tilde("~otheruser/Songs"), "~otheruser/Songs");
}

#[test]
fn expand_tilde_empty_string_unchanged() {
    assert_eq!(expand_tilde(""), "");
}

#[test]
fn pretty_path_empty_path_does_not_panic() {
    // An empty path should not crash and returns its string representation.
    let result = pretty_path(Path::new(""));
    // Just assert it doesn't panic; the exact string is platform-dependent.
    let _ = result.as_ref();
}

/// `pretty_path` normalizes `\` to `/` on Windows (display-only); on other
/// platforms the string passes through untouched.
#[cfg(windows)]
fn normalized(s: &str) -> String {
    s.replace('\\', "/")
}
#[cfg(not(windows))]
fn normalized(s: &str) -> String {
    s.to_string()
}

#[test]
fn pretty_path_strips_windows_verbatim_drive_prefix() {
    // `std::fs::canonicalize` hands back `\\?\C:\…` on Windows; the TUI must show
    // the bare path. Pure string logic, so this is deterministic on any platform
    // (the embedded `C:\Users\cloudy` is never the test host's home → no `~`).
    assert_eq!(
        pretty_path(Path::new(r"\\?\C:\Users\cloudy\Downloads\play later-67")).as_ref(),
        normalized(r"C:\Users\cloudy\Downloads\play later-67")
    );
}

#[test]
fn pretty_path_strips_windows_verbatim_unc_prefix() {
    // `\\?\UNC\server\share` is the verbatim form of `\\server\share`.
    assert_eq!(
        pretty_path(Path::new(r"\\?\UNC\server\share\maps")).as_ref(),
        normalized(r"\\server\share\maps")
    );
}
