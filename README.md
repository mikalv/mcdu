# mcdu - Modern Disk Usage Analyzer & Developer Cleanup Tool

A fast, modern disk usage analyzer with an integrated developer cleanup tool, written in Rust. Think **ncdu** meets **CCleaner for developers**.

![Main View](docs/screens/mainview.png)

## Features

- **Disk usage browser** - Navigate your filesystem with vim-style keybindings, color-coded by size
- **Developer cleanup** - Scan for build artifacts, caches, and reclaimable space across 18+ ecosystems
- **Orphaned app data** (macOS) - Detect leftover data from uninstalled applications in ~/Library
- **Safe deletion** - Double-confirmation dialogs, dry-run mode, and JSON audit logging
- **Fast scanning** - Background async scanning with live progress, jwalk parallelism for cleanup
- **Cross-platform** - macOS (APFS) and Linux (ext4, btrfs, xfs)

## Installation

### Homebrew (macOS and Linux)

```bash
brew install mikalv/mcdu/mcdu
```

### AUR (Arch Linux)

```bash
yay -S mcdu
```

### Prebuilt Binaries

Download signed/static binaries from [GitHub Releases](https://github.com/mikalv/mcdu/releases/latest):

| Platform | Architecture | File |
|----------|-------------|------|
| Linux | x86_64 (static musl) | `mcdu-linux-x86_64-musl.tar.gz` |
| Linux | aarch64 (static musl) | `mcdu-linux-aarch64-musl.tar.gz` |
| Linux | x86_64 (glibc) | `mcdu-linux-x86_64-gnu.tar.gz` |
| macOS | Apple Silicon | `mcdu-macos-aarch64.tar.gz` |
| macOS | Intel | `mcdu-macos-x86_64.tar.gz` |

```bash
# Example: Linux x86_64
wget https://github.com/mikalv/mcdu/releases/latest/download/mcdu-linux-x86_64-musl.tar.gz
tar xzf mcdu-linux-x86_64-musl.tar.gz
sudo mv mcdu /usr/local/bin/
```

macOS binaries are code-signed. Checksums are listed in each release's notes.

### Cargo

```bash
cargo install mcdu
```

### From Source

```bash
git clone https://github.com/mikalv/mcdu.git
cd mcdu
cargo build --release
./target/release/mcdu
```

## Usage

### Disk Usage Browser

```bash
mcdu              # browse current directory
mcdu /path/to     # browse specific directory
```

![Main View](docs/screens/mainview.png)

| Key | Action |
|-----|--------|
| `j/k` or arrows | Navigate |
| `Enter/l` | Enter directory |
| `Backspace/h` | Go up |
| `d` | Delete selected |
| `r` | Rescan selected |
| `R` | Rescan all |
| `C` | Switch to cleanup mode |
| `?` | Help |
| `q` | Quit |

Color coding: red (>100 GB), yellow (>10 GB), cyan (>1 GB), green (<1 GB).

### Developer Cleanup

```bash
mcdu cleanup            # scan default paths
mcdu cleanup ~/repos    # scan specific path
```

Scans for build artifacts, caches, and other reclaimable disk space across your development tools.

![Cleanup Overview](docs/screens/overview.png)

![Cleanup Files](docs/screens/cleanup.png)

| Key | Action |
|-----|--------|
| `Tab` / `1-4` | Switch tabs (Overview, Categories, Files, Quarantine) |
| `j/k` or arrows | Navigate |
| `Space` | Toggle selection |
| `a` / `n` | Select all / none |
| `d` | Delete selected |
| `D` | Dry run |
| `C` | Rescan |
| `q` | Back to disk browser |

**Supported ecosystems:** Rust/Cargo, Node.js, Python, Go, Java/JVM, Elixir/Erlang, Ruby, PHP, .NET, Zig, Deno, Swift/iOS, Docker, Kubernetes, Terraform, IDE caches, browser caches, system caches (Homebrew, Xcode, Trash).

**Default scan paths:** `~/Downloads`, `~/Projects`, `~/Code`, `~/Developer`, `~/repos`, `~/dev`, `~/src`, `~/workspace`

### Orphaned App Data (macOS)

```bash
mcdu orphans              # scan for leftover data from uninstalled apps
```

Detects data in ~/Library (Caches, Application Support, Containers, Preferences, etc.) belonging to applications that are no longer installed. Items are shown unchecked by default for safe manual review.

### Custom Configuration

Place a config file at `~/.config/mcdu/cleanup.toml`:

```toml
scan_paths = ["~/myprojects", "~/work"]

[[rules]]
name = "custom-cache"
category = "Custom"
pattern = "**/.cache"
path = "${HOME}/myprojects"
match_type = "directory"
```

To clean via a tool instead of deleting/quarantining the match:

```toml
[[rules]]
name = "cargo-clean"
category = "Rust"
pattern = "**/target"
path = "${HOME}/Repos"
match_type = "directory"
project_marker = "Cargo.toml"
cleanup_command = "cargo clean --manifest-path '{dir}/Cargo.toml'"
```

Templates in `cleanup_command`: `{path}` (absolute match), `{dir}` (cwd — the directory itself, or its parent for files). Working directory is `{dir}`.

Rules from the config are **merged by `name`** with built-in defaults (user wins — set `enabled = false` on a default rule name to disable it).

> **Security:** Rules with `command = "..."` or `cleanup_command = "..."` run shell commands. Treat a shared/copied `cleanup.toml` as executable config.

## Architecture

```
crates/
  mcdu-core/     # Scanner, cleanup rules, config, platform detection
  mcdu-macos/    # macOS-specific: orphaned app data detection (cfg-gated)
  mcdu-tui/      # Ratatui app state, UI rendering, keybindings
  mcdu/          # CLI binary (clap), entry point
```

### Key design decisions

- **Async scanning** - Directory scanning in background thread via mpsc channels
- **Subtree pruning** - `filter_entry` skips node_modules/target/etc. entirely during cleanup scan
- **Safe defaults** - Final confirm defaults to Cancel, all deletions are audit-logged
- **Workspace crates** - Core logic separated from TUI and CLI for testability

## Dependencies

- **ratatui** + **crossterm** - Terminal UI
- **walkdir** / **jwalk** - Directory traversal (jwalk for parallel cleanup scanning)
- **clap** - CLI argument parsing
- **serde** / **toml** - Configuration
- **rayon** - Parallel processing

## Platform Support

- macOS - Full support with APFS compatibility
- Linux - Full support (ext4, btrfs, xfs, etc.)

## Building & Testing

```bash
cargo build            # debug build
cargo build --release  # optimized release build
cargo test             # run all workspace tests
```

## License

MIT

## Acknowledgments

Inspired by [ncdu](https://dev.yorhel.nl/ncdu) and the need to reclaim disk space eaten by `node_modules` and `target/` directories.
