# Finding maps

The `find` source on the get maps tab searches for beatmaps without an osu!collector collection: type a query and/or set criteria, press `find`, check the results, download. Results land in their own subfolder per search.

## Criteria

| Criterion | Notes |
|---|---|
| Free text query | Works like search on the osu! website |
| Preset | Predefined filters |
| Special | `farm` / `stream` / `ranked mapper` flags from nzbasic's (BBD) database |
| Mode / status / sort | Cycle with <kbd>space</kbd> / <kbd>↵</kbd>; all options render on the row, the active one is bracketed |
| Numeric ranges | stars, AR, CS, OD, HP, BPM, length, keys, favourites |
| Artist / mapper / title | Text matches; semantics differ per route (below) |
| Ranked date | Its own range grammar (below) |
| Limit | Caps result rows; only applies on the nzbasic route |

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

One form, two backends underneath: the osu! api v2 search and nzbasic's [batch-beatmap-downloader](https://github.com/nzbasic/batch-beatmap-downloader) (BBD) database. The form routes automatically by what you set. A `via osu! api` / `via nzbasic` tag on the `find` button's row always shows where the run will go, live as you edit. A combination that needs both backends errors out naming the two clashing fields instead of guessing.

| Forces nzbasic | Forces osu! api |
|---|---|
| A special flag (`farm` / `stream` / `ranked mapper`) | A free text query |
| Sort `bpm ↓` or `length ↑` | Sort `relevance`, `title ↑`, `artist ↑` |
| Status `unranked` | Status `qualified` |
| | A keys, favourites, or ranked-date range |

Everything else runs on either backend; the default is the osu! api. Status `approved` works on both.

## Matching semantics per route

- **Artist / mapper / title**: exact phrases on the osu! api route, substring matches on the nzbasic route. Want contains-matching? Force the nzbasic route with a special flag or the `bpm ↓` / `length ↑` sort.
- **Default sort** is newest ranked first, which runs on either route. Cycling to `relevance` switches the route to the osu! api.

## Results

Results open in a two-pane browse: checkbox list left, preview right. Rows that arrive as bare IDs backfill their titles in pages while a loading cue shows. Press <kbd>m</kbd> to load more results on the osu! api route.

The preview shows the highlighted set's title, artist, mapper, status, favourites, and plays. nzbasic-routed results add the tags, source, genre, language, and ranked / updated dates, plus one difficulty's max combo, drain time, pass count, and hash.

On a terminal with graphics support (sixel, kitty, or iterm2) the highlighted set's cover art renders above the metadata; elsewhere it falls back to unicode half-blocks. Covers load in the background a moment after a row settles under the cursor.

Checked results show an approximate total size on the download button as sizes load: exact sizes come free with nzbasic responses, the osu! route probes them via Nekoha in the background.

> [!NOTE]
> The nzbasic route runs against a solo developer's free instance. Be reasonable with it.
