//! Cache-first navigation optimization for instant folder browsing
//!
//! This module provides utilities to implement cache-first strategy:
//! 1. Check directory cache first (0ms latency)
//! 2. Return cached results immediately
//! 3. Background revalidation on cache hits
//! 4. Fallback to disk reading only on cache miss

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use crate::domain::file_entry::FileEntry;
use crate::infrastructure::directory_cache::DirectoryCache;

/// Implements cache-first strategy for directory navigation
/// 
/// Returns cached entries immediately if available, enabling
/// instant navigation for previously visited folders.
pub fn get_cached_directory(
    directory_cache: &Arc<DirectoryCache>,
    path: &PathBuf,
) -> Option<Vec<FileEntry>> {
    let start = Instant::now();
    
    // Try to get from cache first
    if let Some(cached_entries) = directory_cache.get(path) {
        eprintln!("[CACHE-FIRST] Hit for {:?} - {} entries ({}μs)", 
            path, cached_entries.len(), start.elapsed().as_micros());
        return Some(cached_entries);
    }
    
    eprintln!("[CACHE-FIRST] Miss for {:?} ({}μs)", path, start.elapsed().as_micros());
    None
}

/// Background revalidation check for cached directories
/// 
/// Spawns a lightweight background task to check if directory
/// contents have changed since caching.
pub fn trigger_background_revalidation(
    directory_cache: &Arc<DirectoryCache>,
    path: &PathBuf,
) {
    let cache_clone = directory_cache.clone();
    let path_clone = path.clone();
    
    std::thread::spawn(move || {
        // Check if directory modification time changed
        if let Ok(_current_mtime) = std::fs::metadata(&path_clone)
            .and_then(|meta| meta.modified()) 
        {
            // Simple heuristic: if we can read metadata, directory might have changed
            // More sophisticated checks could compare actual modification times
            eprintln!("[CACHE-REVALIDATION] Directory {:?} accessible - may need refresh", path_clone);
            
            // Invalidate cache entry to force refresh on next navigation
            // This is conservative but ensures we don't serve stale data
            cache_clone.invalidate(&path_clone);
        }
    });
}

/// Cache statistics for monitoring
pub fn get_cache_stats(directory_cache: &Arc<DirectoryCache>) -> (usize, usize) {
    directory_cache.stats()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    
    #[test]
    fn test_cache_first_miss() {
        let cache = Arc::new(DirectoryCache::new());
        let path = PathBuf::from("C:\\test");
        
        // Should be miss on empty cache
        assert!(get_cached_directory(&cache, &path).is_none());
    }
    
    #[test]
    fn test_cache_first_hit() {
        let cache = Arc::new(DirectoryCache::new());
        let path = PathBuf::from("C:\\test");
        
        // Add some dummy entries to cache
        let entries = vec![];
        cache.put(path.clone(), entries.clone());
        
        // Should be hit now
        let cached = get_cached_directory(&cache, &path);
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().len(), 0);
    }
    
    #[test] 
    fn test_background_revalidation() {
        let cache = Arc::new(DirectoryCache::new());
        let path = PathBuf::from("C:\\nonexistent"); // Won't actually check since path doesn't exist
        
        // Should not panic even with invalid path
        trigger_background_revalidation(&cache, &path);
        
        // Give background thread time to complete
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}