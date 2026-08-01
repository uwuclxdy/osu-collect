# Finding maps

The `find` source on the get maps tab searches for beatmaps without an osu!collector collection: type a query and/or set criteria, press `find`, check the results, download. Results land in their own subfolder per search.

## Criteria

| Criterion | Notes |
|---|---|
| Free text query | The search box at the top of the form; works like search on the osu! website |
| Preset | Predefined filters |
| Mode / categories / special | Cycle with <kbd>space</kbd> / <kbd>↵</kbd>; all options render on the row, the active one is bracketed. `special` is `farm` / `stream` / `ranked mapper` from nzbasic's (BBD) database |
| Sort / limit | Shape the result set rather than which maps match. `limit` caps result rows and only applies on the nzbasic route |
| Numeric ranges | stars, AR, CS, OD, HP, BPM, length, keys, favourites; behind `advanced filters` |
| Artist / mapper / title | Text matches; semantics differ per route (below) |
| Ranked date | Its own range grammar (below) |

Rows group under `preset`, `filters` and `results` headings; the `advanced filters` row expands the rest.

## Supporter-only filters

Six more filters appear when you are logged in with an account that has osu!supporter: `explicit` in the filters block, and `genre`, `language`, `extra`, `rank`, `played` at the top of `advanced filters`. osu! only honors these for a supporter token, so without one they are not shown at all.

`extra` and `rank` take several values at once. Each of their chips shows `[x]` when picked and `[ ]` when not. Press <kbd>↵</kbd> or <kbd>space</kbd> to open the row: its gutter mark turns `✎` and a `❯` caret picks out one chip. <kbd>←</kbd> / <kbd>→</kbd> then walk the caret, <kbd>space</kbd> toggles the chip under it. <kbd>↵</kbd>, <kbd>esc</kbd> or <kbd>q</kbd> close the row again; <kbd>↑</kbd> / <kbd>↓</kbd> close it on the way to another row. With the row closed, <kbd>←</kbd> / <kbd>→</kbd> switch tabs as everywhere else.

All six force the osu! api route. Losing supporter status resets them to their defaults.

## Numeric filter syntax

The operator goes on either side of the value; `-` and `..` are interchangeable range separators. Values are never negative, so any `-` reads as a separator.

| Input | Meaning |
|---|---|
| `6` | exactly 6 |
| `7+` or `>=7` | 7 or higher |
| `<=7` | 7 or lower |
| `7>` or `>7` | more than 7 |
| `7<` or `<7` | less than 7 |
| `2-3` or `2..3` | between 2 and 3, inclusive |
| `2..` / `..3` | open-ended range |

Strict `>` / `<` are honored on the osu! api route; the nzbasic route collapses them to inclusive. While a range field is focused, a live plain-english reading of what you typed appears under it; a value that doesn't parse shows the error in place.

## Ranked date

A date range with its own grammar: `2020..2024`, `2020-06-01..`, `..2024`. Inverted bounds are rejected with an error naming the field.

## Backend routing

One form, two backends underneath: the osu! api v2 search and nzbasic's [batch-beatmap-downloader](https://github.com/nzbasic/batch-beatmap-downloader) (BBD) database. The form routes automatically by what you set. A `via osu! api` / `via nzbasic` tag on the `find` button's row always shows where the run will go, live as you edit. A combination that needs both backends errors out naming the two clashing fields instead of guessing. Editing criteria into a different backend after a run drops the loaded results and their selection. A toast names where the criteria moved; run find again to reload under the new route.

| Forces nzbasic | Forces osu! api |
|---|---|
| A special flag (`farm` / `stream` / `ranked mapper`) | A free text query |
| Sort `bpm ↓` or `length ↑` | Sort `relevance`, `title ↑`, `artist ↑` |
| Category `unranked` | Category `qualified` |
| | A keys, favourites, or ranked-date range |
| | Any supporter-only filter set |

Everything else runs on either backend; the default is the osu! api. Category `approved` works on both.

## Matching semantics per route

- **Artist / mapper / title**: exact phrases on the osu! api route, substring matches on the nzbasic route. Want contains-matching? Force the nzbasic route with a special flag or the `bpm ↓` / `length ↑` sort.
- **Default sort** is newest ranked first, which runs on either route. Cycling to `relevance` switches the route to the osu! api.

## Results

Results open in a two-pane browse: checkbox list left, preview right. Rows that arrive as bare IDs backfill their titles in pages while a loading cue shows. Press <kbd>m</kbd> to load more results on the osu! api route.

The preview shows the highlighted set's title, artist, mapper, status, favourites, and plays. nzbasic-routed results add the tags, source, genre, language, and ranked / updated dates, plus one difficulty's max combo, drain time, pass count, and hash.

Under that comes the set's full difficulty list, one line per diff: the name, then its star rating and a ten-cell meter reading one cell per star. Ratings line up in a column past the longest name, and a name too long for the pane is cut short rather than pushing its rating off. <kbd>j</kbd> / <kbd>k</kbd> move through the list when the preview has focus, and the marked diff's own AR / CS / OD / HP, bpm, length, object counts, and pass rate fill the block below. On a narrow pane the meters drop and the ratings stay.

A set with a long difficulty list outgrows the pane. <kbd>pgup</kbd> / <kbd>pgdn</kbd> scroll the preview to the rest of it, since the arrows are busy stepping the difficulty list.

On a terminal with graphics support (sixel, kitty, or iterm2, konsole included) the highlighted set's cover art renders against the preview's right edge, the metadata to its left and across the full pane width once past the bottom of the image. A roomy pane and a short title get a wide crop; a longer title shrinks the cover to a smaller square so the title still fits on one line, and only a title too long for even that wraps to two lines. Elsewhere it falls back to unicode half-blocks, and a cramped preview drops the image and keeps the text. Covers load in the background a moment after a row settles under the cursor. A cover deep enough to reach the difficulty list holds the list back until past the image, so it gets the whole pane width instead of the strip beside the art. On a pane too short to fit it down there the list stays put and takes the strip. The art belongs to the preview's top row: scroll down and the text takes the whole pane, scroll back up and the cover returns.

Checked results show an approximate total size on the download button as sizes load: exact sizes come free with nzbasic responses, the osu! route probes them via Nekoha in the background.

> [!NOTE]
> The nzbasic route runs against a solo developer's free instance. Be reasonable with it.
