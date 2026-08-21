//! Eval corpus loaders, FNDR-Bench harness, baselines, resource probes.
//!
//! M1 skeleton (crate map): the corpus format, the metrics, the baseline
//! comparison, and one honest route: the FTS baseline every ranked route must
//! beat (ADR-006's naive baseline). The full FNDR-Bench with the frozen
//! held-out split, embedding routes, and the faithfulness slice is E05.
//! Never a mock: this measures the same store search the fndr.search tool
//! serves.

use std::path::Path;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use fndr_store::SkeletonStore;

#[derive(Debug, thiserror::Error)]
pub enum BenchError {
    #[error("corpus I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("corpus parse ({file} line {line}): {source}")]
    Parse {
        file: String,
        line: usize,
        source: serde_json::Error,
    },
    #[error("store: {0}")]
    Store(#[from] fndr_store::StoreError),
    #[error("corpus is empty or missing queries")]
    EmptyCorpus,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CorpusRecord {
    pub id: i64,
    pub source: String,
    pub captured_at_ms: i64,
    pub text: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CorpusQuery {
    pub query: String,
    /// Record ids a good retrieval returns for this query.
    pub relevant: Vec<i64>,
}

pub struct Corpus {
    pub name: String,
    pub records: Vec<CorpusRecord>,
    pub queries: Vec<CorpusQuery>,
}

fn load_jsonl<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Vec<T>, BenchError> {
    let content = std::fs::read_to_string(path)?;
    let mut items = Vec::new();
    for (index, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        items.push(
            serde_json::from_str(line).map_err(|source| BenchError::Parse {
                file: path.display().to_string(),
                line: index + 1,
                source,
            })?,
        );
    }
    Ok(items)
}

impl Corpus {
    pub fn load(dir: &Path) -> Result<Self, BenchError> {
        let records: Vec<CorpusRecord> = load_jsonl(&dir.join("records.jsonl"))?;
        let queries: Vec<CorpusQuery> = load_jsonl(&dir.join("queries.jsonl"))?;
        if records.is_empty() || queries.is_empty() {
            return Err(BenchError::EmptyCorpus);
        }
        let name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "corpus".to_string());
        Ok(Self {
            name,
            records,
            queries,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerQuery {
    pub query: String,
    pub recall_at_5: f64,
    pub reciprocal_rank_at_10: f64,
    pub returned: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchReport {
    /// Which retrieval route produced these numbers.
    pub route: String,
    pub corpus: String,
    pub n_records: usize,
    pub n_queries: usize,
    pub recall_at_5: f64,
    pub mrr_at_10: f64,
    /// Recorded for humans; never compared against baselines (machine-dependent).
    pub latency_ms_p50: f64,
    pub latency_ms_p95: f64,
    pub per_query: Vec<PerQuery>,
}

/// recall@k over one ranked list: |relevant in top k| / |relevant|.
pub fn recall_at_k(ranked: &[i64], relevant: &[i64], k: usize) -> f64 {
    if relevant.is_empty() {
        return 0.0;
    }
    let top: Vec<&i64> = ranked.iter().take(k).collect();
    let hit = relevant.iter().filter(|r| top.contains(r)).count();
    hit as f64 / relevant.len() as f64
}

/// Reciprocal rank of the first relevant result within top k; 0 when absent.
pub fn reciprocal_rank_at_k(ranked: &[i64], relevant: &[i64], k: usize) -> f64 {
    for (index, id) in ranked.iter().take(k).enumerate() {
        if relevant.contains(id) {
            return 1.0 / (index as f64 + 1.0);
        }
    }
    0.0
}

fn percentile(sorted_ms: &[f64], p: f64) -> f64 {
    if sorted_ms.is_empty() {
        return 0.0;
    }
    let idx = ((sorted_ms.len() as f64 - 1.0) * p).round() as usize;
    sorted_ms[idx]
}

/// Run the FTS baseline route over a corpus. Loads records into an in-memory
/// store (ids preserved) and issues every query through the same search call
/// the MCP tool serves.
pub fn run_fts_baseline(corpus: &Corpus) -> Result<BenchReport, BenchError> {
    let store = SkeletonStore::open_in_memory()?;
    for record in &corpus.records {
        store.insert_record_with_id(
            record.id,
            record.captured_at_ms,
            &record.source,
            &record.text,
        )?;
    }

    let mut per_query = Vec::with_capacity(corpus.queries.len());
    let mut latencies = Vec::with_capacity(corpus.queries.len());
    for query in &corpus.queries {
        let started = Instant::now();
        let hits = store.search(&query.query, 10)?;
        latencies.push(started.elapsed().as_secs_f64() * 1000.0);
        let ranked: Vec<i64> = hits.iter().map(|h| h.record_id).collect();
        per_query.push(PerQuery {
            query: query.query.clone(),
            recall_at_5: recall_at_k(&ranked, &query.relevant, 5),
            reciprocal_rank_at_10: reciprocal_rank_at_k(&ranked, &query.relevant, 10),
            returned: ranked.len(),
        });
    }

    let n = per_query.len() as f64;
    latencies.sort_by(|a, b| a.partial_cmp(b).expect("finite latencies"));
    Ok(BenchReport {
        route: "fts_baseline".to_string(),
        corpus: corpus.name.clone(),
        n_records: corpus.records.len(),
        n_queries: per_query.len(),
        recall_at_5: per_query.iter().map(|q| q.recall_at_5).sum::<f64>() / n,
        mrr_at_10: per_query
            .iter()
            .map(|q| q.reciprocal_rank_at_10)
            .sum::<f64>()
            / n,
        latency_ms_p50: percentile(&latencies, 0.5),
        latency_ms_p95: percentile(&latencies, 0.95),
        per_query,
    })
}

/// Quality-only comparison against a committed baseline. Latency is never
/// compared (machine-dependent). Returns the list of regressions; empty means
/// the gate passes. Improvements pass and should refresh the baseline.
pub fn compare_to_baseline(report: &BenchReport, baseline: &BenchReport) -> Vec<String> {
    const EPSILON: f64 = 1e-9;
    let mut regressions = Vec::new();
    if report.recall_at_5 + EPSILON < baseline.recall_at_5 {
        regressions.push(format!(
            "recall@5 regressed: {:.4} -> {:.4}",
            baseline.recall_at_5, report.recall_at_5
        ));
    }
    if report.mrr_at_10 + EPSILON < baseline.mrr_at_10 {
        regressions.push(format!(
            "MRR@10 regressed: {:.4} -> {:.4}",
            baseline.mrr_at_10, report.mrr_at_10
        ));
    }
    regressions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recall_and_rr_math() {
        let ranked = [7, 3, 9, 1, 2, 8];
        assert_eq!(recall_at_k(&ranked, &[3, 2], 5), 1.0);
        assert_eq!(recall_at_k(&ranked, &[3, 8], 5), 0.5);
        assert_eq!(recall_at_k(&ranked, &[42], 5), 0.0);
        assert_eq!(reciprocal_rank_at_k(&ranked, &[3], 10), 0.5);
        assert_eq!(reciprocal_rank_at_k(&ranked, &[8], 10), 1.0 / 6.0);
        assert_eq!(reciprocal_rank_at_k(&ranked, &[42], 10), 0.0);
    }

    #[test]
    fn empty_relevant_scores_zero() {
        assert_eq!(recall_at_k(&[1, 2], &[], 5), 0.0);
    }

    #[test]
    fn comparison_flags_regressions_only() {
        let mut report = BenchReport {
            route: "fts_baseline".into(),
            corpus: "t".into(),
            n_records: 1,
            n_queries: 1,
            recall_at_5: 0.8,
            mrr_at_10: 0.7,
            latency_ms_p50: 1.0,
            latency_ms_p95: 2.0,
            per_query: vec![],
        };
        let baseline = report.clone();
        assert!(compare_to_baseline(&report, &baseline).is_empty());
        report.latency_ms_p50 = 99.0;
        assert!(compare_to_baseline(&report, &baseline).is_empty());
        report.recall_at_5 = 0.7;
        assert_eq!(compare_to_baseline(&report, &baseline).len(), 1);
    }
}
