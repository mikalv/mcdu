use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

#[derive(Clone)]
pub struct CachedSize {
    pub size: u64,
    pub mtime: SystemTime,
}

#[derive(Clone)]
pub struct SizeCache {
    cache: Arc<Mutex<HashMap<PathBuf, CachedSize>>>,
}

impl SizeCache {
    pub fn new() -> Self {
        SizeCache {
            cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn get(&self, path: &PathBuf) -> Option<u64> {
        let cache = self.cache.lock().unwrap();

        if let Some(cached) = cache.get(path) {
            // Validate cache by checking directory mtime
            if let Ok(metadata) = std::fs::metadata(path) {
                if let Ok(current_mtime) = metadata.modified() {
                    // If directory hasn't been modified, cache is still valid
                    if current_mtime == cached.mtime {
                        return Some(cached.size);
                    }
                }
            }
        }

        None
    }

    pub fn set(&self, path: PathBuf, size: u64) {
        let mut cache = self.cache.lock().unwrap();

        // Get directory mtime for validation
        if let Ok(metadata) = std::fs::metadata(&path) {
            if let Ok(mtime) = metadata.modified() {
                cache.insert(path, CachedSize { size, mtime });
            }
        }
    }

    pub fn clear(&self) {
        let mut cache = self.cache.lock().unwrap();
        cache.clear();
    }

    pub fn size(&self) -> usize {
        let cache = self.cache.lock().unwrap();
        cache.len()
    }

    #[allow(dead_code)]
    pub fn invalidate(&self, path: &PathBuf) {
        let mut cache = self.cache.lock().unwrap();
        cache.remove(path);
    }
}
