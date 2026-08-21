//! `make bench` entry point. Usage:
//!   fndr-bench --corpus <dir> [--out <metrics.json>] [--baseline <baseline.json>]
//! Exits nonzero on a quality regression against the baseline, so CI and
//! humans get the same gate.

use std::path::PathBuf;
use std::process::ExitCode;

use fndr_bench::{BenchReport, Corpus, compare_to_baseline, run_fts_baseline};

fn main() -> ExitCode {
    let mut corpus_dir: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut baseline: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--corpus" => corpus_dir = args.next().map(PathBuf::from),
            "--out" => out = args.next().map(PathBuf::from),
            "--baseline" => baseline = args.next().map(PathBuf::from),
            other => {
                eprintln!("unknown arg: {other}");
                return ExitCode::from(2);
            }
        }
    }
    let Some(corpus_dir) = corpus_dir else {
        eprintln!("usage: fndr-bench --corpus <dir> [--out <file>] [--baseline <file>]");
        return ExitCode::from(2);
    };

    let corpus = match Corpus::load(&corpus_dir) {
        Ok(corpus) => corpus,
        Err(e) => {
            eprintln!("corpus load failed: {e}");
            return ExitCode::from(1);
        }
    };
    let report = match run_fts_baseline(&corpus) {
        Ok(report) => report,
        Err(e) => {
            eprintln!("bench run failed: {e}");
            return ExitCode::from(1);
        }
    };

    println!(
        "route={} corpus={} records={} queries={}",
        report.route, report.corpus, report.n_records, report.n_queries
    );
    println!(
        "Recall@5={:.4} MRR@10={:.4} latency p50={:.2}ms p95={:.2}ms",
        report.recall_at_5, report.mrr_at_10, report.latency_ms_p50, report.latency_ms_p95
    );

    if let Some(out) = out {
        if let Some(parent) = out.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_string_pretty(&report) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&out, json) {
                    eprintln!("failed writing {}: {e}", out.display());
                    return ExitCode::from(1);
                }
                println!("wrote {}", out.display());
            }
            Err(e) => {
                eprintln!("serialize failed: {e}");
                return ExitCode::from(1);
            }
        }
    }

    if let Some(baseline_path) = baseline {
        let baseline: BenchReport = match std::fs::read_to_string(&baseline_path)
            .map_err(|e| e.to_string())
            .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
        {
            Ok(baseline) => baseline,
            Err(e) => {
                eprintln!("baseline load failed ({}): {e}", baseline_path.display());
                return ExitCode::from(1);
            }
        };
        let regressions = compare_to_baseline(&report, &baseline);
        if regressions.is_empty() {
            println!("baseline check: ok ({})", baseline_path.display());
        } else {
            for regression in &regressions {
                eprintln!("REGRESSION: {regression}");
            }
            return ExitCode::from(1);
        }
    }

    ExitCode::SUCCESS
}
