//! `fndr-vault`: the owner's copy-it-out and move-it commands (T-209).
//!
//! Naming: the roadmap writes these as `fndr backup` / `fndr export`. This
//! workspace has no umbrella `fndr` binary (fndr-mcp, fndr-bench, and
//! fndr-shell are each their own command), so the capability ships beside the
//! crate that owns the vault and becomes `fndr vault ...` if an umbrella CLI
//! ever lands. Argument handling follows the same hand-rolled style as
//! `fndr-mcp`; no CLI framework enters the workspace for three subcommands.
//!
//! Every path here is a local filesystem path. There is no remote target and
//! there never will be one: FNDR has no egress (ADR-004).

use std::path::PathBuf;
use std::process::ExitCode;

use fndr_store::{RestorePolicy, VaultLayout, backup_vault, export_vault, restore_vault};

#[derive(Debug, PartialEq, Eq)]
enum Command {
    Backup {
        data_dir: PathBuf,
        out: PathBuf,
    },
    Export {
        data_dir: PathBuf,
        out: PathBuf,
    },
    Restore {
        from: PathBuf,
        data_dir: PathBuf,
        policy: RestorePolicy,
    },
}

/// An empty error string means "the operator asked for help", matching
/// `fndr-mcp`'s convention of exiting 0 for `--help` and 2 for a real usage
/// error.
type ParseResult = Result<Command, String>;

impl Command {
    fn parse(args: impl IntoIterator<Item = String>) -> ParseResult {
        let mut args = args.into_iter();
        let command = args.next().ok_or_else(String::new)?;
        let mut data_dir: Option<PathBuf> = None;
        let mut out: Option<PathBuf> = None;
        let mut from: Option<PathBuf> = None;
        let mut overwrite = false;

        if command == "--help" || command == "-h" {
            return Err(String::new());
        }
        while let Some(arg) = args.next() {
            let mut value = |flag: &str| -> Result<PathBuf, String> {
                args.next()
                    .map(PathBuf::from)
                    .ok_or_else(|| format!("{flag} requires a path"))
            };
            match arg.as_str() {
                "--data-dir" => data_dir = Some(value("--data-dir")?),
                "--out" => out = Some(value("--out")?),
                "--from" => from = Some(value("--from")?),
                "--overwrite" => overwrite = true,
                "--help" | "-h" => return Err(String::new()),
                other => return Err(format!("unknown option: {other}")),
            }
        }

        match command.as_str() {
            "backup" | "export" => {
                if from.is_some() {
                    return Err(format!("--from is only valid for restore, not {command}"));
                }
                if overwrite {
                    return Err(format!(
                        "{command} never overwrites; choose a destination that does not exist"
                    ));
                }
                let data_dir = data_dir.ok_or_else(|| format!("{command} requires --data-dir"))?;
                let out = out.ok_or_else(|| format!("{command} requires --out"))?;
                Ok(if command == "backup" {
                    Self::Backup { data_dir, out }
                } else {
                    Self::Export { data_dir, out }
                })
            }
            "restore" => {
                if out.is_some() {
                    return Err("restore writes to --data-dir, not --out".to_owned());
                }
                Ok(Self::Restore {
                    from: from.ok_or_else(|| "restore requires --from".to_owned())?,
                    data_dir: data_dir
                        .ok_or_else(|| "restore requires --data-dir".to_owned())?,
                    policy: if overwrite {
                        RestorePolicy::ReplaceExistingVault
                    } else {
                        RestorePolicy::RefuseIfVaultExists
                    },
                })
            }
            other => Err(format!("unknown command: {other}")),
        }
    }
}

fn print_usage() {
    eprintln!(
        "usage:
  fndr-vault backup  --data-dir <vault dir> --out <new backup dir>
  fndr-vault export  --data-dir <vault dir> --out <new export dir>
  fndr-vault restore --from <backup dir> --data-dir <vault dir> [--overwrite]

backup   a restorable snapshot of SQLite truth plus config, safe to take
         while capture is running. The derived Lance index is not included:
         it rebuilds from truth (ADR-002).
export   a readable, portable JSONL dump of your memory. Not a restore
         source; use backup for that.
restore  install a backup into a vault directory. Refuses to replace an
         existing vault unless --overwrite is given. Stop FNDR first: the
         running app owns the data directory."
    );
}

fn main() -> ExitCode {
    let command = match Command::parse(std::env::args().skip(1)) {
        Ok(command) => command,
        Err(error) => {
            if !error.is_empty() {
                eprintln!("fndr-vault: {error}");
            }
            print_usage();
            return ExitCode::from(if error.is_empty() { 0 } else { 2 });
        }
    };
    let layout = VaultLayout::default();

    match command {
        Command::Backup { data_dir, out } => match backup_vault(&data_dir, &out, &layout) {
            Ok(report) => {
                println!("Backup written to {}", report.destination.display());
                println!(
                    "  {} records, {} chunks, schema v{}, {} bytes (integrity check: ok)",
                    report.records, report.chunks, report.schema_version, report.database_bytes
                );
                println!(
                    "  config files: {}",
                    if report.config_files.is_empty() {
                        "none found in the data directory".to_owned()
                    } else {
                        report.config_files.join(", ")
                    }
                );
                for skipped in &report.skipped {
                    println!("  not included: {} ({})", skipped.name, skipped.reason.as_str());
                }
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("fndr-vault backup failed: {error}");
                ExitCode::from(1)
            }
        },
        Command::Export { data_dir, out } => match export_vault(&data_dir, &out, &layout) {
            Ok(report) => {
                println!("Export written to {}", report.destination.display());
                println!(
                    "  {} records, {} chunks, {} decisions, {} bytes of capture text in the clear",
                    report.records, report.chunks, report.decisions, report.text_bytes
                );
                println!("  read it with jq; see README.md in that directory");
                println!("  this is not a restore source; use `fndr-vault backup` to move a vault");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("fndr-vault export failed: {error}");
                ExitCode::from(1)
            }
        },
        Command::Restore {
            from,
            data_dir,
            policy,
        } => match restore_vault(&from, &data_dir, policy, &layout) {
            Ok(report) => {
                println!("Restored {}", report.database.display());
                println!(
                    "  {} records, {} chunks, schema v{} (integrity check: ok)",
                    report.records, report.chunks, report.schema_version
                );
                if report.replaced_vault {
                    println!("  an existing vault was replaced at your explicit request");
                }
                if !report.config_files.is_empty() {
                    println!("  config files: {}", report.config_files.join(", "));
                }
                if let Some(superseded) = &report.superseded_index {
                    println!(
                        "  the previous derived index was moved to {} (delete it after a rebuild)",
                        superseded.display()
                    );
                }
                if report.index_rebuild_required {
                    println!(
                        "  next: keyword search works now; the vector route stays empty until the"
                    );
                    println!(
                        "  Lance index is rebuilt from this truth (fndr-store LanceWriter::rebuild)"
                    );
                }
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("fndr-vault restore failed: {error}");
                ExitCode::from(1)
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::Command;
    use fndr_store::RestorePolicy;
    use std::path::PathBuf;

    fn parse(args: &[&str]) -> Result<Command, String> {
        Command::parse(args.iter().map(|arg| (*arg).to_owned()))
    }

    #[test]
    fn parses_backup_and_export() {
        assert_eq!(
            parse(&["backup", "--data-dir", "/tmp/vault", "--out", "/tmp/b1"]),
            Ok(Command::Backup {
                data_dir: PathBuf::from("/tmp/vault"),
                out: PathBuf::from("/tmp/b1"),
            })
        );
        assert_eq!(
            parse(&["export", "--data-dir", "/tmp/vault", "--out", "/tmp/e1"]),
            Ok(Command::Export {
                data_dir: PathBuf::from("/tmp/vault"),
                out: PathBuf::from("/tmp/e1"),
            })
        );
    }

    #[test]
    fn restore_defaults_to_refusing_an_existing_vault() {
        assert_eq!(
            parse(&["restore", "--from", "/tmp/b1", "--data-dir", "/tmp/vault"]),
            Ok(Command::Restore {
                from: PathBuf::from("/tmp/b1"),
                data_dir: PathBuf::from("/tmp/vault"),
                policy: RestorePolicy::RefuseIfVaultExists,
            })
        );
        assert_eq!(
            parse(&[
                "restore",
                "--from",
                "/tmp/b1",
                "--data-dir",
                "/tmp/vault",
                "--overwrite"
            ]),
            Ok(Command::Restore {
                from: PathBuf::from("/tmp/b1"),
                data_dir: PathBuf::from("/tmp/vault"),
                policy: RestorePolicy::ReplaceExistingVault,
            })
        );
    }

    #[test]
    fn missing_and_crossed_arguments_are_usage_errors() {
        assert_eq!(
            parse(&["backup", "--data-dir", "/tmp/vault"]),
            Err("backup requires --out".to_owned())
        );
        assert_eq!(
            parse(&["restore", "--data-dir", "/tmp/vault"]),
            Err("restore requires --from".to_owned())
        );
        assert_eq!(
            parse(&["restore", "--from", "/tmp/b1", "--out", "/tmp/vault"]),
            Err("restore writes to --data-dir, not --out".to_owned())
        );
        assert_eq!(
            parse(&["backup", "--data-dir", "/tmp/v", "--out", "/tmp/b", "--overwrite"]),
            Err("backup never overwrites; choose a destination that does not exist".to_owned())
        );
        assert_eq!(
            parse(&["archive", "--data-dir", "/tmp/v"]),
            Err("unknown command: archive".to_owned())
        );
        assert_eq!(
            parse(&["backup", "--data-dir"]),
            Err("--data-dir requires a path".to_owned())
        );
        assert_eq!(parse(&[]), Err(String::new()));
        assert_eq!(parse(&["--help"]), Err(String::new()));
    }
}
