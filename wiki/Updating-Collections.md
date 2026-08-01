# Updating collections

The `update` source on the get maps tab re-checks every collection you've downloaded against osu!collector and shows what's missing or was removed.

## Scanning

Set your osu! install path (persisted across restarts), then press the scan button. The scan reads your osu! library (stable `osu!.db` or the lazer client database) to know what you already own, then diffs each downloaded collection against its current state on osu!collector.

Switch between the stable and lazer client with <kbd>c</kbd> on any tab; the choice persists too. Switching clients clears the scan, so rescan after.

## Browsing and selecting

`view N mapsets` opens a two-pane browse: collections on the left, the highlighted collection's missing maps on the right. Selection is per whole collection via checkboxes; <kbd>a</kbd> / <kbd>A</kbd> select all / none. A collection with no updates is inert: it can't be selected and sinks to the bottom in every sort mode.

A set you deleted from your library on purpose stays held back, excluded from the `N new` count and from what downloads, so it won't quietly reappear as freshly missing. The preview tags it `previously deleted`; press <kbd>↵</kbd> or <kbd>space</kbd> on that row to toggle it between held back and `✓ re-included` for this run (on any other preview row those keys do nothing; the checkboxes live in the left pane). Re-including only lasts for the current scan, and a `held back` count joins `known bad` in the scan summary whenever something is being withheld.

The `download (N)` button fetches only the missing maps of the checked collections that aren't held back.

## Marking maps installed

A map you already own can keep showing as missing (for example after a manual import the scan can't see). Fix it in the browse preview; marks persist in `ignored-beatmapsets.json` in the data dir ([Configuration](Configuration#other-files)):

- <kbd>i</kbd> marks the highlighted set as installed; <kbd>I</kbd> marks the whole collection. Marked sets hide from the missing count.
- Marked sets stay visible at the top of the preview pane behind a divider.
- <kbd>u</kbd> / <kbd>U</kbd> reverse a mark without a rescan.
- A later scan that actually finds the map on disk un-hides it automatically.
