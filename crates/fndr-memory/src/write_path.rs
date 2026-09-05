//! The narrow handoff from assembled capture text to SQLite truth.
//!
//! This is deliberately a write seam, not a second capture loop or retrieval
//! path: the scheduler owns the genuine pre-OCR decision, while this boundary
//! rechecks policy and makes the final persistence decision visible to callers.

use fndr_privacy::{
    Blocklist, SafetyContext, SafetyDecision, SafetyReason, evaluate, redact_secret_lines,
};
use fndr_store::{NewChunk, NewRecord, Store, StoreError};

/// The already-assembled capture fields that may become one SQLite record and
/// one chunk. IDs are supplied by the pipeline so this seam does not invent an
/// identity scheme beside the future session/record contracts.
#[derive(Debug, Clone, Copy)]
pub struct CaptureForPersistence<'a> {
    pub record_id: &'a str,
    pub session_id: &'a str,
    pub chunk_id: &'a str,
    pub source: &'a str,
    pub app_name: &'a str,
    pub bundle_id: Option<&'a str>,
    pub url: Option<&'a str>,
    pub window_title: &'a str,
    pub ocr_text: &'a str,
    pub captured_at_ms: i64,
    pub created_at_ms: i64,
}

/// The observable result of the last line of defense before persistence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersistCaptureOutcome {
    Stored {
        record_id: String,
        redaction_count: usize,
    },
    Skipped {
        reason: SafetyReason,
    },
}

/// Recheck policy immediately before writing a capture to SQLite.
///
/// The caller must run the first policy evaluation before OCR. This boundary
/// repeats metadata checks so a future caller cannot persist a blocked record,
/// then either redacts OCR secret lines or writes the record and its chunk in
/// `Store`'s atomic transaction.
pub fn persist_capture(
    store: &mut Store,
    capture: CaptureForPersistence<'_>,
    blocklist: &Blocklist,
) -> Result<PersistCaptureOutcome, StoreError> {
    let metadata = safety_context(capture, None);
    if let SafetyDecision::SkipStorage(reason) = evaluate(metadata, blocklist) {
        return Ok(PersistCaptureOutcome::Skipped { reason });
    }

    let (text, redaction_count) =
        match evaluate(safety_context(capture, Some(capture.ocr_text)), blocklist) {
            SafetyDecision::Allow => (capture.ocr_text.to_owned(), 0),
            SafetyDecision::Redact(_) => redact_secret_lines(capture.ocr_text),
            SafetyDecision::SkipStorage(reason) => {
                return Ok(PersistCaptureOutcome::Skipped { reason });
            }
        };

    store.insert_capture(
        &NewRecord {
            id: capture.record_id.to_owned(),
            session_id: capture.session_id.to_owned(),
            source: capture.source.to_owned(),
            app_name: capture.app_name.to_owned(),
            window_title: capture.window_title.to_owned(),
            captured_at_ms: capture.captured_at_ms,
            created_at_ms: capture.created_at_ms,
        },
        &[NewChunk {
            id: capture.chunk_id.to_owned(),
            ord: 0,
            text,
        }],
    )?;

    Ok(PersistCaptureOutcome::Stored {
        record_id: capture.record_id.to_owned(),
        redaction_count,
    })
}

fn safety_context<'a>(
    capture: CaptureForPersistence<'a>,
    ocr_text: Option<&'a str>,
) -> SafetyContext<'a> {
    SafetyContext {
        app_name: Some(capture.app_name),
        bundle_id: capture.bundle_id,
        url: capture.url,
        window_title: Some(capture.window_title),
        ocr_text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capture<'a>(
        app_name: &'a str,
        url: Option<&'a str>,
        ocr_text: &'a str,
    ) -> CaptureForPersistence<'a> {
        CaptureForPersistence {
            record_id: "record-1",
            session_id: "session-1",
            chunk_id: "chunk-1",
            source: "screen",
            app_name,
            bundle_id: Some("com.example.app"),
            url,
            window_title: "Project notes",
            ocr_text,
            captured_at_ms: 1_000,
            created_at_ms: 1_000,
        }
    }

    #[test]
    fn allowed_capture_becomes_one_pending_chunk() {
        let mut store = Store::open_in_memory().unwrap();
        let outcome = persist_capture(
            &mut store,
            capture(
                "VS Code",
                Some("https://docs.example.com/fndr"),
                "alpha notes",
            ),
            &Blocklist::default(),
        )
        .unwrap();

        assert_eq!(
            outcome,
            PersistCaptureOutcome::Stored {
                record_id: "record-1".to_owned(),
                redaction_count: 0,
            }
        );
        let pending = store.pending_chunks(10).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].record_id, "record-1");
        assert_eq!(pending[0].text, "alpha notes");
    }

    #[test]
    fn secret_text_is_redacted_before_the_real_store_write() {
        let mut store = Store::open_in_memory().unwrap();
        let secret = "notes\napi_key: do-not-store-me\nnext task";
        let outcome = persist_capture(
            &mut store,
            capture("VS Code", Some("https://docs.example.com/fndr"), secret),
            &Blocklist::default(),
        )
        .unwrap();

        assert_eq!(
            outcome,
            PersistCaptureOutcome::Stored {
                record_id: "record-1".to_owned(),
                redaction_count: 1,
            }
        );
        let stored = &store.pending_chunks(10).unwrap()[0].text;
        assert!(!stored.contains("do-not-store-me"));
        assert_eq!(stored, "notes\n[REDACTED: secret pattern]\nnext task");
    }

    #[test]
    fn sensitive_metadata_never_reaches_the_real_store_write() {
        let mut store = Store::open_in_memory().unwrap();
        let outcome = persist_capture(
            &mut store,
            capture("1Password", None, "password vault contents"),
            &Blocklist::default(),
        )
        .unwrap();

        assert_eq!(
            outcome,
            PersistCaptureOutcome::Skipped {
                reason: SafetyReason::PasswordManager,
            }
        );
        assert!(store.pending_chunks(10).unwrap().is_empty());
    }

    #[test]
    fn owner_domain_blocklist_prevents_the_real_store_write() {
        let mut store = Store::open_in_memory().unwrap();
        let blocklist = Blocklist::new::<&str>(&[], &["example.com"]);
        let outcome = persist_capture(
            &mut store,
            capture(
                "Browser",
                Some("https://docs.example.com/fndr"),
                "private plan",
            ),
            &blocklist,
        )
        .unwrap();

        assert_eq!(
            outcome,
            PersistCaptureOutcome::Skipped {
                reason: SafetyReason::UserBlocklist,
            }
        );
        assert!(store.pending_chunks(10).unwrap().is_empty());
    }
}
