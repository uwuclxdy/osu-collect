# Keybindings

Press <kbd>?</kbd> anywhere in the app for this list as an overlay.

## Global

| Keys | Action |
|---|---|
| <kbd>←</kbd> <kbd>→</kbd> <kbd>tab</kbd> <kbd>shift</kbd>+<kbd>tab</kbd> | Switch tabs (<kbd>tab</kbd> path-completes the directory field while editing it) |
| <kbd>↑</kbd> <kbd>↓</kbd> | Move between rows / scroll |
| <kbd>↵</kbd> | Activate, toggle, start a download, or edit a field |
| <kbd>space</kbd> | Toggle or cycle the focused control |
| <kbd>c</kbd> | Switch the osu! client (stable / lazer) |
| <kbd>u</kbd> | Open the changelog + update prompt when a newer release is available |
| <kbd>x</kbd> | Dismiss a notification |
| <kbd>?</kbd> | Toggle the help overlay |
| <kbd>esc</kbd> | Leave edit mode / go back |
| <kbd>q</kbd> | Back / quit (press twice to confirm; running downloads stop) |
| <kbd>ctrl</kbd>+<kbd>c</kbd> | Quit immediately from anywhere |
| <kbd>home</kbd> <kbd>end</kbd> | Jump to the first / last row of a list or form |
| <kbd>pgup</kbd> <kbd>pgdn</kbd> | Page a list up / down |

## Get maps tab

| Keys | Action |
|---|---|
| <kbd>1</kbd>-<kbd>3</kbd> | Jump straight to a source (collection / find / update) |
| <kbd>←</kbd> <kbd>→</kbd> | Focus the list / preview pane inside a browse |
| <kbd>s</kbd> | Jump to the last enabled action button (find / scan / view maps / download); cycle the sort in the update browse |
| <kbd>a</kbd> / <kbd>A</kbd> | Select all / none (update browse) |
| <kbd>i</kbd> / <kbd>I</kbd> | Mark a set / whole collection as installed (update preview) |
| <kbd>u</kbd> / <kbd>U</kbd> | Restore a marked-installed set / whole collection (update preview) |
| <kbd>r</kbd> | Recheck failed collections (update source) |
| <kbd>m</kbd> | Load more results / more titles in a browse |
| <kbd>+</kbd> <kbd>-</kbd> | Adjust the thread count |

## Downloads tab

| Keys | Action |
|---|---|
| <kbd>↵</kbd> | Open the highlighted run / expand its failures |
| <kbd>r</kbd> | Retry failed maps (open run) |
| <kbd>s</kbd> / <kbd>S</kbd> | Defer / drop maps stuck on a rate-limit cooldown |
| <kbd>q</kbd> | Cancel the open run while it's downloading |
| <kbd>←</kbd> <kbd>esc</kbd> | Back to the run list without cancelling |

## Config tab

| Keys | Action |
|---|---|
| <kbd>↵</kbd> on the account row | Open the login panel (<kbd>esc</kbd> / <kbd>q</kbd> closes it) |
| <kbd>⇧</kbd>+<kbd>↑</kbd> / <kbd>⇧</kbd>+<kbd>↓</kbd> | Reorder the focused built-in mirror |
| <kbd>r</kbd> | Re-probe mirror latency |

## Text editing

Text fields support full caret editing: <kbd>home</kbd>, <kbd>end</kbd>, <kbd>delete</kbd>, <kbd>ctrl</kbd>+<kbd>w</kbd> deletes the previous word.

## Vim keymap

Off by default; toggle on the config tab or set `display.vim_keys = true`. A <kbd>vim</kbd> marker shows in the footer when enabled.

| Keys | Action |
|---|---|
| <kbd>h</kbd> <kbd>j</kbd> <kbd>k</kbd> <kbd>l</kbd> | Move (tabs / rows) |
| <kbd>g</kbd><kbd>g</kbd> / <kbd>G</kbd> | Jump to top / bottom |
| <kbd>ctrl</kbd>+<kbd>u</kbd> / <kbd>ctrl</kbd>+<kbd>d</kbd> | Page up / down |
| <kbd>i</kbd> / <kbd>a</kbd> | Start editing the focused field |

A field in edit mode types literally; <kbd>esc</kbd> leaves it.
