//! Fluff stripping for synthesized insight text.
//!
//! Ported from
//! `origin/reference/v1:src-tauri/src/memory_insight/fluff.rs` under
//! ADR-005.
//!
//! Removes content-free padding that doesn't convey what the user was
//! doing, such as repeated app names, project names, website names, or
//! generic "the user did X" prose. Operates entirely on the synthesized
//! text; does NOT touch structured fields, OCR, or embeddings.
//!
//! Design principles (from v1):
//! - No hardcoded per-domain mappings (cricket -> sport, etc.)
//! - Use the metadata already on the record to identify what's redundant
//! - Always preserve grammar; never strip mid-phrase

/// Strip repetitive mentions of app_name, project, and domain from an
/// insight string. Keeps the first occurrence; collapses runs of
/// two-or-more identical noun chunks. Also drops the universally-fluffy
/// "the user ..." pattern.
pub fn strip_fluff(text: &str, app_name: &str, project: &str, domain: &str) -> String {
    if text.trim().is_empty() {
        return text.to_string();
    }

    let mut out = text.to_string();

    // Drop "the user" / "The user" prefixes: the synthesis prompt asks for
    // "You" already, but older models or fallbacks leak this.
    out = drop_pattern_ci(&out, "the user ");
    out = drop_pattern_ci(&out, "user is ");
    out = drop_pattern_ci(&out, "user was ");

    // Strip generic preambles that don't describe the task.
    for preamble in [
        "i see that ",
        "the screen shows ",
        "this is a ",
        "this appears to be ",
        "the image shows ",
        "the user is viewing ",
    ] {
        out = drop_pattern_ci(&out, preamble);
    }

    // Collapse 2+ adjacent identical noun chunks separated by " — ", " - ",
    // " | " (these are the most common forms of fluff repetition, e.g.
    // "Google Chrome — Google Chrome — news article").
    out = collapse_adjacent_duplicates(&out);

    // Strip per-metadata redundancy: when the app/project/domain appears
    // more than once, keep the first occurrence (the UI chip already shows
    // it). Case-insensitive match, preserves the original casing.
    for redundant in [app_name, project, domain] {
        let trimmed = redundant.trim();
        if trimmed.len() < 3 {
            continue;
        }
        out = remove_subsequent_occurrences_ci(&out, trimmed);
    }

    // Tidy double spaces and stray punctuation introduced by removals.
    out = normalize_whitespace(&out);
    out = strip_leading_punct(&out);

    out
}

fn drop_pattern_ci(text: &str, pattern: &str) -> String {
    let lower = text.to_ascii_lowercase();
    if let Some(pos) = lower.find(pattern) {
        // Only drop if it's at start-of-sentence or start-of-string.
        let prefix = &text[..pos];
        let at_boundary = prefix.is_empty()
            || prefix.ends_with(['.', '!', '?', '\n', ' '])
                && prefix.chars().rev().take(2).all(|character| !character.is_alphanumeric());
        if at_boundary || pos == 0 {
            let end = pos + pattern.len();
            let after = &text[end..];
            // Capitalize the next word so the sentence still reads cleanly.
            let after_capitalized = capitalize_first(after);
            return format!("{prefix}{after_capitalized}");
        }
    }
    text.to_string()
}

fn collapse_adjacent_duplicates(text: &str) -> String {
    let separators = [" — ", " - ", " | ", " · "];
    let mut current = text.to_string();
    for sep in separators {
        let parts: Vec<&str> = current.split(sep).collect();
        if parts.len() < 2 {
            continue;
        }
        let mut kept: Vec<&str> = Vec::new();
        let mut last_key: Option<String> = None;
        for part in parts {
            let key = part.trim().to_ascii_lowercase();
            if Some(&key) == last_key.as_ref() {
                continue; // identical to previous chunk: skip
            }
            kept.push(part);
            last_key = Some(key);
        }
        current = kept.join(sep);
    }
    current
}

fn remove_subsequent_occurrences_ci(text: &str, needle: &str) -> String {
    if needle.is_empty() {
        return text.to_string();
    }
    let lower_text = text.to_ascii_lowercase();
    let lower_needle = needle.to_ascii_lowercase();

    let mut out = String::with_capacity(text.len());
    let mut idx = 0;
    let mut seen = false;
    while let Some(rel) = lower_text[idx..].find(&lower_needle) {
        let abs = idx + rel;
        out.push_str(&text[idx..abs]);
        let after = abs + needle.len();
        if !seen {
            // Keep first occurrence verbatim.
            out.push_str(&text[abs..after]);
            seen = true;
        } else {
            // Drop the occurrence; also eat one trailing separator if present.
            let next = text[after..]
                .chars()
                .next()
                .map(|character| (character, character.len_utf8()))
                .unwrap_or((' ', 0));
            if matches!(next.0, ',' | ' ' | '—' | '-' | '|' | '·' | ':') {
                idx = after + next.1;
                continue;
            }
        }
        idx = after;
    }
    out.push_str(&text[idx..]);
    out
}

fn normalize_whitespace(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed
        .replace(" ,", ",")
        .replace(" .", ".")
        .replace(" ;", ";")
        .replace(" —", "")
        .replace(" -—", "")
        .replace("— ,", ",")
}

fn strip_leading_punct(text: &str) -> String {
    text.trim_start_matches(|character: char| {
        matches!(character, ',' | '—' | '-' | '|' | '·' | ':' | ';') || character.is_whitespace()
    })
    .to_string()
}

pub(super) fn capitalize_first(value: &str) -> String {
    let mut characters = value.chars();
    match characters.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().chain(characters).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_fluff_drops_repeated_app_name() {
        let out = strip_fluff(
            "Google Chrome — Google Chrome news article on AI",
            "Google Chrome",
            "",
            "",
        );
        assert!(out.contains("Google Chrome"));
        let count = out.matches("Google Chrome").count();
        assert_eq!(count, 1, "expected one mention, got: {out}");
    }

    #[test]
    fn strip_fluff_drops_the_user_prefix() {
        let out = strip_fluff("the user is viewing a news article", "", "", "");
        assert!(out.to_lowercase().contains("news article"));
        assert!(!out.to_lowercase().starts_with("the user"));
    }

    #[test]
    fn strip_fluff_drops_screen_shows_prefix() {
        let out = strip_fluff("The screen shows a Figma file with components", "Figma", "", "");
        assert!(!out.to_lowercase().starts_with("the screen shows"));
        assert!(out.to_lowercase().contains("figma file"));
    }

    #[test]
    fn strip_fluff_collapses_adjacent_duplicates() {
        let out = collapse_adjacent_duplicates("News - News - article about AI");
        assert_eq!(out, "News - article about AI");
    }

    #[test]
    fn strip_fluff_preserves_first_mention() {
        let out = strip_fluff(
            "You watched a cricket match. Cricket fans were excited about the cricket result.",
            "",
            "Cricket",
            "",
        );
        let count = out.to_lowercase().matches("cricket").count();
        assert!(count >= 1 && count <= 2, "got count={count} text={out}");
    }

    #[test]
    fn strip_fluff_empty_input_returns_empty() {
        assert_eq!(strip_fluff("", "Chrome", "", ""), "");
        assert_eq!(strip_fluff("   ", "Chrome", "", "").trim(), "");
    }

    #[test]
    fn strip_fluff_no_op_when_no_redundancy() {
        let input = "You reviewed the auth refactor PR for security correctness.";
        let out = strip_fluff(input, "GitHub", "", "github.com");
        assert_eq!(out, input);
    }
}
