//! Automatically suggested, unreviewed historical event claims (Atlas 7.1; issue #107).
//!
//! The canonical NLP producer (nlp-stack `text-linguistics`) has no temporal normalizer: its
//! `DATE` entities are untyped token spans and the local `bert-base-NER` model has no date label.
//! Until nlp-stack owns historical temporal normalization (nlp-stack#156), this module is the
//! bounded producer adapter for it: a deterministic recognizer of dated expressions (days, months,
//! years, ranges, decades, centuries, BC/AD, approximation) in transcript segments and visual
//! text (OCR) tracks.
//! Everything else stays with the existing owners: participants and places come from the nlp-stack
//! entity mentions already linked to canonical identities (#104), and claims, events and review
//! are `crate::events` (#106).
//!
//! A suggestion is always `unverified`: it records its producer, run, the exact matched text and
//! byte range, and a confidence, and is never presented as an established fact. Reruns keep
//! reviewed claims (confirmed or rejected) and unchanged suggestions in place and drop only
//! suggestions the new run no longer produces.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::{Captures, Regex};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::entities::{video_text_sources, EntityModality};
use crate::events::{insert_claim, ClaimExtraction, ClaimInput, ClaimPredicate, EventDate};

/// Producer identity recorded on every extraction run and suggested claim.
pub const EVENT_CLAIM_PRODUCER: &str = "youtube-corpus/historical-date-mentions";
/// Bumped whenever the recognizer's rules change, so reruns are attributable.
pub const EVENT_CLAIM_PRODUCER_VERSION: &str = "1";
/// Date expressions below this confidence (e.g. a bare four-digit number) are not suggested.
pub const MIN_DATE_CONFIDENCE: f32 = 0.6;
/// Longest fallback event label taken from the evidence text.
const LABEL_CHARS: usize = 72;

/// One dated expression found in a piece of evidence text.
#[derive(Debug, Clone, PartialEq)]
pub struct DateMention {
    pub byte_start: usize,
    pub byte_end: usize,
    /// The expression exactly as it appears in the text.
    pub text: String,
    pub date: EventDate,
    pub confidence: f32,
    /// `stated` or `hedged` (the text hedges, e.g. "probably").
    pub certainty: &'static str,
    /// `affirms` or `denies` (e.g. "not in 1066").
    pub polarity: &'static str,
}

/// Mentions at or above [`MIN_DATE_CONFIDENCE`], and how many weaker ones were dropped.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DateMentions {
    pub mentions: Vec<DateMention>,
    pub low_confidence_dropped: usize,
}

const MONTH: &str = r"((?:january|february|march|april|may|june|july|august|september|october|november|december|januar|februar|märz|maerz|mai|juni|juli|oktober|dezember|jan|feb|mar|apr|jun|jul|aug|sept|sep|oct|okt|nov|dec|dez)\b)\.?";
const ERA: &str = r"(b\.\s?c\.(?:e\.)?|(?:bce|bc|ad|ce)\b|a\.\s?d\.|c\.e\.|v\.\s?chr\.?|vor christus\b|n\.\s?chr\.?|nach christus\b)";

fn ci(pattern: &str) -> Regex {
    Regex::new(&format!(r"(?i)\b(?:{pattern})")).expect("valid date pattern")
}

static FULL_DAY_FIRST: LazyLock<Regex> = LazyLock::new(|| {
    ci(&format!(
        r"(\d{{1,2}})(?:st|nd|rd|th|\.)?\s+(?:of\s+)?{MONTH},?\s+(\d{{1,4}})(?:\s*{ERA})?"
    ))
});
static FULL_MONTH_FIRST: LazyLock<Regex> = LazyLock::new(|| {
    ci(&format!(
        r"{MONTH}\s+(\d{{1,2}})(?:st|nd|rd|th)?,?\s+(\d{{1,4}})(?:\s*{ERA})?"
    ))
});
static MONTH_YEAR: LazyLock<Regex> =
    LazyLock::new(|| ci(&format!(r"{MONTH}\s+(?:of\s+)?(\d{{1,4}})(?:\s*{ERA})?")));
static RANGE: LazyLock<Regex> = LazyLock::new(|| {
    ci(&format!(
        r"(?:(from|between|von|zwischen)\s+)?(?:the\s+years?\s+)?(\d{{1,4}})(?:\s*{ERA})?\s*(-|–|—|to|until|till|and|bis|und)\s*(\d{{1,4}})(?:\s*{ERA})?"
    ))
});
static CENTURY: LazyLock<Regex> = LazyLock::new(|| {
    ci(&format!(
        r"(?:(early|mid|late|frühen|fruehen|späten|spaeten)[\s-]+)?(\d{{1,2}}(?:st|nd|rd|th|\.)|first|second|third|fourth|fifth|sixth|seventh|eighth|ninth|tenth|eleventh|twelfth|thirteenth|fourteenth|fifteenth|sixteenth|seventeenth|eighteenth|nineteenth|twentieth|twenty-first)\s*-?\s*(century|centuries|jahrhunderts?)(?:\s*{ERA})?"
    ))
});
static DECADE: LazyLock<Regex> =
    LazyLock::new(|| ci(r"(?:(early|mid|late)[\s-]+)?(\d{3})0(?:'?s|er(?:\s+jahren?)?)"));
static ERA_YEAR: LazyLock<Regex> = LazyLock::new(|| {
    ci(&format!(
        r"(\d{{1,4}})\s*{ERA}|(?:a\.\s?d\.|ad\b|anno domini)\s+(\d{{1,4}})"
    ))
});
static CUE_YEAR: LazyLock<Regex> = LazyLock::new(|| {
    ci(&format!(
        r"(in the year|the year|im jahre?|in|since|until|till|before|after|during|from|around|about|circa|ca\.|c\.|approximately|roughly|seit|bis|um|etwa|anno)\s+(\d{{1,4}})(?:\s*{ERA})?"
    ))
});
static BARE_YEAR: LazyLock<Regex> = LazyLock::new(|| ci(r"(1\d{3}|20\d{2})"));

static NAMED_EVENT: LazyLock<Regex> = LazyLock::new(|| {
    ci(
        r"(?:(?:first|second|third|fourth|great|holy)\s+)?(?:battle|siege|treaty|council|fall|sack|war|revolution|conquest|crusade|edict|peace|massacre|coronation|crucifixion|destruction|founding|invasion|discovery|death|birth|assassination|schism|exile|burning|schlacht|belagerung|konzil|vertrag|frieden|zerstörung|eroberung|tod|geburt)\s+(?:of|at|on|von|der|des|bei|zu)\s+(?:the\s+)?[\p{L}'’-]+(?:\s+[\p{L}'’-]+){0,2}|world\s+war\s+(?:one|two|i{1,2}|1|2)|(?:the\s+)?(?:protestant\s+)?reformation|(?:the\s+)?(?:french|american|russian|industrial|glorious)\s+revolution|(?:the\s+)?great\s+schism|(?:the\s+)?black\s+death|(?:the\s+)?holocaust|(?:the\s+)?cold\s+war",
    )
});

/// Words that end a named-event phrase (`Battle of Hastings in 1066` → `Battle of Hastings`).
const LABEL_STOP: &[&str] = &[
    "in", "on", "at", "was", "were", "is", "and", "by", "from", "of", "the", "a", "an", "to",
    "which", "that", "when", "where", "who", "happened", "took", "im", "um", "war", "ist", "und",
    "fand", "statt", "am", "wurde", "die", "das", "der", "around", "about", "during", "after",
    "before", "since", "until", "between",
];
/// Lower-case connecting words of a label.
const LABEL_LOWER: &[&str] = &["of", "the", "at", "on", "von", "der", "des", "bei", "zu"];

/// A number followed by one of these is a quantity, time or reference, never a date.
const UNIT_WORDS: &[&str] = &[
    "people",
    "persons",
    "person",
    "years",
    "year",
    "yrs",
    "percent",
    "%",
    "dollars",
    "euros",
    "euro",
    "pounds",
    "mark",
    "marks",
    "timestamp",
    "times",
    "subscribers",
    "subs",
    "followers",
    "views",
    "likes",
    "comments",
    "meters",
    "metres",
    "feet",
    "foot",
    "miles",
    "km",
    "kilometers",
    "kilometres",
    "pages",
    "page",
    "words",
    "verses",
    "manuscripts",
    "copies",
    "men",
    "women",
    "soldiers",
    "troops",
    "members",
    "students",
    "o'clock",
    "minutes",
    "minute",
    "seconds",
    "second",
    "hours",
    "hour",
    "days",
    "day",
    "am",
    "pm",
    "a.m.",
    "p.m.",
    "point",
    "plus",
    "k",
    "million",
    "billion",
    "thousand",
    "hundred",
    "calories",
    "degrees",
    "menschen",
    "leute",
    "jahre",
    "jahren",
    "prozent",
    "seiten",
    "stunden",
    "minuten",
    "uhr",
];
/// A number preceded by one of these is a reference or amount, never a date.
const REFERENCE_WORDS: &[&str] = &[
    "number", "no.", "nr.", "page", "verse", "chapter", "room", "#", "$", "€", "£", "at", "over",
    "under", "past", "top", "psalm", "seite", "kapitel",
];
const HEDGES: &[&str] = &[
    "probably",
    "possibly",
    "perhaps",
    "maybe",
    "might",
    "may have",
    "supposedly",
    "allegedly",
    "reportedly",
    "apparently",
    "i think",
    "i believe",
    "some say",
    "traditionally",
    "wahrscheinlich",
    "vermutlich",
    "vielleicht",
    "angeblich",
    "möglicherweise",
];
const NEGATIONS: &[&str] = &["not", "never", "nicht", "nie", "kein", "keine"];
const APPROXIMATE: &[&str] = &[
    "around",
    "about",
    "circa",
    "ca.",
    "c.",
    "approximately",
    "roughly",
    "um",
    "etwa",
    "ungefähr",
    "gegen",
];

fn month_number(name: &str) -> Option<i16> {
    let name = name.trim_end_matches('.').to_lowercase();
    Some(match name.as_str() {
        "january" | "januar" | "jan" => 1,
        "february" | "februar" | "feb" => 2,
        "march" | "märz" | "maerz" | "mar" => 3,
        "april" | "apr" => 4,
        "may" | "mai" => 5,
        "june" | "juni" | "jun" => 6,
        "july" | "juli" | "jul" => 7,
        "august" | "aug" => 8,
        "september" | "sept" | "sep" => 9,
        "october" | "oktober" | "oct" | "okt" => 10,
        "november" | "nov" => 11,
        "december" | "dezember" | "dec" | "dez" => 12,
        _ => return None,
    })
}

fn is_bc(era: &str) -> bool {
    let era = era.to_lowercase();
    era.starts_with('b') || era.starts_with('v')
}

fn ordinal(word: &str) -> Option<i32> {
    let digits: String = word.chars().take_while(char::is_ascii_digit).collect();
    if !digits.is_empty() {
        return digits.parse().ok();
    }
    const WORDS: &[&str] = &[
        "first",
        "second",
        "third",
        "fourth",
        "fifth",
        "sixth",
        "seventh",
        "eighth",
        "ninth",
        "tenth",
        "eleventh",
        "twelfth",
        "thirteenth",
        "fourteenth",
        "fifteenth",
        "sixteenth",
        "seventeenth",
        "eighteenth",
        "nineteenth",
        "twentieth",
        "twenty-first",
    ];
    let word = word.to_lowercase();
    WORDS.iter().position(|w| *w == word).map(|i| i as i32 + 1)
}

/// Words before `start` (lower-case, nearest last).
fn words_before(text: &str, start: usize, n: usize) -> Vec<String> {
    let mut words: Vec<String> = text[..start]
        .split_whitespace()
        .rev()
        .take(n)
        .map(|w| {
            w.trim_matches(|c: char| c == ',' || c == '(' || c == '"')
                .to_lowercase()
        })
        .collect();
    words.reverse();
    words
}

fn next_word(text: &str, end: usize) -> String {
    let rest = text[end..].trim_start();
    if rest.starts_with('%') {
        return "%".into();
    }
    rest.split_whitespace()
        .next()
        .unwrap_or("")
        .trim_end_matches([',', '.', ';', ':', '!', '?', ')'])
        .to_lowercase()
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The match does not start or end inside a word or number.
fn on_boundaries(text: &str, start: usize, end: usize) -> bool {
    let before = text[..start].chars().next_back();
    let after = text[end..].chars().next();
    !before.is_some_and(|c| is_word_char(c) || c == '.' && start >= 2)
        && !after.is_some_and(|c| is_word_char(c) || c == ':')
        && !before.is_some_and(|c| c == '$' || c == '#' || c == ':')
}

struct Candidate {
    start: usize,
    end: usize,
    date: EventDate,
    confidence: f32,
}

fn point(year: i32, month: Option<i16>, day: Option<i16>) -> EventDate {
    EventDate {
        start_year: year,
        start_month: month,
        start_day: day,
        ..EventDate::default()
    }
}

fn era_year(year: i32, era: Option<&str>) -> i32 {
    match era {
        Some(era) if is_bc(era) => -year,
        _ => year,
    }
}

fn plausible(year: i32, era: bool) -> bool {
    year != 0 && (era || (100..=2100).contains(&year))
}

fn full_date(
    caps: &Captures,
    day: usize,
    month: usize,
    year: usize,
    era: usize,
) -> Option<EventDate> {
    let day: i16 = caps[day].parse().ok()?;
    let month = month_number(&caps[month])?;
    let era = caps.get(era).map(|m| m.as_str());
    let year: i32 = caps[year].parse().ok()?;
    if !(1..=31).contains(&day) || !plausible(year, era.is_some()) {
        return None;
    }
    Some(point(era_year(year, era), Some(month), Some(day)))
}

fn century_range(n: i32, bc: bool, part: Option<&str>) -> Option<EventDate> {
    if !(1..=21).contains(&n) {
        return None;
    }
    let (from, to, approximate) = match part.map(str::to_lowercase).as_deref() {
        Some("early" | "frühen" | "fruehen") => (1, 33, true),
        Some("mid") => (34, 66, true),
        Some("late" | "späten" | "spaeten") => (67, 100, true),
        _ => (1, 100, false),
    };
    let base = (n - 1) * 100;
    let (start, end) = if bc {
        // The 5th century BC runs from 500 BC down to 401 BC.
        (-(base + 101 - from), -(base + 101 - to))
    } else {
        (base + from, base + to)
    };
    Some(EventDate {
        start_year: start,
        end_year: Some(end),
        approximate,
        ..EventDate::default()
    })
}

fn candidates(text: &str) -> Vec<Candidate> {
    let mut found = Vec::new();
    let mut push = |m: regex::Match, date: Option<EventDate>, confidence: f32| {
        if let Some(date) = date {
            found.push(Candidate {
                start: m.start(),
                end: m.end(),
                date,
                confidence,
            });
        }
    };
    for caps in FULL_DAY_FIRST.captures_iter(text) {
        push(caps.get(0).unwrap(), full_date(&caps, 1, 2, 3, 4), 0.95);
    }
    for caps in FULL_MONTH_FIRST.captures_iter(text) {
        push(caps.get(0).unwrap(), full_date(&caps, 2, 1, 3, 4), 0.95);
    }
    for caps in MONTH_YEAR.captures_iter(text) {
        let era = caps.get(3).map(|m| m.as_str());
        let date = caps[2].parse::<i32>().ok().and_then(|year| {
            let month = month_number(&caps[1])?;
            // "May 12" is a day, not a year; months only take three- or four-digit years.
            (plausible(year, era.is_some()) && (era.is_some() || caps[2].len() >= 3))
                .then(|| point(era_year(year, era), Some(month), None))
        });
        push(caps.get(0).unwrap(), date, 0.9);
    }
    for caps in RANGE.captures_iter(text) {
        let keyword = caps.get(1).map(|m| m.as_str().to_lowercase());
        let separator = caps[4].to_lowercase();
        let (start_text, end_text) = (&caps[2], &caps[5]);
        let (start_era, end_era) = (
            caps.get(3).map(|m| m.as_str()),
            caps.get(6).map(|m| m.as_str()),
        );
        let with_era = start_era.is_some() || end_era.is_some();
        let joins = matches!(separator.as_str(), "and" | "und");
        if joins && !matches!(keyword.as_deref(), Some("between" | "zwischen")) {
            continue;
        }
        if keyword.is_none() && !with_era && (start_text.len() < 4 || separator == "until") {
            continue;
        }
        if !with_era && start_text.len() < 3 {
            continue;
        }
        let Ok(start) = start_text.parse::<i32>() else {
            continue;
        };
        let Ok(mut end) = end_text.parse::<i32>() else {
            continue;
        };
        let dash = matches!(separator.as_str(), "-" | "–" | "—");
        if !with_era && end_text.len() < start_text.len() && (!dash || end_text.len() > 2) {
            continue;
        }
        // "1914-18": an abbreviated end keeps the start's leading digits.
        if !with_era && end_text.len() < start_text.len() {
            let scale = 10_i32.pow(end_text.len() as u32);
            end += start - start % scale;
        }
        let start_era = start_era.or(end_era);
        let end_era = end_era.or(start_era);
        let (start, end) = (era_year(start, start_era), era_year(end, end_era));
        if !plausible(start.abs(), with_era) || !plausible(end.abs(), with_era) || end <= start {
            continue;
        }
        let confidence = if keyword.is_some() || with_era {
            0.9
        } else {
            0.8
        };
        push(
            caps.get(0).unwrap(),
            Some(EventDate {
                start_year: start,
                end_year: Some(end),
                ..EventDate::default()
            }),
            confidence,
        );
    }
    for caps in CENTURY.captures_iter(text) {
        let bc = caps.get(4).is_some_and(|m| is_bc(m.as_str()));
        let date =
            ordinal(&caps[2]).and_then(|n| century_range(n, bc, caps.get(1).map(|m| m.as_str())));
        push(caps.get(0).unwrap(), date, 0.85);
    }
    for caps in DECADE.captures_iter(text) {
        let decade: i32 = caps[2].parse().unwrap_or(0) * 10;
        let (from, to, approximate) =
            match caps.get(1).map(|m| m.as_str().to_lowercase()).as_deref() {
                Some("early") => (0, 3, true),
                Some("mid") => (3, 6, true),
                Some("late") => (6, 9, true),
                _ => (0, 9, false),
            };
        // "1000s of people" is a quantity.
        let quantity = next_word(text, caps.get(0).unwrap().end()) == "of";
        let date = ((1000..=2090).contains(&decade) && !quantity).then(|| EventDate {
            start_year: decade + from,
            end_year: Some(decade + to),
            approximate,
            ..EventDate::default()
        });
        push(caps.get(0).unwrap(), date, 0.8);
    }
    for caps in ERA_YEAR.captures_iter(text) {
        let date = match (caps.get(1), caps.get(3)) {
            (Some(year), _) => year
                .as_str()
                .parse::<i32>()
                .ok()
                .filter(|y| *y > 0)
                .map(|y| point(era_year(y, Some(&caps[2])), None, None)),
            (None, Some(year)) => year
                .as_str()
                .parse::<i32>()
                .ok()
                .filter(|y| *y > 0)
                .map(|y| point(y, None, None)),
            _ => None,
        };
        push(caps.get(0).unwrap(), date, 0.95);
    }
    for caps in CUE_YEAR.captures_iter(text) {
        let cue = caps[1].to_lowercase();
        let year_cue = cue.contains("year") || cue.starts_with("im jahr") || cue == "anno";
        let year: i32 = caps[2].parse().unwrap_or(0);
        let era = caps.get(3).map(|m| m.as_str());
        let ok = if era.is_some() {
            year > 0
        } else if year_cue {
            (100..=2100).contains(&year)
        } else {
            caps[2].len() == 4 && (1000..=2100).contains(&year)
        };
        // A plain cue ("in") only lends confidence; "the year" is part of the expression.
        let whole = caps.get(0).unwrap();
        if ok {
            found.push(Candidate {
                start: if year_cue {
                    whole.start()
                } else {
                    caps.get(2).unwrap().start()
                },
                end: whole.end(),
                date: point(era_year(year, era), None, None),
                confidence: if year_cue || era.is_some() { 0.9 } else { 0.8 },
            });
        }
    }
    for m in BARE_YEAR.find_iter(text) {
        let year: i32 = m.as_str().parse().unwrap_or(0);
        found.push(Candidate {
            start: m.start(),
            end: m.end(),
            date: point(year, None, None),
            confidence: 0.45,
        });
    }
    found
}

/// Finds dated expressions in one piece of evidence text.
pub fn find_date_mentions(text: &str) -> DateMentions {
    let lower = text.to_lowercase();
    // "the 1148 timestamp", "in 1148": four-digit numbers next to video timestamps are times.
    let timestamps = contains_phrase(&lower, "timestamp") || contains_phrase(&lower, "time stamp");
    let mut all: Vec<Candidate> = candidates(text)
        .into_iter()
        .filter(|c| on_boundaries(text, c.start, c.end) && c.date.is_valid())
        .filter(|c| !(timestamps && c.date.end_year.is_none() && c.date.start_month.is_none()))
        .filter(|c| {
            let next = next_word(text, c.end);
            let before = words_before(text, c.start, 1);
            !UNIT_WORDS.contains(&next.as_str())
                && !before
                    .last()
                    .is_some_and(|w| REFERENCE_WORDS.contains(&w.as_str()))
        })
        .collect();
    // Longest, then most confident expression wins; overlapping weaker readings are dropped.
    all.sort_by(|a, b| {
        (b.end - b.start)
            .cmp(&(a.end - a.start))
            .then(b.confidence.total_cmp(&a.confidence))
            .then(a.start.cmp(&b.start))
    });
    let mut chosen: Vec<Candidate> = Vec::new();
    for candidate in all {
        if chosen
            .iter()
            .all(|c| candidate.end <= c.start || c.end <= candidate.start)
        {
            chosen.push(candidate);
        }
    }
    chosen.sort_by_key(|c| c.start);

    let hedged = HEDGES.iter().any(|hedge| contains_phrase(&lower, hedge));
    let mut result = DateMentions::default();
    for mut candidate in chosen {
        if candidate.confidence < MIN_DATE_CONFIDENCE {
            result.low_confidence_dropped += 1;
            continue;
        }
        let before = words_before(text, candidate.start, 3);
        if before
            .last()
            .is_some_and(|w| APPROXIMATE.contains(&w.as_str()))
        {
            candidate.date.approximate = true;
        }
        let denies = before
            .iter()
            .any(|w| NEGATIONS.contains(&w.as_str()) || w.ends_with("n't") || w.ends_with("n’t"));
        result.mentions.push(DateMention {
            byte_start: candidate.start,
            byte_end: candidate.end,
            text: text[candidate.start..candidate.end].to_string(),
            date: candidate.date,
            confidence: candidate.confidence,
            certainty: if hedged { "hedged" } else { "stated" },
            polarity: if denies { "denies" } else { "affirms" },
        });
    }
    result
}

fn contains_phrase(lower: &str, phrase: &str) -> bool {
    lower.match_indices(phrase).any(|(i, _)| {
        let before = lower[..i].chars().next_back();
        let after = lower[i + phrase.len()..].chars().next();
        !before.is_some_and(is_word_char) && !after.is_some_and(is_word_char)
    })
}

fn title_case(phrase: &str) -> String {
    phrase
        .split_whitespace()
        .enumerate()
        .map(|(i, word)| {
            let lower = word.to_lowercase();
            if i > 0 && LABEL_LOWER.contains(&lower.as_str()) {
                lower
            } else {
                let mut chars = lower.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().chain(chars).collect(),
                    None => String::new(),
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The event a date mention is about: the nearest named event in the same text
/// (`Battle of Hastings`), else the evidence text around the mention, quoted.
pub fn event_label(text: &str, mention: &DateMention) -> String {
    let named = NAMED_EVENT
        .find_iter(text)
        .filter(|m| m.end() <= mention.byte_start || m.start() >= mention.byte_end)
        .min_by_key(|m| {
            if m.end() <= mention.byte_start {
                mention.byte_start - m.end()
            } else {
                m.start() - mention.byte_end
            }
        });
    if let Some(m) = named {
        let mut words: Vec<&str> = m.as_str().split_whitespace().collect();
        if words.first().is_some_and(|w| w.eq_ignore_ascii_case("the")) {
            words.remove(0);
        }
        // Cut the place/object after its connector at the first non-name word.
        if let Some(connector) = words
            .iter()
            .position(|w| LABEL_LOWER.contains(&w.to_lowercase().as_str()))
        {
            let mut end = connector + 1;
            while end < words.len() && words[end].eq_ignore_ascii_case("the") {
                end += 1;
            }
            let name_start = end;
            // One name word, more only while they are capitalized ("of Stamford Bridge").
            while end < words.len()
                && (end == name_start || words[end].chars().next().is_some_and(char::is_uppercase))
                && !LABEL_STOP.contains(&words[end].to_lowercase().as_str())
                && !words[end].chars().any(|c| c.is_ascii_digit())
            {
                end += 1;
            }
            if end > name_start {
                words.truncate(end);
                return title_case(&words.join(" "));
            }
        } else {
            return title_case(&words.join(" "));
        }
    }
    let collapsed: Vec<&str> = text.split_whitespace().collect();
    let joined = collapsed.join(" ");
    if joined.chars().count() <= LABEL_CHARS {
        return format!("“{joined}”");
    }
    // Keep the words around the mention.
    let target = mention.text.split_whitespace().next().unwrap_or("");
    let at = collapsed
        .iter()
        .position(|w| w.contains(target))
        .unwrap_or(0);
    let (mut from, mut to) = (at, at + 1);
    let len = |from: usize, to: usize| collapsed[from..to].join(" ").chars().count();
    while (from > 0 || to < collapsed.len()) && len(from, to) < LABEL_CHARS {
        if to < collapsed.len() && len(from, to + 1) <= LABEL_CHARS {
            to += 1;
        }
        if from > 0 && len(from - 1, to) <= LABEL_CHARS {
            from -= 1;
        } else if to >= collapsed.len() || len(from, to + 1) > LABEL_CHARS {
            break;
        }
    }
    format!(
        "“{}{}{}”",
        if from > 0 { "…" } else { "" },
        collapsed[from..to].join(" "),
        if to < collapsed.len() { "…" } else { "" }
    )
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventClaimExtractionReport {
    pub producer: String,
    pub producer_version: String,
    pub videos_processed: usize,
    pub videos_without_text: usize,
    pub segments_analyzed: usize,
    pub visual_text_tracks_analyzed: usize,
    /// New unreviewed suggestions.
    pub claims_suggested: usize,
    /// Earlier unreviewed suggestions the run produced again (kept with their grouping).
    pub claims_kept: usize,
    /// Suggestions matching a claim a researcher already confirmed or rejected.
    pub reviewed_skipped: usize,
    /// Earlier unreviewed suggestions the run no longer produces.
    pub stale_removed: usize,
    pub low_confidence_dropped: usize,
}

impl ClaimExtraction {
    /// A suggestion's provenance; the run id is set when the run is recorded.
    fn new(source_text: &str, start: usize, end: usize, confidence: f32) -> Self {
        Self {
            run_id: Uuid::nil(),
            producer: EVENT_CLAIM_PRODUCER.into(),
            producer_version: EVENT_CLAIM_PRODUCER_VERSION.into(),
            source_text: source_text.into(),
            source_byte_start: start as i32,
            source_byte_end: end as i32,
            confidence,
            extracted_at: None,
        }
    }
}

struct Suggestion {
    label: String,
    claim: ClaimInput,
    extraction: ClaimExtraction,
}

fn suggestion_key(
    evidence: Uuid,
    predicate: &str,
    date: Option<&EventDate>,
    identity: Option<Uuid>,
    source_text: &str,
) -> String {
    format!(
        "{evidence}|{predicate}|{:?}|{identity:?}|{source_text}",
        date.map(|d| (
            d.start_year,
            d.start_month,
            d.start_day,
            d.end_year,
            d.end_month,
            d.end_day,
            d.approximate
        ))
    )
}

/// Participants and places mentioned in one evidence record: nlp-stack entity mentions whose
/// corpus-local group is linked to a canonical identity.
async fn linked_mentions(
    pool: &PgPool,
    record: Uuid,
) -> anyhow::Result<Vec<(ClaimPredicate, Uuid, String, usize, usize, f32)>> {
    let rows = sqlx::query(
        "SELECT DISTINCT ON (l.identity_id)
                m.entity_type, m.surface_text, m.byte_start, m.byte_end, m.confidence, l.identity_id
         FROM entity_mentions m
         JOIN corpus_videos cv ON cv.video_id = m.video_id
         JOIN canonical_identity_links l
           ON l.unit_kind = 'entity_group' AND l.corpus_id = cv.corpus_id
          AND l.entity_type = m.entity_type AND l.entity_key = m.entity_key
         WHERE (m.transcript_segment_id = $1 OR m.visual_text_track_id = $1)
           AND m.entity_type IN ('person', 'organization', 'location')
         ORDER BY l.identity_id, m.byte_start",
    )
    .bind(record)
    .fetch_all(pool)
    .await?;
    let mut found = Vec::new();
    for row in rows {
        let entity_type: String = row.try_get("entity_type")?;
        let identity: Uuid = row.try_get("identity_id")?;
        let Some(identity) = crate::identities::live_identity(pool, identity).await? else {
            continue;
        };
        let start: i32 = row.try_get("byte_start")?;
        let end: i32 = row.try_get("byte_end")?;
        let confidence: Option<f64> = row.try_get("confidence")?;
        found.push((
            if entity_type == "location" {
                ClaimPredicate::Place
            } else {
                ClaimPredicate::Participant
            },
            identity,
            row.try_get("surface_text")?,
            start as usize,
            end as usize,
            confidence.unwrap_or(0.5) as f32,
        ));
    }
    Ok(found)
}

/// Suggests claims for one video, keeping reviewed claims and unchanged suggestions.
pub async fn extract_video_event_claims(
    pool: &PgPool,
    video_id: Uuid,
) -> anyhow::Result<EventClaimExtractionReport> {
    let sources = video_text_sources(pool, video_id).await?;
    let mut report = EventClaimExtractionReport {
        producer: EVENT_CLAIM_PRODUCER.into(),
        producer_version: EVENT_CLAIM_PRODUCER_VERSION.into(),
        videos_processed: 1,
        segments_analyzed: sources.segments,
        visual_text_tracks_analyzed: sources.sources.len() - sources.segments,
        ..EventClaimExtractionReport::default()
    };
    if sources.sources.is_empty() {
        report.videos_without_text = 1;
    }

    let mut suggestions = Vec::new();
    for source in &sources.sources {
        let found = find_date_mentions(&source.text);
        report.low_confidence_dropped += found.low_confidence_dropped;
        if found.mentions.is_empty() {
            continue;
        }
        let modality = match source.modality {
            EntityModality::Spoken => "spoken",
            EntityModality::VisibleText => "visible",
        };
        let base = |predicate, date, identity_id, polarity: &str, certainty: &str| ClaimInput {
            video_id,
            modality: modality.into(),
            evidence_id: source.record_id,
            predicate,
            date,
            identity_id,
            polarity: Some(polarity.into()),
            certainty: Some(certainty.into()),
            qualification: String::new(),
        };
        let linked = linked_mentions(pool, source.record_id).await?;
        let mut labels = Vec::new();
        for mention in &found.mentions {
            let label = event_label(&source.text, mention);
            suggestions.push(Suggestion {
                label: label.clone(),
                claim: base(
                    ClaimPredicate::Date,
                    Some(mention.date.clone()),
                    None,
                    mention.polarity,
                    mention.certainty,
                ),
                extraction: ClaimExtraction::new(
                    &mention.text,
                    mention.byte_start,
                    mention.byte_end,
                    mention.confidence,
                ),
            });
            if !labels.contains(&label) {
                labels.push(label);
            }
        }
        // Who/where the dated text names, inferred from co-occurrence, for each of its events.
        for label in &labels {
            for (predicate, identity, surface, start, end, confidence) in &linked {
                suggestions.push(Suggestion {
                    label: label.clone(),
                    claim: ClaimInput {
                        qualification: "Named in the same evidence as the date".into(),
                        ..base(*predicate, None, Some(*identity), "affirms", "inferred")
                    },
                    extraction: ClaimExtraction::new(surface, *start, *end, *confidence),
                });
            }
        }
    }

    let mut tx = pool.begin().await?;
    let existing = sqlx::query(
        "SELECT id, review_status, evidence_id, predicate, identity_id, source_text,
                date_start_year, date_start_month, date_start_day,
                date_end_year, date_end_month, date_end_day, date_approximate
         FROM historical_event_claims
         WHERE video_id = $1 AND extraction_run_id IS NOT NULL
         FOR UPDATE",
    )
    .bind(video_id)
    .fetch_all(&mut *tx)
    .await?;
    let mut by_key: HashMap<String, (Uuid, String)> = HashMap::new();
    for row in &existing {
        let date = crate::events::date_from_row(row, "")?;
        let predicate: String = row.try_get("predicate")?;
        let source_text: String = row.try_get("source_text")?;
        by_key.insert(
            suggestion_key(
                row.try_get("evidence_id")?,
                &predicate,
                date.as_ref(),
                row.try_get("identity_id")?,
                &source_text,
            ),
            (row.try_get("id")?, row.try_get("review_status")?),
        );
    }

    let run = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO historical_event_extraction_runs
           (id, video_id, producer, producer_version, config, transcript_stream_id,
            segments_analyzed, visual_text_tracks_analyzed, low_confidence_dropped,
            claims_suggested)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 0)",
    )
    .bind(run)
    .bind(video_id)
    .bind(EVENT_CLAIM_PRODUCER)
    .bind(EVENT_CLAIM_PRODUCER_VERSION)
    .bind(format!("min-confidence={MIN_DATE_CONFIDENCE}"))
    .bind(sources.stream_id)
    .bind(report.segments_analyzed as i32)
    .bind(report.visual_text_tracks_analyzed as i32)
    .bind(report.low_confidence_dropped as i32)
    .execute(&mut *tx)
    .await?;

    let mut kept: Vec<Uuid> = Vec::new();
    let mut seen = HashSet::new();
    let mut events: HashMap<String, Uuid> = HashMap::new();
    for suggestion in suggestions {
        let key = suggestion_key(
            suggestion.claim.evidence_id,
            suggestion.claim.predicate.as_str(),
            suggestion.claim.date.as_ref(),
            suggestion.claim.identity_id,
            &suggestion.extraction.source_text,
        );
        if !seen.insert(key.clone()) {
            continue;
        }
        if let Some((id, status)) = by_key.get(&key) {
            if status == "unverified" {
                sqlx::query(
                    "UPDATE historical_event_claims
                     SET extraction_run_id = $2, extraction_confidence = $3,
                         source_byte_start = $4, source_byte_end = $5
                     WHERE id = $1",
                )
                .bind(id)
                .bind(run)
                .bind(suggestion.extraction.confidence)
                .bind(suggestion.extraction.source_byte_start)
                .bind(suggestion.extraction.source_byte_end)
                .execute(&mut *tx)
                .await?;
                kept.push(*id);
                report.claims_kept += 1;
            } else {
                report.reviewed_skipped += 1;
            }
            continue;
        }
        let folded = suggestion.label.to_lowercase();
        let event = match events.get(&folded) {
            Some(event) => *event,
            None => {
                let found: Option<Uuid> = sqlx::query_scalar(
                    "SELECT id FROM historical_events
                     WHERE merged_into IS NULL AND lower(label) = $1
                     ORDER BY created_at, id LIMIT 1",
                )
                .bind(&folded)
                .fetch_optional(&mut *tx)
                .await?;
                let event = match found {
                    Some(event) => event,
                    None => {
                        let event = Uuid::new_v4();
                        sqlx::query("INSERT INTO historical_events (id, label) VALUES ($1, $2)")
                            .bind(event)
                            .bind(&suggestion.label)
                            .execute(&mut *tx)
                            .await?;
                        event
                    }
                };
                events.insert(folded, event);
                event
            }
        };
        let extraction = ClaimExtraction {
            run_id: run,
            ..suggestion.extraction
        };
        let id = insert_claim(&mut tx, event, &suggestion.claim, Some(&extraction)).await?;
        kept.push(id);
        report.claims_suggested += 1;
    }

    let emptied: Vec<Uuid> = sqlx::query_scalar(
        "DELETE FROM historical_event_claims
         WHERE video_id = $1 AND extraction_run_id IS NOT NULL
           AND review_status = 'unverified' AND NOT (id = ANY($2))
         RETURNING event_id",
    )
    .bind(video_id)
    .bind(&kept)
    .fetch_all(&mut *tx)
    .await?;
    report.stale_removed = emptied.len();
    // Events that only held the removed suggestions go with them.
    sqlx::query(
        "DELETE FROM historical_events e
         WHERE e.id = ANY($1) AND e.merged_into IS NULL
           AND NOT EXISTS (SELECT 1 FROM historical_event_claims c WHERE c.event_id = e.id)
           AND NOT EXISTS (SELECT 1 FROM historical_events r WHERE r.merged_into = e.id)",
    )
    .bind(&emptied)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE historical_event_extraction_runs SET claims_suggested = $2 WHERE id = $1")
        .bind(run)
        .bind((report.claims_suggested + report.claims_kept) as i32)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(report)
}

/// Suggests claims for every video of a corpus.
pub async fn extract_corpus_event_claims(
    pool: &PgPool,
    corpus_id: Uuid,
) -> anyhow::Result<EventClaimExtractionReport> {
    let mut video_ids: Vec<Uuid> = crate::corpora::corpus_video_ids(pool, corpus_id)
        .await?
        .into_iter()
        .collect();
    video_ids.sort();
    let mut total = EventClaimExtractionReport {
        producer: EVENT_CLAIM_PRODUCER.into(),
        producer_version: EVENT_CLAIM_PRODUCER_VERSION.into(),
        ..EventClaimExtractionReport::default()
    };
    for video_id in video_ids {
        let report = extract_video_event_claims(pool, video_id).await?;
        total.videos_processed += report.videos_processed;
        total.videos_without_text += report.videos_without_text;
        total.segments_analyzed += report.segments_analyzed;
        total.visual_text_tracks_analyzed += report.visual_text_tracks_analyzed;
        total.claims_suggested += report.claims_suggested;
        total.claims_kept += report.claims_kept;
        total.reviewed_skipped += report.reviewed_skipped;
        total.stale_removed += report.stale_removed;
        total.low_confidence_dropped += report.low_confidence_dropped;
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dates(text: &str) -> Vec<(String, EventDate)> {
        find_date_mentions(text)
            .mentions
            .into_iter()
            .map(|m| (m.text, m.date))
            .collect()
    }

    fn year(y: i32) -> EventDate {
        point(y, None, None)
    }

    fn range(a: i32, b: i32) -> EventDate {
        EventDate {
            start_year: a,
            end_year: Some(b),
            ..EventDate::default()
        }
    }

    // Lines below are excerpts of real YouTube auto-captions from the local evaluation corpus.
    #[test]
    fn real_caption_lines_yield_dates_and_reject_numbers() {
        assert_eq!(
            dates("In 2010, he did a missions trip to"),
            vec![("2010".into(), year(2010))]
        );
        assert_eq!(
            dates("miracles. Hume was an 18th century"),
            vec![("18th century".into(), range(1701, 1800))]
        );
        assert_eq!(
            dates("everybody's favorite 6th century"),
            vec![("6th century".into(), range(501, 600))]
        );
        assert_eq!(
            dates("as in someone writing i a first century"),
            vec![("first century".into(), range(1, 100))]
        );
        assert_eq!(
            dates("population in 2010 to 29% in 2020."),
            vec![("2010".into(), year(2010)), ("2020".into(), year(2020))]
        );
        assert_eq!(
            dates("on the timeline there's the year 2022 ad"),
            vec![("the year 2022 ad".into(), year(2022))]
        );
        // False dates: timestamps, counts and amounts.
        for line in [
            "to the 1148 timestamp if you actually",
            "the 1130 reaper did not kill fred",
            "went to the 1148 Mark of palegia's",
            "heads and you're one of 1000 people or",
            "celebration video for having past 1500",
            "and 11 o'clock at 1107 in 30 seconds",
            "been going on for 2000 years is",
            "At this rate, in a century, -21% of the",
            "divided by 1001 roughly 99.9 percent uh",
            "timestamp in 1148 if there's any shred",
        ] {
            assert_eq!(dates(line), vec![], "{line}");
        }
    }

    #[test]
    fn bare_years_are_low_confidence_and_dropped() {
        let found = find_date_mentions("of skeptical atheism Paul Draper 2015");
        assert!(found.mentions.is_empty());
        assert_eq!(found.low_confidence_dropped, 1);
    }

    #[test]
    fn days_months_eras_and_ranges() {
        assert_eq!(
            dates("The Battle of Hastings was fought on 14 October 1066."),
            vec![("14 October 1066".into(), point(1066, Some(10), Some(14)))]
        );
        assert_eq!(
            dates("on March 15, 44 BC Caesar died"),
            vec![("March 15, 44 BC".into(), point(-44, Some(3), Some(15)))]
        );
        assert_eq!(dates("in 586 BC"), vec![("586 BC".into(), year(-586))]);
        assert_eq!(
            dates("around AD 70"),
            vec![(
                "AD 70".into(),
                EventDate {
                    approximate: true,
                    ..year(70)
                }
            )]
        );
        assert_eq!(
            dates("the war lasted from 1914 to 1918"),
            vec![("from 1914 to 1918".into(), range(1914, 1918))]
        );
        assert_eq!(
            dates("in 1914-18"),
            vec![("1914-18".into(), range(1914, 1918))]
        );
        assert_eq!(
            dates("from 500 to 300 BC"),
            vec![("from 500 to 300 BC".into(), range(-500, -300))]
        );
        assert_eq!(
            dates("in the 1960s"),
            vec![("1960s".into(), range(1960, 1969))]
        );
        assert_eq!(
            dates("the 5th century BC"),
            vec![("5th century BC".into(), range(-500, -401))]
        );
        assert_eq!(
            dates("late 15th century"),
            vec![(
                "late 15th century".into(),
                EventDate {
                    approximate: true,
                    ..range(1467, 1500)
                }
            )]
        );
        assert_eq!(dates("between 2 and 3 people"), vec![]);
    }

    #[test]
    fn german_dates() {
        assert_eq!(
            dates("Die Schlacht fand am 14. Oktober 1066 statt"),
            vec![("14. Oktober 1066".into(), point(1066, Some(10), Some(14)))]
        );
        assert_eq!(
            dates("im 4. Jahrhundert"),
            vec![("4. Jahrhundert".into(), range(301, 400))]
        );
        assert_eq!(
            dates("um 44 v. Chr."),
            vec![(
                "44 v. Chr.".into(),
                EventDate {
                    approximate: true,
                    ..year(-44)
                }
            )]
        );
        assert_eq!(
            dates("im Jahr 1517"),
            vec![("im Jahr 1517".into(), year(1517))]
        );
    }

    #[test]
    fn approximation_hedging_and_negation() {
        let found = find_date_mentions("it was probably written around 1450");
        let mention = &found.mentions[0];
        assert!(mention.date.approximate);
        assert_eq!(mention.certainty, "hedged");
        let found = find_date_mentions("the council did not meet in 1054");
        assert_eq!(found.mentions[0].polarity, "denies");
        assert_eq!(found.mentions[0].certainty, "stated");
    }

    #[test]
    fn labels_name_the_event_or_quote_the_evidence() {
        let text = "The Battle of Hastings was fought on 14 October 1066.";
        let found = find_date_mentions(text);
        assert_eq!(event_label(text, &found.mentions[0]), "Battle of Hastings");
        let text = "the council of nicaea in 325 ad settled it";
        let found = find_date_mentions(text);
        assert_eq!(event_label(text, &found.mentions[0]), "Council of Nicaea");
        let text = "In 2010, he did a missions trip to";
        let found = find_date_mentions(text);
        assert_eq!(
            event_label(text, &found.mentions[0]),
            "“In 2010, he did a missions trip to”"
        );
    }
}
