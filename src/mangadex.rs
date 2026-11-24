//! MangaDex API v5 client.
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::net::{self, encode_param};

const API: &str = "https://api.mangadex.org";
const UPLOADS: &str = "https://uploads.mangadex.org";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MangaSummary {
    pub id: String,
    pub title: String,
    pub cover_url: Option<String>,
    pub status: String,
    pub description: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Chapter {
    pub id: String,
    pub label: String,
    pub number: Option<f64>,
    pub pages: u32,
    pub lang: String,
}

// ---------- JSON helpers ----------

/// Pick a localized string: prefer `prefer` keys, then first non-empty value.
fn localized(map: &Value, prefer: &[&str]) -> Option<String> {
    let obj = map.as_object()?;
    for k in prefer {
        if let Some(v) = obj.get(*k).and_then(Value::as_str) {
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    obj.values()
        .find_map(|v| v.as_str().filter(|s| !s.is_empty()).map(|s| s.to_string()))
}

fn parse_summary(id: &str, attrs: &Value, relationships: &Value) -> MangaSummary {
    let title = attrs
        .get("title")
        .and_then(|t| localized(t, &["ru", "en"]))
        .unwrap_or_else(|| "Без названия".into());
    let description = attrs
        .get("description")
        .and_then(|d| localized(d, &["ru", "en"]))
        .unwrap_or_default();
    let status = attrs
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let cover_url = relationships
        .as_array()
        .and_then(|rels| {
            rels.iter().find_map(|r| {
                if r.get("type").and_then(Value::as_str) == Some("cover_art") {
                    r.pointer("/attributes/fileName")
                        .and_then(Value::as_str)
                        .map(|f| format!("{UPLOADS}/covers/{id}/{f}.512.jpg"))
                } else {
                    None
                }
            })
        });
    MangaSummary {
        id: id.to_string(),
        title,
        cover_url,
        status,
        description,
    }
}

fn parse_manga_list(v: Value) -> Result<(Vec<MangaSummary>, u32), String> {
    if v.get("result").and_then(Value::as_str) != Some("ok") {
        return Err(format!("API вернул ошибку: {}", v.get("errors").map(|e| e.to_string()).unwrap_or_default()));
    }
    let total = v.get("total").and_then(Value::as_u64).unwrap_or(0) as u32;
    let data = v
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok((data
        .iter()
        .filter_map(|m| {
            let id = m.get("id")?.as_str()?;
            let attrs = m.get("attributes")?;
            Some(parse_summary(id, attrs, m.get("relationships").unwrap_or(&Value::Null)))
        })
        .collect(), total))
}

// ---------- Endpoints ----------

pub fn search(query: &str, offset: u32) -> Result<(Vec<MangaSummary>, u32), String> {
    let url = format!(
        "{API}/manga?title={}&limit=24&offset={}&includes[]=cover_art&contentRating[]=safe&contentRating[]=suggestive&contentRating[]=erotica&hasAvailableChapters=true&order[relevance]=desc",
        encode_param(query),
        offset
    );
    parse_manga_list(net::get_json(&url)?)
}

pub fn popular(offset: u32) -> Result<(Vec<MangaSummary>, u32), String> {
    let url = format!(
        "{API}/manga?limit=24&offset={}&includes[]=cover_art&order[followedCount]=desc&contentRating[]=safe&contentRating[]=suggestive&contentRating[]=erotica&hasAvailableChapters=true",
        offset
    );
    parse_manga_list(net::get_json(&url)?)
}

pub fn details(id: &str) -> Result<MangaSummary, String> {
    let url = format!("{API}/manga/{id}?includes[]=cover_art&contentRating[]=safe&contentRating[]=suggestive&contentRating[]=erotica");
    let v = net::get_json(&url)?;
    if v.get("result").and_then(Value::as_str) != Some("ok") {
        return Err("Манга не найдена".into());
    }
    let d = v.get("data").cloned().unwrap_or(Value::Null);
    let id2 = d.get("id").and_then(Value::as_str).unwrap_or(id).to_string();
    let attrs = d.get("attributes").cloned().unwrap_or(Value::Null);
    Ok(parse_summary(
        &id2,
        &attrs,
        d.get("relationships").unwrap_or(&Value::Null),
    ))
}

pub fn fetch_chapters(manga_id: &str, langs: &[String], ratings: &[String]) -> Result<Vec<Chapter>, String> {
    let mut out = Vec::new();
    let mut offset = 0u32;
    loop {
        let mut url = format!(
            "{API}/manga/{manga_id}/feed?limit=500&offset={offset}&order[chapter]=asc&order[volume]=asc&includes[]=scanlation_group&includes[]=user"
        );
        for l in langs {
            url.push_str(&format!("&translatedLanguage[]={}", encode_param(l)));
        }
        for r in ratings {
            url.push_str(&format!("&contentRating[]={}", encode_param(r)));
        }
        let v = net::get_json(&url)?;
        if v.get("result").and_then(Value::as_str) != Some("ok") {
            if out.is_empty() {
                return Err("Не удалось получить список глав".into());
            }
            break;
        }
        let data = v
            .get("data")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let batch: Vec<Chapter> = data
            .iter()
            .filter_map(|c| {
                let id = c.get("id")?.as_str()?.to_string();
                let a = c.get("attributes")?;
                let num = a
                    .get("chapter")
                    .and_then(Value::as_str)
                    .and_then(|s| s.parse::<f64>().ok());
                let title = a
                    .get("title")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string());
                let label = match (num, &title) {
                    (Some(n), Some(t)) => format!("Глава {:?} — {t}", trim_num(n)),
                    (Some(n), None) => format!("Глава {}", trim_num(n)),
                    (None, Some(t)) => t.clone(),
                    (None, None) => "Глава".into(),
                };
                Some(Chapter {
                    id,
                    label,
                    number: num,
                    pages: a.get("pages").and_then(Value::as_u64).unwrap_or(0) as u32,
                    lang: a
                        .get("translatedLanguage")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                })
            })
            .collect();
        let got = batch.len() as u32;
        out.extend(batch);
        let total = v.get("total").and_then(Value::as_u64).unwrap_or(0) as u32;
        offset += got;
        if got == 0 || offset >= total || out.len() > 5000 {
            break;
        }
    }
    Ok(out)
}

fn trim_num(n: f64) -> String {
    let s = format!("{n}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// At-home page server: (base_url, hash, page file names).
pub fn at_home(chapter_id: &str) -> Result<(String, String, Vec<String>), String> {
    let v = net::get_json(&format!("{API}/at-home/server/{chapter_id}"))?;
    if v.get("result").and_then(Value::as_str) != Some("ok") {
        return Err("Сервер страниц недоступен (главы ещё нет на домашнем сервере)".into());
    }
    let base = v
        .get("baseUrl")
        .and_then(Value::as_str)
        .ok_or("baseUrl отсутствует")?
        .to_string();
    let ch = v.get("chapter").cloned().unwrap_or(Value::Null);
    let hash = ch
        .get("hash")
        .and_then(Value::as_str)
        .ok_or("hash отсутствует")?
        .to_string();
    let pages: Vec<String> = ch
        .get("data")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default();
    if pages.is_empty() {
        return Err("Глава не содержит страниц".into());
    }
    Ok((base, hash, pages))
}

pub fn page_url(base: &str, hash: &str, file: &str) -> String {
    format!("{base}/data/{hash}/{file}")
}
