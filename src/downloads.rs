//! Offline chapter downloads from MangaDex.
//! Pages are downloaded in parallel (small worker pool) with live progress,
//! and can be cancelled at any moment (stop button / chapter deletion).
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::jobs::Job;
use crate::mangadex;

/// How many pages of one chapter are downloaded concurrently.
/// Scaled from CPU cores: total parallel requests ≈ number of cores.
fn pages_per_chapter() -> usize {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    (cores / max_active_chapters()).clamp(2, 4)
}

/// How many chapters are downloaded simultaneously.
/// Auto-detected from CPU: cores count, bounded to 5..=10.
pub fn max_active_chapters() -> usize {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    cores.clamp(5, 10)
}

/// Message returned when a download was cancelled by the user.
pub const CANCELLED: &str = "Остановлено";

/// Live download progress of one chapter.
#[derive(Clone, Copy, Debug)]
pub struct ChapterProgress {
    pub done: u32,
    pub total: u32,
    pub failed: bool,
}

impl ChapterProgress {
    pub fn frac(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            (self.done as f32 / self.total as f32).clamp(0.0, 1.0)
        }
    }
}

struct DownloadJob {
    manga_id: String,
    job: Job<Result<(), String>>,
}

pub struct Downloads {
    jobs: HashMap<String, DownloadJob>,
    queue: VecDeque<(String, String, u32)>, // (chapter_id, manga_id, total_pages)
    progress: Arc<Mutex<HashMap<String, ChapterProgress>>>,
    /// cancel flags for running background threads, keyed by chapter id
    cancels: HashMap<String, Arc<AtomicBool>>,
}

impl Default for Downloads {
    fn default() -> Self {
        Downloads {
            jobs: HashMap::new(),
            queue: VecDeque::new(),
            progress: Arc::new(Mutex::new(HashMap::new())),
            cancels: HashMap::new(),
        }
    }
}

pub fn chapter_dir(data_dir: &Path, chapter_id: &str) -> PathBuf {
    data_dir.join("downloads").join(chapter_id)
}

pub fn is_downloaded(data_dir: &Path, chapter_id: &str) -> bool {
    let dir = chapter_dir(data_dir, chapter_id);
    dir.is_dir()
        && std::fs::read_dir(&dir)
            .map(|d| d.count() > 0)
            .unwrap_or(false)
}

/// Number of saved page files for a chapter.
pub fn page_files(data_dir: &Path, chapter_id: &str) -> usize {
    let dir = chapter_dir(data_dir, chapter_id);
    if !dir.is_dir() {
        return 0;
    }
    std::fs::read_dir(&dir)
        .map(|d| {
            d.filter_map(|e| e.ok())
                .filter(|e| e.path().is_file())
                .count()
        })
        .unwrap_or(0)
}

/// True when the chapter has at least `expected` pages on disk.
pub fn is_complete(data_dir: &Path, chapter_id: &str, expected: u32) -> bool {
    expected > 0 && page_files(data_dir, chapter_id) >= expected as usize
}

impl Downloads {
    pub fn running(&self, chapter_id: &str) -> bool {
        self.jobs.contains_key(chapter_id)
    }

    pub fn queued(&self, chapter_id: &str) -> bool {
        self.queue.iter().any(|(id, _, _)| id == chapter_id)
    }

    pub fn pending(&self) -> usize {
        self.queue.len()
    }

    pub fn active(&self) -> usize {
        self.jobs.len()
    }

    /// Live progress of a chapter, if it is being downloaded right now.
    pub fn progress(&self, chapter_id: &str) -> Option<ChapterProgress> {
        self.progress
            .lock()
            .ok()
            .and_then(|m| m.get(chapter_id).copied())
    }

    pub fn start(&mut self, chapter_id: &str, manga_id: String, total_pages: u32, data_dir: PathBuf) {
        if self.jobs.contains_key(chapter_id) || self.queued(chapter_id) {
            return;
        }
        {
            let mut m = self.progress.lock().unwrap();
            m.insert(
                chapter_id.to_string(),
                ChapterProgress {
                    done: 0,
                    total: total_pages,
                    failed: false,
                },
            );
        }
        let id = chapter_id.to_string();
        let progress = self.progress.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancels.insert(id.clone(), cancel.clone());
        let par = pages_per_chapter();
        self.jobs.insert(
            id.clone(),
            DownloadJob {
                manga_id,
                job: Job::spawn(move || download_chapter(&id, &data_dir, progress, &cancel, par)),
            },
        );
    }

    pub fn enqueue(&mut self, chapter_id: String, manga_id: String, total_pages: u32) {
        if self.jobs.contains_key(&chapter_id) || self.queued(&chapter_id) {
            return;
        }
        self.queue.push_back((chapter_id, manga_id, total_pages));
    }

    pub fn clear_queue(&mut self) {
        self.queue.clear();
    }

    /// Stop everything: clear the queue and cancel the running chapter.
    pub fn stop_all(&mut self) {
        self.queue.clear();
        for (id, cancel) in self.cancels.drain() {
            cancel.store(true, Ordering::Relaxed);
            self.jobs.remove(&id);
        }
        if let Ok(mut m) = self.progress.lock() {
            m.clear();
        }
    }

    /// Drop queued/running job for a chapter (e.g. after deletion).
    /// Cancels the background thread — it stops within a second.
    pub fn remove_chapter(&mut self, chapter_id: &str) {
        self.queue.retain(|(id, _, _)| id != chapter_id);
        self.jobs.remove(chapter_id);
        if let Some(cancel) = self.cancels.remove(chapter_id) {
            cancel.store(true, Ordering::Relaxed);
        }
        if let Ok(mut m) = self.progress.lock() {
            m.remove(chapter_id);
        }
    }

    /// Start queued downloads while below the parallel-chapter limit.
    fn maybe_start_next(&mut self, data_dir: &Path) {
        while self.jobs.len() < max_active_chapters() {
            let Some((chapter_id, manga_id, total)) = self.queue.pop_front() else {
                break;
            };
            self.start(&chapter_id, manga_id, total, data_dir.to_path_buf());
        }
    }

    /// Finished downloads this frame: (chapter_id, manga_id, result).
    /// Also starts the next queued chapter.
    pub fn poll(&mut self, data_dir: &Path) -> Vec<(String, String, Result<(), String>)> {
        let mut done = Vec::new();
        let ids: Vec<String> = self.jobs.keys().cloned().collect();
        for id in ids {
            let mut finished = false;
            let mut result = Ok(());
            if let Some(dj) = self.jobs.get_mut(&id) {
                if let Some(res) = dj.job.poll() {
                    finished = true;
                    result = res;
                } else if !dj.job.running() {
                    finished = true;
                }
            }
            if finished {
                if let Ok(mut m) = self.progress.lock() {
                    m.remove(&id);
                }
                self.cancels.remove(&id);
                if let Some(dj) = self.jobs.remove(&id) {
                    done.push((id, dj.manga_id, result));
                }
            }
        }
        self.maybe_start_next(data_dir);
        done
    }
}

fn set_progress(
    progress: &Mutex<HashMap<String, ChapterProgress>>,
    chapter_id: &str,
    f: impl FnOnce(&mut ChapterProgress),
) {
    if let Ok(mut m) = progress.lock() {
        if let Some(p) = m.get_mut(chapter_id) {
            f(p);
        }
    }
}

fn download_chapter(
    chapter_id: &str,
    data_dir: &Path,
    progress: Arc<Mutex<HashMap<String, ChapterProgress>>>,
    cancel: &AtomicBool,
    par: usize,
) -> Result<(), String> {
    let cancelled = || cancel.load(Ordering::Relaxed);
    if cancelled() {
        return Err(CANCELLED.to_string());
    }
    let (base, hash, pages) = mangadex::at_home(chapter_id)?;
    let total = pages.len() as u32;
    set_progress(&progress, chapter_id, |p| {
        p.total = total;
    });
    let dir = chapter_dir(data_dir, chapter_id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Не удалось создать папку загрузки: {e}"))?;

    // shared work queue: (index, file name)
    let work: Mutex<VecDeque<(usize, &String)>> =
        Mutex::new(pages.iter().enumerate().collect());
    let completed = Arc::new(AtomicUsize::new(0));
    let first_error: Mutex<Option<String>> = Mutex::new(None);
    let stop_pool = Arc::new(AtomicBool::new(false));

    std::thread::scope(|s| {
        let dir = &dir;
        let base = &base;
        let hash = &hash;
        let progress = &progress;
        for _ in 0..par.min(pages.len().max(1)) {
            let work = &work;
            let completed = &completed;
            let first_error = &first_error;
            let stop_pool = &stop_pool;
            s.spawn(move || loop {
                // a page failed (or cancelled) somewhere else — stop pulling work
                if cancelled()
                    || stop_pool.load(Ordering::Relaxed)
                    || first_error.lock().unwrap().is_some()
                {
                    break;
                }
                let Some((i, page)) = work.lock().unwrap().pop_front() else {
                    break; // no more work
                };
                let ext = page.rsplit('.').next().unwrap_or("jpg");
                let name = format!("{:04}.{}", i + 1, ext.to_ascii_lowercase());
                let path = dir.join(&name);
                // resume: skip pages already on disk
                let have = std::fs::metadata(&path).map(|m| m.len() > 0).unwrap_or(false);
                if !have {
                    let url = mangadex::page_url(&base, &hash, page);
                    match crate::net::get_bytes_retry_cancel(&url, 3, cancel) {
                        Ok(bytes) => {
                            if let Err(e) = std::fs::write(&path, &bytes) {
                                let msg = format!("Не удалось записать страницу {}: {e}", i + 1);
                                *first_error.lock().unwrap() = Some(msg);
                                stop_pool.store(true, Ordering::Relaxed);
                                break;
                            }
                        }
                        Err(e) => {
                            set_progress(&progress, chapter_id, |p| p.failed = true);
                            let msg = if cancelled() {
                                CANCELLED.to_string()
                            } else {
                                format!("Страница {}: {e}", i + 1)
                            };
                            *first_error.lock().unwrap() = Some(msg);
                            stop_pool.store(true, Ordering::Relaxed);
                            break;
                        }
                    }
                    // be gentle with the API (its ToS asks for delays between requests)
                    for _ in 0..10 {
                        if cancelled() || stop_pool.load(Ordering::Relaxed) {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(30));
                    }
                }
                let n = completed.fetch_add(1, Ordering::Relaxed) + 1;
                set_progress(&progress, chapter_id, |p| p.done = n as u32);
            });
        }
    });

    if cancelled() {
        return Err(CANCELLED.to_string());
    }
    if let Some(e) = first_error.into_inner().unwrap() {
        return Err(e);
    }
    Ok(())
}
