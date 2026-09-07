//! Deterministic session identity and story-continuity policy (T-307).
//!
//! Ported from `origin/reference/v1:src-tauri/src/capture/mod.rs` under
//! ADR-005. This module deliberately has no store or model dependency:
//! callers supply an already-computed vector similarity, and retain ownership
//! of both candidate retrieval and persistence.

use std::collections::HashSet;

/// One record's safe, durable fields used for continuity decisions. `text` is
/// already privacy-filtered OCR or derived text; this policy never sees pixels.
#[derive(Debug, Clone, Copy)]
pub struct ContinuityRecord<'a> {
    pub app_name: &'a str,
    pub url: Option<&'a str>,
    pub window_title: &'a str,
    pub text: &'a str,
    pub snippet: &'a str,
    pub lexical_shadow: &'a str,
    pub captured_at_ms: i64,
}

/// Named scoring signals make a later health or merge-explanation surface
/// possible without rerunning a model or inferring why a merge occurred.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContinuityScore {
    pub score: f32,
    pub lexical: f32,
    pub vector: f32,
    pub anchor_match: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionIdentityError {
    InvalidDay,
    InvalidMinute,
}

/// The v1 30-minute session identity, expressed with a caller-provided local
/// day and minute. Keeping wall-clock conversion at the application boundary
/// prevents this engine crate from owning a platform timezone policy.
pub fn build_session_id(
    local_day_yyyymmdd: &str,
    minute_of_day: u16,
    app_name: &str,
    bundle_id: Option<&str>,
    session_key: &str,
) -> Result<String, SessionIdentityError> {
    if local_day_yyyymmdd.len() != 8
        || !local_day_yyyymmdd.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(SessionIdentityError::InvalidDay);
    }
    if minute_of_day >= 24 * 60 {
        return Err(SessionIdentityError::InvalidMinute);
    }
    let app = bundle_id.unwrap_or(app_name).to_ascii_lowercase();
    let app = app
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    let anchor = session_key
        .split(':')
        .nth(1)
        .unwrap_or("general")
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '-') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    Ok(format!(
        "{local_day_yyyymmdd}-{app}-{anchor}-s{:02}",
        minute_of_day / 30
    ))
}

/// Stable short key for a frontmost capture context. It is useful to a caller
/// for in-flight continuity maps, not as a security boundary.
pub fn build_session_key(app_name: &str, window_title: &str, url: Option<&str>) -> String {
    let app = app_name.trim().to_lowercase().replace(' ', "_");
    let title = window_title
        .trim()
        .to_lowercase()
        .chars()
        .filter(|character| character.is_alphanumeric() || *character == ' ')
        .collect::<String>()
        .split_whitespace()
        .take(5)
        .collect::<Vec<_>>()
        .join("_");
    match domain(url) {
        Some(domain) => format!("{app}:{}:{title}", domain.replace('.', "_")),
        None => format!("{app}:{title}"),
    }
}

/// The cheap anchor route takes precedence over candidate scoring. It uses
/// only domain plus three path segments, never URL credentials, queries, or
/// fragments.
pub fn continuity_anchor(record: ContinuityRecord<'_>) -> Option<String> {
    if let Some(url) = record.url
        && let Some(domain) = domain(Some(url))
    {
        let path = first_path_segments(url, 3);
        if !path.is_empty() {
            return Some(format!("url:{domain}:{path}"));
        }
        return Some(format!("url:{domain}"));
    }
    let app = normalize_app_key(record.app_name);
    let title = normalize_anchor_text(record.window_title);
    if title.len() >= 8 {
        return Some(format!("app:{app}:title:{title}"));
    }
    let snippet = normalize_anchor_text(record.snippet);
    (snippet.len() >= 10).then(|| format!("app:{app}:snippet:{snippet}"))
}

/// Small captures remain standalone evidence rather than being folded into a
/// plausible-looking story on weak context.
pub fn eligible_for_story_merge(record: ContinuityRecord<'_>) -> bool {
    record.text.trim().len() >= 36 || record.snippet.trim().len() >= 18
}

/// Score an already-retrieved candidate using the v1 feature weights.
/// `vector_similarity` is clamped and may be zero when model work is absent.
pub fn score_candidate(
    incoming: ContinuityRecord<'_>,
    candidate: ContinuityRecord<'_>,
    vector_similarity: f32,
) -> ContinuityScore {
    let lexical = token_overlap(incoming.snippet, candidate.snippet) * 0.42
        + token_overlap(incoming.window_title, candidate.window_title) * 0.26
        + token_overlap(
            trim_chars(incoming.text, 1_000),
            trim_chars(candidate.text, 1_000),
        ) * 0.20
        + token_overlap(incoming.lexical_shadow, candidate.lexical_shadow) * 0.12;
    let vector = vector_similarity.clamp(0.0, 1.0);
    let anchor_match = continuity_anchor(incoming)
        .zip(continuity_anchor(candidate))
        .map(|(left, right)| left == right)
        .unwrap_or(false);
    let same_domain = domain(incoming.url)
        .zip(domain(candidate.url))
        .map(|(left, right)| left == right)
        .unwrap_or(false);
    let score = vector * 0.5
        + lexical * 0.42
        + if same_domain { 0.08 } else { 0.0 }
        + if anchor_match { 0.32 } else { 0.0 };
    ContinuityScore {
        score,
        lexical,
        vector,
        anchor_match,
    }
}

/// The final decision preserves v1's strict cross-app rule: a matching URL
/// inside 45 minutes is sufficient to cross applications; otherwise both a
/// shared domain and strong lexical/anchor or vector evidence are required.
pub fn should_merge(
    incoming: ContinuityRecord<'_>,
    candidate: ContinuityRecord<'_>,
    vector_similarity: f32,
) -> Option<ContinuityScore> {
    if !eligible_for_story_merge(incoming) || !eligible_for_story_merge(candidate) {
        return None;
    }
    let score = score_candidate(incoming, candidate, vector_similarity);
    if incoming.app_name != candidate.app_name
        && !allows_cross_app_merge(incoming, candidate, score)
    {
        return None;
    }
    passes_merge_threshold(score).then_some(score)
}

pub fn passes_merge_threshold(score: ContinuityScore) -> bool {
    if score.anchor_match {
        return score.score >= 0.58 && score.lexical >= 0.18;
    }
    if score.lexical >= 0.72 && score.score >= 0.80 {
        return true;
    }
    score.score >= 0.86 && score.vector >= 0.82 && score.lexical >= 0.28
}

/// Merge complementary sentence-sized evidence without duplicating overlap.
/// This is deterministic and bounded; synthesis remains a later, separately
/// grounded stage.
pub fn merge_story_text(existing: &str, incoming: &str, max_chars: usize) -> String {
    let existing = existing.trim();
    let incoming = incoming.trim();
    if existing.is_empty() {
        return trim_chars(incoming, max_chars).to_owned();
    }
    if incoming.is_empty() {
        return trim_chars(existing, max_chars).to_owned();
    }
    let normalized_existing = normalize_overlap(existing);
    let normalized_incoming = normalize_overlap(incoming);
    if normalized_existing.contains(&normalized_incoming) {
        return trim_chars(existing, max_chars).to_owned();
    }
    if normalized_incoming.contains(&normalized_existing) {
        return trim_chars(incoming, max_chars).to_owned();
    }
    let mut merged = existing.to_owned();
    let mut merged_normalized = normalized_existing;
    for segment in incoming
        .split(['\n', '.', '!', '?', ';'])
        .map(str::trim)
        .filter(|segment| segment.len() >= 12)
    {
        let normalized = normalize_overlap(segment);
        if normalized.is_empty() || merged_normalized.contains(&normalized) {
            continue;
        }
        merged.push_str(" • ");
        merged.push_str(segment);
        merged_normalized.push(' ');
        merged_normalized.push_str(&normalized);
        if merged.chars().count() >= max_chars {
            break;
        }
    }
    trim_chars(&merged, max_chars).to_owned()
}

fn allows_cross_app_merge(
    incoming: ContinuityRecord<'_>,
    candidate: ContinuityRecord<'_>,
    score: ContinuityScore,
) -> bool {
    if (incoming.captured_at_ms - candidate.captured_at_ms).unsigned_abs() > 45 * 60 * 1_000 {
        return false;
    }
    if matching_effective_url(incoming.url, candidate.url) {
        return true;
    }
    let same_domain = domain(incoming.url)
        .zip(domain(candidate.url))
        .map(|(left, right)| left == right)
        .unwrap_or(false);
    same_domain
        && ((score.anchor_match && score.lexical >= 0.52)
            || (score.vector >= 0.93 && score.lexical >= 0.70))
}

fn matching_effective_url(left: Option<&str>, right: Option<&str>) -> bool {
    let (Some(left), Some(right)) = (left, right) else {
        return false;
    };
    effective_url(left) == effective_url(right)
}

fn effective_url(url: &str) -> String {
    let url = url.trim().to_lowercase();
    let url = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(&url);
    url.split(['?', '#'])
        .next()
        .unwrap_or(url)
        .trim_end_matches('/')
        .to_owned()
}

fn domain(url: Option<&str>) -> Option<String> {
    let url = url?.trim();
    let after_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let authority = after_scheme.split(['/', '?', '#']).next()?.trim();
    let host = authority.rsplit('@').next()?.split(':').next()?.trim();
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

fn first_path_segments(url: &str, count: usize) -> String {
    let after_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let path = after_scheme
        .split_once('/')
        .map(|(_, path)| path)
        .unwrap_or("");
    path.split(['?', '#'])
        .next()
        .unwrap_or(path)
        .split('/')
        .filter(|segment| !segment.trim().is_empty())
        .map(|segment| segment.trim().to_ascii_lowercase())
        .take(count)
        .collect::<Vec<_>>()
        .join("/")
}

fn normalize_app_key(app_name: &str) -> String {
    let normalized = app_name
        .to_lowercase()
        .chars()
        .map(|character| {
            if character.is_alphanumeric() {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    let normalized = normalized
        .split('_')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    if normalized.is_empty() {
        "unknown".into()
    } else {
        normalized
    }
}

fn normalize_anchor_text(text: &str) -> String {
    text.to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| token.len() > 2 && !is_generic_stop_word(token))
        .take(8)
        .collect::<Vec<_>>()
        .join("_")
}

fn token_overlap(left: &str, right: &str) -> f32 {
    let left = tokenize(left);
    let right = tokenize(right);
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    left.intersection(&right).count() as f32 / left.union(&right).count() as f32
}

fn tokenize(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| token.len() > 2 && !is_generic_stop_word(token))
        .map(str::to_owned)
        .collect()
}

fn is_generic_stop_word(token: &str) -> bool {
    matches!(
        token,
        "the"
            | "and"
            | "for"
            | "with"
            | "this"
            | "that"
            | "from"
            | "your"
            | "you"
            | "are"
            | "was"
            | "were"
            | "have"
            | "has"
            | "into"
            | "about"
            | "after"
            | "before"
            | "then"
            | "just"
            | "there"
            | "here"
            | "user"
            | "app"
            | "window"
            | "tab"
            | "page"
            | "open"
            | "opened"
            | "search"
            | "searched"
            | "www"
            | "http"
            | "https"
            | "com"
    )
}

fn normalize_overlap(text: &str) -> String {
    text.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn trim_chars(text: &str, max_chars: usize) -> &str {
    match text.char_indices().nth(max_chars) {
        Some((index, _)) => &text[..index],
        None => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record<'a>(
        app_name: &'a str,
        url: Option<&'a str>,
        captured_at_ms: i64,
    ) -> ContinuityRecord<'a> {
        ContinuityRecord {
            app_name,
            url,
            window_title: "FNDR retrieval architecture",
            text: "Implementing durable keyword retrieval and capture continuity policy.",
            snippet: "Implementing durable retrieval continuity.",
            lexical_shadow: "fndr retrieval capture continuity policy",
            captured_at_ms,
        }
    }

    #[test]
    fn session_identity_is_sub_day_and_domain_anchored() {
        let key = build_session_key(
            "Google Chrome",
            "Screenpipe Architecture Deep Dive",
            Some("https://docs.screenpi.pe/architecture/memory-cards?secret=no"),
        );
        assert_eq!(
            key,
            "google_chrome:docs_screenpi_pe:screenpipe_architecture_deep_dive"
        );
        assert_eq!(
            build_session_id(
                "20260506",
                21 * 60 + 46,
                "Google Chrome",
                Some("com.google.Chrome"),
                &key
            ),
            Ok("20260506-com.google.chrome-docs_screenpi_pe-s43".into())
        );
    }

    #[test]
    fn anchor_uses_safe_path_not_query_or_credentials() {
        assert_eq!(
            continuity_anchor(record(
                "Browser",
                Some("https://user:secret@docs.example.com/a/b/c/d?token=private#frag"),
                0,
            )),
            Some("url:docs.example.com:a/b/c".into())
        );
    }

    #[test]
    fn same_app_candidate_requires_v1_threshold_and_keeps_score_signals() {
        let incoming = record("Codex", Some("https://docs.example.com/fndr"), 10);
        let candidate = record("Codex", Some("https://docs.example.com/fndr"), 20);
        let score = should_merge(incoming, candidate, 0.90).expect("same work should merge");
        assert!(score.anchor_match);
        assert!(score.lexical > 0.9);
        assert!(score.vector > 0.89);
    }

    #[test]
    fn cross_app_requires_a_tight_safe_connection() {
        let incoming = record(
            "Safari",
            Some("https://docs.example.com/fndr"),
            45 * 60 * 1_000,
        );
        let same_url = record(
            "Codex",
            Some("https://docs.example.com/fndr?ignored=yes"),
            0,
        );
        assert!(should_merge(incoming, same_url, 0.90).is_some());

        let different_domain = record("Codex", Some("https://other.example.com/fndr"), 0);
        assert!(should_merge(incoming, different_domain, 0.99).is_none());

        let old = record(
            "Codex",
            Some("https://docs.example.com/fndr"),
            91 * 60 * 1_000 + 1,
        );
        assert!(should_merge(incoming, old, 0.99).is_none());
    }

    #[test]
    fn story_merge_deduplicates_overlap_and_respects_character_boundary() {
        assert_eq!(
            merge_story_text("Build the durable index.", "Build the durable index.", 80),
            "Build the durable index."
        );
        assert_eq!(
            merge_story_text(
                "alpha evidence",
                "beta evidence details; alpha evidence",
                31
            ),
            "beta evidence details; alpha ev"
        );
    }

    #[test]
    fn small_evidence_never_merges() {
        let short = ContinuityRecord {
            text: "tiny",
            snippet: "short",
            ..record("Codex", None, 0)
        };
        assert!(should_merge(short, record("Codex", None, 0), 1.0).is_none());
    }
}
