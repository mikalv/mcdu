# mcdu - Disk Usage & Safe Delete Tool

A modern, colorful, and safe disk usage analyzer with integrated file deletion capabilities written in Rust.

## Features

### Core Functionality
- 📊 **Recursive directory scanning** - Quickly analyze disk usage across nested directories
- 🎨 **Colorful TUI** - Color-coded display by file size (red for large, green for small)
- 📈 **Live progress tracking** - Watch deletion progress in real-time with percentage bars
- 🔒 **Safer deletion** - Double-confirmation dialogs before destructive operations
- 🏃 **Non-blocking delete** - Continue browsing while files are being deleted in background
- 📋 **Dry-run mode** - Preview what would be deleted without actually deleting
- 📝 **Audit logging** - JSON logs of all deletions saved to `~/.mcdu/logs/`

### User Experience
- ⬆️⬇️ **Arrow key navigation** - Intuitive up/down navigation with vim keybindings (j/k)
- ← → **Modal navigation** - Use arrow keys to select buttons in confirmation dialogs
- 🎯 **Smart selection** - Default to "Cancel" on final confirmation for safety
- ⏱️ **Auto-dismiss notifications** - Notifications disappear after 3 seconds
- 🖥️ **Cross-platform** - Works on macOS and Linux

## Installation

```bash
# Build from source
cargo build --release

# Run
./target/release/mcdu
```

## Usage

### Navigation
- `↑/k` - Move cursor up
- `↓/j` - Move cursor down
- `Enter/→/l` - Enter directory
- `Backspace/←/h` - Go to parent directory
- `d` - Delete selected file/directory
- `r` - Refresh current view
- `?` - Help (placeholder)
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

## UI Design

### Color Coding
- 🔴 **Red** - Files >100 GB
- 🟡 **Yellow** - Files >10 GB
- 🔵 **Cyan** - Files >1 GB
- 🟢 **Green** - Files <1 GB

### Layout
```
┌─────────────────────────────────────────────────────────┐
│ 📊 mcdu - Disk Usage & Safe Delete Tool                │
├─────────────────────────────────────────────────────────┤
│ Path: /Users/username/Projects                         │
│                                                         │
│ 📁 node_modules           123.4 GB  ▓▓▓▓▓░░░░░░       │
│ 📁 .git                    45.2 GB  ▓▓▓░░░░░░░░       │
│ 📁 target                  12.1 GB  ▓░░░░░░░░░░       │
│ 📄 large-file.iso           2.3 GB  ░░░░░░░░░░░       │
│                                                         │
├─────────────────────────────────────────────────────────┤
│ [↑↓] Navigate  [d] Delete  [r] Refresh  [q] Quit       │
└─────────────────────────────────────────────────────────┘
```

## Delete Progress Screen
```
┌─────────────────────────────────────────────────────────┐
│ 🗑️  Deleting...                                         │
│                                                         │
│ Progress: [████████░░░░░░░░░░░░] 45% done             │
│                                                         │
│ Deleted: 45.2 GB / 123.4 GB (2,143 files)             │
│ Speed: ~15 MB/s                                        │
│ Current: node_modules/.bin/webpack                      │
│ ETA: ~1m 30s                                           │
│                                                         │
│ [c] Cancel deletion                                     │
└─────────────────────────────────────────────────────────┘
```

## Confirmation Dialog
Modal-based confirmation with arrow key navigation:

```
┌────────────────────────────────────────┐
│ Delete node_modules (123.4 GB)?        │
│ This cannot be undone!                 │
│                                        │
│   [ Yes ]      No      [d] Dry-run     │ ← Navigate with ← →
└────────────────────────────────────────┘
```

## Logging

All deletions and dry-runs are logged to `~/.mcdu/logs/delete-YYYY-MM-DD.log`

### Log Format (JSON Lines)
```json
{
  "timestamp": "2025-11-08T14:23:45Z",
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

## Architecture

### Module Structure
```
src/
├── main.rs          # Event loop and input handling
├── app.rs           # Application state and logic
├── ui.rs            # TUI rendering with ratatui
├── scan.rs          # Directory scanning logic
├── delete.rs        # File deletion implementation
├── modal.rs         # Modal dialog system
├── platform.rs      # Platform-specific (APFS, Linux)
└── logger.rs        # JSON logging
```

### Key Design Decisions

1. **Background Threading** - Delete operations run in background thread via channel communication
2. **Non-blocking UI** - Ratatui event loop continues even during deletion
3. **Safe Defaults** - Final confirm defaults to "Cancel" button to prevent accidental deletes
4. **JSON Logging** - Structured logs for easy parsing and auditing

## Dependencies

- **ratatui** - Terminal UI framework
- **crossterm** - Terminal control
- **walkdir** - Recursive directory traversal
- **serde/serde_json** - JSON serialization
- **chrono** - Timestamp handling
- **clap** - CLI argument parsing
- **nix** - Unix system calls

## Platform Support

- ✅ **macOS** - Full support with APFS compatibility
- ✅ **Linux** - Full support (ext4, btrfs, etc.)
- ❌ **Windows** - Not currently supported

## Performance

- Scanning large directories: ~1-2 seconds for 100k files
- Deletion: Parallelized with rayon (future enhancement)
- Memory: Efficient streaming with walkdir, minimal memory footprint

## Future Enhancements

- [ ] Parallel deletion using rayon
- [ ] APFS snapshot handling on macOS
- [ ] SELinux attribute handling on Linux
- [ ] Undo functionality
- [ ] Search/filter capabilities
- [ ] Sorting options (by size, date, name)
- [ ] Configuration file support
- [ ] Windows support

## License

MIT

## Contributing

Pull requests welcome! Please ensure:
1. Code compiles without warnings
2. Tests pass
3. Changes are well-documented

## Building from Source

```bash
# Debug build
cargo build

# Release build (optimized)
cargo build --release

# Run tests
cargo test

# Run with RUST_LOG=debug
RUST_LOG=debug cargo run
```

## Known Issues

- Notification timeout is currently fixed at 3 seconds (not configurable)
- Modal buttons don't support mouse clicks (keyboard only)
- Very large directories (>1M files) may cause UI lag during initial scan

## Support

For issues or feature requests, please open an issue on GitHub.
