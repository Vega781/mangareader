//! Offline chapter downloads from MangaDex (sequential, rate-limit friendly).
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use crate::jobs::Job;
use crate::mangadex;

struct DownloadJob {
    manga_id: String,
    job: Job<Result<(), String>>,
}

pub struct Downloads {
    jobs: HashMap<String, DownloadJob>,
    queue: VecDeque<(String, String)>, // (chapter_id, manga_id)
}

impl Default for Downloads {
    fn default() -> Self {
        Downloads {
            jobs: HashMap::new(),
            queue: VecDeque::new(),
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
    pub fn queued(&self, chapter_id: &str) -> bool {
        self.queue.iter().any(|(id, _)| id == chapter_id)
    }

    pub fn pending(&self) -> usize {
        self.queue.len()
    }

    pub fn active(&self) -> usize {
        self.jobs.len()
    }

    pub fn enqueue(&mut self, chapter_id: String, manga_id: String) {
        if self.jobs.contains_key(&chapter_id) || self.queued(&chapter_id) {
            return;
        }
        self.queue.push_back((chapter_id, manga_id));
    }

    pub fn clear_queue(&mut self) {
        self.queue.clear();
    }

    /// Drop queued/running job for a chapter (e.g. after deletion).
    pub fn remove_chapter(&mut self, chapter_id: &str) {
        self.queue.retain(|(id, _)| id != chapter_id);
        self.jobs.remove(chapter_id);
    }

    /// Start next queued download when no job is active.
    fn maybe_start_next(&mut self, data_dir: &Path) {
        if self.jobs.is_empty() {
            if let Some((chapter_id, manga_id)) = self.queue.pop_front() {
                self.start(&chapter_id, manga_id, data_dir.to_path_buf());
            }
        }
    }

    pub fn running(&self, chapter_id: &str) -> bool {
        self.jobs.contains_key(chapter_id)
    }

    pub fn count(&self) -> usize {
        self.jobs.len()
    }

    pub fn start(&mut self, chapter_id: &str, manga_id: String, data_dir: PathBuf) {
        if self.jobs.contains_key(chapter_id) {
            return;
        }
        let id = chapter_id.to_string();
        self.jobs.insert(
            id.clone(),
            DownloadJob {
                manga_id,
                job: Job::spawn(move || download_chapter(&id, &data_dir)),
            },
        );
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
                if let Some(dj) = self.jobs.remove(&id) {
                    done.push((id, dj.manga_id, result));
                }
            }
        }
        self.maybe_start_next(data_dir);
        done
    }
}

fn download_chapter(chapter_id: &str, data_dir: &Path) -> Result<(), String> {
    let (base, hash, pages) = mangadex::at_home(chapter_id)?;
    let dir = chapter_dir(data_dir, chapter_id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Не удалось создать папку загрузки: {e}"))?;
    for (i, page) in pages.iter().enumerate() {
        let ext = page.rsplit('.').next().unwrap_or("jpg");
        let name = format!("{:04}.{}", i + 1, ext.to_ascii_lowercase());
        let path = dir.join(&name);
        // resume: skip pages already on disk
        let have = std::fs::metadata(&path).map(|m| m.len() > 0).unwrap_or(false);
        if have {
            continue;
        }
        let url = mangadex::page_url(&base, &hash, page);
        let bytes = crate::net::get_bytes_retry(&url, 3)
            .map_err(|e| format!("Страница {}: {e}", i + 1))?;
        std::fs::write(&path, &bytes)
            .map_err(|e| format!("Не удалось записать страницу: {e}"))?;
        // be gentle with the API (its ToS asks for delays between requests)
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
    Ok(())
}
