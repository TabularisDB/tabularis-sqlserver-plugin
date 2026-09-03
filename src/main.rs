//! Entry point: read JSON-RPC lines from stdin, dispatch, write responses.
//!
//! Requests are fanned out to a small worker pool so a slow query on one
//! connection does not block a `ping` or metadata call on another. Responses
//! are funneled through a single writer task so concurrent handlers never
//! interleave bytes on stdout.

use std::sync::Arc;

use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::{mpsc, watch, Mutex},
    time::sleep,
};

mod common;
mod connection;
mod driver;
mod handlers;
mod models;
mod pool_manager;
mod rpc;
mod settings;

// This controls JSON-RPC dispatch concurrency rather than database
// connection concurrency, which is configured separately by max_pool_size.
const WORKER_POOL_SIZE: usize = 4;

// Both sides are bounded so backpressure reaches stdin even when the host is
// slow to consume responses. With four in-flight handlers this caps a burst at
// 134 queued or active payloads instead of moving it into an unbounded output
// queue (64 requests + 64 responses + 4 workers + reader + writer).
const REQUEST_QUEUE_CAPACITY: usize = 64;
const RESPONSE_QUEUE_CAPACITY: usize = 64;

// The TDS client's async call chains produce large futures (especially in
// debug builds). A local SQL Server 2022 debug execute_query probe overflowed
// tokio's default 2 MiB stack while 4 MiB completed. The full release live
// suite also passed at 4 MiB in SS-045, but that does not remove the debug or
// cross-platform risk. Keep 16 MiB as a deliberate 4x margin until the preview
// client flattens those polling chains or CI stress proves less everywhere.
const WORKER_STACK_SIZE: usize = 16 * 1024 * 1024;

fn main() {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(WORKER_STACK_SIZE)
        .build()
        .expect("failed to build tokio runtime")
        .block_on(run());
}

async fn run() {
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let cleanup_handle = tokio::spawn(run_pool_cleanup(shutdown_rx));

    let (req_tx, req_rx) = mpsc::channel::<String>(REQUEST_QUEUE_CAPACITY);
    let req_rx = Arc::new(Mutex::new(req_rx));

    let (resp_tx, resp_rx) = mpsc::channel::<String>(RESPONSE_QUEUE_CAPACITY);
    let writer_handle = tokio::spawn(run_writer(resp_rx));

    let worker_handles: Vec<_> = (0..WORKER_POOL_SIZE)
        .map(|_| tokio::spawn(run_worker(req_rx.clone(), resp_tx.clone())))
        .collect();
    drop(resp_tx);

    run_reader(req_tx).await;

    let _ = shutdown_tx.send(true);

    for handle in worker_handles {
        let _ = handle.await;
    }
    let _ = writer_handle.await;
    let _ = cleanup_handle.await;
}

async fn run_pool_cleanup(mut shutdown_rx: watch::Receiver<bool>) {
    let mut settings_rx = settings::subscribe();
    loop {
        let cleanup_interval = settings::current().pool_idle_eviction_interval();
        tokio::select! {
            _ = sleep(cleanup_interval) => pool_manager::cleanup_idle_pools().await,
            // Reset the timer immediately when initialize supplies an override.
            result = settings_rx.changed() => {
                if result.is_err() {
                    break;
                }
            },
            _ = shutdown_rx.changed() => break,
        }
    }
}

async fn run_reader(req_tx: mpsc::Sender<String>) {
    let mut lines = BufReader::new(tokio::io::stdin()).lines();

    loop {
        let line = match lines.next_line().await {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(err) => {
                eprintln!("stdin read error, exiting: {err}");
                break;
            }
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Blocks when the queue is full, applying backpressure to reading.
        if req_tx.send(trimmed.to_string()).await.is_err() {
            break;
        }
    }
}

async fn run_worker(req_rx: Arc<Mutex<mpsc::Receiver<String>>>, resp_tx: mpsc::Sender<String>) {
    loop {
        let line = {
            let mut rx = req_rx.lock().await;
            rx.recv().await
        };
        let Some(line) = line else { break };

        // Box the dispatch future itself: it embeds every handler's state
        // machine, so constructing only a boxed handler result later would
        // still leave the large dispatch enum on the worker stack.
        let response = Box::pin(rpc::handle_line(&line)).await;
        let body = match serde_json::to_string(&response) {
            Ok(s) => s,
            Err(err) => format!(
                "{{\"jsonrpc\":\"2.0\",\"error\":{{\"code\":-32603,\"message\":\"serialization failed: {err}\"}},\"id\":null}}",
            ),
        };

        if resp_tx.send(body).await.is_err() {
            break;
        }
    }
}

async fn run_writer(mut resp_rx: mpsc::Receiver<String>) {
    let mut stdout = tokio::io::stdout();
    while let Some(mut body) = resp_rx.recv().await {
        body.push('\n');
        if stdout.write_all(body.as_bytes()).await.is_err() {
            break;
        }
        let _ = stdout.flush().await;
    }
}
