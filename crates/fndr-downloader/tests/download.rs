//! Download semantics against a local one-shot HTTP server: full transfer,
//! resume via Range, checksum-mismatch deletion, and on-disk verification.
//! No internet involved; loopback only.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;

use fndr_downloader::{DownloadError, download_verified, verify_file};
use sha2::{Digest, Sha256};

const PAYLOAD_LEN: usize = 200_000;

fn payload() -> Vec<u8> {
    (0..PAYLOAD_LEN).map(|i| (i % 251) as u8).collect()
}

fn sha_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Serve `body` for exactly `requests` HTTP requests, honoring Range.
/// Optionally corrupt the payload to exercise the checksum path.
fn serve(body: Vec<u8>, requests: usize, corrupt: bool) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for _ in 0..requests {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let n = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..n]).into_owned();
            let range_start = request
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("range: bytes=")
                        .map(String::from)
                })
                .and_then(|r| r.trim_end_matches('-').parse::<usize>().ok());

            let mut data = body.clone();
            if corrupt {
                data[0] ^= 0xFF;
            }
            match range_start {
                Some(start) => {
                    let slice = &data[start..];
                    let header = format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{}/{}\r\nConnection: close\r\n\r\n",
                        slice.len(),
                        data.len() - 1,
                        data.len()
                    );
                    stream.write_all(header.as_bytes()).unwrap();
                    stream.write_all(slice).unwrap();
                }
                None => {
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        data.len()
                    );
                    stream.write_all(header.as_bytes()).unwrap();
                    stream.write_all(&data).unwrap();
                }
            }
        }
    });
    format!("http://{addr}/model.gguf")
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fndr-dl-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn full_download_verifies_and_lands_atomically() {
    let body = payload();
    let sha = sha_hex(&body);
    let url = serve(body.clone(), 1, false);
    let dir = scratch("full");
    let dest = dir.join("model.gguf");

    download_verified(&url, &dest, &sha, body.len() as u64).unwrap();
    assert!(dest.is_file());
    assert!(!dir.join("model.part").exists(), "no leftover part file");
    verify_file(&dest, &sha, body.len() as u64).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn resume_completes_from_partial_file() {
    let body = payload();
    let sha = sha_hex(&body);
    let url = serve(body.clone(), 1, false);
    let dir = scratch("resume");
    let dest = dir.join("model.gguf");

    // Simulate an interrupted transfer: first half already on disk.
    std::fs::write(dest.with_extension("part"), &body[..PAYLOAD_LEN / 2]).unwrap();
    download_verified(&url, &dest, &sha, body.len() as u64).unwrap();
    verify_file(&dest, &sha, body.len() as u64).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn checksum_mismatch_deletes_artifact_and_fails_loudly() {
    let body = payload();
    let sha = sha_hex(&body);
    let url = serve(body.clone(), 1, true);
    let dir = scratch("corrupt");
    let dest = dir.join("model.gguf");

    match download_verified(&url, &dest, &sha, body.len() as u64) {
        Err(DownloadError::ChecksumMismatch { .. }) => {}
        other => panic!("expected ChecksumMismatch, got {other:?}"),
    }
    assert!(!dest.exists(), "nothing lands at the final path");
    assert!(!dest.with_extension("part").exists(), "artifact deleted");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn verify_file_rejects_wrong_size_and_hash() {
    let dir = scratch("verify");
    let path = dir.join("f.bin");
    std::fs::write(&path, b"hello world").unwrap();
    let good = sha_hex(b"hello world");

    verify_file(&path, &good, 11).unwrap();
    assert!(matches!(
        verify_file(&path, &good, 12),
        Err(DownloadError::SizeMismatch { .. })
    ));
    let flipped = if good.starts_with('0') { "f" } else { "0" };
    let bad = format!("{flipped}{}", good.get(1..).unwrap());
    assert!(matches!(
        verify_file(&path, &bad, 11),
        Err(DownloadError::ChecksumMismatch { .. })
    ));
    let _ = std::fs::remove_dir_all(&dir);
}
