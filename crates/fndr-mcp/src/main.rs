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
}

impl LaunchOptions {
    fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut store_path = None;
        let mut port = 0;
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
                "--help" | "-h" => return Err(String::new()),
                other => return Err(format!("unknown option: {other}")),
            }
        }

        let store_path = store_path.ok_or_else(|| "--store is required".to_owned())?;
        Ok(Self { store_path, port })
    }
}

fn print_usage() {
    eprintln!("usage: fndr-mcp --store PATH [--port PORT]");
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
    let token = generate_token();
    let runtime = tokio::runtime::Runtime::new().expect("Tokio runtime must initialize");

    runtime.block_on(async move {
        let (addr, handle) = serve_loopback(FndrMcpServer::new(store), token.clone(), options.port)
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
}
