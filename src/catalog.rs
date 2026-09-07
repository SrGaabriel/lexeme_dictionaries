use std::collections::HashSet;
use std::fmt::Write as _;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

const BASE: &str = "https://kaikki.org/dictionary";

#[derive(Deserialize)]
struct Catalog {
    #[serde(default)]
    defaults: Defaults,
    language: Vec<Record>,
}

#[derive(Default, Deserialize)]
struct Defaults {
    #[serde(default)]
    license: String,
}

#[derive(Deserialize)]
struct Record {
    code: String,
    name: String,
    license: Option<String>,
    url: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct Language {
    pub code: String,
    pub name: String,
    pub license: String,
    pub url: String,
}

#[derive(Serialize)]
pub struct Matrix {
    pub include: Vec<Language>,
}

pub fn load(path: &Path) -> io::Result<Vec<Language>> {
    let text = std::fs::read_to_string(path)?;
    let catalog: Catalog = toml::from_str(&text).map_err(io::Error::other)?;

    let mut seen = HashSet::new();
    let mut languages = Vec::with_capacity(catalog.language.len());
    for record in catalog.language {
        if !seen.insert(record.code.clone()) {
            return Err(io::Error::other(format!(
                "duplicate language code: {}",
                record.code
            )));
        }
        let url = record.url.unwrap_or_else(|| source_url(&record.name));
        let license = record
            .license
            .unwrap_or_else(|| catalog.defaults.license.clone());
        languages.push(Language {
            code: record.code,
            name: record.name,
            license,
            url,
        });
    }
    Ok(languages)
}

pub fn select(languages: Vec<Language>, only: &str) -> io::Result<Vec<Language>> {
    let wanted: Vec<&str> = only
        .split(',')
        .map(str::trim)
        .filter(|code| !code.is_empty())
        .collect();
    if wanted.is_empty() {
        return Ok(languages);
    }

    let known: HashSet<&str> = languages.iter().map(|l| l.code.as_str()).collect();
    let unknown: Vec<&str> = wanted
        .iter()
        .copied()
        .filter(|code| !known.contains(code))
        .collect();
    if !unknown.is_empty() {
        return Err(io::Error::other(format!(
            "unknown language codes: {}",
            unknown.join(", ")
        )));
    }

    Ok(languages
        .into_iter()
        .filter(|l| wanted.contains(&l.code.as_str()))
        .collect())
}

pub fn source_url(name: &str) -> String {
    let compact: String = name.chars().filter(|c| c.is_alphanumeric()).collect();
    format!(
        "{BASE}/{}/kaikki.org-dictionary-{}.jsonl.gz",
        encode(name),
        encode(&compact)
    )
}

fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}
