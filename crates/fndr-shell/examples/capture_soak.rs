//! T-310's missing instrument: a real, bounded capture soak.
//!
//! Nothing in the product owns `start_real_capture_worker` yet — T-901's
//! desktop lifecycle is unstarted — so the full composition (ScreenCaptureKit
//! → Vision OCR → privacy gate → SQLite → queued embedder → Lance) has never
//! run for longer than a single test tick. This runner owns it for a bounded
//! number of minutes and reports what happened, which is exactly what T-310's
//! soak needs and what a desktop bootstrap will later do for real.
//!
//! It is a CLI, not a desktop lifecycle. Do not read a green soak as
//! "auto-capture works"; read it as "the composition survives N minutes".
//!
//! ```sh
//! cargo run -p fndr-shell --example capture_soak -- \
//!   --minutes 5 \
//!   --store /tmp/fndr-soak.sqlite3 \
//!   --index /tmp/fndr-soak-index \
//!   --model /path/to/embedding-model.gguf
//! ```
//!
//! THIS CAPTURES YOUR SCREEN. It needs Screen Recording permission and it
//! records what is visible for the whole run. Run it deliberately, on a
//! screen you are willing to have stored, and pass `--block-app` /
//! `--block-domain` for anything you are not.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use fndr_capture::SamplingPolicy;
use fndr_privacy::Blocklist;
use fndr_shell::capture_scheduler::RealSchedulerConfig;
use fndr_shell::capture_worker::{RealCaptureWorkerConfig, start_real_capture_worker};

fn main() {
    let mut minutes: u64 = 1;
    let mut store_path: Option<String> = None;
    let mut index_dir: Option<String> = None;
    let mut model_path: Option<String> = None;
    let mut blocked_apps: Vec<String> = Vec::new();
    let mut blocked_domains: Vec<String> = Vec::new();
    let mut display_index: usize = 0;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .unwrap_or_else(|| fail(&format!("{arg} needs a value")))
        };
        match arg.as_str() {
            "--minutes" => {
                minutes = value()
                    .parse()
                    .unwrap_or_else(|_| fail("--minutes must be a number"))
            }
            "--store" => store_path = Some(value()),
            "--index" => index_dir = Some(value()),
            "--model" => model_path = Some(value()),
            "--display" => {
                display_index = value()
                    .parse()
                    .unwrap_or_else(|_| fail("--display must be a number"))
            }
            "--block-app" => blocked_apps.push(value()),
            "--block-domain" => blocked_domains.push(value()),
            other => fail(&format!("unknown arg: {other}")),
        }
    }

    let (Some(store_path), Some(index_dir), Some(model_path)) = (store_path, index_dir, model_path)
    else {
        fail("--store, --index, and --model are all required");
    };

    if minutes == 0 {
        fail("--minutes must be at least 1");
    }

    let blocklist = Blocklist::new(&blocked_apps, &blocked_domains);
    println!("FNDR capture soak");
    println!("  duration:  {minutes} minute(s)");
    println!("  store:     {store_path}");
    println!("  index:     {index_dir}");
    println!(
        "  blocklist: {} app(s), {} domain(s)",
        blocklist.app_count(),
        blocklist.domain_count()
    );
    println!("\nThis captures your screen for the whole run. Ctrl-C to stop early.\n");

    let config = RealCaptureWorkerConfig {
        scheduler: RealSchedulerConfig {
            database_path: PathBuf::from(store_path),
            index_dir: PathBuf::from(index_dir),
            model_path: PathBuf::from(model_path),
            blocklist,
            session_id: format!("soak-{}", std::process::id()),
            display_index,
            flush_interval: Duration::from_secs(30),
            model_idle_timeout: Duration::from_secs(60),
        },
        sampling: SamplingPolicy::default(),
    };

    let (worker, events) = match start_real_capture_worker(config) {
        Ok(started) => started,
        Err(error) => {
            // Invariant 4: a refusal to start is typed and loud, never a
            // silent no-op that looks like a quiet screen.
            eprintln!("capture worker did not start: {error}");
            eprintln!(
                "hint: a missing model file, a missing Screen Recording grant, or an unreadable store path all surface here"
            );
            std::process::exit(1);
        }
    };

    let deadline = Instant::now() + Duration::from_secs(minutes * 60);
    let mut outcomes: BTreeMap<String, u64> = BTreeMap::new();
    let mut observed = 0u64;
    // T-310's AC asks for an RSS trend, because the crate's issue history is
    // leaks and stalled callbacks. A soak that only proves "it did not crash"
    // would miss exactly the failure this ticket exists for.
    let mut rss_samples: Vec<u64> = resident_kb().into_iter().collect();
    let mut next_rss_sample = Instant::now() + RSS_SAMPLE_INTERVAL;

    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match events.recv_timeout(remaining.min(Duration::from_secs(5))) {
            Ok(event) => {
                observed += 1;
                *outcomes
                    .entry(format!("{:?}", event.outcome.capture))
                    .or_default() += 1;
                *outcomes
                    .entry(format!("flush::{:?}", event.outcome.flush))
                    .or_default() += 1;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                eprintln!("worker stopped emitting before the deadline");
                break;
            }
        }
        if Instant::now() >= next_rss_sample {
            rss_samples.extend(resident_kb());
            next_rss_sample = Instant::now() + RSS_SAMPLE_INTERVAL;
        }
    }
    rss_samples.extend(resident_kb());

    println!("\nsoak finished; draining");
    match worker.shutdown() {
        Ok(report) => {
            println!("  ticks:            {}", report.ticks);
            println!("  events observed:  {observed}");
            println!(
                "  flushed on exit:  {} chunk(s)",
                report.shutdown_flushed_chunks
            );
            println!("  outcomes:");
            for (outcome, count) in &outcomes {
                println!("    {outcome}: {count}");
            }
            report_rss_trend(&rss_samples);
            // A soak that captured nothing is a finding, not a success.
            if report.ticks == 0 {
                eprintln!(
                    "\nno ticks occurred: the sampler stayed in deep idle, or the worker never ran"
                );
                std::process::exit(2);
            }
        }
        Err(error) => {
            eprintln!("shutdown failed: {error}");
            std::process::exit(1);
        }
    }
}

const RSS_SAMPLE_INTERVAL: Duration = Duration::from_secs(15);

/// This process's resident set size in KB, via `ps`. Shelling out avoids an
/// unsafe `task_info` binding for a diagnostic that runs a few times a
/// minute; `None` means the sample failed and is simply skipped rather than
/// being recorded as zero, which would fake a downward trend.
fn resident_kb() -> Option<u64> {
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p"])
        .arg(std::process::id().to_string())
        .output()
        .ok()?;
    String::from_utf8(output.stdout)
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
}

fn report_rss_trend(samples: &[u64]) {
    let (Some(first), Some(last)) = (samples.first(), samples.last()) else {
        println!("  rss: unavailable (no samples taken)");
        return;
    };
    let peak = samples.iter().copied().max().unwrap_or(*last);
    let growth = last.saturating_sub(*first);
    println!(
        "  rss: start {first} KB, end {last} KB, peak {peak} KB, growth {growth} KB over {} sample(s)",
        samples.len()
    );
    // Not an assertion: one short run cannot separate a leak from a warm
    // cache. T-310's AC wants a multi-day trend, and this prints the series a
    // human reads to make that call.
    if samples.len() < 4 {
        println!("       (too few samples to read a trend; run longer for T-310's assertion)");
    }
}

fn fail(message: &str) -> ! {
    eprintln!("{message}");
    eprintln!(
        "usage: capture_soak --store <path> --index <dir> --model <gguf> [--minutes N] [--display N] [--block-app NAME] [--block-domain DOMAIN]"
    );
    std::process::exit(2);
}
