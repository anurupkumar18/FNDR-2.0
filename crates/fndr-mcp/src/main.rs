//! Durable local MCP host for an existing FNDR store.
//!
//! Capture and MCP deliberately remain separate processes in this alpha cut:
//! the desktop host owns capture lifecycle, while this command exposes the
//! exact same SQLite store to an owner-configured MCP client over loopback.

use std::path::PathBuf;

use fndr_mcp::{FndrMcpServer, generate_token, serve_loopback};
use fndr_store::Store;

#[derive(Debug, PartialEq, Eq)]
struct LaunchOptions {
    store_path: PathBuf,
    port: u16,
    /// When given together with index_dir, fndr.search also queries the
    /// real vector route. Absent by default: an alpha demo host with no
    /// model still serves keyword search exactly as before.
    model_path: Option<PathBuf>,
    index_dir: Option<PathBuf>,
}

impl LaunchOptions {
    fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut store_path = None;
        let mut port = 0;
        let mut model_path = None;
        let mut index_dir = None;
        let mut args = args.into_iter();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--store" => {
                    let value = args
                        .next()
                        .ok_or_else(|| "--store requires a path".to_owned())?;
                    store_path = Some(PathBuf::from(value));
                }
                "--port" => {
                    let value = args
                        .next()
                        .ok_or_else(|| "--port requires a number".to_owned())?;
                    port = value
                        .parse::<u16>()
                        .map_err(|_| "--port must be between 0 and 65535".to_owned())?;
                }
                "--model" => {
                    let value = args
                        .next()
                        .ok_or_else(|| "--model requires a path".to_owned())?;
                    model_path = Some(PathBuf::from(value));
                }
                "--index-dir" => {
                    let value = args
                        .next()
                        .ok_or_else(|| "--index-dir requires a path".to_owned())?;
                    index_dir = Some(PathBuf::from(value));
                }
                "--help" | "-h" => return Err(String::new()),
                other => return Err(format!("unknown option: {other}")),
            }
        }

        let store_path = store_path.ok_or_else(|| "--store is required".to_owned())?;
        if model_path.is_some() != index_dir.is_some() {
            return Err("--model and --index-dir must be given together".to_owned());
        }
        Ok(Self {
            store_path,
            port,
            model_path,
            index_dir,
        })
    }
}

fn print_usage() {
    eprintln!("usage: fndr-mcp --store PATH [--port PORT] [--model PATH --index-dir PATH]");
}

fn main() {
    let options = LaunchOptions::parse(std::env::args().skip(1)).unwrap_or_else(|error| {
        if !error.is_empty() {
            eprintln!("FNDR MCP launch options: {error}");
        }
        print_usage();
        std::process::exit(if error.is_empty() { 0 } else { 2 });
    });
    let store = Store::open(&options.store_path).unwrap_or_else(|error| {
        eprintln!(
            "FNDR MCP could not open {}: {error}",
            options.store_path.display()
        );
        std::process::exit(1);
    });

    let server = match (options.model_path, options.index_dir) {
        (Some(model_path), Some(index_dir)) => {
            let spec = fndr_inference::CHUNK_EMBEDDING_V1;
            let embedder =
                fndr_inference::GgufEmbedder::load(&model_path, spec).unwrap_or_else(|error| {
                    eprintln!("FNDR MCP could not load {}: {error}", model_path.display());
                    std::process::exit(1);
                });
            println!("Vector route enabled: {}", model_path.display());
            FndrMcpServer::with_vector_route(
                store,
                fndr_privacy::Blocklist::default(),
                std::sync::Arc::new(embedder),
                index_dir,
            )
        }
        (None, None) => {
            println!("Vector route disabled (no --model given); fndr.search is keyword-only.");
            FndrMcpServer::new(store)
        }
        _ => unreachable!("LaunchOptions::parse already rejected a partial pair"),
    };

    let token = generate_token();
    let runtime = tokio::runtime::Runtime::new().expect("Tokio runtime must initialize");

    runtime.block_on(async move {
        let (addr, handle) = serve_loopback(server, token.clone(), options.port)
            .await
            .unwrap_or_else(|error| {
                eprintln!("FNDR MCP could not bind: {error}");
                std::process::exit(1);
            });
        println!("FNDR MCP serving at http://{addr}/mcp");
        println!("Authorization: Bearer {token}");
        println!("\nAdd to Claude Code:");
        println!(
            "  claude mcp add fndr --transport http http://{addr}/mcp --header \"Authorization: Bearer {token}\""
        );
        println!("\nCtrl-C to stop. The bearer token is process-local; do not record or commit it.");

        let _ = tokio::signal::ctrl_c().await;
        handle.abort();
    });
}

#[cfg(test)]
mod tests {
    use super::LaunchOptions;
    use std::path::PathBuf;

    #[test]
    fn parses_required_store_and_optional_port() {
        assert_eq!(
            LaunchOptions::parse(
                ["--store", "/tmp/fndr.sqlite3", "--port", "4711"].map(str::to_owned)
            ),
            Ok(LaunchOptions {
                store_path: PathBuf::from("/tmp/fndr.sqlite3"),
                port: 4711,
                model_path: None,
                index_dir: None,
            })
        );
    }

    #[test]
    fn refuses_missing_store_and_invalid_port() {
        assert_eq!(
            LaunchOptions::parse(Vec::new()),
            Err("--store is required".into())
        );
        assert_eq!(
            LaunchOptions::parse(
                ["--store", "memory.sqlite3", "--port", "nope"].map(str::to_owned)
            ),
            Err("--port must be between 0 and 65535".into())
        );
    }

    #[test]
    fn parses_optional_model_and_index_dir() {
        assert_eq!(
            LaunchOptions::parse(
                [
                    "--store",
                    "/tmp/fndr.sqlite3",
                    "--model",
                    "/tmp/model.gguf",
                    "--index-dir",
                    "/tmp/index",
                ]
                .map(str::to_owned)
            ),
            Ok(LaunchOptions {
                store_path: PathBuf::from("/tmp/fndr.sqlite3"),
                port: 0,
                model_path: Some(PathBuf::from("/tmp/model.gguf")),
                index_dir: Some(PathBuf::from("/tmp/index")),
            })
        );
    }

    #[test]
    fn model_and_index_dir_default_to_none() {
        let opts =
            LaunchOptions::parse(["--store", "/tmp/fndr.sqlite3"].map(str::to_owned)).unwrap();
        assert_eq!(opts.model_path, None);
        assert_eq!(opts.index_dir, None);
    }

    #[test]
    fn model_without_index_dir_is_rejected() {
        assert_eq!(
            LaunchOptions::parse(
                ["--store", "/tmp/fndr.sqlite3", "--model", "/tmp/model.gguf"].map(str::to_owned)
            ),
            Err("--model and --index-dir must be given together".to_owned())
        );
    }

    #[test]
    fn index_dir_without_model_is_rejected() {
        assert_eq!(
            LaunchOptions::parse(
                ["--store", "/tmp/fndr.sqlite3", "--index-dir", "/tmp/index"].map(str::to_owned)
            ),
            Err("--model and --index-dir must be given together".to_owned())
        );
    }
}
