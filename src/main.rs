use std::collections::HashSet;
use std::env;
use std::fmt::Write as FmtWrite;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::process;

use chrono::NaiveDateTime;
use regex::Regex;
use sha2::{Digest, Sha256};

struct Clipping {
    title: String,
    author: String,
    author_citation: String,
    page: Option<String>,
    location: Option<String>,
    date_raw: String,
    date_parsed: Option<NaiveDateTime>,
    body: String,
}

impl Clipping {
    fn message_id(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.title.as_bytes());
        hasher.update(b"\x00");
        hasher.update(self.author.as_bytes());
        hasher.update(b"\x00");
        if let Some(ref loc) = self.location {
            hasher.update(loc.as_bytes());
        } else if let Some(ref page) = self.page {
            hasher.update(page.as_bytes());
        }
        let hash = hasher.finalize();
        let hex: String = hash.iter().take(20).map(|b| format!("{:02x}", b)).collect();
        format!("<{}@clippings2mbox>", hex)
    }

    fn subject(&self) -> String {
        let mut s = if self.author_citation.is_empty() {
            format!("\"{}\"", self.title)
        } else {
            format!("{}, \"{}\"", self.author_citation, self.title)
        };
        if let Some(ref page) = self.page {
            write!(s, ", {}", page).unwrap();
        }
        s
    }

    fn from_name(&self) -> String {
        if self.author.is_empty() {
            let escaped = self.title.replace('\\', "\\\\").replace('"', "\\\"");
            format!("\"{}\"", escaped)
        } else {
            self.author.clone()
        }
    }

    fn to_mbox(&self) -> String {
        let envelope_sender = "kindle@clippings2mbox";
        let date_rfc2822 = self
            .date_parsed
            .map(|d| d.format("%a, %d %b %Y %H:%M:%S +0000").to_string())
            .unwrap_or_else(|| self.date_raw.clone());
        let envelope_date = self
            .date_parsed
            .map(|d| d.format("%a %b %e %H:%M:%S %Y").to_string())
            .unwrap_or_else(|| "Thu Jan  1 00:00:00 1970".to_string());

        let mut msg = String::new();
        writeln!(msg, "From {} {}", envelope_sender, envelope_date).unwrap();
        writeln!(msg, "From: {} <{}>", self.from_name(), envelope_sender).unwrap();
        writeln!(msg, "Date: {}", date_rfc2822).unwrap();
        writeln!(msg, "Subject: {}", self.subject()).unwrap();
        writeln!(msg, "Message-ID: {}", self.message_id()).unwrap();
        writeln!(msg, "MIME-Version: 1.0").unwrap();
        writeln!(msg, "Content-Type: text/plain; charset=utf-8").unwrap();
        writeln!(msg).unwrap();
        for line in self.body.lines() {
            if line.starts_with("From ") {
                writeln!(msg, ">{}", line).unwrap();
            } else {
                writeln!(msg, "{}", line).unwrap();
            }
        }
        writeln!(msg).unwrap();
        msg
    }
}

fn parse_date(s: &str) -> Option<NaiveDateTime> {
    let s = s.trim();
    for fmt in &[
        "%A, %B %d, %Y %I:%M:%S %p",
        "%A, %B %d, %Y %H:%M:%S",
    ] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(s, fmt) {
            return Some(dt);
        }
    }
    None
}

fn parse_clippings(content: &str) -> Vec<Clipping> {
    let page_re = Regex::new(r"(?i)\bpage\s+([\w-]+)").unwrap();
    let location_re = Regex::new(r"(?i)\blocation\s+(\d[\d-]*)").unwrap();
    let date_re = Regex::new(r"(?i)\bAdded on\s+(.+)$").unwrap();

    let mut clippings = Vec::new();

    for entry in content.split("==========") {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }

        let lines: Vec<&str> = entry.lines().collect();
        if lines.len() < 2 {
            continue;
        }

        let first_line = lines[0].trim().trim_start_matches('\u{feff}');

        let (title, author, author_citation) = if first_line.ends_with(')') {
            if let Some(pos) = first_line.rfind('(') {
                let raw = first_line[pos + 1..first_line.len() - 1].trim();
                let (author, citation) = if let Some((last, first)) = raw.split_once(',') {
                    (format!("{} {}", first.trim(), last.trim()), last.trim().to_string())
                } else {
                    (raw.to_string(), raw.to_string())
                };
                (first_line[..pos].trim().to_string(), author, citation)
            } else {
                (first_line.to_string(), String::new(), String::new())
            }
        } else {
            (first_line.trim().to_string(), String::new(), String::new())
        };

        let meta = lines[1].trim();
        let page = page_re.captures(meta).map(|c| c[1].to_string());
        let location = location_re.captures(meta).map(|c| c[1].to_string());
        let date_raw = date_re
            .captures(meta)
            .map(|c| c[1].trim().to_string())
            .unwrap_or_default();
        let date_parsed = parse_date(&date_raw);

        let mut body_start = 2;
        while body_start < lines.len() && lines[body_start].trim().is_empty() {
            body_start += 1;
        }
        let body: String = lines[body_start..].join("\n").trim().to_string();

        if body.is_empty() {
            continue;
        }

        clippings.push(Clipping {
            title,
            author,
            author_citation,
            page,
            location,
            date_raw,
            date_parsed,
            body,
        });
    }

    clippings
}

fn read_existing_ids(path: &Path) -> HashSet<String> {
    let mut ids = HashSet::new();
    if let Ok(content) = fs::read_to_string(path) {
        let re = Regex::new(r"(?m)^Message-ID:\s*(.+)$").unwrap();
        for cap in re.captures_iter(&content) {
            ids.insert(cap[1].trim().to_string());
        }
    }
    ids
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: {} <clippings.txt> <output.mbox>", args[0]);
        process::exit(1);
    }

    let input = &args[1];
    let output = Path::new(&args[2]);

    let content = fs::read_to_string(input).unwrap_or_else(|e| {
        eprintln!("Error reading {}: {}", input, e);
        process::exit(1);
    });

    let clippings = parse_clippings(&content);
    let mut seen = read_existing_ids(output);

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(output)
        .unwrap_or_else(|e| {
            eprintln!("Error opening {}: {}", output.display(), e);
            process::exit(1);
        });

    let mut added = 0;
    let mut skipped = 0;

    for clip in &clippings {
        let id = clip.message_id();
        if !seen.insert(id) {
            skipped += 1;
            continue;
        }
        file.write_all(clip.to_mbox().as_bytes()).unwrap();
        added += 1;
    }

    eprintln!(
        "{} clippings: {} added, {} duplicates skipped",
        clippings.len(),
        added,
        skipped
    );
}
