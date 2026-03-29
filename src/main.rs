use std::collections::{HashMap, HashSet};
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
    location_start: Option<u32>,
    location_end: Option<u32>,
    date_raw: String,
    date_parsed: Option<NaiveDateTime>,
    body: String,
}

impl Clipping {
    fn hdr_message_id(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.title.as_bytes());
        hasher.update(b"\x00");
        hasher.update(self.author.as_bytes());
        hasher.update(b"\x00");
        if let Some(start) = self.location_start {
            let loc = match self.location_end {
                Some(end) => format!("{}-{}", start, end),
                None => start.to_string(),
            };
            hasher.update(loc.as_bytes());
        } else if let Some(ref page) = self.page {
            hasher.update(page.as_bytes());
        }
        let hash = hasher.finalize();
        let hex: String = hash.iter().take(20).map(|b| format!("{:02x}", b)).collect();
        format!("<{}@clippings2mbox>", hex)
    }

    fn hdr_subject(&self) -> String {
        let mut s = if self.author_citation.is_empty() {
            format!("\"{}\"", self.title)
        } else {
            format!("{}, \"{}\"", self.author_citation, self.title)
        };
        if let Some(ref page) = self.page {
            let p = match page.split_once('-') {
                Some((a, b)) if a == b => a,
                _ => page.as_str(),
            };
            write!(s, ", {}", p).unwrap();
        }
        s
    }

    fn hdr_from_name(&self) -> String {
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
        writeln!(
            msg,
            "From: {} <{}>",
            rfc2047_encode(&self.hdr_from_name()),
            envelope_sender
        )
        .unwrap();
        writeln!(msg, "Date: {}", date_rfc2822).unwrap();
        writeln!(msg, "Subject: {}", rfc2047_encode(&self.hdr_subject())).unwrap();
        writeln!(msg, "Message-ID: {}", self.hdr_message_id()).unwrap();
        if let Some(start) = self.location_start {
            writeln!(msg, "X-Kindle-Location-Start: {}", start).unwrap();
        }
        if let Some(end) = self.location_end {
            writeln!(msg, "X-Kindle-Location-End: {}", end).unwrap();
        }
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
    for fmt in &["%A, %B %d, %Y %I:%M:%S %p", "%A, %B %d, %Y %H:%M:%S"] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(s, fmt) {
            return Some(dt);
        }
    }
    None
}

fn parse_clippings(content: &str) -> Vec<Clipping> {
    let page_re = Regex::new(r"(?i)\bpage\s+([\w-]+)").unwrap();
    let location_re = Regex::new(r"(?i)\blocation\s+(\d+)(?:-(\d+))?").unwrap();
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
                    (
                        format!("{} {}", first.trim(), last.trim()),
                        last.trim().to_string(),
                    )
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
        let (location_start, location_end) = match location_re.captures(meta) {
            Some(cap) => (
                cap[1].parse::<u32>().ok(),
                cap.get(2).and_then(|m| m.as_str().parse::<u32>().ok()),
            ),
            None => (None, None),
        };
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
            location_start,
            location_end,
            date_raw,
            date_parsed,
            body,
        });
    }

    clippings
}

const MIN_OVERLAP: usize = 10;

fn suffix_prefix_overlap(a: &str, b: &str) -> usize {
    let (ab, bb) = (a.as_bytes(), b.as_bytes());
    let max_k = ab.len().min(bb.len());
    for k in (MIN_OVERLAP..=max_k).rev() {
        if ab[ab.len() - k..] == bb[..k] {
            return k;
        }
    }
    0
}

fn merge_clippings(clippings: Vec<Clipping>) -> Vec<Clipping> {
    // Record book first-appearance order
    let mut book_order: Vec<(String, String)> = Vec::new();
    let mut seen_books: HashSet<(String, String)> = HashSet::new();
    for clip in &clippings {
        let key = (clip.title.clone(), clip.author.clone());
        if seen_books.insert(key.clone()) {
            book_order.push(key);
        }
    }

    // Group by (title, author)
    let mut groups: HashMap<(String, String), Vec<Clipping>> = HashMap::new();
    for clip in clippings {
        let key = (clip.title.clone(), clip.author.clone());
        groups.entry(key).or_default().push(clip);
    }

    // Process each group
    let mut result = Vec::new();
    for key in book_order {
        let mut group = groups.remove(&key).unwrap();

        // Sort by location_start asc, then location_end desc (wider range first)
        group.sort_by(|a, b| {
            let ls_a = a.location_start.unwrap_or(u32::MAX);
            let ls_b = b.location_start.unwrap_or(u32::MAX);
            ls_a.cmp(&ls_b).then_with(|| {
                let le_a = a.location_end.unwrap_or(0);
                let le_b = b.location_end.unwrap_or(0);
                le_b.cmp(&le_a)
            })
        });

        // Greedy merge pass
        let mut merged: Vec<Clipping> = Vec::new();
        for b in group {
            if merged.is_empty() {
                merged.push(b);
                continue;
            }
            let a = merged.last_mut().unwrap();
            if a.body.contains(b.body.as_str()) {
                // b is a subset of a
                a.location_end = max_opt(a.location_end, b.location_end);
                a.date_parsed = min_date(a.date_parsed, b.date_parsed);
            } else if b.body.contains(a.body.as_str()) {
                // a is a subset of b; replace a with b, keeping a's metadata
                let saved_start = a.location_start;
                let saved_page = a.page.clone();
                let saved_date_parsed = a.date_parsed;
                let saved_date_raw = a.date_raw.clone();
                *a = b;
                a.location_start = min_opt(saved_start, a.location_start);
                a.page = saved_page;
                if min_date(saved_date_parsed, a.date_parsed) == saved_date_parsed {
                    a.date_parsed = saved_date_parsed;
                    a.date_raw = saved_date_raw;
                }
            } else {
                let mut overlap = suffix_prefix_overlap(&a.body, &b.body);
                // Ensure `overlap` is on a valid UTF-8 char boundary before slicing.
                while overlap > 0 && !b.body.is_char_boundary(overlap) {
                    overlap -= 1;
                }
                if overlap > 0 {
                    let tail = b.body[overlap..].to_string();
                    a.body.push_str(&tail);
                    a.location_end = max_opt(a.location_end, b.location_end);
                    a.date_parsed = min_date(a.date_parsed, b.date_parsed);
                } else if a.location_end.is_some() && a.location_end == b.location_start {
                    a.body.push(' ');
                    a.body.push_str(&b.body);
                    a.location_end = b.location_end;
                    a.date_parsed = min_date(a.date_parsed, b.date_parsed);
                } else {
                    merged.push(b);
                }
            }
        }

        // Re-sort by date for mbox ordering
        merged.sort_by(|a, b| a.date_parsed.cmp(&b.date_parsed));
        result.extend(merged);
    }

    result
}

fn min_opt(a: Option<u32>, b: Option<u32>) -> Option<u32> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (Some(x), None) | (None, Some(x)) => Some(x),
        (None, None) => None,
    }
}

fn max_opt(a: Option<u32>, b: Option<u32>) -> Option<u32> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.max(y)),
        (Some(x), None) | (None, Some(x)) => Some(x),
        (None, None) => None,
    }
}

fn min_date(a: Option<NaiveDateTime>, b: Option<NaiveDateTime>) -> Option<NaiveDateTime> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (Some(x), None) | (None, Some(x)) => Some(x),
        (None, None) => None,
    }
}

fn rfc2047_encode(s: &str) -> String {
    let mut encoded = String::new();
    for byte in s.bytes() {
        match byte {
            b' ' => encoded.push('_'),
            b'=' | b'?' | b'_' | 0..=31 | 127..=255 => {
                write!(encoded, "={:02X}", byte).unwrap();
            }
            _ => encoded.push(byte as char),
        }
    }
    format!("=?UTF-8?Q?{}?=", encoded)
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

#[cfg(test)]
mod tests {
    use super::*;

    const SELF_CONTAINED: &str = "\
\u{feff}The Morning Star (Knausgaard, Karl Ove)
- Your Highlight on page 309 | Location 5115-5117 | Added on Thursday, September 18, 2025 8:46:43 PM

I thought to myself, or pictured in my mind—for thoughts come as both pictures and words, a bit like light, which comes as both particles and waves, one could imagine, as indeed I had, many times.
==========";

    const SUBSET_PAIR: &str = "\
\u{feff}The Morning Star (Knausgaard, Karl Ove)
- Your Highlight on page 407 | Location 6552-6552 | Added on Friday, September 19, 2025 1:54:25 PM

mistake we make time and again, we think thoughts
==========
\u{feff}The Morning Star (Knausgaard, Karl Ove)
- Your Highlight on page 407 | Location 6552-6554 | Added on Friday, September 19, 2025 1:54:35 PM

But this is a mistake we make time and again, we think thoughts are isolated units, apart not only from our emotions, but also from the surroundings in which they are conceived. Which is probably why philosophers have always been so concerned with building systems, for in the system a thought is given its own designated place, independently of whatever happens outside of it;
==========";

    #[test]
    fn test_parse_self_contained() {
        let clips = parse_clippings(SELF_CONTAINED);
        assert_eq!(clips.len(), 1);
        let c = &clips[0];
        assert_eq!(c.title, "The Morning Star");
        assert_eq!(c.author, "Karl Ove Knausgaard");
        assert_eq!(c.author_citation, "Knausgaard");
        assert_eq!(c.page.as_deref(), Some("309"));
        assert_eq!(c.location_start, Some(5115));
        assert_eq!(c.location_end, Some(5117));
        assert_eq!(
            c.body,
            "I thought to myself, or pictured in my mind\u{2014}for thoughts come as both \
pictures and words, a bit like light, which comes as both particles and waves, one could \
imagine, as indeed I had, many times."
        );
        assert_eq!(
            c.hdr_message_id(),
            "<9af5bd567f2bb98d157aa10aac09d5931401e6b0@clippings2mbox>"
        );
    }

    #[test]
    fn test_merge_subset() {
        let clips = parse_clippings(SUBSET_PAIR);
        assert_eq!(clips.len(), 2);
        let merged = merge_clippings(clips);
        assert_eq!(merged.len(), 1);
        let c = &merged[0];
        assert_eq!(
            c.body,
            "But this is a mistake we make time and again, we think thoughts are isolated units, \
apart not only from our emotions, but also from the surroundings in which they are conceived. \
Which is probably why philosophers have always been so concerned with building systems, for in \
the system a thought is given its own designated place, independently of whatever happens outside of it;"
        );
        assert_eq!(c.location_start, Some(6552));
        assert_eq!(c.location_end, Some(6554));
    }

    const ADJACENT_PAIR: &str = "\
\u{feff}The Morning Star (Knausgaard, Karl Ove)
- Your Highlight on page 52 | Location 900-903 | Added on Friday, September 12, 2025 11:51:23 PM

I missed the discussions we always had, the openness toward the world and life that had felt so much a part of them. Perhaps that openness had been naive, but it was certainly genuine. In those days I'd thought that was how life was going to be. We squandered our time and thoughts, and only when it was over did I understand that it had all been unique and would never return. That is what life is like, is it not? When we're young we think there's more to come, that this is only the beginning, whereas in fact it's all there
==========
\u{feff}The Morning Star (Knausgaard, Karl Ove)
- Your Highlight on page 52 | Location 903-906 | Added on Friday, September 12, 2025 11:51:40 PM

and what we have now, and barely even think about, will soon be the only thing we ever had. There had been no new abundance of friends, only Camilla, Helle and Sigbj\u{f8}rn, and no new abundance of thoughts; the ones we'd had then were the ones we still have now.
==========";

    #[test]
    fn test_merge_adjacent() {
        let clips = parse_clippings(ADJACENT_PAIR);
        assert_eq!(clips.len(), 2);
        let merged = merge_clippings(clips);
        assert_eq!(merged.len(), 1);
        let c = &merged[0];
        assert!(
            c.body.contains("it's all there and what we have now"),
            "adjacent clips must be joined with a space; got: {:?}",
            &c.body[c.body.find("there").unwrap_or(0)..],
        );
        assert_eq!(c.location_start, Some(900));
        assert_eq!(c.location_end, Some(906));
    }
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
    let clippings = merge_clippings(clippings);
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
        let id = clip.hdr_message_id();
        if !seen.insert(id) {
            skipped += 1;
            continue;
        }
        file.write_all(clip.to_mbox().as_bytes())
            .unwrap_or_else(|e| {
                eprintln!("Error writing {}: {}", output.display(), e);
                process::exit(1);
            });
        added += 1;
    }

    eprintln!(
        "{} clippings: {} added, {} duplicates skipped",
        clippings.len(),
        added,
        skipped
    );
}
