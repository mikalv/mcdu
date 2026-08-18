# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.6.0] - 2025-08-18

### Added
- **`mcdu devclean` (alias `mcdu dc`)** — fast non-interactive cleanup of build artifacts (`_build` profiles, `target/*` except `release`, age-gated `node_modules`/`deps`, `__pycache__`, `.venv`, `dist`, `build`, `.next`, `.turbo`, `.parcel-cache`, `cmake-build-*`) with dry-run (`-n`), `-y`, `--force-age`, and `--all` (drop release builds) flags; settings in `~/.mcdu.toml` `[devclean]` (`min_age_days`, `keep_release`, `age_gate_node_modules`, `extra_dirs`, `extra_age_gated`, `max_depth`, `skip_dirs`)
- **`cleanup_command` on rules** — when set, cleanup runs the shell command (with `{path}` / `{dir}` templates) instead of quarantine/delete
- Fix pre-existing clippy warning (`manual_is_multiple_of` in tree.rs)

### Fixed
- Browser single-file and symlink deletion (no more false success / half-deleted trees)
- Cleanup empty selection no longer deletes everything
- Cleanup deletes go through quarantine with incremental manifests; Quarantine tab restore/purge works
- `git gc` is opt-in (`YES + git gc`), without `--prune=now`, errors reported
- `risky` rules propagate to candidates and are not auto-selected
- UTF-8-safe name/path truncation; panic hook + alternate screen; KeyEventKind::Press filter
- `replace_subtree` size propagation and nav_stack remap after sort; cancellable scans; idle redraw
- Files-tab selection/sort; Categories 2-line viewport scroll; cleanup without blocking tree splash
- Scanner: file dedup across rules, scan_path∩base_path walk roots, `matches()` uses resolved base
- Config: merge rules by name (user overrides defaults); atomic state writes
- macOS orphans: abort when mdfind returns 0 apps; plutil/plist bundle-id reads; multi-suffix strip + whitelist
- Esc no longer quits browser; help lists cleanup keys; bars scale to largest entry; data dir for logs

### Removed
- Unused TUI modules `cache.rs`, `scan.rs`, `changes.rs`

## [0.5.0]

See the [v0.5.0 release notes](https://github.com/mikalv/mcdu/releases/tag/v0.5.0).

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
