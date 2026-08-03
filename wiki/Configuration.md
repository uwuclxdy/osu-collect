# Configuration

osu!collect reads an optional config file. Every key has a default, so a missing file or missing keys are not an issue.

| OS | Path |
|---|---|
| Linux / macOS | `~/.config/osu-collect/config.toml` |
| Windows | `%APPDATA%\osu-collect\config.toml` |

Most settings are also editable live on the config tab, where changes apply and save immediately. A commented example lives at [`config.toml.example`](https://github.com/uwuclxdy/osu-collect/blob/main/config.toml.example).

## `[mirror]`

Built-in mirror toggles. Hosts and what each mirror is good at: [Mirrors](Mirrors).

| Key | Default | Notes |
|---|---|---|
| `nerinyan` | `true` | |
| `osu_direct` | `true` | |
| `sayobot` | `true` | |
| `nekoha` | `true` | |
| `beatconnect` | `true` | |
| `osudl` | `true` | |
| `catboy` | `true` | |
| `hinamizawa` | `false` | |
| `nzbasic` | `false` | |
| `osu_official` | `false` | Needs osu! login ([Mirrors](Mirrors#logging-in-with-your-osu-account)) |
| `urls` | `[]` | Custom mirror URL templates, each containing `{id}`. Tried after the built-ins, in list order |
| `order` | `[]` | Try-order for the built-ins as host keys (reorder on the config tab with <kbd>⇧</kbd>+<kbd>↑</kbd>/<kbd>↓</kbd>). Unknown keys are dropped, missing built-ins are appended |

## `[download]`

| Key | Default | Notes |
|---|---|---|
| `concurrent` | CPU core count | Parallel downloads; values above 50 trigger a warning |
| `video` | `true` | Include beatmap videos; `false` uses each mirror's no-video variant where supported |
| `archive_validation` | `"magic"` | `.osz` integrity check: `"off"`, `"magic"` (ZIP header) or `"eocd"` (full footer scan) |
| `retry_failed_on_download` | `"ask"` | When a new download overlaps previously failed maps: `"ask"`, `"yes"` (always retry), `"no"` (skip silently) |
| `auto_skip_rate_limited` | `true` | Give up on a map once it has waited out rate-limit cooldowns for `rate_limit_skip_secs` |
| `rate_limit_skip_secs` | `60` | Counts only time actually spent waiting on cooldowns |
| `skip_already_imported` | `true` | Skip maps already in your osu! library; they still land in the generated `collection.db` |

## `[display]`

| Key | Default | Notes |
|---|---|---|
| `theme` | absent | `"full"` (truecolor) or `"compatible"` (xterm-256). |
| `vim_keys` | `false` | Vim navigation keymap → [Keybindings](Keybindings#vim-keymap) |
| `jump_to_downloads` | `true` | Switch to the downloads tab when a download starts |
| `confirm_delete_history` | `true` | Ask before deleting a downloads-tab entry (<kbd>d</kbd>). The prompt's "don't ask again" flips this off; set it back to `true` here to restore the confirmation |

## `[logging]`

| Key | Default | Notes |
|---|---|---|
| `enabled` | `false` | Saves structured tracing output into a logfile |
| `level` | `"info"` | `error`, `warn`, `info`, `debug`, `trace` |
| `format` | `"compact"` | Console format: `"compact"` or `"pretty"` |
| `file_dir` | unset | Optional directory for rolled JSON log files |

## `[update]`

| Key | Default | Notes |
|---|---|---|
| `auto_update` | `true` | Download and install a newer release automatically ([Installation](Installation#self-updating)) |
| `prereleases` | `false` | Allow updating to GitHub prereleases |

## `[recent]`

Managed automatically: last collection, last download directory, osu! client, install path. Don't edit this.

## Other files

`auth.json` (osu! token) sits next to `config.toml`. State files live in a per-user data dir: `~/.local/share/osu-collect` on Linux, `~/Library/Application Support/osu-collect` on macOS, `%APPDATA%\osu-collect` on Windows.

| File | Holds |
|---|---|
| `download_history.json` | Past runs shown on the downloads tab |
| `failed-beatmapsets.json` | Failed maps per collection, for retries across runs |
| `ignored-beatmapsets.json` | Sets you marked installed ([Updating collections](Updating-Collections#marking-maps-installed)) |
| `library-cache.json` | Memoized owned-beatmapset lookup, keyed by library path + mtime |
