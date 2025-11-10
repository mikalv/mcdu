# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2025-01-10

### Added
- **Async scanning** - Non-blocking directory scanning in background thread
- **Smart caching** - Thread-safe size cache with mtime-based validation
- **Cache management** - `c` key to clear cache and force rescan
- **Live scanning progress** - Real-time progress overlay showing current file and percentage
- **Disk space monitoring** - Title bar shows available/total disk space
- **Viewport scrolling** - Automatic scrolling keeps selected item visible
- **Loading overlays** - Solid background overlays prevent UI bleed-through
- **Platform-specific optimizations** - Correct APFS disk space using `f_frsize`

### Performance
- **3x faster deletions** - Single-pass deletion algorithm (one walk instead of three)
- **Instant navigation** - Cached directories load in <10ms
- **No UI freezing** - All I/O operations run in background threads
- **Optimized metadata** - Reuse stat() results, no redundant calls
- **Memory efficient** - Only caches directory sizes, not full trees

### Fixed
- **APFS disk space bug** - Was showing 256x too large values due to using `f_bsize` instead of `f_frsize`
- **UI z-ordering** - Fixed file list bleeding through dialogs
- **Progress display** - Now shows relative names instead of full paths
- **Scroll behavior** - Selected items no longer disappear off-screen

### Changed
- Version bumped from 0.1.0 to 0.2.0
- `r` now uses cache (fast refresh)
- `c` added for cache-clearing hard refresh

## [0.1.0] - 2025-01-08

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

### v0.3.0
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
