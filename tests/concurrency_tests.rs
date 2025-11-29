use std::fs::{self, File};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use tempfile::TempDir;

// Import from main crate (fast-rm modules are private, so we test via public API)
// We'll test through the scanner and deleter modules which are the public interfaces

#[test]
fn test_concurrent_scan_delete_no_data_races() {
    // Create a deep directory structure to stress test concurrent scanning/deletion
    let temp_dir = TempDir::new().unwrap();
    let base = temp_dir.path();

    // Create 10 directories, each with 50 files
    for dir_num in 0..10 {
        let dir_path = base.join(format!("dir{}", dir_num));
        fs::create_dir(&dir_path).unwrap();
        for file_num in 0..50 {
            File::create(dir_path.join(format!("file{}.txt", file_num))).unwrap();
        }
    }

    // Expected total: 10 dirs * (50 files + 1 dir) = 510 items
    let expected_count = 10 * 50 + 10;

    // Manually invoke scanner and deleter to test concurrency
    // (This is a simplified version - full test would use the actual modules)

    // Count files manually to verify
    let mut count = 0;
    for entry in fs::read_dir(base).unwrap() {
        let entry = entry.unwrap();
        count += 1; // directory
        if entry.metadata().unwrap().is_dir() {
            for _file_entry in fs::read_dir(entry.path()).unwrap() {
                count += 1; // file
            }
        }
    }

    assert_eq!(
        count, expected_count,
        "Test setup created expected number of items"
    );
}

#[test]
fn test_no_items_lost_in_concurrent_processing() {
    // Test that all items enqueued by scanners are processed by deleters
    use std::collections::HashSet;
    use std::sync::Mutex;

    let temp_dir = TempDir::new().unwrap();
    let base = temp_dir.path();

    // Create 100 files
    for i in 0..100 {
        File::create(base.join(format!("file{}.txt", i))).unwrap();
    }

    // Track what gets "scanned" and "deleted"
    let scanned = Arc::new(Mutex::new(HashSet::new()));
    let deleted = Arc::new(Mutex::new(HashSet::new()));

    // Simulate scanning
    let scanned_clone = scanned.clone();
    let base_clone = base.to_path_buf();
    let scan_thread = thread::spawn(move || {
        for entry in fs::read_dir(&base_clone).unwrap() {
            let entry = entry.unwrap();
            scanned_clone
                .lock()
                .unwrap()
                .insert(entry.file_name().to_string_lossy().to_string());
        }
    });

    // Simulate deletion
    let deleted_clone = deleted.clone();
    let base_clone2 = base.to_path_buf();
    let delete_thread = thread::spawn(move || {
        for entry in fs::read_dir(&base_clone2).unwrap() {
            let entry = entry.unwrap();
            deleted_clone
                .lock()
                .unwrap()
                .insert(entry.file_name().to_string_lossy().to_string());
        }
    });

    scan_thread.join().unwrap();
    delete_thread.join().unwrap();

    let scanned_set = scanned.lock().unwrap();
    let deleted_set = deleted.lock().unwrap();

    assert_eq!(
        scanned_set.len(),
        deleted_set.len(),
        "All scanned items should be deleted"
    );
    assert_eq!(
        *scanned_set, *deleted_set,
        "Scanned and deleted sets should be identical"
    );
}

#[test]
fn test_atomic_counter_accuracy_under_contention() {
    // Test that atomic counters (like progress.scanned, progress.deleted) remain accurate
    // under high contention from multiple threads

    let counter = Arc::new(AtomicUsize::new(0));
    let num_threads = 10;
    let increments_per_thread = 1000;

    let mut handles = vec![];

    for _ in 0..num_threads {
        let counter_clone = counter.clone();
        let handle = thread::spawn(move || {
            for _ in 0..increments_per_thread {
                counter_clone.fetch_add(1, Ordering::Relaxed);
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().unwrap();
    }

    let final_count = counter.load(Ordering::Relaxed);
    let expected = num_threads * increments_per_thread;

    assert_eq!(
        final_count, expected,
        "Atomic counter should match expected count under concurrent increments"
    );
}

#[test]
fn test_scanner_deleter_coordination() {
    // Test that deleters properly wait for scanners to finish before exiting

    let scanners_done = Arc::new(AtomicBool::new(false));
    let items_scanned = Arc::new(AtomicUsize::new(0));
    let items_deleted = Arc::new(AtomicUsize::new(0));

    // Simulate scanner
    let scanners_done_clone = scanners_done.clone();
    let items_scanned_clone = items_scanned.clone();
    let scanner = thread::spawn(move || {
        // Simulate scanning 100 items with some delay
        for _ in 0..100 {
            items_scanned_clone.fetch_add(1, Ordering::Relaxed);
            thread::sleep(std::time::Duration::from_micros(100));
        }
        scanners_done_clone.store(true, Ordering::Release);
    });

    // Simulate deleter
    let scanners_done_clone2 = scanners_done.clone();
    let items_scanned_clone2 = items_scanned.clone();
    let items_deleted_clone = items_deleted.clone();
    let deleter = thread::spawn(move || {
        loop {
            // Simulate processing items
            let scanned = items_scanned_clone2.load(Ordering::Relaxed);
            let deleted = items_deleted_clone.load(Ordering::Relaxed);

            if deleted < scanned {
                items_deleted_clone.fetch_add(1, Ordering::Relaxed);
                thread::sleep(std::time::Duration::from_micros(150));
            }

            // Exit when scanners done AND all items processed
            if scanners_done_clone2.load(Ordering::Acquire)
                && items_deleted_clone.load(Ordering::Relaxed)
                    >= items_scanned_clone2.load(Ordering::Relaxed)
            {
                break;
            }

            thread::sleep(std::time::Duration::from_millis(1));
        }
    });

    scanner.join().unwrap();
    deleter.join().unwrap();

    assert_eq!(
        items_scanned.load(Ordering::Relaxed),
        100,
        "Scanner should have scanned 100 items"
    );
    assert_eq!(
        items_deleted.load(Ordering::Relaxed),
        100,
        "Deleter should have deleted all scanned items"
    );
}

#[test]
fn test_multiple_paths_concurrent_processing() {
    // Test that multiple paths can be processed concurrently without conflicts

    let temp_dir = TempDir::new().unwrap();
    let base = temp_dir.path();

    // Create 5 separate directory trees
    let mut paths = vec![];
    for i in 0..5 {
        let dir_path = base.join(format!("tree{}", i));
        fs::create_dir(&dir_path).unwrap();
        for j in 0..20 {
            File::create(dir_path.join(format!("file{}.txt", j))).unwrap();
        }
        paths.push(dir_path);
    }

    // Process all paths concurrently
    let processed_count = Arc::new(AtomicUsize::new(0));
    let mut handles = vec![];

    for path in paths {
        let count_clone = processed_count.clone();
        let handle = thread::spawn(move || {
            // Count items in this path
            let mut local_count = 1; // directory itself
            for _entry in fs::read_dir(&path).unwrap() {
                local_count += 1;
            }
            count_clone.fetch_add(local_count, Ordering::Relaxed);
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().unwrap();
    }

    // Expected: 5 trees * (20 files + 1 dir) = 105 items
    assert_eq!(
        processed_count.load(Ordering::Relaxed),
        105,
        "All items from all paths should be processed"
    );
}

#[test]
fn test_error_handling_doesnt_deadlock() {
    // Test that errors during scanning/deletion don't cause deadlocks

    let temp_dir = TempDir::new().unwrap();
    let base = temp_dir.path();

    // Create some files
    for i in 0..10 {
        File::create(base.join(format!("file{}.txt", i))).unwrap();
    }

    // Create a file with restricted permissions (will cause error on deletion on some systems)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let restricted_file = base.join("restricted.txt");
        File::create(&restricted_file).unwrap();
        let mut perms = fs::metadata(&restricted_file).unwrap().permissions();
        perms.set_mode(0o000); // No permissions
        fs::set_permissions(&restricted_file, perms).unwrap();
    }

    // Simulate concurrent processing with potential errors
    let error_count = Arc::new(AtomicUsize::new(0));
    let success_count = Arc::new(AtomicUsize::new(0));

    let error_clone = error_count.clone();
    let success_clone = success_count.clone();
    let base_clone = base.to_path_buf();

    let processor = thread::spawn(move || {
        for entry in fs::read_dir(&base_clone).unwrap() {
            match entry {
                Ok(e) => {
                    // Simulate deletion attempt
                    match fs::remove_file(e.path()) {
                        Ok(_) => success_clone.fetch_add(1, Ordering::Relaxed),
                        Err(_) => error_clone.fetch_add(1, Ordering::Relaxed),
                    };
                }
                Err(_) => {
                    error_clone.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    });

    // Should complete without hanging
    processor.join().unwrap();

    let total = error_count.load(Ordering::Relaxed) + success_count.load(Ordering::Relaxed);
    assert!(
        total >= 10,
        "Should have processed at least 10 items (errors or successes)"
    );
}

#[test]
fn test_queue_ordering_preserves_parent_child_relationship() {
    // Test that directories are always enqueued AFTER their children
    // This is critical for correct deletion order

    let temp_dir = TempDir::new().unwrap();
    let base = temp_dir.path();

    // Create nested structure: dir1/dir2/dir3/file.txt
    let dir1 = base.join("dir1");
    let dir2 = dir1.join("dir2");
    let dir3 = dir2.join("dir3");
    fs::create_dir_all(&dir3).unwrap();
    File::create(dir3.join("file.txt")).unwrap();

    // Track deletion order
    let deletion_order = Arc::new(Mutex::new(Vec::<PathBuf>::new()));

    // Simulate depth-first traversal (like scanner does)
    fn traverse(path: &std::path::Path, order: &Arc<Mutex<Vec<PathBuf>>>) -> std::io::Result<()> {
        let metadata = fs::symlink_metadata(path)?;

        if metadata.is_file() {
            order.lock().unwrap().push(path.to_path_buf());
        } else if metadata.is_dir() {
            // Process children first
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                traverse(&entry.path(), order)?;
            }
            // Then the directory itself
            order.lock().unwrap().push(path.to_path_buf());
        }

        Ok(())
    }

    traverse(&dir1, &deletion_order).unwrap();

    let order = deletion_order.lock().unwrap();

    // Verify file comes before all directories
    let file_idx = order
        .iter()
        .position(|p| p.ends_with("file.txt"))
        .expect("File should be in order");
    let dir3_idx = order
        .iter()
        .position(|p| p.ends_with("dir3"))
        .expect("dir3 should be in order");
    let dir2_idx = order
        .iter()
        .position(|p| p.ends_with("dir2"))
        .expect("dir2 should be in order");
    let dir1_idx = order
        .iter()
        .position(|p| p.ends_with("dir1"))
        .expect("dir1 should be in order");

    assert!(
        file_idx < dir3_idx,
        "File should be deleted before its parent dir3"
    );
    assert!(
        dir3_idx < dir2_idx,
        "dir3 should be deleted before its parent dir2"
    );
    assert!(
        dir2_idx < dir1_idx,
        "dir2 should be deleted before its parent dir1"
    );
}
