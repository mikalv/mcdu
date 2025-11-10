// Platform-specific features
use std::path::Path;

#[derive(Debug, Clone)]
pub struct DiskSpace {
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub used_bytes: u64,
}

/// Get disk space information for the filesystem containing the given path
/// Works on both macOS (APFS, HFS+, etc.) and Linux (ext4, btrfs, xfs, etc.)
#[cfg(unix)]
pub fn get_disk_space(path: &Path) -> Option<DiskSpace> {
    use nix::sys::statvfs::statvfs;

    match statvfs(path) {
        Ok(stat) => {
            // IMPORTANT: Use fragment_size() (f_frsize), NOT block_size() (f_bsize)!
            // f_frsize is the fundamental filesystem block size (usually 4KB)
            // f_bsize is the preferred I/O block size (can be 1MB on APFS, giving wrong results!)
            let fragment_size = stat.fragment_size() as u64;
            let total_blocks = stat.blocks() as u64;
            let available_blocks = stat.blocks_available() as u64;

            let total_bytes = total_blocks * fragment_size;
            let available_bytes = available_blocks * fragment_size;
            let used_bytes = total_bytes.saturating_sub(available_bytes);

            Some(DiskSpace {
                total_bytes,
                available_bytes,
                used_bytes,
            })
        }
        Err(_) => None,
    }
}

#[cfg(not(unix))]
pub fn get_disk_space(_path: &Path) -> Option<DiskSpace> {
    // Windows support could be added here using GetDiskFreeSpaceEx
    None
}

#[cfg(target_os = "macos")]
pub mod _macos {
    // TODO: Future macOS-specific features:
    // - APFS snapshot removal
    // - Extended attributes (xattr) cleanup
    // - ACL handling
}

#[cfg(target_os = "linux")]
pub mod _linux {
    // TODO: Future Linux-specific features:
    // - Immutable flag removal (chattr -i)
    // - SELinux context handling
    // - Extended attributes cleanup
}
