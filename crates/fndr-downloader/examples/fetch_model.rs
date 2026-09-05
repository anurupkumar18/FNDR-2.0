//! Dev bootstrap utility: fetch one pinned model from the `fndr-inference`
//! registry into `models/` (repo-root, gitignored) through the real
//! production `download_verified` path (T-401). Useful for anyone setting up
//! a local dev environment before real embedder/inference work (T-402+)
//! needs an actual model file on disk.
//!
//! Usage: `cargo run -p fndr-downloader --example fetch_model -- [model-id]`
//! Defaults to the chunk-embedding contract's model
//! (`fndr_inference::CHUNK_EMBEDDING_V1.model_id`) when no id is given.

use fndr_downloader::download_verified;
use fndr_inference::{CHUNK_EMBEDDING_V1, artifact};

fn main() {
    let id = std::env::args()
        .nth(1)
        .unwrap_or_else(|| CHUNK_EMBEDDING_V1.model_id.to_owned());
    let Some(m) = artifact(&id) else {
        eprintln!("unknown registry id: {id}");
        std::process::exit(1);
    };
    let dest = std::path::Path::new("models").join(m.filename);
    if dest.is_file() && fndr_downloader::verify_file(&dest, m.sha256, m.size_bytes).is_ok() {
        println!("already present and verified: {}", dest.display());
        return;
    }
    println!(
        "fetching {} ({} bytes) -> {}",
        m.url,
        m.size_bytes,
        dest.display()
    );
    match download_verified(m.url, &dest, m.sha256, m.size_bytes) {
        Ok(()) => println!("ok: verified and landed at {}", dest.display()),
        Err(e) => {
            eprintln!("failed: {e}");
            std::process::exit(1);
        }
    }
}
