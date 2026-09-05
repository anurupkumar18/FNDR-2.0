//! The model-worker priority queue (T-403): every llama.cpp call in the
//! engine goes through one dedicated worker thread, in priority order
//! (interactive > synthesis > review > backfill), with the model loaded
//! lazily and unloaded after an idle timeout. This exists so nothing calls
//! an `Embedder` (or any future inference type) directly from more than
//! one place at once, and so idle RAM is actually reclaimed (ARCHITECTURE
//! section 2: "Loads/unloads GGUF models with idle timers").
//!
//! Generic over a `loader` closure rather than hardcoding `GgufEmbedder` so
//! tests can inject a deterministic, controllable fake instead of paying
//! for a real model load (invariant 4: test embedders live in test code).

use std::cmp::Ordering;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::embedding::{EmbedError, Embedder};

/// Queue priority. Ordered so the derived `Ord` sorts `Interactive`
/// highest (ARCHITECTURE section 2's stated order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Priority {
    Backfill,
    Review,
    Synthesis,
    Interactive,
}

enum WorkKind {
    Documents(Vec<String>),
    Query(String),
}

type JobResult = Result<Vec<Vec<f32>>, EmbedError>;

struct Job {
    priority: Priority,
    seq: u64,
    kind: WorkKind,
    respond: std::sync::mpsc::Sender<JobResult>,
}

impl PartialEq for Job {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority && self.seq == other.seq
    }
}
impl Eq for Job {}

impl PartialOrd for Job {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Job {
    /// Higher priority sorts greater (so `BinaryHeap`, a max-heap, pops it
    /// first); within the same priority, the earlier-submitted job (lower
    /// `seq`) sorts greater, so submission order is preserved (FIFO).
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority
            .cmp(&other.priority)
            .then_with(|| other.seq.cmp(&self.seq))
    }
}

struct Shared {
    queue: Mutex<std::collections::BinaryHeap<Job>>,
    condvar: Condvar,
    shutdown: std::sync::atomic::AtomicBool,
}

/// A live model-worker thread plus the handle used to submit work to it.
/// Dropping the handle asks the worker thread to shut down and joins it,
/// so no orphaned thread outlives its owner.
pub struct ModelWorkerHandle {
    shared: Arc<Shared>,
    seq: AtomicU64,
    join: Option<JoinHandle<()>>,
}

impl ModelWorkerHandle {
    /// Spawn the worker thread. `loader` is called (on the worker thread)
    /// whenever a model is needed and none is currently loaded; its
    /// result is held until `idle_timeout` elapses with no work queued,
    /// at which point it is dropped (unloaded) and `loader` runs again on
    /// the next submission.
    pub fn spawn<L>(loader: L, idle_timeout: Duration) -> Self
    where
        L: Fn() -> Result<Box<dyn Embedder>, EmbedError> + Send + 'static,
    {
        let shared = Arc::new(Shared {
            queue: Mutex::new(std::collections::BinaryHeap::new()),
            condvar: Condvar::new(),
            shutdown: std::sync::atomic::AtomicBool::new(false),
        });
        let worker_shared = Arc::clone(&shared);
        let join = std::thread::spawn(move || run_worker(worker_shared, loader, idle_timeout));
        Self {
            shared,
            seq: AtomicU64::new(0),
            join: Some(join),
        }
    }

    fn submit(&self, priority: Priority, kind: WorkKind) -> JobResult {
        let (tx, rx) = std::sync::mpsc::channel();
        let seq = self.seq.fetch_add(1, AtomicOrdering::Relaxed);
        {
            let mut queue = self.shared.queue.lock().unwrap();
            queue.push(Job {
                priority,
                seq,
                kind,
                respond: tx,
            });
        }
        self.shared.condvar.notify_one();
        rx.recv()
            .unwrap_or_else(|_| Err(EmbedError::Failed("worker thread gone".to_owned())))
    }

    /// Enqueue a document-embedding job at `priority` and block until it
    /// completes (in priority order relative to everything else queued).
    pub fn submit_embed_documents(
        &self,
        priority: Priority,
        texts: Vec<String>,
    ) -> Result<Vec<Vec<f32>>, EmbedError> {
        self.submit(priority, WorkKind::Documents(texts))
    }

    /// Enqueue a query-embedding job at `priority`.
    pub fn submit_embed_query(
        &self,
        priority: Priority,
        query: String,
    ) -> Result<Vec<f32>, EmbedError> {
        let mut rows = self.submit(priority, WorkKind::Query(query))?;
        rows.pop()
            .ok_or_else(|| EmbedError::Failed("worker returned no vector for one query".into()))
    }
}

impl Drop for ModelWorkerHandle {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, AtomicOrdering::SeqCst);
        self.shared.condvar.notify_all();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn run_worker<L>(shared: Arc<Shared>, loader: L, idle_timeout: Duration)
where
    L: Fn() -> Result<Box<dyn Embedder>, EmbedError> + Send + 'static,
{
    let mut model: Option<Box<dyn Embedder>> = None;
    loop {
        if shared.shutdown.load(AtomicOrdering::SeqCst) {
            return;
        }
        let job = {
            let mut queue = shared.queue.lock().unwrap();
            loop {
                if shared.shutdown.load(AtomicOrdering::SeqCst) {
                    return;
                }
                if let Some(job) = queue.pop() {
                    break job;
                }
                let (guard, timeout_result) =
                    shared.condvar.wait_timeout(queue, idle_timeout).unwrap();
                queue = guard;
                if timeout_result.timed_out() && queue.is_empty() && model.is_some() {
                    // Idle: unload now, then keep waiting (indefinitely,
                    // since there is nothing left to reclaim).
                    model = None;
                }
            }
        };

        if model.is_none() {
            match loader() {
                Ok(m) => model = Some(m),
                Err(e) => {
                    let _ = job.respond.send(Err(e));
                    continue;
                }
            }
        }
        let embedder = model.as_deref().expect("just loaded or already present");
        let result = match job.kind {
            WorkKind::Documents(texts) => embedder.embed_documents(&texts),
            WorkKind::Query(text) => embedder.embed_query(&text).map(|v| vec![v]),
        };
        let _ = job.respond.send(result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding::EmbeddingSpec;
    use std::sync::atomic::AtomicUsize;

    const TEST_SPEC: EmbeddingSpec = EmbeddingSpec {
        model_id: "test",
        dim: 3,
        lance_table: "test_table",
    };

    /// Deterministic, controllable stand-in for a real model (invariant 4:
    /// test-only). The first text of each job is used as that job's
    /// identity: `embed_documents` blocks on `gate` (if set, and only for
    /// the very first call) so a test can guarantee other jobs are queued
    /// before this one finishes, then records `texts[0]` into `order` so
    /// the test can assert on actual processing order, not just timing.
    struct RecordingEmbedder {
        gate: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
        order: Arc<Mutex<Vec<String>>>,
    }

    impl Embedder for RecordingEmbedder {
        fn spec(&self) -> &EmbeddingSpec {
            &TEST_SPEC
        }
        fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
            if let Some(gate) = self.gate.lock().unwrap().take() {
                let _ = gate.recv();
            }
            self.order.lock().unwrap().push(texts[0].clone());
            Ok(texts.iter().map(|_| vec![1.0, 2.0, 3.0]).collect())
        }
    }

    #[test]
    fn higher_priority_job_is_processed_before_lower_priority_jobs_queued_earlier() {
        let order: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let (gate_tx, gate_rx) = std::sync::mpsc::channel::<()>();
        let order_for_loader = Arc::clone(&order);
        let gate_rx = Arc::new(Mutex::new(Some(gate_rx)));

        let handle = ModelWorkerHandle::spawn(
            move || {
                Ok(Box::new(RecordingEmbedder {
                    gate: Mutex::new(gate_rx.lock().unwrap().take()),
                    order: Arc::clone(&order_for_loader),
                }) as Box<dyn Embedder>)
            },
            Duration::from_secs(60),
        );
        let handle = Arc::new(handle);

        // r0 (Backfill) is picked up immediately and blocks on the gate,
        // holding the worker thread while r1 and r2 both get queued.
        let h0 = Arc::clone(&handle);
        let r0 = std::thread::spawn(move || {
            h0.submit_embed_documents(Priority::Backfill, vec!["r0".to_owned()])
        });
        std::thread::sleep(Duration::from_millis(150));

        let h1 = Arc::clone(&handle);
        let r1 = std::thread::spawn(move || {
            h1.submit_embed_documents(Priority::Backfill, vec!["r1".to_owned()])
        });
        std::thread::sleep(Duration::from_millis(30));
        let h2 = Arc::clone(&handle);
        let r2 = std::thread::spawn(move || {
            h2.submit_embed_documents(Priority::Interactive, vec!["r2".to_owned()])
        });
        std::thread::sleep(Duration::from_millis(30));

        // Release r0. r1 and r2 are both queued now, r1 (Backfill) first
        // by submission order but r2 (Interactive) must run first anyway.
        gate_tx.send(()).unwrap();

        r0.join().unwrap().unwrap();
        r1.join().unwrap().unwrap();
        r2.join().unwrap().unwrap();

        let recorded = order.lock().unwrap().clone();
        assert_eq!(
            recorded,
            vec!["r0", "r2", "r1"],
            "r2 (Interactive) must be processed before r1 (Backfill), \
             even though r1 was queued first"
        );

        drop(handle);
    }

    #[test]
    fn idle_timeout_unloads_and_next_job_reloads() {
        let load_count = Arc::new(AtomicUsize::new(0));
        let load_count_for_loader = Arc::clone(&load_count);
        let order: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

        let handle = ModelWorkerHandle::spawn(
            move || {
                load_count_for_loader.fetch_add(1, AtomicOrdering::SeqCst);
                Ok(Box::new(RecordingEmbedder {
                    gate: Mutex::new(None),
                    order: Arc::clone(&order),
                }) as Box<dyn Embedder>)
            },
            Duration::from_millis(50),
        );

        handle
            .submit_embed_documents(Priority::Interactive, vec!["a".to_owned()])
            .unwrap();
        assert_eq!(load_count.load(AtomicOrdering::SeqCst), 1);

        // Wait past the idle timeout so the worker unloads the model.
        std::thread::sleep(Duration::from_millis(200));

        handle
            .submit_embed_documents(Priority::Interactive, vec!["b".to_owned()])
            .unwrap();
        assert_eq!(
            load_count.load(AtomicOrdering::SeqCst),
            2,
            "a second submission after the idle window should reload the model"
        );
    }

    #[test]
    fn loader_error_is_returned_without_poisoning_future_jobs() {
        let attempt = Arc::new(AtomicUsize::new(0));
        let attempt_for_loader = Arc::clone(&attempt);
        let order: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

        let handle = ModelWorkerHandle::spawn(
            move || {
                let n = attempt_for_loader.fetch_add(1, AtomicOrdering::SeqCst);
                if n == 0 {
                    Err(EmbedError::Unavailable("first attempt fails".to_owned()))
                } else {
                    Ok(Box::new(RecordingEmbedder {
                        gate: Mutex::new(None),
                        order: Arc::clone(&order),
                    }) as Box<dyn Embedder>)
                }
            },
            Duration::from_secs(60),
        );

        let first = handle.submit_embed_documents(Priority::Interactive, vec!["a".to_owned()]);
        assert!(matches!(first, Err(EmbedError::Unavailable(_))));

        let second = handle.submit_embed_documents(Priority::Interactive, vec!["b".to_owned()]);
        assert!(second.is_ok(), "a later job must still be served");
    }
}
