//! Offline chapter downloads from MangaDex (sequential, rate-limit friendly).
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::jobs::Job;
use crate::mangadex;

struct DownloadJob {
    manga_id: String,
    job: Job<Result<(), String>>,
}

pub struct Downloads {
    jobs: HashMap<String, DownloadJob>,
}

impl Default for Downloads {
    fn default() -> Self {
        Downloads { jobs: HashMap::new() }
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

impl Downloads {
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
    pub fn poll(&mut self) -> Vec<(String, String, Result<(), String>)> {
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
        done
    }
}

fn download_chapter(chapter_id: &str, data_dir: &Path) -> Result<(), String> {
    let (base, hash, pages) = mangadex::at_home(chapter_id)?;
    let dir = chapter_dir(data_dir, chapter_id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Не удалось создать папку загрузки: {e}"))?;
    for (i, page) in pages.iter().enumerate() {
        let url = mangadex::page_url(&base, &hash, page);
        let bytes =
            crate::net::get_bytes(&url).map_err(|e| format!("Страница {}: {e}", i + 1))?;
        let ext = page.rsplit('.').next().unwrap_or("jpg");
        let name = format!("{:04}.{}", i + 1, ext.to_ascii_lowercase());
        std::fs::write(dir.join(name), &bytes)
            .map_err(|e| format!("Не удалось записать страницу: {e}"))?;
        // be gentle with the API (its ToS asks for delays between requests)
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
    Ok(())
}
