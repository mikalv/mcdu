# mcdu - Features & Capabilities

## ✨ **Change Detection System**

### How It Works
- **Fingerprinting**: Scans each directory once and saves a lightweight "fingerprint" to `~/.mcdu/cache/`
- **Bitmap-based**: Stores only `name:size:mtime` (never stores full paths)
- **Smart Comparison**: On next visit, compares old vs new fingerprint
- **Visual Indicators**: Shows size deltas with `⬆ X%` (grew) or `⬇ X%` (shrunk)

### Use Cases
1. **Track growing directories**
   - `node_modules` grew from 100GB to 145GB? Shows `⬆ 45%`
   - Instantly spot directories that need cleanup

2. **Monitor cache directories**
   - See which build caches are shrinking after cleanup
   - Shows `⬇ 20%` if cache reduced since last run

3. **Find unexpected growth**
   - `.git` directory doubled in size? `⬆ 100%` immediately visible
   - No manual note-taking needed

### Color Coding
- **Yellow text/highlight**: Directory **grew** since last scan `⬆`
- **Cyan text/highlight**: Directory **shrunk** since last scan `⬇`
- **Normal**: No change since last scan

---

## 🎯 **Core Features**

### Directory Scanning
- ✅ Recursive size calculation
- ✅ Fast traversal with `walkdir`
- ✅ Item counting for subdirectories
- ✅ Real-time size updates

### Safe Deletion
- ✅ Double-confirmation dialog
- ✅ Final "YES, DELETE" confirmation
- ✅ Dry-run mode to preview deletions
- ✅ Background thread deletion with progress bar
- ✅ Automatic logging to JSON

### User Interface
- ✅ Colorful terminal UI with ratatui
- ✅ Size-based color coding (red=large, green=small)
- ✅ Animated progress bars
- ✅ Modal dialogs with arrow-key navigation
- ✅ Help screen with all keybindings
- ✅ Dynamic title bar showing current path

### Navigation
- ✅ Vim-style keybindings (hjkl)
- ✅ Arrow key navigation
- ✅ Parent directory (..) entry
- ✅ Quick jump to parent with backspace

### Keyboard Shortcuts
```
Navigation:
  ↑/↓/j/k      Navigate up/down
  Enter/→/l    Open directory
  Backspace/←/h Go to parent
  h/l/←/→      Arrow keys work too

Deletion:
  d            Delete selected item
  y/n/d        Quick confirm (yes/no/dry-run)
  Tab/←→       Navigate modal buttons

General:
  r            Refresh current directory
  ?            Show help screen
  q/Esc        Quit application
```

### Visual Indicators
```
Colors by Size:
  🔴 Red    = >100 GB    (massive)
  🟡 Yellow = 10-100 GB  (large)
  🔵 Cyan   = 1-10 GB    (medium)
  🟢 Green  = <1 GB      (small)

Change Indicators:
  📈 ⬆ 45%  = Size increased by 45%
  📉 ⬇ 20%  = Size decreased by 20%
```

---

## 📊 **Data Storage**

### Cache Files
Location: `~/.mcdu/cache/fp_*.txt`

Format (human-readable):
```
directory_name:size_in_bytes:modification_time
node_modules:132548901234:1699452345
.git:48373921024:1699451230
```

Benefits:
- ✅ No full paths stored (privacy)
- ✅ Compact format (few KB per directory)
- ✅ Human-readable for debugging
- ✅ Easy to clear if needed

### Log Files
Location: `~/.mcdu/logs/delete-YYYY-MM-DD.log`

Format (JSON Lines):
```json
{
  "timestamp": "2025-11-08T14:23:45Z",
  "action": "delete",
  "path": "/Users/username/node_modules",
  "size_bytes": 132548901234,
  "dry_run": false,
  "status": "success",
  "files_deleted": 45821,
  "duration_ms": 3421
}
```

---

## 🚀 **Performance**

- Scanning: ~1-2 seconds for 100k files
- Deletion: Multi-threaded (future optimization)
- Memory: Minimal footprint (no full path storage)
- Cache: Persists between runs

---

## 🔮 **Future Enhancements**

- [ ] Parallel deletion with rayon
- [ ] APFS snapshot handling (macOS)
- [ ] SELinux support (Linux)
- [ ] Search/filter functionality
- [ ] Custom sorting options
- [ ] Configuration file support
- [ ] Undo functionality (trash directory)
- [ ] Mouse support
- [ ] Windows support

---

## 📝 **Example Workflow**

**First Run:**
```
$ mcdu

📊 mcdu v0.1.0 | /Users/username/Projects    10 items

📁 node_modules         123.4 GB  ▓▓▓▓▓░░░░░░
📁 .git                  45.2 GB  ▓▓▓░░░░░░░░
📁 target                12.1 GB  ▓░░░░░░░░░░
```
*(No change indicators - first scan)*

**Second Run (after dev work):**
```
$ mcdu

📊 mcdu v0.1.0 | /Users/username/Projects    10 items

📁 node_modules         135.8 GB  ▓▓▓▓▓░░░░░░ ⬆ 10%
📁 .git                  45.2 GB  ▓▓▓░░░░░░░░
📁 target                 8.3 GB  ░░░░░░░░░░░ ⬇ 31%
```
*(Shows changes since last run)*

**Cleanup:**
```
Press 'd' on node_modules
↓
Modal: Delete node_modules (135.8 GB)?
[Yes] [No] [Dry-run]
↓
Second confirmation: FINAL CONFIRMATION - Delete node_modules?
[YES, DELETE] [Cancel]
↓
Progress bar shows deletion in background
✓ Deleted 45821 files (135.8 GB)
```

---

## 💡 **Tips & Tricks**

1. **Track specific directories**
   - Navigate into project folders regularly
   - Change indicators help spot growth patterns

2. **Use dry-run mode**
   - Press 'd' then select [Dry-run]
   - See what would be deleted without risk

3. **Check logs**
   - `cat ~/.mcdu/logs/delete-*.log | jq`
   - Full audit trail of all deletions

4. **Clear cache if needed**
   - `rm ~/.mcdu/cache/*`
   - Resets all change detection

5. **Integration**
   - Add alias: `alias mcdu='~/path/to/mcdu'`
   - Chain with other tools for automation
