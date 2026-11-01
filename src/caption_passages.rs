//! Retrieval passages for rolling automatic captions (#92).
//!
//! YouTube automatic captions roll: each cue redisplays the previous line and short refresh cues
//! repeat it again, so indexing cues directly yields duplicated, stuttering 10 ms fragments.
//! Foundation's `media_core::project_rolling_captions` (owner decision nlp-stack#123 Q1: A)
//! turns the raw cues into each spoken word once, with its timing and introducing cue; this
//! module groups those words into the corpus's retrieval passages. The raw cues stay untouched
//! and are persisted next to the passages (`transcript_source_cues`).

use std::collections::BTreeMap;

use media_core::{
    project_rolling_captions, RollingCaptionWord, RollingWordTiming, TimedTextContract,
    TimedTextSegmentContract, ROLLING_CAPTION_PROJECTION,
};
use text_transcripts::{TranscriptSegmentContract, TranscriptWordContract};

/// Version of the passage representation. Bump it whenever the passages built from the same
/// cues can change, since passage boundaries define the `span_sha256` of citations.
pub const CAPTION_PASSAGES: &str = "rolling-caption-passages@1";

/// A passage closes at a sentence end once it holds this many words.
pub const SENTENCE_BREAK_MIN_WORDS: usize = 12;
/// A passage never holds more words than this.
pub const MAX_WORDS: usize = 40;
/// A passage never spans more seconds than this.
pub const MAX_SECONDS: f64 = 30.0;
/// A silence longer than this between two words closes the passage.
pub const PAUSE_SECONDS: f64 = 2.0;

/// YouTube's speaker-change marker: a passage boundary, not speech.
const SPEAKER_CHANGE: &str = ">>";

/// Provenance recorded in the stream's processing config.
pub fn segmentation_provenance() -> serde_json::Value {
    serde_json::json!({
        "representation": CAPTION_PASSAGES,
        "projection": ROLLING_CAPTION_PROJECTION,
        "sourceCues": "transcript_source_cues",
        "sentenceBreakMinWords": SENTENCE_BREAK_MIN_WORDS,
        "maxWords": MAX_WORDS,
        "maxSeconds": MAX_SECONDS,
        "pauseSeconds": PAUSE_SECONDS,
    })
}

/// Builds passages from raw (un-normalized) cues. Cue indices are kept, so a passage names the
/// source cues it came from.
pub fn build_passages(
    raw_cues: &[TranscriptSegmentContract],
) -> anyhow::Result<Vec<TranscriptSegmentContract>> {
    let contract = TimedTextContract::new(
        raw_cues
            .iter()
            .filter(|cue| cue.start_seconds.is_some() && cue.end_seconds.is_some())
            .map(|cue| {
                TimedTextSegmentContract::new(cue.index, cue.text.clone())
                    .with_time_range(cue.start_seconds, cue.end_seconds)
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| anyhow::anyhow!("invalid caption cue: {error:?}"))?,
    );
    let projection = project_rolling_captions(&contract)
        .map_err(|error| anyhow::anyhow!("rolling caption projection failed: {error:?}"))?;

    let mut passages = Vec::new();
    let mut current: Vec<&RollingCaptionWord> = Vec::new();
    for word in &projection.words {
        if word.text == SPEAKER_CHANGE {
            close(&mut current, &mut passages);
            continue;
        }
        if let Some(last) = current.last() {
            let first = current[0];
            if word.start_seconds - last.end_seconds > PAUSE_SECONDS
                || word.end_seconds - first.start_seconds > MAX_SECONDS
                || current.len() >= MAX_WORDS
            {
                close(&mut current, &mut passages);
            }
        }
        current.push(word);
        if current.len() >= SENTENCE_BREAK_MIN_WORDS && ends_sentence(&word.text) {
            close(&mut current, &mut passages);
        }
    }
    close(&mut current, &mut passages);
    Ok(passages)
}

fn ends_sentence(word: &str) -> bool {
    word.trim_end_matches(['"', '\'', ')', ']'])
        .ends_with(['.', '?', '!', '…'])
}

fn close(current: &mut Vec<&RollingCaptionWord>, passages: &mut Vec<TranscriptSegmentContract>) {
    if current.is_empty() {
        return;
    }
    let words = std::mem::take(current);
    let text = text_transcripts::normalize_subtitle_text(
        &words
            .iter()
            .map(|word| word.text.as_str())
            .collect::<Vec<_>>()
            .join(" "),
        text_transcripts::SubtitleNormalizationOptions::default(),
    );
    if text.is_empty() {
        return;
    }
    let mut passage = TranscriptSegmentContract::new(passages.len() as u64, text);
    passage.start_seconds = Some(words[0].start_seconds);
    passage.end_seconds = Some(words[words.len() - 1].end_seconds);
    passage.attributes = BTreeMap::from([
        ("firstSourceCue".to_string(), words[0].cue_index.to_string()),
        (
            "lastSourceCue".to_string(),
            words[words.len() - 1].cue_index.to_string(),
        ),
    ]);
    passage.words = words
        .iter()
        .map(|word| TranscriptWordContract {
            text: word.text.clone(),
            start_seconds: Some(word.start_seconds),
            end_seconds: Some(word.end_seconds),
            confidence: None,
            speaker: None,
            attributes: BTreeMap::from([
                ("sourceCue".to_string(), word.cue_index.to_string()),
                ("timing".to_string(), timing_label(word.timing).to_string()),
            ]),
        })
        .collect();
    passages.push(passage);
}

fn timing_label(timing: RollingWordTiming) -> &'static str {
    match timing {
        RollingWordTiming::Inline => "inline",
        RollingWordTiming::CueStart => "cue_start",
        RollingWordTiming::Interpolated => "interpolated",
    }
}

/// The passage fields stored in `transcript_segments.metadata` (`None` for a plain cue segment).
pub fn passage_metadata(segment: &TranscriptSegmentContract) -> Option<serde_json::Value> {
    if segment.words.is_empty() && segment.attributes.is_empty() {
        return None;
    }
    let cue = |key: &str| {
        segment
            .attributes
            .get(key)
            .and_then(|value| value.parse::<u64>().ok())
    };
    Some(serde_json::json!({
        "representation": CAPTION_PASSAGES,
        "firstSourceCue": cue("firstSourceCue"),
        "lastSourceCue": cue("lastSourceCue"),
        "words": segment.words.iter().map(|word| serde_json::json!({
            "text": word.text,
            "startSeconds": word.start_seconds,
            "endSeconds": word.end_seconds,
            "sourceCue": word.attributes.get("sourceCue").and_then(|value| value.parse::<u64>().ok()),
            "timing": word.attributes.get("timing"),
        })).collect::<Vec<_>>(),
    }))
}

/// Share of words that repeat the word sequence just before them (the stutter rolling captions
/// leave): for each segment, words covered by the longest suffix/prefix overlap with the
/// previous segment, over all words.
pub fn duplicated_word_rate(segments: &[&str]) -> f64 {
    let mut total = 0usize;
    let mut duplicated = 0usize;
    let mut previous: Vec<&str> = Vec::new();
    for segment in segments {
        let words = segment.split_whitespace().collect::<Vec<_>>();
        let overlap = (1..=words.len().min(previous.len()))
            .rev()
            .find(|&count| previous[previous.len() - count..] == words[..count])
            .unwrap_or(0);
        total += words.len();
        duplicated += overlap;
        previous = words;
    }
    if total == 0 {
        0.0
    } else {
        duplicated as f64 / total as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cue(index: u64, start: f64, end: f64, text: &str) -> TranscriptSegmentContract {
        let mut cue = TranscriptSegmentContract::new(index, text);
        cue.start_seconds = Some(start);
        cue.end_seconds = Some(end);
        cue
    }

    #[test]
    fn rolling_cues_become_clean_timed_passages() {
        // The two-cue replay from #92 plus the following YouTube-style rolling cues.
        let cues = [
            cue(
                0,
                26.44,
                27.95,
                "First, what are the requirements for consciousness?",
            ),
            cue(1, 27.95, 27.96, "consciousness?"),
            cue(
                2,
                27.96,
                30.0,
                "consciousness? Well<00:00:28.400><c> it</c><00:00:28.800><c> depends.</c>",
            ),
            cue(3, 30.0, 30.01, "Well it depends."),
            cue(4, 30.01, 31.0, "&gt;&gt; Right."),
        ];
        let passages = build_passages(&cues).unwrap();
        let texts = passages
            .iter()
            .map(|passage| passage.text.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            texts,
            [
                "First, what are the requirements for consciousness? Well it depends.",
                "Right."
            ]
        );
        assert_eq!(passages[0].start_seconds, Some(26.44));
        assert_eq!(passages[0].end_seconds, Some(30.0));
        let metadata = passage_metadata(&passages[0]).unwrap();
        assert_eq!(metadata["firstSourceCue"], 0);
        assert_eq!(metadata["lastSourceCue"], 2);
        assert_eq!(metadata["words"][8]["timing"], "inline");
        assert_eq!(metadata["words"][8]["startSeconds"], 28.4);
        // Short genuine speech survives as its own passage.
        assert!(passages[1].start_seconds.unwrap() >= 30.01);
        assert_eq!(passages[1].end_seconds, Some(31.0));
        assert_eq!(duplicated_word_rate(&texts), 0.0);
        let raw = cues
            .iter()
            .map(|cue| {
                text_transcripts::normalize_subtitle_text(
                    &cue.text,
                    text_transcripts::SubtitleNormalizationOptions::default(),
                )
            })
            .collect::<Vec<_>>();
        let raw = raw.iter().map(String::as_str).collect::<Vec<_>>();
        assert!(duplicated_word_rate(&raw) > 0.2);
    }

    #[test]
    fn long_speech_splits_at_sentences_and_limits() {
        let text = (0..50)
            .map(|index| {
                if index % 15 == 14 {
                    format!("w{index}.")
                } else {
                    format!("w{index}")
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        let passages = build_passages(&[cue(0, 0.0, 25.0, &text)]).unwrap();
        let lengths = passages
            .iter()
            .map(|passage| passage.words.len())
            .collect::<Vec<_>>();
        assert_eq!(lengths, [15, 15, 15, 5]);
        assert!(passages
            .windows(2)
            .all(|pair| pair[0].end_seconds <= pair[1].start_seconds));
    }
}
