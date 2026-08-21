//! Model and asset downloads. One of the only two crates allowed HTTP egress (ADR-004).
//!
//! T-401 semantics: downloads stream to a `.part` file (resumable via HTTP
//! Range), then the completed file's SHA-256 is verified against the pinned
//! registry value before an atomic rename into place. A checksum mismatch
//! deletes the artifact and fails loudly; nothing unverified ever sits at
//! the final path.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("http error: {0}")]
    Http(#[from] Box<ureq::Error>),
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("server ignored the resume request (no 206); restarting is required")]
    ResumeNotSupported,
    #[error(
        "checksum mismatch for {filename}: expected {expected}, got {actual}; artifact deleted"
    )]
    ChecksumMismatch {
        filename: String,
        expected: String,
        actual: String,
    },
    #[error("size mismatch for {filename}: expected {expected} bytes, got {actual}")]
    SizeMismatch {
        filename: String,
        expected: u64,
        actual: u64,
    },
}

fn sha256_of(path: &Path) -> Result<String, std::io::Error> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Download `url` to `dest`, resuming a partial transfer when possible, and
/// verify size and SHA-256 before the file appears at `dest`.
pub fn download_verified(
    url: &str,
    dest: &Path,
    expected_sha256: &str,
    expected_size: u64,
) -> Result<(), DownloadError> {
    let part = dest.with_extension("part");
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let already = part.metadata().map(|m| m.len()).unwrap_or(0);
    let mut file;
    let mut response = if already > 0 && already < expected_size {
        tracing::info!(url, resumed_from = already, "resuming download");
        let response = ureq::get(url)
            .header("Range", &format!("bytes={already}-"))
            .call()
            .map_err(Box::new)?;
        if response.status() != 206 {
            // Server will not resume: start over rather than corrupt.
            std::fs::remove_file(&part)?;
            return Err(DownloadError::ResumeNotSupported);
        }
        file = std::fs::OpenOptions::new().write(true).open(&part)?;
        file.seek(SeekFrom::End(0))?;
        response
    } else {
        let response = ureq::get(url).call().map_err(Box::new)?;
        file = std::fs::File::create(&part)?;
        response
    };

    let mut reader = response.body_mut().as_reader();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
    }
    file.sync_all()?;
    drop(file);

    let filename = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let actual_size = part.metadata()?.len();
    if actual_size != expected_size {
        std::fs::remove_file(&part)?;
        return Err(DownloadError::SizeMismatch {
            filename,
            expected: expected_size,
            actual: actual_size,
        });
    }
    let actual = sha256_of(&part)?;
    if !actual.eq_ignore_ascii_case(expected_sha256) {
        // AC: mismatch deletes the artifact and fails loudly.
        std::fs::remove_file(&part)?;
        return Err(DownloadError::ChecksumMismatch {
            filename,
            expected: expected_sha256.to_string(),
            actual,
        });
    }

    std::fs::rename(&part, dest)?;
    Ok(())
}

/// Verify an artifact already on disk (registry preflight and doctor use).
pub fn verify_file(
    path: &Path,
    expected_sha256: &str,
    expected_size: u64,
) -> Result<(), DownloadError> {
    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let actual_size = path.metadata()?.len();
    if actual_size != expected_size {
        return Err(DownloadError::SizeMismatch {
            filename,
            expected: expected_size,
            actual: actual_size,
        });
    }
    let actual = sha256_of(path)?;
    if !actual.eq_ignore_ascii_case(expected_sha256) {
        return Err(DownloadError::ChecksumMismatch {
            filename,
            expected: expected_sha256.to_string(),
            actual,
        });
    }
    Ok(())
}
