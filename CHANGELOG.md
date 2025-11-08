# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Initial release of mcdu
- Directory scanning with recursive size calculation
- Terminal UI with ratatui
- Safe deletion with double-confirmation dialogs
- Dry-run mode for preview deletions
- Change detection using bitmap-based fingerprinting
- JSON logging of all deletions
- Vim-style keyboard navigation (hjkl)
- Color-coded display by file size
- Real-time deletion progress tracking
- Background thread deletion
- Help screen with keybindings
- GitHub Actions CI/CD pipeline
- Automatic release builds for macOS and Linux

### Performance
- Responsive UI with ~100ms event polling
- Fast directory scanning with parallel processing
- Quick size estimation for large directories (>100MB)
- Optimized fingerprint loading without redundant filesystem calls

### Platform Support
- macOS (x86_64, aarch64)
- Linux (x86_64)

### Keyboard Shortcuts
- `↑/k` - Navigate up
- `↓/j` - Navigate down
- `Enter/→/l` - Open directory
- `Backspace/←/h` - Go to parent
- `d` - Delete selected item
- `r` - Refresh
- `?` - Show help
- `q/Esc` - Quit

## Future Plans

### v0.2.0
- [ ] Windows support
- [ ] Configuration file support
- [ ] Search/filter functionality
- [ ] Custom sorting options

### v0.3.0
- [ ] Undo functionality
- [ ] Mouse support
- [ ] APFS snapshot handling
- [ ] SELinux attribute support

### v0.4.0+
- [ ] Parallel deletion optimization
- [ ] Progress estimation
- [ ] Network filesystem detection
- [ ] Exclude patterns
