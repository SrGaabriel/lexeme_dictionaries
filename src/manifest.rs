use std::io;
use std::path::Path;

use lexeme::manifest::{FILE_NAME, Language, Manifest};
use sha2::{Digest, Sha256};

use crate::build::{Stats, write_json};

const NOTICE: &str = "
# Attribution

The dictionary files published alongside this notice are derived from\
[Wiktionary](https://www.wiktionary.org/), extracted using\
[Wiktextract](https://github.com/tatuylonen/wiktextract) and distributed by\
[kaikki.org](https://kaikki.org/).

Wiktionary content is licensed under\
[CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/)\
and these derived word lists are distributed under the same terms.
";

pub fn assemble(dist: &Path, tag: &str, repo: &str, generated: &str) -> io::Result<()> {
    let mut manifest = Manifest::new(tag.to_owned(), generated.to_owned());

    let mut reports: Vec<_> = std::fs::read_dir(dist)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(".stats.json"))
        })
        .collect();
    reports.sort();

    for report in reports {
        let text = std::fs::read_to_string(&report)?;
        let stats: Stats = serde_json::from_str(&text).map_err(io::Error::other)?;

        let file = format!("{}.lex.zst", stats.lang);
        let lexicon = dist.join(&file);
        if !lexicon.exists() {
            return Err(io::Error::other(format!(
                "{} has no matching {file}",
                report.display()
            )));
        }
        let bytes = std::fs::read(&lexicon)?;
        let sha256 = format!("{:x}", Sha256::digest(&bytes));

        manifest.languages.insert(
            stats.lang.clone(),
            Language {
                url: format!("https://github.com/{repo}/releases/download/{tag}/{file}"),
                code: stats.lang,
                name: stats.name,
                file,
                sha256,
                bytes: bytes.len() as u64,
                entries: stats.entries,
                forms: stats.forms,
                pct_with_forms: round(stats.pct_with_forms),
                pct_with_syllables: round(stats.pct_with_syllables),
                license: stats.license,
                source: stats.source,
                generated: stats.generated,
            },
        );
    }

    write_json(&dist.join(FILE_NAME), &manifest)?;
    std::fs::write(dist.join("NOTICE.md"), NOTICE)?;

    let total: u64 = manifest.languages.values().map(|l| l.bytes).sum();
    eprintln!(
        "{} languages, {:.1} MB total",
        manifest.languages.len(),
        total as f64 / 1_048_576.0
    );
    Ok(())
}

fn round(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}
