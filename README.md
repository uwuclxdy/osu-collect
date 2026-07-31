<div align="center">

<img src="media/osu-collect.png" alt="osu!collect banner" width="640">

<h1></h1>

<p style="font-size: 22px" ><b>Free osu!collector downloader in your terminal</b></p>

[![Build](https://shields.uwuclxdy.dev/github/actions/workflow/status/uwuclxdy/osu-collect/release.yml?style=for-the-badge&logo=githubactions&logoColor=white&label=build&color=ff66aa)](https://github.com/uwuclxdy/osu-collect/actions/workflows/release.yml)
[![Latest release](https://shields.uwuclxdy.dev/github/v/release/uwuclxdy/osu-collect?style=for-the-badge&logo=github&color=ff66aa)](https://github.com/uwuclxdy/osu-collect/releases/latest)
[![Downloads](https://shields.uwuclxdy.dev/github/downloads/uwuclxdy/osu-collect/total?style=for-the-badge&logo=github&color=ff66aa)](https://github.com/uwuclxdy/osu-collect/releases)
![Platforms](https://shields.uwuclxdy.dev/badge/platform-Windows%20%7C%20Linux%20%7C%20macOS-ff66aa?style=for-the-badge)

[Features](#features) · [Install](#installation) · [Usage](#usage) · [Import](#importing-into-osu) · [Configuration](#configuration) · [FAQ](#faq) · [Wiki](https://github.com/uwuclxdy/osu-collect/wiki)

</div>

osu!collect is a terminal app (TUI) that **downloads osu! beatmap collections from [osu!collector](https://osucollector.com)**. Paste a collection link, pick a folder, and it batch-downloads every map across multiple mirrors, generates a ready-to-import `collection.db`, and can re-check the collection later to grab only the maps you're missing.

<div align="center">

<img src="media/osu-collect-home.png" alt="osu!collect: paste an osu!collector collection, pick mirrors and a download folder" width="49%">
<img src="media/osu-collect-downloading.png" alt="osu!collect downloading a collection with live per-map progress, speed and ETA" width="49%">

</div>

## Features

- **Batch downloads** from any osu!collector collection. Paste a URL or ID, press <kbd>↵</kbd>.
- **Find maps without a collection**: free-text search plus per-difficulty criteria (stars, AR, CS, OD, HP, BPM, length, keys, favourites, ranked date, farm/stream/ranked-mapper flags), auto-routed between the osu! api and nzbasic's community database. Genre, language, video/storyboard, achieved rank, played state and explicit-content filters join them on a supporter account. [Criteria + filter syntax.](https://github.com/uwuclxdy/osu-collect/wiki/Finding-Maps)
- **Mirrors with automatic failover**: osu!direct, Nerinyan, Sayobot, Nekoha, Beatconnect, osu!dl, catboy.best, the Hinamizawa cascade, the nzbasic CDN, your own custom mirrors. Logging in adds the official osu! servers. [Full list.](https://github.com/uwuclxdy/osu-collect/wiki/Mirrors)
- **Rate-limit aware**: throttled mirrors sit out with per-map cooldown countdowns while the rest keep downloading; a map that hits a limit re-queues instead of getting dropped. [How it's handled.](https://github.com/uwuclxdy/osu-collect/wiki/Mirrors#rate-limit-handling)
- **Collections updater**: re-check a collection later and download only the maps that are missing or newly added. [Guide.](https://github.com/uwuclxdy/osu-collect/wiki/Updating-Collections)
- **Ez import with `collection.db`**: maps arrive as a proper osu! collection, not a loose folder of `.osz` files.
- **Integrity verification**: MD5 plus archive validation on every download; files already on disk are verified and skipped.
- **Skips what you already own**: reads your osu! library (stable `osu!.db` / lazer client database) and skips maps you've already imported instead of re-fetching them; they still go into the generated `collection.db`.
- **Retry failed maps**: failures persist between runs. Retry them with one key, or on the next download.
- **Self-updating**: checks for a newer release on launch and installs it. Flip auto-update off in config to only get a notice; then press <kbd>u</kbd> to read the changelog and update when you want.
- **Downloads tab with history**: active and past runs live on one list; open a run for live per-map progress. Past runs (including cancelled ones) survive restarts.

## Installation

### One-line install (recommended)

**Linux x64 / macOS Apple Silicon**:

```bash
curl -fsSL https://raw.githubusercontent.com/uwuclxdy/osu-collect/main/install.sh | bash
```

**Windows x64 (PowerShell)**:

```powershell
iwr https://raw.githubusercontent.com/uwuclxdy/osu-collect/main/install.bat -OutFile "$env:TEMP\osu-install.bat"; & "$env:TEMP\osu-install.bat"
```

Installs to `%LOCALAPPDATA%\Programs\osu-collect`, adds it to `PATH`, creates a desktop shortcut and registers in **Settings → Apps → Installed apps**. No admin needed.

### Prebuilt binary

Download from [Releases](https://github.com/uwuclxdy/osu-collect/releases/latest) and run it in a terminal.

> [!NOTE]
> osu!collect runs in a terminal. Windows users should be able to open it with a double click as well, but it's not guaranteed. Open an [Issue](https://github.com/uwuclxdy/osu-collect/issues/new/choose) if it doesn't work. Windows Terminal or PowerShell 7+ are recommended.

### Install latest from source (Rust 1.85+)

```bash
cargo install --git https://github.com/uwuclxdy/osu-collect
```

Building from source and more: [Installation](https://github.com/uwuclxdy/osu-collect/wiki/Installation) in the wiki.

## Uninstall

**Windows**: Settings → Apps → Installed apps → **osu!collect** → Uninstall.

**Linux/macOS**: run `rm ~/.local/bin/osu-collect`.

## Usage

```bash
osu-collect
```

Paste a collection link, pick a directory, press <kbd>↵</kbd>. The get maps tab has two more sources: **find** searches maps by per-difficulty criteria with no collection needed; **update** re-checks collections you downloaded and fetches only what's missing. Press <kbd>?</kbd> in-app for every key.

Full guides in the wiki:

- [Usage](https://github.com/uwuclxdy/osu-collect/wiki/Usage)
- [Finding maps](https://github.com/uwuclxdy/osu-collect/wiki/Finding-Maps)
- [Updating collections](https://github.com/uwuclxdy/osu-collect/wiki/Updating-Collections)
- [Keybindings](https://github.com/uwuclxdy/osu-collect/wiki/Keybindings)

## Importing into osu!

<details open>
<summary><b>osu! lazer</b></summary>

1. Import all downloaded maps into lazer.
2. Click `Run first time setup`, then `Next` until the **Import screen**.
3. Set `previous osu! install` to the **folder of the collection** you downloaded.
4. Click `Import content from previous version`.
5. Done. Both the maps and the collection are imported.

</details>

<details>
<summary><b>osu! stable</b></summary>

Drag the downloaded `.osz` files into osu!, then merge the generated `collection.db` with a tool like [Collection Manager](https://github.com/Piotrekol/CollectionManager). If you have no existing collections, you can just replace your `collection.db` with the generated one.

</details>

More detail: [Importing into osu!](https://github.com/uwuclxdy/osu-collect/wiki/Importing-into-osu) in the wiki.

## Configuration

osu!collect reads an optional config file; most settings are also editable live on the config tab, where changes apply and save immediately.

| OS | Path |
|---|---|
| Linux / macOS | `~/.config/osu-collect/config.toml` |
| Windows | `%APPDATA%\osu-collect\config.toml` |

See [config.toml.example](config.toml.example) and [Configuration](https://github.com/uwuclxdy/osu-collect/wiki/Configuration) (wiki).

## Alternatives

| Tool | How osu!collect differs |
|---|---|
| [osu!Collector desktop client](https://osucollector.com/app) | The official app that needs a paid subscription; osu!collect is free. |
| [BatchBeatmapDownloader](https://github.com/nzbasic/batch-beatmap-downloader) | Downloads by filters and criteria rather than osu!collector collections. |
| [osu-collector-dl](https://github.com/roogue/osu-collector-dl) | A CLI script with no TUI, no `collection.db` generation, and no updater. |
| [OsuCollectionDownloader](https://github.com/waylaa/OsuCollectionDownloader) | Generates `.osdb` files and needs the .NET runtime. |
| [Collection Manager](https://github.com/Piotrekol/CollectionManager) | Manages and merges existing collections; pairs well with osu!collect for stable imports. |

## FAQ

**How do I download an osu!collector collection for free?**
Run osu!collect, paste the collection URL, press <kbd>↵</kbd>. Downloads come from public beatmap mirrors, or the official servers if you log in. No subscription needed.

**Does it work with osu! lazer?**
Yes. See [Importing into osu!](#importing-into-osu). The generated `collection.db` imports through lazer's "first-time-setup".

**Do I need an osu! account?**
No. Logging in is optional; it adds the official osu! servers as an extra source, and unlocks a few find filters osu! only honors for supporter accounts.

**Can it update a collection I downloaded earlier?**
Yes. The update source on the get maps tab diffs your downloaded collections against osu!collector and fetches only what's missing.

**A download failed or got rate-limited. What now?**
Failures save per collection. Open the run on the downloads tab and press <kbd>r</kbd> to retry them all, or accept the retry prompt next time you download that collection. Rate-limited mirrors cool down on their own while the others keep going. A map that stays throttled past the auto-defer delay (60s by default, configurable) goes back in the queue on its own and retries later, so the run never stalls; press <kbd>s</kbd> to defer the currently-stuck maps yourself, or <kbd>S</kbd> to drop them for the rest of the run.

## Documentation

See more on the wiki:

| Page | Covers |
|---|---|
| [Installation](https://github.com/uwuclxdy/osu-collect/wiki/Installation) | Install details, self-update, uninstall, building from source |
| [Usage](https://github.com/uwuclxdy/osu-collect/wiki/Usage) | The get maps form, downloads tab, failed-map retries |
| [Finding maps](https://github.com/uwuclxdy/osu-collect/wiki/Finding-Maps) | Search criteria, filter syntax, backend routing |
| [Updating collections](https://github.com/uwuclxdy/osu-collect/wiki/Updating-Collections) | Re-check scans, marking maps installed |
| [Importing into osu!](https://github.com/uwuclxdy/osu-collect/wiki/Importing-into-osu) | Step-by-step lazer + stable import |
| [Keybindings](https://github.com/uwuclxdy/osu-collect/wiki/Keybindings) | Every key, vim keymap |
| [Configuration](https://github.com/uwuclxdy/osu-collect/wiki/Configuration) | Every config key with its default |
| [Mirrors](https://github.com/uwuclxdy/osu-collect/wiki/Mirrors) | Mirror list, rate limits, custom mirrors, osu! login |

## Roadmap

- [ ] All features of [BatchBeatmapDownloader](https://github.com/nzbasic/batch-beatmap-downloader) (🚧 in the works)
- [ ] Integrate osu!Stats

## Acknowledgments

Powered by [osu-downloader](osu-downloader/) (the bundled Rust library handling mirrors, failover, validation and events), [osu-db](https://crates.io/crates/osu-db) and [ratatui](https://ratatui.rs). Inspired by [BatchBeatmapDownloader](https://github.com/nzbasic/batch-beatmap-downloader).

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) at your option. Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this work shall be dual licensed as above, without any additional terms or conditions.
