//! Persistent state: library, progress, bookmarks, history, settings.
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::local::PageSource;
use crate::mangadex::MangaSummary;

/// Where the cover image comes from.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum CoverSource {
    /// MangaDex cover URL (cached on disk under data_dir/covers).
    MdUrl(String),
    /// A page of local manga used as the cover.
    LocalPage(PageSource),
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum MangaSource {
    Mangadex { id: String },
    Local { path: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Bookmark {
    pub chapter: String,
    pub chapter_label: String,
    pub page: u32,
    pub total: u32,
    pub at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MangaEntry {
    pub id: String, // "md:{uuid}" or "local:{path}"
    pub source: MangaSource,
    pub title: String,
    pub description: String,
    pub status: String,
    pub cover: Option<CoverSource>,
    pub added_at: u64,
    /// chapter_key -> last read page index
    pub progress: BTreeMap<String, u32>,
    /// chapter_key -> total pages (for percent display)
    pub totals: BTreeMap<String, u32>,
    pub last_chapter: Option<String>,
    pub bookmarks: Vec<Bookmark>,
    /// downloaded mangadex chapter ids
    pub downloaded: Vec<String>,
    /// fully read chapters
    pub finished: Vec<String>,
}

impl MangaEntry {
    pub fn from_summary(s: &MangaSummary) -> Self {
        MangaEntry {
            id: format!("md:{}", s.id),
            source: MangaSource::Mangadex { id: s.id.clone() },
            title: s.title.clone(),
            description: s.description.clone(),
            status: s.status.clone(),
            cover: s.cover_url.clone().map(CoverSource::MdUrl),
            added_at: now(),
            progress: BTreeMap::new(),
            totals: BTreeMap::new(),
            last_chapter: None,
            bookmarks: Vec::new(),
            downloaded: Vec::new(),
            finished: Vec::new(),
        }
    }

    pub fn md_uuid(&self) -> Option<&str> {
        if let MangaSource::Mangadex { id } = &self.source {
            Some(id)
        } else {
            None
        }
    }

    pub fn is_local(&self) -> bool {
        matches!(self.source, MangaSource::Local { .. })
    }

    pub fn progress_percent(&self) -> Option<f32> {
        let mut read: u32 = 0;
        let mut total: u32 = 0;
        for (ch, page) in &self.progress {
            let t = self.totals.get(ch).copied().unwrap_or(0);
            if t == 0 {
                continue;
            }
            read += (*page).min(t);
            total += t;
        }
        if total > 0 {
            Some(100.0 * read as f32 / total as f32)
        } else {
            None
        }
    }

    pub fn set_progress(&mut self, chapter: &str, page: u32, total: u32) {
        let prev = self.progress.get(chapter).copied().unwrap_or(0);
        if page >= prev || prev > total {
            self.progress.insert(chapter.to_string(), page.min(total.max(page)));
        }
        if total > 0 {
            self.totals.insert(chapter.to_string(), total);
        }
        self.last_chapter = Some(chapter.to_string());
        if total > 0 && page + 1 >= total {
            if !self.finished.iter().any(|c| c == chapter) {
                self.finished.push(chapter.to_string());
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryItem {
    pub manga_id: String,
    pub title: String,
    pub chapter: String,
    pub chapter_label: String,
    pub page: u32,
    pub total: u32,
    pub at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    /// MangaDex translated languages, most preferred first.
    pub languages: Vec<String>,
    /// MangaDex content ratings to include.
    pub content_ratings: Vec<String>,
    pub rtl: bool,
    /// 0 = paged, 1 = strip, 2 = horizontal
    pub default_mode: usize,
    pub page_turn_on_click: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            languages: vec!["en".into(), "ru".into()],
            content_ratings: vec!["safe".into(), "suggestive".into()],
            rtl: true,
            default_mode: 0,
            page_turn_on_click: true,
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
pub struct Library {
    pub entries: Vec<MangaEntry>,
    pub history: Vec<HistoryItem>,
    pub settings: Settings,
}

impl Library {
    pub fn entry(&self, id: &str) -> Option<&MangaEntry> {
        self.entries.iter().find(|e| e.id == id)
    }
    pub fn entry_mut(&mut self, id: &str) -> Option<&mut MangaEntry> {
        self.entries.iter_mut().find(|e| e.id == id)
    }
    pub fn contains(&self, id: &str) -> bool {
        self.entries.iter().any(|e| e.id == id)
    }
    pub fn add(&mut self, e: MangaEntry) {
        if !self.contains(&e.id) {
            self.entries.insert(0, e);
        }
    }
    pub fn remove(&mut self, id: &str) {
        self.entries.retain(|e| e.id != id);
    }
    pub fn push_history(&mut self, item: HistoryItem) {
        self.history.retain(|h| !(h.manga_id == item.manga_id && h.chapter == item.chapter));
        self.history.insert(0, item);
        self.history.truncate(200);
    }
    pub fn last_history_for(&self, manga_id: &str) -> Option<&HistoryItem> {
        self.history.iter().find(|h| h.manga_id == manga_id)
    }
}

// ---------- persistence on disk ----------

pub fn data_dir() -> PathBuf {
    let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("mangareader")
}

pub fn load(data_dir: &std::path::Path) -> Library {
    let path = data_dir.join("library.json");
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => Library::default(),
    }
}

pub fn save(data_dir: &std::path::Path, lib: &Library) {
    let path = data_dir.join("library.json");
    if let Ok(json) = serde_json::to_vec_pretty(lib) {
        let _ = std::fs::create_dir_all(data_dir);
        // atomic-ish write
        let tmp = data_dir.join(".library.json.tmp");
        if std::fs::write(&tmp, &json).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}
