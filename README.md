# osu!collect

---

A CLI program to download osu! beatmap collections from [osu!collector](https://osucollector.com) for free :3 

Supports alternative download mirrors.

## Installation

---

Download the binary for your platform from the latest CI run on [releases page](https://github.com/uwuclxdy/osu-collect/releases).

Or compile and install from source:
```bash
git clone https://github.com/uwuclxdy/osu-collect
cd osu-collect
cargo install --path .
```

## Usage

---

### IMPORTANT: importing to osu! lazer

After downloading, **follow the steps below** to correctly **import collection**:
1. Import all downloaded maps into lazer
2. Click `Run first time setup` and `Next` until the **Import screen**
3. Set `previous osu! install` to the **directory of the collection** you've downloaded
4. Click `Import content from previous version`
5. That's it, you can close the setup screen, and the collection should be imported!

---

> Replace `osu-collect` with the binary name that you've downloaded in the commands below. 

**Only `-c` (collection) is required.** If `-d` is not specified, a **subfolder will be automatically created** in current directory.

_Command line arguments:_
```bash
  -c, --collection <COLLECTION>  Collection URL or ID
  -d, --directory <DIRECTORY>    Download directory
  -m, --mirror <MIRROR>          Mirror base URL
  -y, --yes                      Auto-overwrite existing files
      --skip-existing            Skip existing files
```

#### _Download all maps in a collection:_
```bash
osu-collect -c "https://osucollector.com/collections/17503" -d ~/Downloads
```

#### _Using an alternative mirror:_
```bash
osu-collect -c "https://osucollector.com/collections/17503" \
  -d ~/Downloads \
  -m "https://catboy.best/d/{id}"
```

#### _Auto-skip existing files:_
```bash
osu-collect -c "https://osucollector.com/collections/17503" \
  -d ~/Downloads \
  --skip-existing
```

> **Note for Windows Users:** Windows Terminal or PowerShell 7+ are recommended

## Configuration

---

You can create a configuration file to set default options:

### Linux/macOS
`~/.config/osu-collect/config.toml`

### Windows
`%APPDATA%\osu-collect\config.toml`

### Example Configuration
```toml
[mirror]
url = "https://api.nerinyan.moe/d/{id}"

[download]
skip_existing = false
concurrent = 3
```

#### Configuration Options
- `mirror.url`: Default mirror URL template (must contain `{id}`)
- `download.skip_existing`: Skip existing files by default (true/false)
- `download.concurrent`: Number of concurrent downloads (1-50, recommended: 3-10)

## Building from Source & Contributing

---

Check TODO section to see what I have planned for the future of this project.

### Prerequisites
- Rust 1.70 or later
- For Windows: MSVC or MinGW-w64 toolchain

#### Install Windows Target (MinGW)
```bash
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu
```

#### Cross-compilation
The included `build.sh` script can build for both Linux and Windows:
```bash
./build.sh
```

Outputs will be in the `build/` directory.

## TODO
- [ ] A GUI interface or at least TUI
- [ ] Many other things I can't think of..

## Star History

[![Star History Chart](https://api.star-history.com/svg?repos=uwuclxdy/osu-collect&type=date&legend=top-left)](https://www.star-history.com/#uwuclxdy/osu-collect&type=date&legend=top-left)
