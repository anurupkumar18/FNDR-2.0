//! T-208 spike: prove BM25 FTS, metadata prefilters, hybrid search, scalar
//! point lookups, and index maintenance from Rust on a 100k-row fixture,
//! measuring everything ADR-002's index design assumes. Usage:
//!   cargo run --release -- [--rows 100000] [--dim 768] [--dir <path>]

use std::sync::Arc;
use std::time::Instant;

use arrow_array::types::Float32Type;
use arrow_array::{
    FixedSizeListArray, Int64Array, RecordBatch, RecordBatchIterator, StringArray,
};
use arrow_schema::{DataType, Field, Schema};
use futures::TryStreamExt;
use lance_index::scalar::FullTextSearchQuery;
use lancedb::index::Index;
use lancedb::index::scalar::{BTreeIndexBuilder, FtsIndexBuilder};
use lancedb::index::vector::IvfPqIndexBuilder;
use lancedb::query::{ExecutableQuery, QueryBase, QueryExecutionOptions};
use lancedb::table::{OptimizeAction, Table};

const TOPICS: &[&str] = &[
    "kubernetes deployment rollout",
    "quarterly revenue forecast spreadsheet",
    "rust borrow checker lifetimes",
    "meeting notes action items",
    "database migration schema",
    "onboarding permission flow",
    "retrieval benchmark recall",
    "invoice payment reconciliation",
    "screen capture pipeline",
    "model registry checksum",
];

struct Xorshift(u64);
impl Xorshift {
    fn next_f32(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32 - 0.5
    }
}

fn row_vector(id: i64, dim: usize) -> Vec<f32> {
    let mut rng = Xorshift(0x9E3779B97F4A7C15 ^ (id as u64).wrapping_mul(0xD1B54A32D192ED03));
    (0..dim).map(|_| rng.next_f32()).collect()
}

fn row_text(id: i64) -> String {
    let topic = TOPICS[(id as usize) % TOPICS.len()];
    format!("{topic} discussion continued in window rec{id} with follow up details")
}

fn dir_size_mb(path: &std::path::Path) -> f64 {
    fn walk(path: &std::path::Path) -> u64 {
        let mut total = 0;
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    total += walk(&p);
                } else if let Ok(meta) = entry.metadata() {
                    total += meta.len();
                }
            }
        }
        total
    }
    walk(path) as f64 / (1024.0 * 1024.0)
}

async fn count_hits(
    stream: impl futures::Stream<Item = lancedb::error::Result<RecordBatch>> + Unpin,
) -> (usize, Vec<i64>) {
    let batches: Vec<RecordBatch> = stream.try_collect().await.expect("query stream");
    let mut ids = Vec::new();
    for batch in &batches {
        if let Some(col) = batch.column_by_name("id") {
            let col = col.as_any().downcast_ref::<Int64Array>().expect("id i64");
            ids.extend(col.iter().flatten());
        }
    }
    (ids.len(), ids)
}

async fn timed<T, F: std::future::Future<Output = T>>(label: &str, fut: F) -> T {
    let started = Instant::now();
    let out = fut.await;
    println!("  {label}: {:.1} ms", started.elapsed().as_secs_f64() * 1000.0);
    out
}

#[tokio::main]
async fn main() {
    let mut rows: i64 = 100_000;
    let mut dim: usize = 768;
    let mut dir: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--rows" => rows = args.next().unwrap().parse().unwrap(),
            "--dim" => dim = args.next().unwrap().parse().unwrap(),
            "--dir" => dir = args.next(),
            other => panic!("unknown arg {other}"),
        }
    }
    let dir = dir.unwrap_or_else(|| {
        std::env::temp_dir()
            .join(format!("fndr-lance-spike-{}", std::process::id()))
            .to_string_lossy()
            .into_owned()
    });
    let _ = std::fs::remove_dir_all(&dir);
    println!("spike: rows={rows} dim={dim} dir={dir}");

    let db = lancedb::connect(&dir).execute().await.expect("connect");

    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("source", DataType::Utf8, false),
        Field::new("captured_at_ms", DataType::Int64, false),
        Field::new("text", DataType::Utf8, false),
        Field::new(
            "vector",
            DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), dim as i32),
            false,
        ),
    ]));

    // Ingest in 1k batches: the shape the T-202 flush writer will use.
    let ingest_start = Instant::now();
    let batch_size: i64 = 1000;
    let mut table: Option<Table> = None;
    for start in (0..rows).step_by(batch_size as usize) {
        let end = (start + batch_size).min(rows);
        let ids: Vec<i64> = (start..end).collect();
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Int64Array::from(ids.clone())),
                Arc::new(StringArray::from_iter_values(ids.iter().map(|i| {
                    if i % 5 == 0 { "meeting" } else { "screen" }
                }))),
                Arc::new(Int64Array::from(
                    ids.iter().map(|i| 1_755_000_000_000 + i * 1000).collect::<Vec<_>>(),
                )),
                Arc::new(StringArray::from_iter_values(ids.iter().map(|i| row_text(*i)))),
                Arc::new(FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
                    ids.iter().map(|i| Some(row_vector(*i, dim).into_iter().map(Some).collect::<Vec<_>>())),
                    dim as i32,
                )),
            ],
        )
        .expect("batch");
        let reader = RecordBatchIterator::new(vec![Ok(batch)], schema.clone());
        match &table {
            None => {
                table = Some(
                    db.create_table("chunks_spike", Box::new(reader))
                        .execute()
                        .await
                        .expect("create table"),
                );
            }
            Some(t) => t.add(Box::new(reader)).execute().await.expect("append"),
        }
    }
    let table = table.expect("table");
    println!(
        "ingest: {rows} rows in {:.1} s ({} batches), version={}, size={:.1} MB",
        ingest_start.elapsed().as_secs_f64(),
        rows / batch_size,
        table.version().await.unwrap(),
        dir_size_mb(std::path::Path::new(&dir)),
    );

    // Query latency with NO index (the v1 production state).
    let probe = row_vector(4242, dim);
    let (_, ids) = timed("vector query UNINDEXED (flat scan)", async {
        let stream = table
            .query()
            .nearest_to(probe.clone())
            .expect("query")
            .limit(10)
            .execute()
            .await
            .expect("exec");
        count_hits(stream).await
    })
    .await;
    assert_eq!(ids.first(), Some(&4242), "flat scan must find the seed row");

    // Index builds.
    println!("index builds:");
    timed("  BTree scalar (id)", async {
        table
            .create_index(&["id"], Index::BTree(BTreeIndexBuilder::default()))
            .execute()
            .await
            .expect("btree id");
    })
    .await;
    timed("  BTree scalar (captured_at_ms)", async {
        table
            .create_index(&["captured_at_ms"], Index::BTree(BTreeIndexBuilder::default()))
            .execute()
            .await
            .expect("btree ts");
    })
    .await;
    timed("  FTS BM25 (text)", async {
        table
            .create_index(&["text"], Index::FTS(FtsIndexBuilder::default()))
            .execute()
            .await
            .expect("fts");
    })
    .await;
    timed("  IVF_PQ (vector)", async {
        table
            .create_index(&["vector"], Index::IvfPq(IvfPqIndexBuilder::default()))
            .execute()
            .await
            .expect("ivfpq");
    })
    .await;
    println!(
        "  indices: {:?}",
        table
            .list_indices()
            .await
            .unwrap()
            .iter()
            .map(|i| format!("{}:{:?}", i.name, i.index_type))
            .collect::<Vec<_>>()
    );

    // Indexed queries.
    println!("queries (indexed):");
    let (_, ids) = timed("  vector ANN top10", async {
        let stream = table
            .query()
            .nearest_to(probe.clone())
            .expect("q")
            .limit(10)
            .execute()
            .await
            .expect("exec");
        count_hits(stream).await
    })
    .await;
    let ann_recall_hit = ids.contains(&4242);
    println!("  ANN recall spot check (seed row in top10): {ann_recall_hit}");

    let (n, _) = timed("  vector ANN + prefilter (source & time window)", async {
        let stream = table
            .query()
            .only_if("source = 'meeting' AND captured_at_ms > 1755020000000")
            .nearest_to(probe.clone())
            .expect("q")
            .limit(10)
            .execute()
            .await
            .expect("exec");
        count_hits(stream).await
    })
    .await;
    println!("  prefiltered hits: {n}");

    let (n, ids) = timed("  FTS BM25 (phrase terms)", async {
        let stream = table
            .query()
            .full_text_search(FullTextSearchQuery::new("borrow checker lifetimes".into()))
            .limit(10)
            .execute()
            .await
            .expect("exec");
        count_hits(stream).await
    })
    .await;
    println!("  fts hits: {n} (ids sample {:?})", &ids[..ids.len().min(3)]);

    let (n, ids) = timed("  FTS unique-token point hit", async {
        let stream = table
            .query()
            .full_text_search(FullTextSearchQuery::new("rec77777".into()))
            .limit(5)
            .execute()
            .await
            .expect("exec");
        count_hits(stream).await
    })
    .await;
    println!("  unique-token hits: {n} {ids:?}");

    let (n, _) = timed("  hybrid (FTS + vector, RRF)", async {
        let stream = table
            .query()
            .full_text_search(FullTextSearchQuery::new("retrieval benchmark recall".into()))
            .nearest_to(probe.clone())
            .expect("q")
            .limit(10)
            .execute_hybrid(QueryExecutionOptions::default())
            .await
            .expect("hybrid");
        count_hits(stream).await
    })
    .await;
    println!("  hybrid hits: {n}");

    let (n, _) = timed("  scalar point lookup (id = 4242)", async {
        let stream = table
            .query()
            .only_if("id = 4242")
            .execute()
            .await
            .expect("exec");
        count_hits(stream).await
    })
    .await;
    assert_eq!(n, 1);

    // Maintenance: append more, measure staleness handling + optimize.
    println!("maintenance:");
    let before_version = table.version().await.unwrap();
    let before_size = dir_size_mb(std::path::Path::new(&dir));
    let stats = timed("  optimize(All): compact + prune + index optimize", async {
        table.optimize(OptimizeAction::All).await.expect("optimize")
    })
    .await;
    println!(
        "  optimize stats: compaction={:?} prune={:?}",
        stats.compaction, stats.prune
    );
    println!(
        "  version {} -> {}, size {:.1} -> {:.1} MB",
        before_version,
        table.version().await.unwrap(),
        before_size,
        dir_size_mb(std::path::Path::new(&dir)),
    );

    println!("spike complete");
}
