//! Entry point: read JSON-RPC lines from stdin, dispatch, write responses.
//!
//! Requests are fanned out to a small worker pool so a slow query on one
//! connection does not block a `ping` or metadata call on another. Responses
//! are funneled through a single writer task so concurrent handlers never
//! interleave bytes on stdout.

use std::{sync::Arc, time::Duration};

use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::{mpsc, watch, Mutex},
    time::interval,
};

mod common;
mod driver;
mod handlers;
mod models;
mod pool_manager;
mod rpc;

const WORKER_POOL_SIZE: usize = 4;

// Bounded so a burst of requests applies backpressure to the stdin reader
// instead of buffering unboundedly in memory.
const REQUEST_QUEUE_CAPACITY: usize = 64;

const POOL_CLEANUP_INTERVAL: Duration = Duration::from_secs(600); // 10 minutes

#[tokio::main]
async fn main() {
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let cleanup_handle = tokio::spawn(run_pool_cleanup(shutdown_rx));

    let (req_tx, req_rx) = mpsc::channel::<String>(REQUEST_QUEUE_CAPACITY);
    let req_rx = Arc::new(Mutex::new(req_rx));

    let (resp_tx, resp_rx) = mpsc::unbounded_channel::<String>();
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
    let mut timer = interval(POOL_CLEANUP_INTERVAL);
    loop {
        tokio::select! {
            _ = timer.tick() => pool_manager::cleanup_idle_pools().await,
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

async fn run_worker(
    req_rx: Arc<Mutex<mpsc::Receiver<String>>>,
    resp_tx: mpsc::UnboundedSender<String>,
) {
    loop {
        let line = {
            let mut rx = req_rx.lock().await;
            rx.recv().await
        };
        let Some(line) = line else { break };

        let response = rpc::handle_line(&line).await;
        let body = match serde_json::to_string(&response) {
            Ok(s) => s,
            Err(err) => format!(
                "{{\"jsonrpc\":\"2.0\",\"error\":{{\"code\":-32603,\"message\":\"serialization failed: {err}\"}},\"id\":null}}",
            ),
        };

        if resp_tx.send(body).is_err() {
            break;
        }
    }
}

async fn run_writer(mut resp_rx: mpsc::UnboundedReceiver<String>) {
    let mut stdout = tokio::io::stdout();
    while let Some(mut body) = resp_rx.recv().await {
        body.push('\n');
        if stdout.write_all(body.as_bytes()).await.is_err() {
            break;
        }
        let _ = stdout.flush().await;
    }
}
