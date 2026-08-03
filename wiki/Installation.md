# Installation

## One-line install (recommended)

**Linux x64 / macOS Apple Silicon**:

```bash
curl -fsSL https://raw.githubusercontent.com/uwuclxdy/osu-collect/main/install.sh | bash
```

**Windows x64 (PowerShell)**:

```powershell
iwr https://raw.githubusercontent.com/uwuclxdy/osu-collect/main/install.bat -OutFile "$env:TEMP\osu-install.bat"; & "$env:TEMP\osu-install.bat"
```

The Windows installer needs no admin rights. It does the following:

- installs to `%LOCALAPPDATA%\Programs\osu-collect`
- adds the install folder to your `PATH`
- creates a desktop shortcut
- registers an uninstall entry in **Settings → Apps → Installed apps**

## Prebuilt binary

Download the binary for your OS from [Releases](https://github.com/uwuclxdy/osu-collect/releases/latest) and run it in a terminal.

osu!collect runs in a terminal. On Windows a double click usually opens it too, though that's not guaranteed; Windows Terminal or PowerShell 7+ are recommended. If it doesn't start, open an [issue](https://github.com/uwuclxdy/osu-collect/issues/new/choose).

## Install from source

Requires Rust 1.85+ (edition 2024):

```bash
cargo install --git https://github.com/uwuclxdy/osu-collect
```

## Self-updating

osu!collect checks GitHub for a newer release on launch. With `update.auto_update = true` (the default) it downloads the release, verifies its checksum against the published one, then replaces itself. With `auto_update = false` it still checks and only notifies (toast + header indicator); press <kbd>u</kbd> to read the changelog and apply the update when you want. Opt into prerelease builds with `update.prereleases = true`. See [Configuration](Configuration#update).

## Uninstall

- **Windows**: Settings → Apps → Installed apps → **osu!collect** → Uninstall.
- **Linux / macOS**: `rm ~/.local/bin/osu-collect`

Settings live in a separate config folder, run history in a data folder; delete both for a full cleanup (paths in [Configuration](Configuration#other-files)).

## Building from source (development)

```bash
cargo build --release
```

`build.sh` cross-builds Linux + Windows binaries into `build/`. The bundled `osu-downloader` library is a path dependency, not a workspace member; test it separately with `cargo test --manifest-path osu-downloader/Cargo.toml --all-features`.
