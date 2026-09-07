use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use clap::Parser;
use flate2::read::MultiGzDecoder;
use lexeme::lexicon::{Meta, write_entry};
use serde::Deserialize;

const JUNK_TAGS: [&str; 4] = [
    "table-tags",
    "inflection-template",
    "class",
    "error-unrecognized-form",
];
const JUNK_FORMS: [&str; 4] = ["", "-", "\u{2014}", "no-table-tags"];
const IPA_VOWELS: &str = "aeiouyæɑɐɒɛɜɞəɘɪɨʊʉʌɔøœɶɤɯɵʏɚɝ";

#[derive(Parser)]
#[command(about = "Compile a Wiktextract dump into a lexeme lexicon")]
struct Args {
    #[arg(long)]
    lang: String,
    #[arg(long)]
    name: String,
    #[arg(long)]
    source: String,
    #[arg(long, default_value = "-")]
    input: PathBuf,
    #[arg(long)]
    out: PathBuf,
    #[arg(long)]
    stats: Option<PathBuf>,
    #[arg(long, default_value = "CC-BY-SA-4.0")]
    license: String,
    #[arg(long, default_value_t = 19)]
    level: i32,
    #[arg(long)]
    skip_multiword: bool,
    #[arg(long)]
    lang_code: Option<String>,
    #[arg(long)]
    generated: Option<String>,
}

#[derive(Deserialize)]
struct RawEntry {
    #[serde(default)]
    word: String,
    #[serde(default)]
    pos: String,
    #[serde(default)]
    lang_code: String,
    #[serde(default)]
    redirect: Option<String>,
    #[serde(default)]
    forms: Vec<RawForm>,
    #[serde(default)]
    senses: Vec<RawSense>,
    #[serde(default)]
    sounds: Vec<RawSound>,
}

#[derive(Deserialize)]
struct RawForm {
    #[serde(default)]
    form: String,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Deserialize)]
struct RawSense {
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    form_of: Vec<FormOf>,
}

#[derive(Deserialize)]
struct FormOf {
    #[serde(default)]
    word: String,
}

#[derive(Deserialize)]
struct RawSound {
    #[serde(default)]
    ipa: Option<String>,
}

#[derive(Default)]
struct Acc {
    ipa: String,
    forms: HashSet<(String, String)>,
}

struct Link {
    lemma: String,
    pos: String,
    surface: String,
    tags: String,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("dictgen: {err}");
        std::process::exit(1);
    }
}

fn run() -> io::Result<()> {
    let args = Args::parse();
    let mut input = open_input(&args.input)?;

    let mut entries: HashMap<(String, String), Acc> = HashMap::new();
    let mut links: Vec<Link> = Vec::new();
    let mut tag_counts: HashMap<String, u64> = HashMap::new();

    let mut lines_read = 0u64;
    let mut parse_errors = 0u64;
    let mut skipped_lang = 0u64;
    let mut skipped_redirect = 0u64;
    let mut form_of_entries = 0u64;

    let mut line = String::new();
    loop {
        line.clear();
        if input.read_line(&mut line)? == 0 {
            break;
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            continue;
        }
        lines_read += 1;
        if lines_read.is_multiple_of(250_000) {
            eprintln!("  read {lines_read} lines, {} lemmas", entries.len());
        }

        let raw: RawEntry = if let Ok(raw) = serde_json::from_str(trimmed) {
            raw
        } else {
            parse_errors += 1;
            continue;
        };

        if raw.redirect.is_some() {
            skipped_redirect += 1;
            continue;
        }
        if raw.word.is_empty() || raw.pos.is_empty() {
            continue;
        }
        if let Some(expected) = &args.lang_code
            && !raw.lang_code.is_empty()
            && &raw.lang_code != expected
        {
            skipped_lang += 1;
            continue;
        }
        if args.skip_multiword && raw.word.split_whitespace().count() > 1 {
            continue;
        }
        if !is_clean(&raw.word) {
            continue;
        }

        for sense in &raw.senses {
            let Some(target) = sense.form_of.first() else {
                continue;
            };
            if target.word.is_empty() || target.word == raw.word {
                continue;
            }
            let tags = join_tags(sense.tags.iter().filter(|t| *t != "form-of"));
            links.push(Link {
                lemma: target.word.clone(),
                pos: raw.pos.clone(),
                surface: raw.word.clone(),
                tags,
            });
        }

        let all_senses_are_forms = !raw.senses.is_empty()
            && raw
                .senses
                .iter()
                .all(|s| !s.form_of.is_empty() || s.tags.iter().any(|t| t == "form-of"));
        if all_senses_are_forms {
            form_of_entries += 1;
            continue;
        }

        let acc = entries
            .entry((raw.word.clone(), raw.pos.clone()))
            .or_default();

        if acc.ipa.is_empty()
            && let Some(ipa) = raw.sounds.iter().find_map(|s| s.ipa.as_deref())
        {
            let ipa = strip_ipa_delimiters(ipa);
            if !ipa.is_empty() && is_clean(ipa) {
                ipa.clone_into(&mut acc.ipa);
            }
        }

        for form in &raw.forms {
            if form.tags.iter().any(|t| JUNK_TAGS.contains(&t.as_str())) {
                continue;
            }
            let surface = form.form.trim();
            if JUNK_FORMS.contains(&surface) || !is_clean(surface) {
                continue;
            }
            if surface == raw.word && form.tags.is_empty() {
                continue;
            }
            for tag in &form.tags {
                *tag_counts.entry(tag.clone()).or_default() += 1;
            }
            acc.forms
                .insert((surface.to_owned(), join_tags(form.tags.iter())));
        }
    }

    let mut unresolved_links = 0u64;
    for link in links {
        match entries.get_mut(&(link.lemma, link.pos)) {
            Some(acc) => {
                acc.forms.insert((link.surface, link.tags));
            }
            None => unresolved_links += 1,
        }
    }

    let mut ordered: Vec<_> = entries.into_iter().collect();
    ordered.sort_by(|a, b| a.0.cmp(&b.0));

    let generated = args.generated.clone().unwrap_or_else(now_rfc3339);
    let mut meta = Meta {
        lang: args.lang.clone(),
        name: args.name.clone(),
        source: args.source.clone(),
        generated: generated.clone(),
        license: args.license.clone(),
        entries: ordered.len() as u64,
        forms: 0,
    };

    let mut entries_with_forms = 0u64;
    let mut entries_with_ipa = 0u64;
    let mut entries_with_syllables = 0u64;
    let mut body = Vec::new();
    for ((word, pos), acc) in &ordered {
        let mut forms: Vec<_> = acc.forms.iter().cloned().collect();
        forms.sort();
        let syllables = syllables_from_ipa(&acc.ipa);

        meta.forms += forms.len() as u64;
        if !forms.is_empty() {
            entries_with_forms += 1;
        }
        if !acc.ipa.is_empty() {
            entries_with_ipa += 1;
        }
        if syllables > 0 {
            entries_with_syllables += 1;
        }
        write_entry(&mut body, word, pos, syllables, &acc.ipa, &forms)?;
    }

    if let Some(parent) = args.out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let mut out = zstd::Encoder::new(File::create(&args.out)?, args.level)?;
    meta.write_to(&mut out)?;
    out.write_all(&body)?;
    out.finish()?.flush()?;

    let written = std::fs::metadata(&args.out)?.len();
    eprintln!(
        "{}: {} entries, {} forms, {:.1}% with forms, {:.1}% with syllables, {} bytes",
        args.lang,
        meta.entries,
        meta.forms,
        pct(entries_with_forms, meta.entries),
        pct(entries_with_syllables, meta.entries),
        written,
    );

    if let Some(path) = &args.stats {
        let mut top_tags: Vec<_> = tag_counts.into_iter().collect();
        top_tags.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let top_tags: BTreeMap<_, _> = top_tags.into_iter().take(30).collect();

        let report = serde_json::json!({
            "lang": args.lang,
            "name": args.name,
            "source": args.source,
            "generated": generated,
            "license": args.license,
            "bytes": written,
            "lines_read": lines_read,
            "parse_errors": parse_errors,
            "skipped_redirect": skipped_redirect,
            "skipped_lang": skipped_lang,
            "form_of_entries": form_of_entries,
            "unresolved_links": unresolved_links,
            "entries": meta.entries,
            "forms": meta.forms,
            "entries_with_forms": entries_with_forms,
            "entries_with_ipa": entries_with_ipa,
            "entries_with_syllables": entries_with_syllables,
            "pct_with_forms": pct(entries_with_forms, meta.entries),
            "pct_with_syllables": pct(entries_with_syllables, meta.entries),
            "top_tags": top_tags,
        });
        write_json(path, &report)?;
    }

    Ok(())
}

fn open_input(path: &Path) -> io::Result<Box<dyn BufRead>> {
    let raw: Box<dyn Read> = if path.as_os_str() == "-" {
        Box::new(io::stdin().lock())
    } else {
        Box::new(File::open(path)?)
    };
    let mut buf = BufReader::with_capacity(1 << 20, raw);
    let gzipped = buf.fill_buf()?.starts_with(&[0x1f, 0x8b]);
    Ok(if gzipped {
        Box::new(BufReader::with_capacity(1 << 20, MultiGzDecoder::new(buf)))
    } else {
        Box::new(buf)
    })
}

fn is_clean(s: &str) -> bool {
    !s.is_empty() && !s.chars().any(|c| c.is_control())
}

fn join_tags<'a>(tags: impl Iterator<Item = &'a String>) -> String {
    let mut tags: Vec<&str> = tags
        .map(String::as_str)
        .filter(|t| !t.is_empty() && !t.contains(',') && !t.contains(':') && !t.contains('\u{1f}'))
        .collect();
    tags.sort_unstable();
    tags.dedup();
    tags.join(",")
}

fn strip_ipa_delimiters(ipa: &str) -> &str {
    ipa.trim().trim_matches(|c| matches!(c, '/' | '[' | ']'))
}

fn syllables_from_ipa(ipa: &str) -> u8 {
    if ipa.is_empty() {
        return 0;
    }
    let count = if ipa.contains('.') {
        ipa.split('.').filter(|part| has_nucleus(part)).count()
    } else {
        let mut count = 0usize;
        let mut in_nucleus = false;
        for ch in ipa.chars() {
            if is_syllabic_mark(ch) {
                if !in_nucleus {
                    count += 1;
                    in_nucleus = true;
                }
            } else if IPA_VOWELS.contains(ch) {
                if !in_nucleus {
                    count += 1;
                }
                in_nucleus = true;
            } else if !is_nucleus_continuation(ch) {
                in_nucleus = false;
            }
        }
        count
    };
    count.min(u8::MAX as usize) as u8
}

fn has_nucleus(part: &str) -> bool {
    part.chars()
        .any(|c| IPA_VOWELS.contains(c) || is_syllabic_mark(c))
}

fn is_syllabic_mark(ch: char) -> bool {
    ch == '\u{329}' || ch == '\u{30d}'
}

fn is_nucleus_continuation(ch: char) -> bool {
    matches!(ch, '\u{300}'..='\u{36f}') || ch == 'ː' || ch == 'ˑ'
}

fn pct(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        return 0.0;
    }
    (part as f64 / whole as f64) * 100.0
}

fn write_json(path: &Path, value: &serde_json::Value) -> io::Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = File::create(path)?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.write_all(b"\n")
}

fn now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
        .cast_signed();
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
