// Ported from FNDR v1 src-tauri/src/capture/text_cleanup.rs (reference/v1) per ADR-005.
// Heuristic constants and their tuning comments move exactly (they encode the why).
//! Drop obvious browser chrome from OCR text before embeddings and storage.
//!
//! Vision still sees the full screenshot; we only trim lines that usually come from
//! tab strips and compact toolbar captions so memory records favor page content,
//! titles, and body text already kept by the OCR noise filter.

use std::collections::HashSet;

/// Match the default OCR `min_line_length` so we do not resurrect junk lines.
const MIN_LINE_LEN: usize = 7;
const MAX_FALLBACK_SNIPPET_CHARS: usize = 220;

#[derive(Debug, Clone, Copy, Default)]
pub struct CaptureQualityStats {
    pub total_lines: usize,
    pub kept_lines: usize,
    pub low_conf_lines: usize,
    pub dropped_noise_lines: usize,
    pub dropped_low_signal_lines: usize,
    pub avg_line_score: f32,
}

#[derive(Debug, Clone, Default)]
pub struct HighSignalText {
    pub text: String,
    pub stats: CaptureQualityStats,
}

const GENERIC_BROWSER_LABELS: &[&str] = &[
    "new tab",
    "home",
    "trending",
    "for you",
    "shorts",
    "explore",
    "discover",
    "notifications",
    "inbox",
    "starred",
    "settings",
    "untitled",
];

/// Lines with several middots and short segments are almost always Safari/Chrome tab rows.
fn looks_like_tab_strip_line(line: &str) -> bool {
    let dots = line.matches('·').count();
    if dots < 2 {
        return false;
    }
    let segments: Vec<usize> = line.split('·').map(|s| s.trim().len()).collect();
    if segments.is_empty() {
        return false;
    }
    let max_seg = *segments.iter().max().unwrap_or(&0);
    max_seg <= 42 && line.len() <= 220
}

/// Same idea for toolbars that OCR as "A | B | C" with short labels.
fn looks_like_pipe_tab_row(line: &str) -> bool {
    let pipes = line.matches('|').count();
    if pipes < 2 {
        return false;
    }
    let segments: Vec<usize> = line.split('|').map(|s| s.trim().len()).collect();
    if segments.len() < 3 {
        return false;
    }
    let max_seg = *segments.iter().max().unwrap_or(&0);
    max_seg <= 36 && line.len() <= 220
}

/// Very short lines that are almost always window or browser chrome (conservative).
fn is_compact_chrome_caption(line: &str) -> bool {
    if line.len() > 64 {
        return false;
    }
    let lower = line.to_lowercase();
    // OCR often glues adjacent toolbar labels into one token.
    if matches!(lower.as_str(), "backforward" | "forwardback") {
        return true;
    }
    lower.contains("back")
        && lower.contains("forward")
        && lower.len() < 42
        && (lower.contains("reload") || lower.contains("refresh"))
}

fn is_separator_line(line: &str) -> bool {
    !line.is_empty()
        && line
            .chars()
            .all(|ch| ch == '-' || ch == '_' || ch == '=' || ch == '.' || ch == ' ')
}

pub fn symbol_ratio(line: &str) -> f32 {
    let chars: Vec<char> = line.chars().collect();
    if chars.is_empty() {
        return 1.0;
    }
    let symbol_count = chars
        .iter()
        .filter(|ch| !ch.is_alphanumeric() && !ch.is_whitespace())
        .count();
    symbol_count as f32 / chars.len() as f32
}

pub fn looks_like_file_inventory(line: &str) -> bool {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    if tokens.len() < 4 {
        return false;
    }

    let pathish = tokens
        .iter()
        .filter(|token| {
            let token = token.trim_matches(|ch: char| ",;:()[]{}".contains(ch));
            token.contains('/')
                || token.contains('\\')
                || (token.contains('.')
                    && (token.contains('_')
                        || token.contains('-')
                        || token.ends_with(".rs")
                        || token.ends_with(".ts")
                        || token.ends_with(".json")
                        || token.ends_with(".md")))
        })
        .count();

    pathish >= 3
}

fn looks_like_json_inventory(line: &str) -> bool {
    let trimmed = line.trim();
    let lower = trimmed.to_lowercase();
    if (trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'))
    {
        return trimmed.len() > 50;
    }

    lower.contains("\"files\"")
        || lower.contains("\"path\"")
        || lower.contains("\"nodes\"")
        || lower.contains("\"items\"")
}

fn looks_like_notification_fragment(line: &str) -> bool {
    let lower = line.to_lowercase();
    lower.contains(" minutes ago")
        || lower.contains(" hours ago")
        || lower.contains(" liked this")
        || lower.contains(" replied")
        || lower.contains("suggested for you")
        || lower.starts_with("breaking:")
}

fn looks_like_feed_fragment(line: &str) -> bool {
    if line.len() > 90 {
        return false;
    }

    let words = line.split_whitespace().count();
    if words <= 2 {
        return true;
    }

    let lower = line.to_lowercase();
    lower.starts_with("sponsored")
        || lower == "watch now"
        || lower == "learn more"
        || lower == "follow"
        || lower == "share"
        || lower == "like"
}

fn looks_like_animation_fragment(line: &str) -> bool {
    let lower = line.to_lowercase();
    lower == "loading"
        || lower == "loading..."
        || lower == "please wait"
        || lower == "retry"
        || lower == "refresh"
        || lower == "updated just now"
}

fn normalize_inline(line: &str) -> String {
    line.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_low_conf_marker(line: &str) -> String {
    normalize_inline(&line.replace("[LOW_CONF]", " "))
}

fn truncate_snippet(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let keep = max_chars.saturating_sub(3);
    let mut out: String = text.chars().take(keep).collect();
    out.push_str("...");
    out
}

fn snippet_dedup_key(value: &str) -> String {
    normalize_inline(value).to_lowercase()
}

fn title_is_generic_for_app(app_name: &str, title: &str) -> bool {
    let title_lower = title.to_lowercase();
    let app_lower = app_name.to_lowercase();

    if !app_lower.is_empty() && title_lower == app_lower {
        return true;
    }

    matches!(
        title_lower.as_str(),
        "new tab" | "untitled" | "home" | "settings" | "preferences" | "dashboard" | "start page"
    )
}

fn is_browser_app(app_name: &str) -> bool {
    let lower = app_name.to_lowercase();
    lower.contains("chrome")
        || lower.contains("safari")
        || lower.contains("arc")
        || lower.contains("firefox")
        || lower.contains("edge")
}

fn is_code_app(app_name: &str) -> bool {
    let lower = app_name.to_lowercase();
    lower.contains("terminal")
        || lower.contains("iterm")
        || lower.contains("vscode")
        || lower.contains("code")
        || lower.contains("cursor")
}

fn is_mail_app(app_name: &str) -> bool {
    let lower = app_name.to_lowercase();
    lower.contains("gmail")
        || lower.contains("mail")
        || lower.contains("outlook")
        || lower.contains("superhuman")
}

fn looks_like_email_header(line: &str) -> bool {
    let lower = line.to_lowercase();
    lower.starts_with("from:")
        || lower.starts_with("to:")
        || lower.starts_with("subject:")
        || lower.starts_with("cc:")
        || lower.starts_with("bcc:")
}

fn looks_like_url_line(line: &str) -> bool {
    let lower = line.trim().to_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://") || lower.contains("www.")
}

fn is_generic_browser_label(line: &str) -> bool {
    let lower = line.to_lowercase();
    GENERIC_BROWSER_LABELS
        .iter()
        .any(|label| lower == *label || lower.starts_with(&format!("{} ", label)))
}

fn should_drop_line(app_name: &str, line: &str) -> bool {
    let browser_app = is_browser_app(app_name);
    let code_app = is_code_app(app_name);
    let mail_app = is_mail_app(app_name);

    if is_separator_line(line) {
        return true;
    }

    if looks_like_tab_strip_line(line)
        || looks_like_pipe_tab_row(line)
        || is_compact_chrome_caption(line)
    {
        return true;
    }

    if browser_app && is_generic_browser_label(line) {
        return true;
    }

    // Mail sidebars reuse the same short nav labels as browser chrome in OCR.
    if mail_app && is_generic_browser_label(line) {
        return true;
    }

    if looks_like_url_line(line) {
        return false;
    }

    if browser_app && (looks_like_notification_fragment(line) || looks_like_feed_fragment(line)) {
        return true;
    }

    if looks_like_animation_fragment(line) {
        return true;
    }

    if !code_app && (looks_like_file_inventory(line) || looks_like_json_inventory(line)) {
        return true;
    }

    // Keep email metadata in mail apps.
    if mail_app && looks_like_email_header(line) {
        return false;
    }

    // Symbol-heavy single lines in browser/feed contexts are usually junk.
    let ratio = symbol_ratio(line);
    if browser_app && ratio > 0.38 {
        return true;
    }

    if !code_app && ratio > 0.58 {
        return true;
    }

    false
}

fn is_useful_snippet_line(app_name: &str, line: &str) -> bool {
    let normalized = normalize_inline(line);
    if normalized.len() < MIN_LINE_LEN {
        return false;
    }
    if normalized.len() > 240 {
        return false;
    }
    if should_drop_line(app_name, &normalized) {
        return false;
    }
    if title_is_generic_for_app(app_name, &normalized) {
        return false;
    }
    true
}

/// Estimate noise score for ranking penalties (0 = clean, 1 = mostly noise).
pub fn estimate_noise_score(app_name: &str, text: &str) -> f32 {
    let mut total = 0usize;
    let mut noisy_weight = 0.0_f32;
    for line in text.lines() {
        let line = normalize_inline(line.trim());
        if line.is_empty() {
            continue;
        }
        total += 1;
        if should_drop_line(app_name, &line) || line.len() < MIN_LINE_LEN {
            noisy_weight += 1.0;
            continue;
        }

        let symbol = symbol_ratio(&line);
        if symbol > 0.50 {
            noisy_weight += ((symbol - 0.50) * 1.8).clamp(0.0, 1.0);
        }
    }

    if total == 0 {
        return 1.0;
    }

    (noisy_weight / total as f32).clamp(0.0, 1.0)
}

/// Build a compact fallback snippet when model summarization is unavailable.
pub fn concise_fallback_snippet(app_name: &str, window_title: &str, text: &str) -> String {
    let normalized_title = normalize_inline(window_title.trim());
    let title_is_useful =
        !normalized_title.is_empty() && is_useful_snippet_line(app_name, &normalized_title);
    let mut details = Vec::new();
    let mut seen = HashSet::new();
    if title_is_useful {
        seen.insert(snippet_dedup_key(&normalized_title));
    }
    for line in text.lines() {
        if is_useful_snippet_line(app_name, line) {
            let normalized = normalize_inline(line);
            if normalized.is_empty() {
                continue;
            }
            if looks_like_file_inventory(&normalized) || looks_like_json_inventory(&normalized) {
                continue;
            }
            let key = snippet_dedup_key(&normalized);
            if seen.insert(key) {
                details.push(normalized);
            }
            if details.len() >= 2 {
                break;
            }
        }
    }

    if title_is_useful {
        let mut snippet = normalized_title.clone();
        if let Some(first) = details.first() {
            snippet.push_str(": ");
            snippet.push_str(first);
            if let Some(second) = details.get(1) {
                snippet.push_str(" | ");
                snippet.push_str(second);
            }
        }
        return truncate_snippet(&snippet, MAX_FALLBACK_SNIPPET_CHARS);
    }

    if !details.is_empty() {
        return truncate_snippet(&details.join(" | "), MAX_FALLBACK_SNIPPET_CHARS);
    }

    if !normalized_title.is_empty() {
        return truncate_snippet(&normalized_title, MAX_FALLBACK_SNIPPET_CHARS);
    }

    if !app_name.trim().is_empty() {
        return format!("Using {}", app_name.trim());
    }

    String::new()
}

/// Remove noisy lines; keep structure and duplicates handled upstream in OCR when possible.
pub fn reduce_chrome_noise_for_app(app_name: &str, text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut seen = HashSet::new();

    for line in text.lines() {
        let trimmed = normalize_inline(line.trim());
        if trimmed.len() < MIN_LINE_LEN {
            continue;
        }
        if should_drop_line(app_name, &trimmed) {
            tracing::trace!("Dropped likely chrome/noise line from capture text");
            continue;
        }
        let dedup_key = snippet_dedup_key(&trimmed);
        if !seen.insert(dedup_key) {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&trimmed);
    }

    out
}

pub fn build_high_signal_text_for_app(app_name: &str, text: &str) -> HighSignalText {
    let mut stats = CaptureQualityStats::default();
    let mut out = String::new();
    let mut seen = HashSet::new();
    let mut score_sum = 0.0f32;
    let mut scored_lines = 0usize;

    for line in text.lines() {
        let has_low_conf = line.contains("[LOW_CONF]");
        let normalized = strip_low_conf_marker(line.trim());
        if normalized.is_empty() {
            continue;
        }
        stats.total_lines += 1;
        if has_low_conf {
            stats.low_conf_lines += 1;
        }

        let quality = line_quality_score(app_name, &normalized, has_low_conf);
        score_sum += quality;
        scored_lines += 1;

        if should_drop_line(app_name, &normalized) {
            stats.dropped_noise_lines += 1;
            continue;
        }
        if quality < 0.36 || normalized.len() < MIN_LINE_LEN {
            stats.dropped_low_signal_lines += 1;
            continue;
        }
        let dedup_key = snippet_dedup_key(&normalized);
        if !seen.insert(dedup_key) {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&normalized);
        stats.kept_lines += 1;
    }

    stats.avg_line_score = if scored_lines == 0 {
        0.0
    } else {
        (score_sum / scored_lines as f32).clamp(0.0, 1.0)
    };

    if out.trim().is_empty() {
        out = reduce_chrome_noise_for_app(app_name, &text.replace("[LOW_CONF]", " "));
    }

    HighSignalText { text: out, stats }
}

fn line_quality_score(app_name: &str, line: &str, has_low_conf: bool) -> f32 {
    let symbol = symbol_ratio(line).clamp(0.0, 1.0);
    let alpha = if line.is_empty() {
        0.0
    } else {
        line.chars().filter(|ch| ch.is_alphanumeric()).count() as f32 / line.len() as f32
    };
    let len_score = (line.len().min(180) as f32 / 180.0).clamp(0.0, 1.0);
    let mut score = alpha * 0.52 + (1.0 - symbol) * 0.28 + len_score * 0.20;

    if has_low_conf {
        score *= 0.78;
    }
    if should_drop_line(app_name, line) {
        score *= 0.16;
    }
    if !is_code_app(app_name) && looks_like_file_inventory(line) {
        score *= 0.25;
    }

    score.clamp(0.0, 1.0)
}

/// Backward-compatible wrapper when app context is unavailable.
pub fn reduce_chrome_noise(text: &str) -> String {
    reduce_chrome_noise_for_app("", text)
}

// ── Semantic salience ranking ────────────────────────────────────────────────
//
// `rank_salient_spans`, `compress_to_salient_evidence`, and
// `salience_concentration` extend the existing `line_quality_score` /
// `estimate_noise_score` primitives one level up: a salient *span* is the
// blank-line- or sentence-bounded fragment likely to carry meaning rather than
// chrome. All signals are morphological / lexical / structural, no app names
// or URL hosts.

#[derive(Debug, Clone)]
pub struct SalientSpan {
    pub text: String,
    pub score: f32,
}

/// Maximum number of spans considered for salience aggregation; bounded so
/// pathological inputs don't run quadratic ranking.
const MAX_RANKED_SPANS: usize = 64;

fn split_block_into_sentences(block: &str) -> Vec<String> {
    let trimmed = block.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut buffer = String::new();
    let mut chars = trimmed.chars().peekable();
    while let Some(ch) = chars.next() {
        buffer.push(ch);
        let is_terminator = matches!(ch, '.' | '!' | '?');
        let next_is_break = matches!(chars.peek(), Some(' ') | Some('\n') | None);
        if (is_terminator && next_is_break) || ch == '\n' {
            let candidate = buffer.trim().to_string();
            if candidate.chars().count() >= 12 {
                out.push(candidate);
                buffer.clear();
            }
        }
    }
    let tail = buffer.trim().to_string();
    if !tail.is_empty() {
        out.push(tail);
    }
    if out.is_empty() {
        out.push(trimmed.to_string());
    }
    out
}

fn span_score(app_name: &str, span: &str) -> f32 {
    let normalized = normalize_inline(span);
    if normalized.chars().count() < 10 {
        return 0.0;
    }

    let mut line_sum = 0.0_f32;
    let mut line_count = 0usize;
    for line in span.lines() {
        let line = normalize_inline(line.trim());
        if line.is_empty() {
            continue;
        }
        line_sum += line_quality_score(app_name, &line, false);
        line_count += 1;
    }
    let avg_line = if line_count == 0 {
        line_quality_score(app_name, &normalized, false)
    } else {
        line_sum / line_count as f32
    };

    let tokens: Vec<&str> = normalized.split_whitespace().collect();
    if tokens.is_empty() {
        return 0.0;
    }

    let cap_count = tokens
        .iter()
        .filter(|tok| {
            tok.chars()
                .next()
                .map(|c| c.is_uppercase())
                .unwrap_or(false)
        })
        .count();
    let cap_ratio = ((cap_count as f32 / tokens.len() as f32).min(0.45) / 0.45).clamp(0.0, 1.0);

    let verb_count = tokens
        .iter()
        .filter(|tok| {
            let lower = tok.to_ascii_lowercase();
            lower.ends_with("ed")
                || lower.ends_with("ing")
                || lower.ends_with("tion")
                || lower.ends_with("sion")
                || lower.ends_with("ment")
                || lower.ends_with("able")
                || lower.ends_with("ible")
        })
        .count();
    let verb_ratio = ((verb_count as f32 / tokens.len() as f32).min(0.30) / 0.30).clamp(0.0, 1.0);

    let lower = normalized.to_ascii_lowercase();
    const CUE_WORDS: &[&str] = &[
        "because",
        "however",
        "therefore",
        "should",
        "must",
        "todo",
        "next",
        "plan",
        "need to",
        "consider",
        "implement",
        "review",
        "fix",
        "blocker",
        "decided",
        "open question",
    ];
    let cue_count = CUE_WORDS.iter().filter(|w| lower.contains(*w)).count();
    let cue_bonus = (cue_count as f32 * 0.05).min(0.18);

    let length_ratio = (normalized.chars().count().min(280) as f32 / 280.0).clamp(0.0, 1.0);

    let distinct: HashSet<&&str> = tokens.iter().collect();
    let distinct_ratio = (distinct.len() as f32 / tokens.len() as f32).clamp(0.0, 1.0);

    let score = avg_line * 0.40
        + cap_ratio * 0.12
        + verb_ratio * 0.10
        + length_ratio * 0.16
        + distinct_ratio * 0.12
        + cue_bonus;

    score.clamp(0.0, 1.0)
}

/// Score and order content-bearing spans in `cleaned`. Spans are split on
/// blank-line boundaries first, then on sentence terminators inside each
/// block. The returned vector is sorted high-score first.
pub fn rank_salient_spans(cleaned: &str, app_name: &str) -> Vec<SalientSpan> {
    if cleaned.trim().is_empty() {
        return Vec::new();
    }
    let mut spans: Vec<SalientSpan> = Vec::new();
    for block in cleaned.split("\n\n") {
        for sentence in split_block_into_sentences(block) {
            if spans.len() >= MAX_RANKED_SPANS {
                break;
            }
            let trimmed = sentence.trim();
            if trimmed.is_empty() {
                continue;
            }
            let score = span_score(app_name, trimmed);
            if score <= 0.0 {
                continue;
            }
            spans.push(SalientSpan {
                text: trimmed.to_string(),
                score,
            });
        }
        if spans.len() >= MAX_RANKED_SPANS {
            break;
        }
    }
    spans.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    spans
}

/// Replace the blind 320-character tail used by embedding composition with the
/// highest-ranked salient spans, capped at `max_chars`. Falls back to the raw
/// head of `cleaned` only when no scoring spans are recovered (e.g. very short
/// inputs).
pub fn compress_to_salient_evidence(cleaned: &str, app_name: &str, max_chars: usize) -> String {
    if max_chars == 0 || cleaned.trim().is_empty() {
        return String::new();
    }
    let spans = rank_salient_spans(cleaned, app_name);
    if spans.is_empty() {
        return cleaned.chars().take(max_chars).collect::<String>();
    }
    let mut out = String::new();
    for span in spans {
        if out.chars().count() >= max_chars {
            break;
        }
        if !out.is_empty() {
            out.push_str(" • ");
        }
        if out.chars().count() + span.text.chars().count() > max_chars {
            let remaining = max_chars.saturating_sub(out.chars().count());
            out.extend(span.text.chars().take(remaining));
            break;
        }
        out.push_str(&span.text);
    }
    out
}

/// Ratio of top-k span score mass to total span score mass; 1.0 means a few
/// dense spans carry the document's signal, near-0 means the signal is diffuse
/// (typically dominated by chrome / OCR noise).
pub fn salience_concentration(cleaned: &str, app_name: &str) -> f32 {
    let spans = rank_salient_spans(cleaned, app_name);
    if spans.is_empty() {
        return 0.0;
    }
    let total: f32 = spans.iter().map(|s| s.score).sum();
    if total <= 1e-6 {
        return 0.0;
    }
    let top_k = 3.min(spans.len());
    let top_sum: f32 = spans.iter().take(top_k).map(|s| s.score).sum();
    (top_sum / total).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_tab_strip_middots() {
        let raw = "Project roadmap for Q2\nGmail · Calendar · Drive · GitHub\nActual paragraph content here";
        let cleaned = reduce_chrome_noise_for_app("Safari", raw);
        assert!(cleaned.contains("Project roadmap"));
        assert!(cleaned.contains("Actual paragraph"));
        assert!(!cleaned.contains("Gmail"));
        assert!(!cleaned.contains("Calendar"));
    }

    #[test]
    fn drops_generic_browser_tab_labels() {
        let raw = "New Tab\nHome\nTrending\nPreparing launch checklist for FNDR search";
        let cleaned = reduce_chrome_noise_for_app("Chrome", raw);
        assert!(!cleaned.to_lowercase().contains("new tab"));
        assert!(!cleaned.to_lowercase().contains("home"));
        assert!(cleaned.contains("Preparing launch checklist"));
    }

    #[test]
    fn preserves_url_lines_for_chunk_boundaries() {
        let raw = "Project planning notes for launch\nhttps://example.com/path?query=value";
        let cleaned = reduce_chrome_noise_for_app("Chrome", raw);
        assert!(cleaned.contains("Project planning notes"));
        assert!(cleaned.contains("https://example.com/path?query=value"));
    }

    #[test]
    fn preserves_terminal_code_lines() {
        let raw = "cargo test --package fndr\nlet cards: Vec<MemoryCard> = synthesize();\nsrc/main.rs src/lib.rs src/search/mod.rs";
        let cleaned = reduce_chrome_noise_for_app("Terminal", raw);
        assert!(cleaned.contains("cargo test"));
        assert!(cleaned.contains("Vec<MemoryCard>"));
        assert!(cleaned.contains("src/main.rs"));
    }

    #[test]
    fn fallback_prefers_window_title() {
        let snippet = concise_fallback_snippet(
            "VSCode",
            "fndr - download_model.sh",
            "src app.rs src/lib.rs src/main.rs src-tauri/src/graph/mod.rs",
        );
        assert_eq!(snippet, "fndr - download_model.sh");
    }

    #[test]
    fn fallback_skips_file_inventory_lines() {
        let snippet = concise_fallback_snippet(
            "Chrome",
            "New Tab",
            "src/app.tsx src/lib.rs src/main.rs src-tauri/src/store/schema.rs\nFix memory summarization for OCR snippets",
        );
        assert_eq!(snippet, "Fix memory summarization for OCR snippets");
    }

    #[test]
    fn fallback_combines_title_with_useful_lines() {
        let snippet = concise_fallback_snippet(
            "Canva",
            "Series A investor deck",
            "Resizing design for instagram post and story sizes\nUpdated CTA slide with pricing details",
        );
        assert!(snippet.contains("Series A investor deck"));
        assert!(snippet.contains("Resizing design for instagram"));
    }

    #[test]
    fn marks_noisy_browser_payload_high_noise_score() {
        let raw = "New Tab\nHome\nTrending\nNotifications\nSuggested for you";
        let score = estimate_noise_score("Chrome", raw);
        assert!(score > 0.7);
    }

    #[test]
    fn high_signal_builder_strips_low_conf_markers_and_keeps_signal_lines() {
        let raw = "[LOW_CONF] New Tab\n[LOW_CONF] Home\nImplement robust OCR cleanup for capture pipeline\n[LOW_CONF] src/main.rs src/lib.rs src/api.ts";
        let out = build_high_signal_text_for_app("Chrome", raw);
        assert!(out.text.contains("Implement robust OCR cleanup"));
        assert!(!out.text.contains("[LOW_CONF]"));
        assert!(out.stats.total_lines >= 3);
        assert!(out.stats.kept_lines >= 1);
    }

    #[test]
    fn high_signal_builder_degrades_noisy_browser_frames() {
        let raw = "New Tab\nHome\nTrending\nSuggested for you\nNotifications\nExplore";
        let out = build_high_signal_text_for_app("Google Chrome", raw);
        assert!(out.stats.total_lines >= 5);
        assert_eq!(out.text.trim(), "");
        assert!(out.stats.kept_lines <= 1);
    }

    #[test]
    fn high_signal_builder_preserves_code_lines_in_developer_apps() {
        let raw = "cargo check\nsrc-tauri/src/capture/mod.rs\nfn validate_structured_memory_extraction(...)";
        let out = build_high_signal_text_for_app("Terminal", raw);
        assert!(out.text.contains("cargo check"));
        assert!(out.text.contains("validate_structured_memory_extraction"));
        assert!(out.stats.kept_lines >= 2);
    }

    #[test]
    fn high_signal_builder_keeps_email_semantics_without_navigation_noise() {
        let raw = "Inbox\nStarred\nSubject: Updated deployment plan\nPlease review the rollout risks before 4 PM.";
        let out = build_high_signal_text_for_app("Mail", raw);
        assert!(out.text.contains("Subject: Updated deployment plan"));
        assert!(out.text.contains("Please review the rollout risks"));
        assert!(!out.text.to_lowercase().contains("starred"));
    }

    #[test]
    fn rank_salient_spans_ranks_navigation_below_content() {
        let raw = "Home\n\nDiscover\n\nWe should consider refactoring the synthesis module because the durable context needs to survive across captures.\n\nNotifications";
        let spans = rank_salient_spans(raw, "GenericApp");
        assert!(!spans.is_empty(), "spans must not be empty");
        let top = &spans[0];
        assert!(
            top.text.to_lowercase().contains("durable context")
                || top.text.to_lowercase().contains("refactoring"),
            "top span should be the meaty sentence, got {:?}",
            top.text
        );
        let nav_top = spans.iter().position(|s| {
            s.text.eq_ignore_ascii_case("home") || s.text.eq_ignore_ascii_case("discover")
        });
        if let Some(idx) = nav_top {
            assert!(idx > 0, "nav-style spans must rank below content spans");
        }
    }

    #[test]
    fn compress_to_salient_evidence_respects_byte_budget() {
        let raw = "Reviewing the architecture document.\n\nWe decided to consolidate alias generation into a single helper to avoid drift between capture and rebuild paths.\n\nNext steps: validate retrieval scores on the regression fixtures and update the design doc.";
        let compressed = compress_to_salient_evidence(raw, "Editor", 120);
        assert!(compressed.chars().count() <= 120);
        assert!(
            compressed.to_lowercase().contains("alias")
                || compressed.to_lowercase().contains("consolidate")
                || compressed.to_lowercase().contains("next steps"),
            "compressed evidence should carry semantic spans, got: {}",
            compressed
        );
    }

    #[test]
    fn salience_concentration_increases_with_signal_density() {
        let polluted = "Home\nDiscover\nTrending\nNew Tab\nNotifications";
        let dense = "We must finalize the durable memory context implementation before the next release. The reopen anchor needs to survive truncation. Decisions made today should be documented.";
        let polluted_score = salience_concentration(polluted, "Chrome");
        let dense_score = salience_concentration(dense, "Editor");
        assert!(
            dense_score >= polluted_score,
            "dense content should have >= concentration than nav frames (dense={dense_score}, polluted={polluted_score})"
        );
    }
}
