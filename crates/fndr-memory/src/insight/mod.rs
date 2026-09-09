//! Deterministic derivation of the "insight fields" (T-601): the no-LLM,
//! no-VLM description of a capture, built from already-assembled structured
//! fields and OCR salience rather than a model call.
//!
//! Ported from
//! `origin/reference/v1:src-tauri/src/memory_insight/{derive.rs,mod.rs}`
//! under ADR-005. Like `continuity.rs` and `fusion.rs`, this module has no
//! store or model dependency: callers supply already-computed structured
//! fields (from [`crate::fusion::build_low_ram_semantic_fusion`] or a future
//! VLM/LLM synthesis stage) and salient text, and retain ownership of
//! persistence.
//!
//! v1's `derive_insight_for_record` mutated a 104-field `MemoryRecord` in
//! place, the flattened-table shape ADR-005 lists under DISCARD. This port
//! instead reads the same [`crate::fusion::StructuredExtraction`] the
//! composer functions already consume, so this crate never needs that flat
//! shape.

mod fluff;

pub use fluff::strip_fluff;

use fndr_textsignal::{AppIdentity, SalientSpan, rank_salient_spans, salience_concentration};

use crate::fusion::StructuredExtraction;

const TOP_SPAN_DEBUG: usize = 8;
const DROP_SCORE: f32 = 0.12;
const MAX_WHAT_CHARS: usize = 280;
const MAX_WHY_CHARS: usize = 320;
const MAX_CHANGED_CHARS: usize = 400;

/// Insight fields already supplied by a prior synthesis pass (VLM/LLM,
/// T-602). When `what_happened` is non-empty, [`derive_insight`] only
/// fluff-strips these values instead of deriving fresh ones: v1's idempotent
/// pass-through, ported so a later synthesis stage composes cleanly with
/// this one without rework.
#[derive(Debug, Clone, Copy, Default)]
pub struct PrefilledInsight<'a> {
    pub what_happened: &'a str,
    pub why_mattered: &'a str,
    pub what_changed: &'a str,
    pub context_thread: &'a str,
    pub card_confidence: f32,
}

/// Already-assembled fields this module derives insight text from. Borrowed;
/// the caller still owns gathering these and writing the result.
#[derive(Debug, Clone, Copy, Default)]
pub struct AssembledMemory<'a> {
    pub app_name: &'a str,
    pub bundle_id: Option<&'a str>,
    pub window_title: &'a str,
    pub url: Option<&'a str>,
    /// Already-cleaned OCR text (e.g. `fndr_textsignal::HighSignalText::text`).
    pub clean_text: &'a str,
    /// A VLM/LLM-produced summary, if any; empty when only OCR ran.
    pub display_summary: &'a str,
    pub snippet: &'a str,
    pub session_id: &'a str,
    pub related_memory_ids: &'a [String],
    pub ocr_confidence: f32,
    pub ocr_noise_score: f32,
    pub extraction: Option<&'a StructuredExtraction>,
    pub prefilled: Option<PrefilledInsight<'a>>,
}

/// The derived insight fields. `what_happened` is `Option` because it is the
/// one field the T-601 acceptance criterion ("no-LLM path produces complete
/// records") treats as load-bearing: `None` is the typed, visible signal
/// that nothing here could be derived, so a caller never mistakes an empty
/// string for a real (if terse) description (invariant 4, "no silent
/// degradation"). `why_mattered`/`what_changed`/`context_thread` stay plain
/// `String`s because v1's own test suite treats an empty value for those as
/// a legitimate, non-degraded outcome (e.g. a why-mattered-less capture is
/// still a complete record; see `why_mattered_does_not_dump_short_ocr_span`).
#[derive(Debug, Clone, Default)]
pub struct DerivedInsight {
    pub what_happened: Option<String>,
    pub why_mattered: String,
    pub what_changed: String,
    pub context_thread: String,
    pub card_confidence: f32,
    pub top_spans: Vec<SalientSpan>,
    pub dropped_spans: Vec<String>,
}

fn pollution_for_insight(record: &AssembledMemory<'_>, app: AppIdentity<'_>) -> f32 {
    let noise = record.ocr_noise_score.clamp(0.0, 1.0);
    let concentration = salience_concentration(record.clean_text, app);
    let diffusion = (1.0 - concentration).clamp(0.0, 1.0);
    ((noise * 0.55) + (diffusion * 0.45)).clamp(0.0, 1.0)
}

/// Returns true when `value` is a machine-generated placeholder rather than
/// real content: filenames with timestamps, "Screen capture (visual): X.png.
/// App", "Captured recent activity at HH:MM", etc.
fn is_template_summary(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return true;
    }
    let lower = trimmed.to_ascii_lowercase();

    // Filename with embedded timestamp: contains .png AND 6+ digits.
    let has_png = lower.contains(".png");
    let digit_count = lower.chars().filter(|character| character.is_ascii_digit()).count();
    if has_png && digit_count >= 6 {
        return true;
    }

    lower.starts_with("screen capture (visual)")
        || lower.starts_with("captured recent")
        || lower.starts_with("viewed content on")
        || lower.starts_with("url-only surface capture")
        || (lower.starts_with("viewed ") && lower.contains(" at "))
}

/// Strip redundant repetition of app/project names from a candidate insight
/// string, e.g. "Google Chrome - Google Chrome - Google Chrome".
fn dedupe_repeating_phrases(value: &str) -> String {
    let parts: Vec<&str> = value.split(['|', '·', '—']).collect();
    if parts.len() < 2 {
        return value.to_string();
    }
    let mut seen = std::collections::HashSet::new();
    let mut kept: Vec<&str> = Vec::new();
    for part in parts {
        let key = part.trim().to_ascii_lowercase();
        if key.is_empty() {
            continue;
        }
        if seen.insert(key) {
            kept.push(part.trim());
        }
    }
    kept.join(" — ")
}

fn clip_chars(value: String, max: usize) -> String {
    if value.chars().count() <= max {
        return value;
    }
    let trimmed: String = value.chars().take(max.saturating_sub(1)).collect();
    let trimmed = trimmed.trim_end_matches([',', ';', '—', '-', ' ']);
    format!("{trimmed}…")
}

fn strip_if_present(value: &str, app_name: &str, project: &str, domain: &str) -> String {
    if value.is_empty() {
        String::new()
    } else {
        fluff::strip_fluff(value, app_name, project, domain)
    }
}

/// Build a coherent first-person what-happened from structured metadata,
/// without touching OCR text or `display_summary`. The hierarchy prefers the
/// most specific signal first.
fn coherent_what_happened_from_metadata(record: &AssembledMemory<'_>) -> String {
    let win = record.window_title.trim();
    let app = record.app_name.trim();
    let activity = record.extraction.map(|extraction| extraction.activity_type.trim()).unwrap_or("");
    let topic = record.extraction.map(|extraction| extraction.topic.trim()).unwrap_or("");
    let intent = record.extraction.map(|extraction| extraction.user_intent.trim()).unwrap_or("");
    let entities: Vec<&str> = record
        .extraction
        .map(|extraction| {
            extraction
                .entities
                .iter()
                .filter(|value| !value.trim().is_empty())
                .take(2)
                .map(String::as_str)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let win_is_meaningful = !win.is_empty() && !win.eq_ignore_ascii_case(app) && win.len() <= 120;

    // Preferred shape: "You {intent} {window_title}": concrete, specific,
    // does NOT mention the app name (the app chip already shows that).
    if win_is_meaningful {
        if !intent.is_empty() && intent != "unknown" {
            return format!("You were {intent} {win}.");
        }
        if !activity.is_empty() && activity != "unknown" {
            return format!("You were {activity} {win}.");
        }
        return format!("You were on \"{win}\".");
    }

    // No useful window title: try entities + activity.
    if !entities.is_empty() {
        let entity_join = entities.join(", ");
        if !activity.is_empty() && activity != "unknown" {
            return format!("You were {activity} regarding {entity_join}.");
        }
        if !topic.is_empty() && topic != "unknown" {
            return format!("You engaged with {entity_join} on {topic}.");
        }
        return format!("You engaged with {entity_join}.");
    }

    // No window title, no entities: last-resort metadata sentence.
    if !topic.is_empty() && topic != "unknown" {
        if !activity.is_empty() && activity != "unknown" {
            return format!("You were {activity} on {topic}.");
        }
        return format!("Activity related to {topic}.");
    }
    if !activity.is_empty() && activity != "unknown" {
        return format!("You were {activity}.");
    }

    String::new()
}

/// Build a coherent why-mattered sentence from structured signals. Order:
/// decisions > errors > blockers > first paragraph of memory_context (only
/// if coherent narrative) > entity/intent-based construction.
fn coherent_why_mattered_from_metadata(record: &AssembledMemory<'_>) -> String {
    let Some(extraction) = record.extraction else {
        return String::new();
    };

    if let Some(decision) =
        extraction.decisions.iter().find(|value| !value.trim().is_empty() && !is_template_summary(value))
    {
        return decision.chars().take(MAX_WHY_CHARS).collect();
    }
    if let Some(error) = extraction.errors.iter().find(|value| !value.trim().is_empty()) {
        return format!(
            "Encountered error: {}",
            error.chars().take(MAX_WHY_CHARS - 20).collect::<String>()
        );
    }
    if let Some(blocker) = extraction.blockers.iter().find(|value| !value.trim().is_empty()) {
        return format!(
            "Blocked on: {}",
            blocker.chars().take(MAX_WHY_CHARS - 12).collect::<String>()
        );
    }

    // Use memory_context's first sentence ONLY if it's a real narrative.
    let context_first =
        extraction.memory_context.split_terminator(['.', '!', '?']).next().unwrap_or("").trim();
    if !context_first.is_empty()
        && !is_template_summary(context_first)
        && context_first.split_whitespace().count() >= 5
    {
        return context_first.chars().take(MAX_WHY_CHARS).collect();
    }

    // Build from intent + entities.
    let intent = extraction.user_intent.trim();
    let entities: Vec<&str> =
        extraction.entities.iter().filter(|value| !value.trim().is_empty()).take(3).map(String::as_str).collect();

    if !intent.is_empty() && intent != "unknown" && !entities.is_empty() {
        return format!("Engaged in {intent} involving {}.", entities.join(", "));
    }
    if !entities.is_empty() {
        let activity = extraction.activity_type.trim();
        if !activity.is_empty() && activity != "unknown" {
            return format!("{} involving {}.", fluff::capitalize_first(activity), entities.join(", "));
        }
    }

    String::new()
}

/// Populate the insight fields for one already-assembled record.
///
/// When `record.prefilled` already carries a non-empty `what_happened` (a
/// prior VLM/LLM synthesis pass), this only fluff-strips those values
/// (idempotent: safe to call more than once). Otherwise it derives fresh
/// text from structured fields and OCR salience, with no model call.
pub fn derive_insight(record: &AssembledMemory<'_>) -> DerivedInsight {
    let project = record.extraction.map(|extraction| extraction.project.as_str()).unwrap_or("");
    let domain = record
        .url
        .and_then(|url| crate::continuity::domain(Some(url)))
        .unwrap_or_default();

    if let Some(prefilled) = record.prefilled
        && !prefilled.what_happened.trim().is_empty()
    {
        return DerivedInsight {
            what_happened: Some(fluff::strip_fluff(prefilled.what_happened, record.app_name, project, &domain)),
            why_mattered: strip_if_present(prefilled.why_mattered, record.app_name, project, &domain),
            what_changed: strip_if_present(prefilled.what_changed, record.app_name, project, &domain),
            context_thread: prefilled.context_thread.to_string(),
            card_confidence: prefilled.card_confidence,
            top_spans: Vec::new(),
            dropped_spans: Vec::new(),
        };
    }

    let app = AppIdentity::new(record.app_name, record.bundle_id);
    let spans = rank_salient_spans(record.clean_text, app);
    let top_spans: Vec<SalientSpan> = spans.iter().take(TOP_SPAN_DEBUG).cloned().collect();
    let dropped_spans: Vec<String> = spans
        .iter()
        .skip(TOP_SPAN_DEBUG)
        .filter(|span| span.score < DROP_SCORE)
        .map(|span| span.text.chars().take(200).collect())
        .collect();

    // --- what_happened ---
    // Priority: display_summary (if not a template/filename) -> metadata
    // construction -> snippet.
    let what = {
        let candidate = record.display_summary.trim().to_string();
        if !candidate.is_empty() && !is_template_summary(&candidate) {
            dedupe_repeating_phrases(&candidate)
        } else {
            let from_meta = coherent_what_happened_from_metadata(record);
            if !from_meta.is_empty() {
                from_meta
            } else if !record.snippet.trim().is_empty() && !is_template_summary(record.snippet) {
                record.snippet.trim().to_string()
            } else {
                String::new()
            }
        }
    };
    let what_happened = if what.trim().is_empty() {
        None
    } else {
        Some(strip_if_present(
            &clip_chars(what, MAX_WHAT_CHARS),
            record.app_name,
            project,
            &domain,
        ))
    };

    // --- why_mattered ---
    let why = coherent_why_mattered_from_metadata(record);
    let why = if !why.is_empty() {
        why
    } else if let Some(span) = spans.first() {
        // Only use a salient OCR span as a last resort, and only when it
        // looks like a meaningful phrase (>= 4 words, not a single proper
        // noun chunk).
        if span.text.split_whitespace().count() >= 4 && !is_template_summary(&span.text) {
            span.text.chars().take(MAX_WHY_CHARS).collect()
        } else {
            String::new()
        }
    } else {
        String::new()
    };
    let why_mattered = strip_if_present(&clip_chars(why, MAX_WHY_CHARS), record.app_name, project, &domain);

    // --- what_changed ---
    let mut changed: Vec<String> = Vec::new();
    if let Some(extraction) = record.extraction {
        changed.extend(extraction.next_steps.iter().filter(|value| !value.trim().is_empty()).cloned());
        changed.extend(extraction.files_touched.iter().filter(|value| !value.trim().is_empty()).cloned());
    }
    let joined = changed.join("; ");
    let what_changed = if joined.is_empty() {
        String::new()
    } else {
        strip_if_present(&clip_chars(joined, MAX_CHANGED_CHARS), record.app_name, project, &domain)
    };

    // --- context_thread (only when we have real links; avoid fabrication) ---
    let context_thread = if !record.related_memory_ids.is_empty() {
        format!("{} linked memories", record.related_memory_ids.len())
    } else if !record.session_id.trim().is_empty() {
        let short: String = record.session_id.chars().take(8).collect();
        format!("session …{short}")
    } else {
        String::new()
    };

    let pollution = pollution_for_insight(record, app);
    let salience = spans.first().map(|span| span.score).unwrap_or(0.0).clamp(0.0, 1.0);
    let card_confidence =
        (salience * (1.0 - pollution) * record.ocr_confidence.clamp(0.0, 1.0)).clamp(0.0, 1.0);

    DerivedInsight {
        what_happened,
        why_mattered,
        what_changed,
        context_thread,
        card_confidence,
        top_spans,
        dropped_spans,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extraction(mutate: impl FnOnce(&mut StructuredExtraction)) -> StructuredExtraction {
        let mut extraction = StructuredExtraction::default();
        mutate(&mut extraction);
        extraction
    }

    #[test]
    fn is_template_catches_filename_patterns() {
        assert!(is_template_summary("Screen capture (visual): Claude_1778938598807.png. Claude"));
        assert!(is_template_summary("Captured recent activity at 08:30 AM"));
        assert!(is_template_summary("URL-only surface capture for x.com at 12:00 PM"));
        assert!(is_template_summary(""));
        assert!(!is_template_summary("You watched the IPL match on Willow TV"));
        assert!(!is_template_summary("Reviewed authentication PR for security correctness"));
    }

    #[test]
    fn what_happened_strips_filename_pattern_and_signals_no_evidence() {
        // Nothing beyond a template display_summary and app-name==window-
        // title is available, so this is exactly the "cannot produce a
        // complete record deterministically" case: the typed `None`
        // (invariant 4), not the template string leaking through as an
        // empty-but-"complete" field the way v1 silently allowed.
        let record = AssembledMemory {
            app_name: "Claude",
            window_title: "Claude",
            display_summary: "Screen capture (visual): Claude_1778938598807.png. Claude",
            ocr_confidence: 0.85,
            ..Default::default()
        };
        let derived = derive_insight(&record);
        assert!(derived.what_happened.is_none(), "got: {:?}", derived.what_happened);
    }

    #[test]
    fn what_happened_uses_window_title_when_more_specific_than_app() {
        let extraction = extraction(|extraction| extraction.activity_type = "research".to_string());
        let record = AssembledMemory {
            app_name: "Google Chrome",
            window_title: "React hooks documentation - MDN",
            extraction: Some(&extraction),
            ocr_confidence: 0.85,
            ..Default::default()
        };
        let what = derive_insight(&record).what_happened.expect("should derive a sentence");
        assert!(what.contains("React") || what.contains("MDN"), "specific window content missing: {what}");
        assert!(!what.contains("Google Chrome"), "app name leaked into what_happened: {what}");
    }

    #[test]
    fn what_happened_falls_back_to_entities_when_no_window_title() {
        let extraction = extraction(|extraction| {
            extraction.entities = vec!["cargo build".to_string(), "release target".to_string()];
            extraction.activity_type = "building".to_string();
        });
        let record = AssembledMemory {
            app_name: "Terminal",
            window_title: "Terminal",
            extraction: Some(&extraction),
            ocr_confidence: 0.85,
            ..Default::default()
        };
        let what = derive_insight(&record).what_happened.expect("should derive a sentence");
        assert!(what.starts_with("You"), "should start with You: {what}");
        assert!(
            what.contains("cargo build") || what.contains("release target"),
            "entities missing: {what}"
        );
    }

    #[test]
    fn why_mattered_prefers_decisions_over_ocr_span() {
        let extraction =
            extraction(|extraction| extraction.decisions = vec!["Switched to JWT for stateless auth".to_string()]);
        let record = AssembledMemory {
            clean_text: "irrelevant noisy OCR text that shouldnt appear in why_mattered",
            extraction: Some(&extraction),
            ocr_confidence: 0.85,
            ..Default::default()
        };
        let why = derive_insight(&record).why_mattered;
        assert!(why.contains("JWT"), "got: {why}");
    }

    #[test]
    fn why_mattered_does_not_dump_short_ocr_span() {
        let record = AssembledMemory {
            app_name: "Chrome",
            clean_text: "Page Title",
            ocr_confidence: 0.85,
            ..Default::default()
        };
        let why = derive_insight(&record).why_mattered;
        assert!(
            why.is_empty() || why.split_whitespace().count() >= 4,
            "got short fragment: {why}"
        );
    }

    #[test]
    fn dedupe_repeating_phrases_collapses_runs() {
        assert_eq!(
            dedupe_repeating_phrases("Google Chrome — Google Chrome — News article"),
            "Google Chrome — News article"
        );
        assert_eq!(dedupe_repeating_phrases("You watched the match"), "You watched the match");
    }

    #[test]
    fn idempotent_when_what_happened_already_set() {
        let record = AssembledMemory {
            prefilled: Some(PrefilledInsight {
                what_happened: "Pre-filled by LLM synthesis.",
                ..Default::default()
            }),
            ocr_confidence: 0.85,
            ..Default::default()
        };
        let what = derive_insight(&record).what_happened;
        assert_eq!(what.as_deref(), Some("Pre-filled by LLM synthesis."));
    }

    #[test]
    fn fluff_strip_runs_on_prefilled_insights() {
        let record = AssembledMemory {
            app_name: "Google Chrome",
            prefilled: Some(PrefilledInsight {
                what_happened: "Google Chrome — Google Chrome news article on AI",
                ..Default::default()
            }),
            ocr_confidence: 0.85,
            ..Default::default()
        };
        let what = derive_insight(&record).what_happened.expect("prefilled should pass through");
        let count = what.matches("Google Chrome").count();
        assert_eq!(count, 1, "expected one app mention, got: {what}");
    }

    #[test]
    fn fluff_strip_drops_the_user_prefix_from_llm_output() {
        let record = AssembledMemory {
            prefilled: Some(PrefilledInsight {
                what_happened: "the user is debugging a Rust borrow error",
                ..Default::default()
            }),
            ocr_confidence: 0.85,
            ..Default::default()
        };
        let what = derive_insight(&record).what_happened.expect("prefilled should pass through");
        assert!(!what.to_lowercase().starts_with("the user"), "the user prefix leaked: {what}");
    }

    #[test]
    fn context_thread_prefers_related_memory_count_over_session_id() {
        let ids = vec!["a".to_string(), "b".to_string()];
        let record = AssembledMemory {
            session_id: "session-0123456789",
            related_memory_ids: &ids,
            ocr_confidence: 0.85,
            ..Default::default()
        };
        assert_eq!(derive_insight(&record).context_thread, "2 linked memories");

        let record_no_links =
            AssembledMemory { session_id: "session-0123456789", ocr_confidence: 0.85, ..Default::default() };
        assert_eq!(derive_insight(&record_no_links).context_thread, "session …session-");
    }

    #[test]
    fn card_confidence_stays_within_unit_range() {
        let record = AssembledMemory {
            app_name: "Chrome",
            window_title: "React hooks documentation - MDN",
            clean_text: "React hooks documentation. useEffect cleanup patterns explained clearly.",
            ocr_confidence: 0.9,
            ocr_noise_score: 0.1,
            ..Default::default()
        };
        let confidence = derive_insight(&record).card_confidence;
        assert!((0.0..=1.0).contains(&confidence), "out of range: {confidence}");
    }
}
