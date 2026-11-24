//! Image loading cache: background threads fetch + decode, textures stored here,
//! LRU eviction of decoded textures.
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc;

use eframe::egui::{self, Color32, ColorImage, TextureHandle, TextureOptions};

use crate::local::PageSource;

/// How to obtain image bytes.
#[derive(Clone, Debug)]
pub enum ImgSource {
    Url(String),
    /// Fetch from URL, but cache bytes on disk so covers survive offline restarts.
    UrlDisk { url: String, local: PathBuf },
    Page(PageSource),
}

impl ImgSource {
    pub fn load(&self) -> Result<Vec<u8>, String> {
        match self {
            ImgSource::Url(url) => crate::net::get_bytes(url),
            ImgSource::UrlDisk { url, local } => {
                if let Ok(bytes) = std::fs::read(local) {
                    if !bytes.is_empty() {
                        return Ok(bytes);
                    }
                }
                let bytes = crate::net::get_bytes(url)?;
                if let Some(parent) = local.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(local, &bytes);
                Ok(bytes)
            }
            ImgSource::Page(p) => p.load(),
        }
    }
}

enum Entry {
    Loading,
    Ready(TextureHandle),
    Failed(String),
}

type Decoded = Result<ColorImage, String>;

pub struct ImageCache {
    entries: HashMap<String, Entry>,
    lru: VecDeque<String>,
    tx: mpsc::Sender<(String, Decoded)>,
    rx: mpsc::Receiver<(String, Decoded)>,
    max_textures: usize,
}

fn decode(bytes: Vec<u8>) -> Result<ColorImage, String> {
    let img = image::load_from_memory(&bytes).map_err(|e| format!("Не удалось декодировать изображение: {e}"))?;
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width() as usize, rgba.height() as usize);
    let pixels = rgba
        .into_raw()
        .chunks_exact(4)
        .map(|p| Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]))
        .collect();
    Ok(ColorImage { size: [w, h], pixels })
}

impl ImageCache {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        ImageCache {
            entries: HashMap::new(),
            lru: VecDeque::new(),
            tx,
            rx,
            max_textures: 48,
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Start background load if not already cached/loading. Returns true when ready.
    pub fn is_ready(&self, key: &str) -> bool {
        matches!(self.entries.get(key), Some(Entry::Ready(_)))
    }

    pub fn failed(&self, key: &str) -> Option<&str> {
        match self.entries.get(key) {
            Some(Entry::Failed(e)) => Some(e),
            _ => None,
        }
    }

    pub fn is_loading(&self, key: &str) -> bool {
        matches!(self.entries.get(key), Some(Entry::Loading))
    }

    /// Request load in background.
    pub fn request(&mut self, key: &str, src: ImgSource) {
        if self.entries.contains_key(key) {
            return;
        }
        self.entries.insert(key.to_string(), Entry::Loading);
        let tx = self.tx.clone();
        let key = key.to_string();
        std::thread::spawn(move || {
            let result = src.load().and_then(decode);
            let _ = tx.send((key, result));
        });
    }

    /// Drain completed loads, create textures. Call every frame.
    pub fn poll(&mut self, ctx: &egui::Context) {
        while let Ok((key, result)) = self.rx.try_recv() {
            match result {
                Ok(img) => {
                    let handle = ctx.load_texture(&format!("img:{key}"), img, TextureOptions::LINEAR);
                    self.entries.insert(key.clone(), Entry::Ready(handle));
                    self.lru.push_back(key);
                }
                Err(e) => {
                    self.entries.insert(key, Entry::Failed(e));
                }
            }
        }
        // evict oldest ready textures beyond the cap
        while self.lru.len() > self.max_textures {
            let key = self.lru.pop_front().unwrap();
            self.entries.remove(&key); // drops TextureHandle -> frees GPU texture
        }
    }

    pub fn get(&mut self, key: &str) -> Option<&TextureHandle> {
        match self.entries.get(key) {
            Some(Entry::Ready(h)) => {
                // touch LRU
                if let Some(pos) = self.lru.iter().position(|k| k == key) {
                    self.lru.remove(pos);
                    self.lru.push_back(key.to_string());
                }
                Some(h)
            }
            _ => None,
        }
    }

    pub fn get_or_request(&mut self, key: &str, src: ImgSource) -> Option<&TextureHandle> {
        self.request(key, src);
        self.get(key)
    }

    /// Drop everything (e.g. "clear cache" button).
    pub fn clear(&mut self) {
        self.entries.clear();
        self.lru.clear();
    }
}
