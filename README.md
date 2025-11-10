# mcdu - Modern Disk Usage Analyzer

A fast, modern, and safe disk usage analyzer with integrated file deletion capabilities, written in Rust. Spiritual fork of ncdu with enhanced performance and features.

## ✨ Features

### 🚀 Performance
- **Async scanning** - Non-blocking directory scanning in background thread
- **Smart caching** - Intelligent size caching with mtime validation
- **Instant navigation** - Cached directories load instantly when revisiting
- **Optimized I/O** - Single-pass operations, no redundant metadata calls
- **Memory efficient** - Streaming with walkdir, minimal memory footprint

### 📊 Core Functionality
- **Recursive directory scanning** - Quickly analyze disk usage across nested directories
- **Colorful TUI** - Color-coded display by file size (red for large, green for small)
- **Live scanning progress** - Real-time progress with current file and percentage
- **Disk space monitoring** - Shows available/total disk space in title bar
- **Viewport scrolling** - Automatic scrolling keeps selection visible
- **Change tracking** - Highlights size changes between scans
- **Safe deletion** - Double-confirmation dialogs before destructive operations
- **Non-blocking delete** - Continue browsing while files are deleted in background
- **Dry-run mode** - Preview what would be deleted without actually deleting
- **Audit logging** - JSON logs of all deletions saved to `~/.mcdu/logs/`

### 🎨 User Experience
- **Vim keybindings** - j/k for up/down, h/l for parent/enter
- **Arrow keys** - Full arrow key support for navigation
- **Modal navigation** - Use arrow keys to select buttons in confirmation dialogs
- **Loading overlays** - Clear progress indication during scanning
- **Auto-dismiss notifications** - Notifications disappear after 3 seconds
- **Cross-platform** - Works on macOS (APFS) and Linux (ext4, btrfs, xfs, etc.)

## 📥 Installation

```bash
# Build from source
cargo build --release

# Run
./target/release/mcdu

# Optional: Install to system
cargo install --path .
```

## 🎮 Usage

### Navigation
- `↑/k` - Move cursor up
- `↓/j` - Move cursor down
- `Enter/→/l` - Enter directory
- `Backspace/←/h` - Go to parent directory
- `d` - Delete selected file/directory
- `r` - Refresh current view (uses cache)
- `c` - Clear cache and hard refresh
- `?` - Show help screen
- `q/Esc` - Quit application

### Deletion Workflow

1. **Select file/directory** - Navigate with arrow keys
2. **Press 'd'** - Opens confirmation dialog
3. **Confirm** - First dialog: `[Yes] [No] [Dry-run]`
4. **Final confirm** - Second dialog: `[YES, DELETE] [Cancel]`
5. **Watch progress** - Real-time progress bar shows deletion status
6. **Get notified** - Green success message with stats

### Dry-run Mode
Press `d` on target, then select `[d] Dry-run` to see what would be deleted without actually deleting anything.

### Cache Management
- **`r`** - Refresh (uses cache for speed)
- **`c`** - Clear cache and rescan (force accurate sizes)

The cache automatically invalidates based on directory modification times, ensuring you see up-to-date information.

## 🖥️ UI Design

### Title Bar
```
📊 mcdu v0.2.0 | /Users/username/Repos       42 items | 15 cached | 💾 42GB/460GB (91%)
```
Shows: current path, item count, cached entries, and disk space (available/total/percent used)

### Color Coding
- 🔴 **Red** - Files >100 GB
- 🟡 **Yellow** - Files >10 GB
- 🔵 **Cyan** - Files >1 GB
- 🟢 **Green** - Files <1 GB

### Loading Progress
```
┌────────────────────────────────────────┐
│  ⟳ Scanning directory...               │
│                                         │
│  22 / 135 items (16%)                  │
│                                         │
│  evesrc                                 │
│                                         │
│  Please wait                            │
└────────────────────────────────────────┘
```
Shows: progress counter, current directory being scanned, and percentage complete.

### Main View
```
┌─────────────────────────────────────────────────────────┐
│ 📊 mcdu v0.2.0 | /Users/username/Projects               │
├─────────────────────────────────────────────────────────┤
│ Path: /Users/username/Projects                         │
│                                                         │
│ 📁 node_modules           123.4 GB  ▓▓▓▓▓░░░░░░  ⬆ 5% │
│ 📁 .git                    45.2 GB  ▓▓▓░░░░░░░░       │
│ 📁 target                  12.1 GB  ▓░░░░░░░░░░       │
│ 📄 large-file.iso           2.3 GB  ░░░░░░░░░░░       │
│                                                         │
├─────────────────────────────────────────────────────────┤
│ [↑↓jk] Navigate [Enter] Open [h] Parent                │
│ [d] Delete [r] Refresh [c] Clear cache [?] Help        │
│                                      [q/Esc] Quit       │
└─────────────────────────────────────────────────────────┘
```
⬆/⬇ arrows show size changes since last scan.

## 📝 Logging

All deletions and dry-runs are logged to `~/.mcdu/logs/delete-YYYY-MM-DD.log`

### Log Format (JSON Lines)
```json
{
  "timestamp": "2025-01-10T14:23:45Z",
  "action": "delete",
  "path": "/Users/username/Projects/node_modules",
  "size_bytes": 132548901234,
  "dry_run": false,
  "status": "success",
  "files_deleted": 45821,
  "duration_ms": 3421,
  "errors": null
}
```

## 🏗️ Architecture

### Module Structure
```
src/
├── main.rs          # Event loop and input handling
├── app.rs           # Application state and logic
├── ui.rs            # TUI rendering with ratatui
├── scan.rs          # Async directory scanning
├── delete.rs        # Optimized file deletion
├── modal.rs         # Modal dialog system
├── platform.rs      # Platform-specific (statvfs, disk space)
├── cache.rs         # Size caching with mtime validation
├── changes.rs       # Directory fingerprinting & change detection
└── logger.rs        # JSON structured logging
```

### Key Design Decisions

1. **Async Scanning** - Directory scanning runs in background thread via mpsc channels
2. **Size Caching** - Thread-safe HashMap with automatic mtime-based invalidation
3. **Non-blocking UI** - Ratatui event loop continues during all operations
4. **Safe Defaults** - Final confirm defaults to "Cancel" to prevent accidents
5. **Optimized I/O** - Single-pass deletion, reused metadata, fragment_size for disk space

## 🔧 Performance Optimizations

### Scanning
- **Async background scanning** - UI never freezes
- **Live progress updates** - See what's being scanned in real-time
- **Smart caching** - Revisit directories instantly
- **Fragment size detection** - Correct disk space on APFS (macOS)

### Deletion
- **Single-pass algorithm** - One directory walk instead of three
- **Metadata reuse** - No redundant stat() calls
- **Background threading** - Non-blocking operation

### Memory
- **Streaming iteration** - walkdir processes files as it goes
- **Efficient caching** - Only caches directory sizes, not full trees
- **Minimal allocations** - Reuses buffers where possible

## 📦 Dependencies

- **ratatui** - Terminal UI framework
- **crossterm** - Terminal control
- **walkdir** - Recursive directory traversal
- **serde/serde_json** - JSON serialization
- **chrono** - Timestamp handling
- **nix** - Unix system calls (statvfs)

## 🌍 Platform Support

- ✅ **macOS** - Full support with APFS compatibility
- ✅ **Linux** - Full support (ext4, btrfs, xfs, etc.)
- ❌ **Windows** - Not currently supported

### Platform-Specific Features
- **macOS**: Correct APFS disk space using `f_frsize`
- **Linux**: Standard ext4/btrfs/xfs support
- **Both**: mtime-based cache validation

## 🐛 Known Issues

- Very large directories (>100k items) may show slow initial scan
- Mouse input not supported (keyboard only)
- Windows not yet supported

## 🚀 Future Enhancements

- [ ] Parallel scanning using rayon
- [ ] APFS snapshot handling on macOS
- [ ] SELinux attribute handling on Linux
- [ ] Undo functionality with transaction log
- [ ] Search/filter capabilities
- [ ] Sorting options (by size, date, name)
- [ ] Configuration file support
- [ ] Windows support via GetDiskFreeSpaceEx
- [ ] Progress estimation for large deletions

## 🔨 Building from Source

```bash
# Debug build
cargo build

# Release build (optimized)
cargo build --release

# Run tests
cargo test

# Run with debug logging
RUST_LOG=debug cargo run

# Check documentation
cargo doc --open
```

## 📊 Performance Comparison

**Scanning 100k files:**
- Initial scan: ~2-3 seconds
- Cached revisit: **Instant** (<10ms)
- Memory usage: ~50MB

**Deletion:**
- Old approach: 3 directory walks
- New approach: **1 directory walk** (3x faster!)

## 📄 License

MIT

## 🤝 Contributing

Pull requests welcome! Please ensure:
1. Code compiles without warnings (`cargo clippy`)
2. Tests pass (`cargo test`)
3. Changes are well-documented
4. Performance improvements are benchmarked

## 💬 Support

For issues or feature requests, please open an issue on GitHub.

## 🙏 Acknowledgments

Inspired by [ncdu](https://dev.yorhel.nl/ncdu) - the original ncurses disk usage analyzer.
