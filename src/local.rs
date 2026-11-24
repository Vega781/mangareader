//! Local manga sources: folders and CBZ (zip) archives.
use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "webp", "gif", "bmp", "avif"];
pub const ARCHIVE_EXTS: &[&str] = &["cbz", "zip"];

pub fn is_image(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| IMAGE_EXTS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

fn is_archive(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| ARCHIVE_EXTS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Where the bytes of one page come from.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum PageSource {
    File(PathBuf),
    /// Path to a .cbz/.zip plus the entry name inside it.
    Zip { path: PathBuf, entry: String },
}

impl PageSource {
    pub fn load(&self) -> Result<Vec<u8>, String> {
        match self {
            PageSource::File(p) => {
                fs::read(p).map_err(|e| format!("Не удалось прочитать {}: {e}", p.display()))
            }
            PageSource::Zip { path, entry } => {
                let f = fs::File::open(path)
                    .map_err(|e| format!("Не удалось открыть {}: {e}", path.display()))?;
                let mut archive = zip::ZipArchive::new(f)
                    .map_err(|e| format!("Повреждённый архив {}: {e}", path.display()))?;
                let mut zf = archive
                    .by_name(entry)
                    .map_err(|e| format!("Нет страницы {entry} в {}: {e}", path.display()))?;
                let mut buf = Vec::new();
                zf.read_to_end(&mut buf)
                    .map_err(|e| format!("Ошибка чтения {entry}: {e}"))?;
                Ok(buf)
            }
        }
    }

    pub fn key(&self) -> String {
        match self {
            PageSource::File(p) => format!("file:{}", p.display()),
            PageSource::Zip { path, entry } => format!("zip:{}!{}", path.display(), entry),
        }
    }
}

use std::io::Read;

/// Natural sort: digits compared numerically ("page2" < "page10").
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut ai, mut bi) = (0, 0);
    let (ab, bb) = (a.as_bytes(), b.as_bytes());
    loop {
        if ai == ab.len() && bi == bb.len() {
            return Ordering::Equal;
        }
        if ai == ab.len() {
            return Ordering::Less;
        }
        if bi == bb.len() {
            return Ordering::Greater;
        }
        let (ca, cb) = (ab[ai], bb[bi]);
        if ca.is_ascii_digit() && cb.is_ascii_digit() {
            let (a0, b0) = (ai, bi);
            while ai < ab.len() && ab[ai].is_ascii_digit() {
                ai += 1;
            }
            while bi < bb.len() && bb[bi].is_ascii_digit() {
                bi += 1;
            }
            // compare digit runs: strip leading zeros, then by length, then lexicographically
            let da = &ab[a0..ai];
            let db = &bb[b0..bi];
            let sa = da.iter().position(|&c| c != b'0').unwrap_or(da.len());
            let sb = db.iter().position(|&c| c != b'0').unwrap_or(db.len());
            let (ta, tb) = (&da[sa..], &db[sb..]);
            let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
            if ord != Ordering::Equal {
                return ord;
            }
        } else {
            let ord = ca.to_ascii_lowercase().cmp(&cb.to_ascii_lowercase());
            if ord != Ordering::Equal {
                return ord;
            }
            ai += 1;
            bi += 1;
        }
    }
}

#[derive(Clone, Debug)]
pub struct LocalChapter {
    pub path: PathBuf,
    pub title: String,
}

#[derive(Clone, Debug)]
pub struct LocalManga {
    pub path: PathBuf,
    pub title: String,
    pub chapters: Vec<LocalChapter>,
}

/// Scan a path into a manga with chapters.
/// - a .cbz/.zip file                       → one-chapter manga
/// - a folder of images                     → one-chapter manga
/// - a folder of .cbz files                 → one chapter per archive
/// - a folder of subfolders                 → one chapter per subfolder
pub fn scan(path: &Path) -> Result<LocalManga, String> {
    if !path.exists() {
        return Err(format!("Путь не существует: {}", path.display()));
    }
    if path.is_file() && is_archive(path) {
        return Ok(LocalManga {
            path: path.to_path_buf(),
            title: file_stem(path),
            chapters: vec![LocalChapter {
                path: path.to_path_buf(),
                title: file_stem(path),
            }],
        });
    }
    if !path.is_dir() {
        return Err(format!("Поддерживаются папки и файлы .cbz/.zip: {}", path.display()));
    }

    let mut subs: Vec<PathBuf> = Vec::new();
    let mut files: Vec<PathBuf> = Vec::new();
    for e in fs::read_dir(path).map_err(|e| format!("Не удалось открыть папку: {e}"))? {
        let p = e.map_err(|e| e.to_string())?.path();
        if p.is_dir() {
            subs.push(p);
        } else if p.is_file() {
            files.push(p);
        }
    }

    let archives: Vec<PathBuf> = files.iter().filter(|p| is_archive(p)).cloned().collect();
    let images: Vec<PathBuf> = files.iter().filter(|p| is_image(p)).cloned().collect();

    if !archives.is_empty() {
        // folder of archives: one chapter each
        let mut chapters: Vec<LocalChapter> = archives
            .into_iter()
            .map(|p| LocalChapter {
                title: file_stem(&p),
                path: p,
            })
            .collect();
        chapters.sort_by(|a, b| natural_cmp(&a.title, &b.title));
        return Ok(LocalManga {
            path: path.to_path_buf(),
            title: file_stem(path),
            chapters,
        });
    }

    if !subs.is_empty() {
        subs.sort_by(|a, b| natural_cmp(file_name_of(a), file_name_of(b)));
        let chapters: Vec<LocalChapter> = subs
            .into_iter()
            .map(|p| LocalChapter {
                title: file_name_of(&p).to_string(),
                path: p,
            })
            .collect();
        return Ok(LocalManga {
            path: path.to_path_buf(),
            title: file_stem(path),
            chapters,
        });
    }

    if !images.is_empty() {
        return Ok(LocalManga {
            path: path.to_path_buf(),
            title: file_stem(path),
            chapters: vec![LocalChapter {
                path: path.to_path_buf(),
                title: file_stem(path),
            }],
        });
    }

    Err(format!("В папке {} нет изображений, архивов или подпапок с главами", path.display()))
}

/// List pages of one chapter (images inside dir, or entries inside archive).
pub fn chapter_pages(chapter_path: &Path) -> Result<Vec<PageSource>, String> {
    if chapter_path.is_file() {
        let f = fs::File::open(chapter_path)
            .map_err(|e| format!("Не удалось открыть {}: {e}", chapter_path.display()))?;
        let mut ar = zip::ZipArchive::new(f)
            .map_err(|e| format!("Повреждённый архив: {e}"))?;
        let mut names: Vec<String> = (0..ar.len())
            .filter_map(|i| ar.by_index(i).ok().map(|f| f.name().to_string()))
            .filter(|n| {
                let ext = n.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
                IMAGE_EXTS.contains(&ext.as_str()) && !n.ends_with('/')
            })
            .collect();
        names.sort_by(|a, b| natural_cmp(a, b));
        Ok(names
            .into_iter()
            .map(|entry| PageSource::Zip {
                path: chapter_path.to_path_buf(),
                entry,
            })
            .collect())
    } else if chapter_path.is_dir() {
        let mut pages: Vec<PathBuf> = Vec::new();
        for e in fs::read_dir(chapter_path).map_err(|e| e.to_string())? {
            let p = e.map_err(|e| e.to_string())?.path();
            if p.is_file() && is_image(&p) {
                pages.push(p);
            }
        }
        pages.sort_by(|a, b| natural_cmp(file_name_of(a), file_name_of(b)));
        Ok(pages.into_iter().map(PageSource::File).collect())
    } else {
        Err(format!("Глава не найдена: {}", chapter_path.display()))
    }
}

pub fn file_stem(p: &Path) -> String {
    p.file_stem().and_then(|s| s.to_str()).unwrap_or("Манга").to_string()
}

pub fn file_name_of(p: &Path) -> &str {
    p.file_name().and_then(|s| s.to_str()).unwrap_or("")
}
