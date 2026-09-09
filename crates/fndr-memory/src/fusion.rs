//! Deterministic structured-extraction assembly: the no-LLM fallback and the
//! durable `memory_context` composer (T-601).
//!
//! Ported from `origin/reference/v1:src-tauri/src/capture/mod.rs` under
//! ADR-005 (`build_low_ram_semantic_fusion`, `build_durable_memory_context`
//! and their private helpers). Like `continuity.rs`, this module has no store
//! or model dependency: callers supply already-cleaned text and already
//! retrieved prior-context snippets, and retain ownership of persistence.
//!
//! `validate_structured_memory_extraction` (the grounding validator ADR-005
//! names alongside these two) is intentionally NOT ported here: ROADMAP
//! T-602 ("Port VLM synthesis prompts and grounding validation") owns it,
//! since in v1 it validates synthesis (VLM/LLM) output against evidence, a
//! concern that only exists once a generative branch exists to validate.

use fndr_textsignal::{AppIdentity, CaptureQualityStats, SalientSpan, rank_salient_spans};

use crate::continuity::domain;

/// Which branch produced a [`StructuredExtraction`]. v1 threaded this as a
/// free string (`"vlm" | "llm" | "browser_semantic" | "fallback"`); a
/// persisted discriminant is an enum here per the crate's lifecycle-state
/// convention. Only the branch this ticket ports is a variant; T-602 adds
/// the generative branches when it lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SynthesisBranch {
    /// The deterministic, no-model fallback in this module.
    LowRamFusion,
}

/// The shared structured-extraction shape either the deterministic fallback
/// (this module) or VLM/LLM synthesis (T-602) produces. Fields no ported
/// function reads (v1's `dedup_fingerprint`, `tags`, `git_stats`,
/// `symbols_changed`, `outcome`, `todos`, `open_questions`,
/// `topic_categories`, `commands`, `session_key`) are not carried; a later
/// ticket adds them alongside the consumer that needs them.
#[derive(Debug, Clone, Default)]
pub struct StructuredExtraction {
    pub activity_type: String,
    pub project: String,
    pub topic: String,
    pub memory_context: String,
    pub workflow: String,
    pub user_intent: String,
    pub files_touched: Vec<String>,
    pub entities: Vec<String>,
    pub search_aliases: Vec<String>,
    pub decisions: Vec<String>,
    pub errors: Vec<String>,
    pub blockers: Vec<String>,
    pub next_steps: Vec<String>,
    pub results: Vec<String>,
    pub confidence: f32,
    pub synthesis_branch: Option<SynthesisBranch>,
}

/// The minimal slice of v1's `BrowserSemanticContent` the fusion fallback
/// reads. A local, borrowed type rather than a dependency on a capture-lane
/// crate: no browser-semantics producer is wired in v2 yet, and this module
/// must stay caller-supplied (`ADR-005`, "no store or model dependency").
#[derive(Debug, Clone, Copy, Default)]
pub struct BrowserSemanticHint<'a> {
    pub h1: &'a str,
    pub title: &'a str,
    pub meta_description: &'a str,
    pub article_excerpt: &'a str,
    pub content_signal_score: f32,
}

impl BrowserSemanticHint<'_> {
    /// Ported from `BrowserSemanticContent::has_signal`; the three
    /// thresholds are v1's tuning (a strong content-signal score alone
    /// qualifies, or enough article/description volume to trust the page
    /// even at a middling score).
    fn has_signal(&self) -> bool {
        self.content_signal_score >= 0.18
            || self.article_excerpt.split_whitespace().count() >= 24
            || self.meta_description.split_whitespace().count() >= 10
    }
}

/// One deterministically fused candidate record, with the sources it drew on
/// for a future explainability surface (PRD P0.11 pipeline legibility).
#[derive(Debug, Clone)]
pub struct SemanticFusionDraft {
    pub extraction: StructuredExtraction,
    pub sources: Vec<&'static str>,
    pub reason: &'static str,
}

/// Ported from `capture::mod::infer_review_activity`.
fn infer_review_activity(clean_text: &str) -> (&'static str, &'static str, &'static str) {
    let lower = clean_text.to_ascii_lowercase();
    if lower.contains("error") || lower.contains("failed") || lower.contains("debug") {
        ("debugging", "debugging", "debugging visible issue context")
    } else if lower.contains("todo")
        || lower.contains("planned")
        || lower.contains("roadmap")
        || lower.contains("implemented")
        || lower.contains("docs")
        || lower.contains("design")
    {
        (
            "reviewing",
            "reviewing",
            "reviewing implementation status and supporting context",
        )
    } else {
        ("reviewing", "reviewing", "reviewing visible screen context")
    }
}

fn clean_file_reference(token: &str) -> String {
    token
        .trim_matches(|ch: char| {
            ch.is_whitespace()
                || matches!(
                    ch,
                    ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}' | '"' | '\'' | '`' | '•'
                )
        })
        .trim_end_matches([':', '.', ')', ']'])
        .to_string()
}

fn looks_like_file_reference(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    const EXTENSIONS: &[&str] = &[
        ".md", ".rs", ".ts", ".tsx", ".js", ".jsx", ".json", ".toml", ".yaml", ".yml", ".css",
        ".html", ".py", ".swift", ".sh",
    ];
    EXTENSIONS
        .iter()
        .any(|ext| lower.ends_with(ext) || lower.contains(&format!("{ext}:")))
}

/// Ported from `capture::mod::extract_file_references`.
fn extract_file_references(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for raw in text.split_whitespace() {
        let mut token = clean_file_reference(raw);
        if let Some((head, _line)) = token.rsplit_once(':')
            && looks_like_file_reference(head)
        {
            token = clean_file_reference(head);
        }
        if !looks_like_file_reference(&token) {
            continue;
        }
        let key = token.to_ascii_lowercase();
        if seen.insert(key) {
            out.push(token);
            if out.len() >= 12 {
                break;
            }
        }
    }
    out
}

fn merge_unique_strings(existing: &mut Vec<String>, incoming: impl IntoIterator<Item = String>) {
    let mut seen: std::collections::HashSet<String> = existing
        .iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .collect();
    for value in incoming {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen.insert(trimmed.to_ascii_lowercase()) {
            existing.push(trimmed.to_string());
        }
    }
}

/// The no-LLM fallback: build a [`SemanticFusionDraft`] from OCR-cleaned
/// text, window/app context, and an optional browser-semantics hint, with no
/// model call. Returns `None` (a typed, visible "insufficient evidence"
/// outcome, never a mostly-empty draft) when the text is short and no
/// browser-semantic signal backs it.
///
/// Ported from `capture::mod::build_low_ram_semantic_fusion`.
#[allow(clippy::too_many_arguments)]
pub fn build_low_ram_semantic_fusion(
    app_name: &str,
    bundle_id: Option<&str>,
    window_title: &str,
    url: Option<&str>,
    clean_text: &str,
    semantic_page: Option<BrowserSemanticHint<'_>>,
    capture_quality: &CaptureQualityStats,
    source_kind: &str,
) -> Option<SemanticFusionDraft> {
    let text = clean_text.trim();
    if text.len() < 80 && semantic_page.map(|page| !page.has_signal()).unwrap_or(true) {
        return None;
    }

    let app = AppIdentity::new(app_name, bundle_id);
    let spans: Vec<SalientSpan> = rank_salient_spans(text, app);
    let salient: Vec<String> = spans
        .iter()
        .filter(|span| span.score >= 0.30)
        .take(3)
        .map(|span| span.text.clone())
        .collect();
    let files = extract_file_references(text);

    let semantic_title = semantic_page
        .and_then(|page| {
            [page.h1, page.title, page.meta_description]
                .into_iter()
                .map(str::trim)
                .find(|value| !value.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_default();

    let topic = if !semantic_title.trim().is_empty() {
        semantic_title.chars().take(120).collect::<String>()
    } else if !files.is_empty() {
        files.iter().take(3).cloned().collect::<Vec<_>>().join(" and ")
    } else if let Some(first) = salient.first() {
        first.chars().take(120).collect::<String>()
    } else if !window_title.trim().is_empty() {
        window_title.trim().chars().take(120).collect::<String>()
    } else {
        app_name.trim().chars().take(120).collect::<String>()
    };

    let (activity, workflow, user_intent) = infer_review_activity(text);
    let subject = if !files.is_empty() {
        format!(
            "visible files {}",
            files.iter().take(4).cloned().collect::<Vec<_>>().join(", ")
        )
    } else if !topic.trim().is_empty() {
        topic.clone()
    } else {
        window_title.trim().to_string()
    };

    let mut sentences = Vec::new();
    let surface = if !window_title.trim().is_empty() {
        format!("{} in {}", app_name.trim(), window_title.trim())
    } else {
        app_name.trim().to_string()
    };
    sentences.push(format!("You were reviewing {subject} on {surface}."));
    if let Some(page_domain) = url.and_then(|value| domain(Some(value))) {
        sentences.push(format!("The visible page was from {page_domain}."));
    }
    if !semantic_title.trim().is_empty() && !sentences.join(" ").contains(&semantic_title) {
        sentences.push(format!("Browser context: {semantic_title}."));
    }
    let lower_text = text.to_ascii_lowercase();
    if lower_text.contains("planned")
        || lower_text.contains("implemented")
        || lower_text.contains("roadmap")
        || lower_text.contains("design")
        || lower_text.contains("docs")
    {
        sentences.push(
            "The visible context was about implementation status, docs, or roadmap items."
                .to_string(),
        );
    }

    let mut entities = Vec::new();
    merge_unique_strings(&mut entities, files.iter().cloned());
    if !semantic_title.trim().is_empty() {
        merge_unique_strings(&mut entities, [semantic_title.clone()]);
    }
    if let Some(page_domain) = url.and_then(|value| domain(Some(value))) {
        merge_unique_strings(&mut entities, [page_domain]);
    }

    let mut aliases = Vec::new();
    merge_unique_strings(&mut aliases, files.iter().cloned());
    merge_unique_strings(&mut aliases, [topic.clone()]);

    let mut sources = vec!["ocr_salient_spans", "app_window"];
    if !files.is_empty() {
        sources.push("file_references");
    }
    if semantic_page.is_some() {
        sources.push("browser_semantic");
    }
    if source_kind == "browser_semantic" {
        sources.push("browser_text_source");
    }

    // Base 0.58 plus weighted span/semantic/keep-ratio evidence, clamped to
    // v1's tuned [0.60, 0.86] band: enough headroom below 1.0 that a
    // deterministic fallback never outranks a grounded VLM/LLM extraction.
    let keep_ratio = if capture_quality.total_lines == 0 {
        0.0
    } else {
        capture_quality.kept_lines as f32 / capture_quality.total_lines as f32
    };
    let avg_span_score = if spans.is_empty() {
        0.0
    } else {
        spans.iter().take(3).map(|span| span.score).sum::<f32>() / spans.len().min(3) as f32
    };
    let semantic_score = semantic_page.map(|page| page.content_signal_score).unwrap_or(0.0);
    let confidence =
        (0.58 + avg_span_score * 0.16 + semantic_score * 0.12 + keep_ratio.clamp(0.0, 1.0) * 0.08)
            .clamp(0.60, 0.86);

    Some(SemanticFusionDraft {
        extraction: StructuredExtraction {
            activity_type: activity.to_string(),
            project: String::new(),
            topic,
            memory_context: sentences.join(" "),
            workflow: workflow.to_string(),
            user_intent: user_intent.to_string(),
            files_touched: files,
            entities,
            search_aliases: aliases,
            confidence,
            synthesis_branch: Some(SynthesisBranch::LowRamFusion),
            ..Default::default()
        },
        sources,
        reason: "low_ram_deterministic_semantic_fusion",
    })
}

/// Bounds for [`build_durable_memory_context`]. Named config per the crate
/// convention (no scattered literals); defaults are v1's tuned values.
#[derive(Debug, Clone, Copy)]
pub struct DurableContextConfig {
    pub min_chars: usize,
    pub max_chars: usize,
}

impl Default for DurableContextConfig {
    fn default() -> Self {
        // Ported from v1 `config.rs` `DEFAULT_MEMORY_CONTEXT_{MIN,MAX}_CHARS`:
        // padding stops below 220 chars (weak-grounding signal downstream),
        // truncation starts above 1800 chars (stay embedding-friendly).
        Self {
            min_chars: 220,
            max_chars: 1_800,
        }
    }
}

/// The durable `memory_context` of one prior, already-retrieved record in
/// the continuity chain. A minimal borrowed slice of v1's `SearchResult`:
/// `build_continuation_footer` reads only this field, and this module must
/// not depend on the retrieval crate's result type.
#[derive(Debug, Clone, Copy)]
pub struct PriorContext<'a> {
    pub memory_context: &'a str,
}

fn narrative_mentions(narrative: Option<&str>, topic: &str) -> bool {
    let Some(narrative) = narrative else {
        return false;
    };
    let topic_tokens: Vec<String> = topic
        .to_ascii_lowercase()
        .chars()
        .map(|character| if character.is_ascii_alphanumeric() { character } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .filter(|token| token.len() >= 3)
        .map(str::to_string)
        .collect();
    if topic_tokens.is_empty() {
        return false;
    }
    let narrative_lower = narrative.to_ascii_lowercase();
    let hits = topic_tokens
        .iter()
        .filter(|token| narrative_lower.contains(token.as_str()))
        .count();
    (hits as f32 / topic_tokens.len() as f32) >= 0.6
}

/// Pick a deterministic semantic anchor for the durable memory context.
/// Priority: structured topic -> first salient span head -> window-title.
/// No app names: fully content-derived.
fn pick_semantic_center(
    extraction: Option<&StructuredExtraction>,
    app: AppIdentity<'_>,
    window_title: &str,
    clean_text: &str,
) -> String {
    if let Some(mem) = extraction {
        let topic = mem.topic.trim();
        if !topic.is_empty() && !topic.eq_ignore_ascii_case("unknown") {
            return topic.to_string();
        }
    }
    let spans = rank_salient_spans(clean_text, app);
    if let Some(top) = spans.first() {
        let trimmed = top
            .text
            .split_terminator(['.', '!', '?', '\n'])
            .next()
            .unwrap_or(&top.text)
            .trim();
        if !trimmed.is_empty() {
            return trimmed.chars().take(120).collect::<String>();
        }
    }
    let title = window_title.trim();
    if !title.is_empty() {
        return title.chars().take(120).collect();
    }
    String::new()
}

/// Compose a human-readable continuity footer.
fn build_continuation_footer(prior_chain: &[PriorContext<'_>]) -> String {
    let mut lines: Vec<String> = Vec::new();
    if let Some(prev) = prior_chain.first() {
        let head: String = prev
            .memory_context
            .trim()
            .split('\n')
            .next()
            .unwrap_or("")
            .chars()
            .take(80)
            .collect();
        if !head.trim().is_empty() {
            lines.push(format!("This continues earlier related work: {}.", head.trim()));
        } else {
            lines.push("This continues earlier related work from the same session.".to_string());
        }
    }
    lines.join("\n")
}

/// Pad short contexts with grounded structured fields. Pure helper; no I/O
/// and no raw OCR tail copying into the durable `memory_context`. (v1 took a
/// `clean_text` parameter here that its own body never read, marked with
/// `let _ = clean_text;`; dropped in this port as dead weight.)
fn pad_with_structured(
    base: &str,
    extraction: Option<&StructuredExtraction>,
    app_name: &str,
    min_chars: usize,
) -> String {
    if base.chars().count() >= min_chars {
        return base.to_string();
    }
    let mut out = base.to_string();
    let mut extras: Vec<String> = Vec::new();
    let base_norm = normalize_text_for_overlap(base);
    if let Some(mem) = extraction {
        let topic_norm = normalize_text_for_overlap(mem.topic.trim());
        if !mem.topic.trim().is_empty()
            && !mem.topic.trim().eq_ignore_ascii_case("unknown")
            && (topic_norm.is_empty() || !base_norm.contains(&topic_norm))
        {
            extras.push(format!("Topic: {}", mem.topic.trim()));
        }
        if !mem.user_intent.trim().is_empty() {
            extras.push(format!("Intent: {}", mem.user_intent.trim()));
        }
        if !mem.workflow.trim().is_empty() && !mem.workflow.trim().eq_ignore_ascii_case("unknown") {
            extras.push(format!("Workflow: {}", mem.workflow.trim()));
        }
        if !mem.files_touched.is_empty() {
            extras.push(format!(
                "Files: {}",
                mem.files_touched.iter().take(4).cloned().collect::<Vec<_>>().join(", ")
            ));
        }
        if !mem.entities.is_empty() {
            extras.push(format!(
                "Entities: {}",
                mem.entities.iter().take(4).cloned().collect::<Vec<_>>().join(", ")
            ));
        }
        if !mem.decisions.is_empty() {
            extras.push(format!(
                "Decisions: {}",
                mem.decisions.iter().take(2).cloned().collect::<Vec<_>>().join("; ")
            ));
        }
        if !mem.results.is_empty() {
            extras.push(format!(
                "Results: {}",
                mem.results.iter().take(2).cloned().collect::<Vec<_>>().join("; ")
            ));
        }
        if !mem.next_steps.is_empty() {
            extras.push(format!(
                "Next: {}",
                mem.next_steps.iter().take(2).cloned().collect::<Vec<_>>().join("; ")
            ));
        }
    }
    for extra in extras {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&extra);
        if out.chars().count() >= min_chars {
            return out;
        }
    }
    if out.chars().count() < min_chars {
        let surface = if app_name.trim().is_empty() {
            String::new()
        } else {
            format!("Source app: {}", app_name.trim())
        };
        if !surface.trim().is_empty() && !out.contains(&surface) {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&surface);
        }
    }
    out
}

fn normalize_text_for_overlap(text: &str) -> String {
    text.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Capture-time durable `memory_context`. Composes three sections (what /
/// state / where) bounded by config min/max chars, appends a continuation
/// footer, and falls back gracefully when structured extraction is absent.
///
/// Ported from `capture::mod::build_durable_memory_context`. v1's signature
/// carried `_bundle_id` and `_url` parameters its own body never read
/// (prefixed `_`); dropped here as dead weight the port does not need to
/// preserve.
pub fn build_durable_memory_context(
    extraction: Option<&StructuredExtraction>,
    app_name: &str,
    bundle_id: Option<&str>,
    window_title: &str,
    clean_text: &str,
    display_summary: &str,
    prior_chain: &[PriorContext<'_>],
    config: &DurableContextConfig,
) -> String {
    let app = AppIdentity::new(app_name, bundle_id);
    let center = pick_semantic_center(extraction, app, window_title, clean_text);

    // Narrative-first: the LLM's free-form `memory_context` is the most
    // human-readable description available and what humans/agents want to
    // see in retrieval surfaces. Lead with it and only fall back to
    // structured "Topic:/You were/Activity:" lines when no narrative is
    // present. Topic: is appended only if it adds tokens the narrative does
    // not already cover.
    let narrative: Option<String> = extraction.and_then(|mem| {
        let trimmed = mem.memory_context.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    });
    let mut what_lines: Vec<String> = Vec::new();
    if let Some(ref narrative_text) = narrative {
        what_lines.push(narrative_text.clone());
    }
    if !center.is_empty() && !narrative_mentions(narrative.as_deref(), &center) {
        what_lines.push(format!("Topic: {center}"));
    }
    if narrative.is_none()
        && let Some(mem) = extraction
    {
        let intent = mem.user_intent.trim();
        if !intent.is_empty() {
            what_lines.push(format!("You were {intent}."));
        } else if !mem.activity_type.trim().is_empty()
            && !mem.activity_type.trim().eq_ignore_ascii_case("unknown")
        {
            what_lines.push(format!("Activity: {}.", mem.activity_type.trim()));
        }
    }
    if what_lines.is_empty() && !display_summary.trim().is_empty() {
        what_lines.push(display_summary.trim().to_string());
    }
    let what_section = what_lines.join("\n");

    let why_section = extraction
        .map(|mem| {
            let mut bits: Vec<String> = Vec::new();
            if !mem.decisions.is_empty() {
                bits.push(format!(
                    "Decisions: {}",
                    mem.decisions.iter().take(3).cloned().collect::<Vec<_>>().join("; ")
                ));
            }
            if !mem.errors.is_empty() {
                bits.push(format!(
                    "Errors: {}",
                    mem.errors.iter().take(3).cloned().collect::<Vec<_>>().join("; ")
                ));
            }
            if !mem.blockers.is_empty() {
                bits.push(format!(
                    "Blockers: {}",
                    mem.blockers.iter().take(3).cloned().collect::<Vec<_>>().join("; ")
                ));
            }
            if !mem.next_steps.is_empty() {
                bits.push(format!(
                    "Next: {}",
                    mem.next_steps.iter().take(3).cloned().collect::<Vec<_>>().join("; ")
                ));
            }
            if !mem.results.is_empty() {
                bits.push(format!(
                    "Results: {}",
                    mem.results.iter().take(2).cloned().collect::<Vec<_>>().join("; ")
                ));
            }
            bits.join("\n")
        })
        .unwrap_or_default();

    let where_section = build_continuation_footer(prior_chain);

    let mut sections: Vec<String> = Vec::new();
    if !what_section.trim().is_empty() {
        sections.push(what_section);
    }
    if !why_section.trim().is_empty() {
        sections.push(why_section);
    }
    if !where_section.trim().is_empty() {
        sections.push(where_section);
    }

    let mut combined = sections.join("\n\n");
    if combined.chars().count() < config.min_chars {
        combined = pad_with_structured(&combined, extraction, app_name, config.min_chars);
    }
    if combined.chars().count() > config.max_chars {
        let mut truncated: String =
            combined.chars().take(config.max_chars.saturating_sub(3)).collect();
        truncated.push_str("...");
        combined = truncated;
    }
    combined
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quality_ok_text() -> &'static str {
        "Push latest changes\nREADME.md\nDESIGN_DIRECTION.md\nRemoved untracked planning/artifact files and folders.\nStill planned but not implemented (from current committed docs):\n1. Optional Qwen3-VL + mmproj photo-import vision path is documented as optional setup, not baseline behavior yet (README.md:202).\n2. Image-aware retrieval is explicitly future (CLIP vector stored now, richer retrieval later) (README.md:129).\n3. Future graph enrichment runtime is noted as future/additive in design direction (DESIGN_DIRECTION.md:106).\nFuture Roadmap\nNear Term\nAdvanced idle detection\nMedium Term\nSemantic timeline (group by topic, not just time)\nActivity patterns and insights dashboard\n"
    }

    fn quality_stats() -> CaptureQualityStats {
        CaptureQualityStats {
            total_lines: 51,
            kept_lines: 50,
            low_conf_lines: 51,
            dropped_noise_lines: 0,
            dropped_low_signal_lines: 1,
            avg_line_score: 0.58,
        }
    }

    #[test]
    fn low_ram_semantic_fusion_builds_codex_docs_memory_without_topic_scaffold() {
        let fusion = build_low_ram_semantic_fusion(
            "Codex",
            None,
            "Push latest changes",
            None,
            quality_ok_text(),
            None,
            &quality_stats(),
            "ocr",
        )
        .expect("fusion should build from OCR, file refs, and app/window context");

        assert!(fusion.extraction.memory_context.contains("README.md"));
        assert!(fusion.extraction.memory_context.contains("DESIGN_DIRECTION.md"));
        assert!(fusion.extraction.user_intent.contains("implementation status"));
        assert!(!fusion.extraction.memory_context.starts_with("Topic:"));
        assert!(!fusion.extraction.search_aliases.iter().any(|alias| alias.contains("tspbn")));
        assert!(fusion.extraction.files_touched.iter().any(|file| file == "README.md"));
        assert!(fusion.extraction.files_touched.iter().any(|file| file == "DESIGN_DIRECTION.md"));
    }

    #[test]
    fn low_ram_semantic_fusion_returns_none_for_short_unbacked_text() {
        // Typed, visible "insufficient evidence" outcome (invariant 4): a
        // short capture with no browser-semantic backing must not produce a
        // mostly-empty draft.
        assert!(
            build_low_ram_semantic_fusion(
                "Notes",
                None,
                "Untitled",
                None,
                "hi",
                None,
                &CaptureQualityStats::default(),
                "ocr",
            )
            .is_none()
        );
    }

    #[test]
    fn low_ram_semantic_fusion_admits_short_text_with_semantic_backing() {
        let hint = BrowserSemanticHint {
            h1: "Screenpipe architecture deep dive",
            title: "",
            meta_description: "",
            article_excerpt: "",
            content_signal_score: 0.4,
        };
        assert!(
            build_low_ram_semantic_fusion(
                "Google Chrome",
                None,
                "Docs",
                Some("https://docs.screenpi.pe/architecture"),
                "short",
                Some(hint),
                &CaptureQualityStats::default(),
                "browser_semantic",
            )
            .is_some()
        );
    }

    fn durable_context_config(min: usize, max: usize) -> DurableContextConfig {
        DurableContextConfig { min_chars: min, max_chars: max }
    }

    #[test]
    fn durable_memory_context_respects_min_max_bounds_with_empty_chain() {
        let extraction = StructuredExtraction {
            user_intent: "Refactor the synthesis pipeline".to_string(),
            project: "FNDR".to_string(),
            topic: "memory synthesis".to_string(),
            decisions: vec!["Use durable memory_context as embedding seed".to_string()],
            next_steps: vec!["Wire compress_to_salient_evidence into the tail".to_string()],
            ..Default::default()
        };
        let cfg = durable_context_config(220, 1800);
        let context = build_durable_memory_context(
            Some(&extraction),
            "GenericEditor",
            Some("com.example.editor"),
            "synthesis-doc.md",
            "We are aligning the embedding text composition.",
            "Refactored OCR cleanup.",
            &[],
            &cfg,
        );
        assert!(
            context.chars().count() >= cfg.min_chars,
            "durable context shorter than min ({}): {}",
            context.chars().count(),
            context
        );
        assert!(context.chars().count() <= cfg.max_chars);
        assert!(
            context.to_lowercase().contains("topic")
                || context.to_lowercase().contains("memory synthesis"),
            "should surface semantic center"
        );
    }

    #[test]
    fn durable_memory_context_references_prior_work_without_machine_marker() {
        let extraction = StructuredExtraction {
            user_intent: "Continue refactor".to_string(),
            topic: "alias generation".to_string(),
            decisions: vec!["Adopt noun-phrase sourcing".to_string()],
            ..Default::default()
        };
        let cfg = durable_context_config(160, 1800);
        let prior = PriorContext {
            memory_context: "Earlier card outlining the durable memory context plan.",
        };
        let context = build_durable_memory_context(
            Some(&extraction),
            "GenericEditor",
            None,
            "doc",
            "More work",
            "",
            std::slice::from_ref(&prior),
            &cfg,
        );
        assert!(
            context.contains("This continues earlier related work"),
            "continuation footer missing: {context}"
        );
        assert!(
            !context.contains("Continues from "),
            "machine continuation marker leaked into memory_context: {context}"
        );
    }

    #[test]
    fn durable_memory_context_keeps_reopen_marker_out_of_context() {
        let cfg = durable_context_config(160, 1800);
        let context = build_durable_memory_context(
            None,
            "GenericEditor",
            None,
            "doc",
            "Worked on the design doc and reviewed the spec.",
            "Reviewed design doc.",
            &[],
            &cfg,
        );
        assert!(!context.contains("Reopen:"), "reopen marker leaked into memory_context: {context}");
    }

    #[test]
    fn durable_memory_context_truncates_to_max_with_three_priors() {
        let extraction = StructuredExtraction {
            user_intent: "Long-running multi-session refactor".to_string(),
            topic: "memory synthesis".to_string(),
            decisions: vec!["a; ".repeat(60).trim_end().to_string()],
            errors: vec!["x; ".repeat(60).trim_end().to_string()],
            next_steps: vec!["n; ".repeat(60).trim_end().to_string()],
            results: vec!["r; ".repeat(60).trim_end().to_string()],
            ..Default::default()
        };
        let cfg = durable_context_config(220, 600);
        let priors = vec![
            PriorContext { memory_context: "Prior alpha context." },
            PriorContext { memory_context: "Prior beta context." },
            PriorContext { memory_context: "Prior gamma context." },
        ];
        let context = build_durable_memory_context(
            Some(&extraction),
            "GenericEditor",
            None,
            "doc",
            "evidence body",
            "Reviewed design doc.",
            &priors,
            &cfg,
        );
        assert!(context.chars().count() <= cfg.max_chars);
    }

    #[test]
    fn durable_memory_context_leads_with_narrative_and_drops_topic_when_covered() {
        let extraction = StructuredExtraction {
            user_intent: "researching".to_string(),
            topic: "memory synthesis".to_string(),
            activity_type: "researching".to_string(),
            memory_context: "Continued the memory synthesis investigation, comparing two ranking strategies."
                .to_string(),
            ..Default::default()
        };
        let cfg = durable_context_config(80, 1800);
        let context = build_durable_memory_context(
            Some(&extraction),
            "GenericEditor",
            None,
            "research-doc.md",
            "evidence body",
            "Reviewed design doc.",
            &[],
            &cfg,
        );
        let head_line = context.lines().next().unwrap_or("");
        assert!(
            head_line.starts_with("Continued the memory synthesis"),
            "narrative should lead, got: {head_line:?}"
        );
        assert!(
            !context.contains("Topic: memory synthesis"),
            "Topic: line must be dropped when narrative covers it; got: {context}"
        );
        assert!(!context.contains("You were "));
        assert!(!context.contains("Activity: "));
    }

    #[test]
    fn durable_memory_context_appends_topic_when_narrative_lacks_it() {
        let extraction = StructuredExtraction {
            topic: "ranking quality".to_string(),
            memory_context: "Focused on chart styling improvements for the dashboard.".to_string(),
            ..Default::default()
        };
        let cfg = durable_context_config(80, 1800);
        let context = build_durable_memory_context(
            Some(&extraction),
            "GenericEditor",
            None,
            "doc",
            "evidence body",
            "Reviewed design doc.",
            &[],
            &cfg,
        );
        assert!(
            context.contains("Topic: ranking quality"),
            "Topic should be appended when narrative omits the topic tokens; got: {context}"
        );
    }

    #[test]
    fn narrative_mentions_detects_majority_overlap() {
        assert!(narrative_mentions(
            Some("The memory synthesis pipeline keeps cards short."),
            "memory synthesis pipeline",
        ));
        assert!(!narrative_mentions(
            Some("Browsed unrelated news articles."),
            "memory synthesis pipeline",
        ));
        assert!(!narrative_mentions(None, "any topic"));
    }
}
