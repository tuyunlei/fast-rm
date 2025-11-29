use crossbeam_channel::{bounded, Receiver, Sender};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::config::Verbosity;

pub struct RemoveProgress {
    pub scanned: AtomicUsize,
    _pad1: [u8; 56], // 64 - 8 bytes = 56 (cache line padding)
    pub deleted: AtomicUsize,
    _pad2: [u8; 56],
    pub errors: AtomicUsize,
    _pad3: [u8; 56],
    recent_tx: Sender<Arc<Path>>,
    pub recent_rx: Receiver<Arc<Path>>,
    error_tx: Sender<(Arc<Path>, String)>,
    pub error_rx: Receiver<(Arc<Path>, String)>,
    start_time: Instant,
}

impl std::fmt::Debug for RemoveProgress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoveProgress")
            .field("scanned", &self.scanned)
            .field("deleted", &self.deleted)
            .field("errors", &self.errors)
            .field("start_time", &self.start_time)
            .finish()
    }
}

impl RemoveProgress {
    pub fn new() -> Arc<Self> {
        let (recent_tx, recent_rx) = bounded(1000);
        let (error_tx, error_rx) = bounded(100);

        Arc::new(Self {
            scanned: AtomicUsize::new(0),
            _pad1: [0; 56],
            deleted: AtomicUsize::new(0),
            _pad2: [0; 56],
            errors: AtomicUsize::new(0),
            _pad3: [0; 56],
            recent_tx,
            recent_rx,
            error_tx,
            error_rx,
            start_time: Instant::now(),
        })
    }

    pub fn inc_scanned(&self) {
        self.scanned.fetch_add(1, Ordering::Relaxed);
    }
    pub fn inc_deleted(&self, path: &Path) {
        self.deleted.fetch_add(1, Ordering::Relaxed);
        // Non-blocking send, drops if channel full (acceptable for display)
        // Create Arc once instead of cloning PathBuf multiple times
        let _ = self.recent_tx.try_send(Arc::from(path));
    }
    pub fn inc_error(&self, path: &Path, error: String) {
        self.errors.fetch_add(1, Ordering::Relaxed);
        // Non-blocking send, drops if channel full (acceptable for display)
        // Create Arc once instead of cloning PathBuf
        let _ = self.error_tx.try_send((Arc::from(path), error));
    }

    pub fn get_stats(&self) -> (usize, usize, usize, f64, f64) {
        let scanned = self.scanned.load(Ordering::Relaxed);
        let deleted = self.deleted.load(Ordering::Relaxed);
        let errors = self.errors.load(Ordering::Relaxed);
        let elapsed = self.start_time.elapsed().as_secs_f64();
        let speed = if elapsed > 0.0 {
            deleted as f64 / elapsed
        } else {
            0.0
        };
        let eta = if speed > 0.0 && scanned > deleted {
            (scanned - deleted) as f64 / speed
        } else {
            0.0
        };
        (scanned, deleted, errors, speed, eta)
    }

    pub fn get_recent_files(&self) -> Vec<Arc<Path>> {
        let mut files = Vec::new();
        while let Ok(path) = self.recent_rx.try_recv() {
            files.push(path);
            // Keep only last 50 items
            if files.len() > 50 {
                files.remove(0);
            }
        }
        files
    }
    pub fn get_error_files(&self) -> Vec<(Arc<Path>, String)> {
        let mut errors = Vec::new();
        while let Ok(error) = self.error_rx.try_recv() {
            errors.push(error);
            // Keep only last 50 items
            if errors.len() > 50 {
                errors.remove(0);
            }
        }
        errors
    }
}

pub struct ProgressDisplay {
    multi: MultiProgress,
    main_bar: ProgressBar,
    file_bars: Vec<ProgressBar>,
    error_bar: Option<ProgressBar>,
    verbosity: Verbosity,
    // TUI-local caches to avoid allocating Vec on every update
    recent_cache: Mutex<std::collections::VecDeque<Arc<Path>>>,
    error_cache: Mutex<std::collections::VecDeque<(Arc<Path>, String)>>,
}

impl ProgressDisplay {
    pub fn new(verbosity: Verbosity, dry_run: bool) -> Self {
        let multi = MultiProgress::new();
        let main_bar = multi.add(ProgressBar::new_spinner());
        let template = if dry_run {
            "[Dry Run] Scanned: {msg}"
        } else {
            "Deleted: {msg}"
        };
        main_bar.set_style(ProgressStyle::default_spinner().template(template).unwrap());

        let mut file_bars = Vec::new();
        let mut error_bar = None;

        match verbosity {
            Verbosity::Simple => {}
            Verbosity::Standard => {
                for _ in 0..10 {
                    let bar = multi.add(ProgressBar::new_spinner());
                    bar.set_style(
                        ProgressStyle::default_spinner()
                            .template("  {msg}")
                            .unwrap(),
                    );
                    file_bars.push(bar);
                }
            }
            Verbosity::Detailed => {
                let height = crossterm::terminal::size()
                    .map(|(_, h)| h as usize)
                    .unwrap_or(24);
                let file_count = (height.saturating_sub(5)).min(50).max(5);
                for _ in 0..file_count {
                    let bar = multi.add(ProgressBar::new_spinner());
                    bar.set_style(
                        ProgressStyle::default_spinner()
                            .template("  {msg}")
                            .unwrap(),
                    );
                    file_bars.push(bar);
                }
            }
        }

        let err_bar = multi.add(ProgressBar::new_spinner());
        err_bar.set_style(ProgressStyle::default_spinner().template("{msg}").unwrap());
        error_bar = Some(err_bar);

        Self {
            multi,
            main_bar,
            file_bars,
            error_bar,
            verbosity,
            recent_cache: Mutex::new(std::collections::VecDeque::new()),
            error_cache: Mutex::new(std::collections::VecDeque::new()),
        }
    }

    pub fn update(&self, progress: &RemoveProgress, dry_run: bool) {
        let (scanned, deleted, errors, speed, _eta) = progress.get_stats();
        let main_msg = if dry_run {
            format!(
                "{} scanned | {} errors | {:.1} items/s",
                scanned, errors, speed
            )
        } else {
            format!(
                "{} deleted | {} errors | {:.1} items/s",
                deleted, errors, speed
            )
        };
        self.main_bar.set_message(main_msg);

        if !self.file_bars.is_empty() {
            // Drain new files from channel into local cache
            let mut cache = self.recent_cache.lock().unwrap();
            while let Ok(path) = progress.recent_rx.try_recv() {
                cache.push_back(path);
                // Keep only last 50 items
                while cache.len() > 50 {
                    cache.pop_front();
                }
            }

            // Display recent files from cache (no allocation)
            let display_count = self.file_bars.len().min(cache.len());
            for (i, bar) in self.file_bars.iter().enumerate() {
                if i < display_count {
                    let file = &cache[cache.len() - display_count + i];
                    bar.set_message(format!("{:?}", file));
                } else {
                    bar.set_message("");
                }
            }
        }

        if let Some(err_bar) = &self.error_bar {
            if errors > 0 {
                // Drain new errors from channel into local cache
                let mut cache = self.error_cache.lock().unwrap();
                while let Ok(error) = progress.error_rx.try_recv() {
                    cache.push_back(error);
                    // Keep only last 50 items
                    while cache.len() > 50 {
                        cache.pop_front();
                    }
                }

                // Display last error from cache (no allocation)
                if let Some((path, msg)) = cache.back() {
                    err_bar.set_message(format!("Last error: {:?} - {}", path, msg));
                }
            } else {
                err_bar.set_message("");
            }
        }
    }

    pub fn finish(&self, progress: &RemoveProgress, dry_run: bool) {
        let (scanned, deleted, errors, _, _) = progress.get_stats();
        let final_msg = if dry_run {
            format!(
                "✓ Dry run complete: {} items scanned, {} errors",
                scanned, errors
            )
        } else {
            format!("✓ Complete: {} items deleted, {} errors", deleted, errors)
        };
        self.main_bar.finish_with_message(final_msg);
        for bar in &self.file_bars {
            bar.finish_and_clear();
        }
        if errors == 0 {
            if let Some(err_bar) = &self.error_bar {
                err_bar.finish_and_clear();
            }
        }
    }
}

pub struct TuiHandle {
    pub is_done: Arc<AtomicBool>,
}
