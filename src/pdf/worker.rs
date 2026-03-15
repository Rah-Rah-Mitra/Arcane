//! Persistent OCR worker — IPC client/server over a TCP loopback socket.
//!
//! The worker runs as a background process spawned by `arcane ocr start`.
//! It loads ONNX Runtime and PaddleOCR models once, then serves requests
//! from any `arcane ocr run` or pipeline command, eliminating the ~5 s
//! model-load overhead on every invocation.
//!
//! # Architecture
//!
//! ```text
//!  ┌──────────────────────────────────────┐
//!  │  arcane ocr start                    │
//!  │  Spawns: arcane worker-serve         │
//!  └──────────────────────────────────────┘
//!            ↓ (detached background process)
//!  ┌──────────────────────────────────────┐
//!  │  arcane worker-serve                 │
//!  │  ● Loads OCR models (OnceLock)       │
//!  │  ● Binds 127.0.0.1:0 (random port)  │
//!  │  ● Writes ~/Arcane/ocr-worker.json   │
//!  │  ● Handles one request at a time     │
//!  └──────────────────────────────────────┘
//!            ↑↓ TCP loopback (length-prefixed JSON)
//!  ┌──────────────────────────────────────┐
//!  │  arcane ocr run / pipeline code      │
//!  │  ● Reads ~/Arcane/ocr-worker.json    │
//!  │  ● Connects to port on 127.0.0.1     │
//!  │  ● Sends OcrRequest, reads OcrResponse│
//!  └──────────────────────────────────────┘
//! ```
//!
//! # State file
//!
//! `~/Arcane/ocr-worker.json` records the worker PID and listening port.
//! Written on startup, removed on clean shutdown.
//!
//! # Wire protocol
//!
//! Each message is a 4-byte big-endian `u32` length followed by a UTF-8 JSON
//! body.  Requests use a `"type"` tag; responses use a `"status"` tag.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::layout::PositionedText;
use super::ocr::{self, OcrPageResult};

// ---------------------------------------------------------------------------
// Worker state file
// ---------------------------------------------------------------------------

/// Runtime state written to `~/Arcane/ocr-worker.json` on startup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerState {
    pub pid: u32,
    pub port: u16,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idle_timeout_secs: Option<u64>,
}

pub(crate) fn write_worker_state(state: &WorkerState) -> Result<()> {
    let path = crate::storage::filesystem::worker_state_path()?;
    let json = serde_json::to_string_pretty(state).context("serialise worker state")?;
    std::fs::write(&path, &json)
        .with_context(|| format!("write worker state to {}", path.display()))
}

pub(crate) fn read_worker_state() -> Result<WorkerState> {
    let path = crate::storage::filesystem::worker_state_path()?;
    let json = std::fs::read_to_string(&path).with_context(|| {
        format!(
            "worker state file not found at {}\n\
             Run `arcane ocr start` to launch the worker.",
            path.display()
        )
    })?;
    serde_json::from_str(&json).context("parse worker state file")
}

fn remove_worker_state() {
    if let Ok(path) = crate::storage::filesystem::worker_state_path() {
        let _ = std::fs::remove_file(path);
    }
}

// ---------------------------------------------------------------------------
// Wire protocol
// ---------------------------------------------------------------------------

/// Requests sent by the client to the worker.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OcrRequest {
    /// Check liveness; returns `Pong`.
    Ping,
    /// Ask the worker to shut down gracefully.
    Stop,
    /// Run full-text OCR on the given PDF pages.
    ExtractText {
        pdf: String,
        pages: Vec<u32>,
        dpi: u32,
    },
    /// Run heading-focused OCR on the given PDF pages.
    ExtractHeadings {
        pdf: String,
        pages: Vec<u32>,
        dpi: u32,
    },
}

/// Responses sent by the worker.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum OcrResponse {
    Pong {
        pid: u32,
        uptime_secs: u64,
        requests_served: u64,
    },
    Stopping,
    Text {
        results: Vec<OcrPageResult>,
    },
    Headings {
        results: Vec<PositionedText>,
    },
    Error {
        message: String,
    },
}

// ---------------------------------------------------------------------------
// Wire helpers
// ---------------------------------------------------------------------------

/// Maximum accepted message size (256 MiB) — guards against malformed length headers.
const MAX_MSG_BYTES: usize = 256 * 1024 * 1024;

fn write_msg(stream: &mut TcpStream, payload: &[u8]) -> Result<()> {
    let len = payload.len() as u32;
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(payload)?;
    Ok(())
}

fn read_msg(stream: &mut TcpStream) -> Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    anyhow::ensure!(len <= MAX_MSG_BYTES, "incoming message too large ({len} bytes)");
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf)?;
    Ok(buf)
}

// ---------------------------------------------------------------------------
// Client helpers
// ---------------------------------------------------------------------------

/// Try to open a connection to the running worker.
/// Returns `None` if no worker state file exists or the connection is refused.
pub fn try_connect() -> Option<TcpStream> {
    let state = read_worker_state().ok()?;
    let stream = TcpStream::connect(("127.0.0.1", state.port)).ok()?;
    // Generous timeouts: large PDFs can take minutes.
    let _ = stream.set_read_timeout(Some(Duration::from_secs(600)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(60)));
    Some(stream)
}

/// Send one request to `stream` and receive the response.
pub fn transact(stream: &mut TcpStream, req: &OcrRequest) -> Result<OcrResponse> {
    let payload = serde_json::to_vec(req).context("serialise OCR request")?;
    write_msg(stream, &payload)?;
    let resp_bytes = read_msg(stream)?;
    serde_json::from_slice(&resp_bytes).context("parse OCR response")
}

/// Try to route `extract_text_ocr` through the running worker.
/// Returns `None` when no worker is available (caller falls back to in-process).
pub fn try_extract_text(
    path: &Path,
    page_indices: &[u32],
    dpi: u32,
) -> Option<Result<Vec<OcrPageResult>>> {
    let mut stream = try_connect()?;
    let req = OcrRequest::ExtractText {
        pdf: path.to_string_lossy().into_owned(),
        pages: page_indices.to_vec(),
        dpi,
    };
    Some(match transact(&mut stream, &req) {
        Ok(OcrResponse::Text { results }) => Ok(results),
        Ok(OcrResponse::Error { message }) => Err(anyhow::anyhow!("worker error: {message}")),
        Ok(other) => Err(anyhow::anyhow!("unexpected worker response: {other:?}")),
        Err(e) => Err(e),
    })
}

/// Try to route `extract_headings_ocr` through the running worker.
/// Returns `None` when no worker is available.
pub fn try_extract_headings(
    path: &Path,
    page_indices: &[u32],
    dpi: u32,
) -> Option<Result<Vec<PositionedText>>> {
    let mut stream = try_connect()?;
    let req = OcrRequest::ExtractHeadings {
        pdf: path.to_string_lossy().into_owned(),
        pages: page_indices.to_vec(),
        dpi,
    };
    Some(match transact(&mut stream, &req) {
        Ok(OcrResponse::Headings { results }) => Ok(results),
        Ok(OcrResponse::Error { message }) => Err(anyhow::anyhow!("worker error: {message}")),
        Ok(other) => Err(anyhow::anyhow!("unexpected worker response: {other:?}")),
        Err(e) => Err(e),
    })
}

// ---------------------------------------------------------------------------
// Command implementations (called from cli/commands.rs)
// ---------------------------------------------------------------------------

/// Start the OCR worker as a detached background process.
pub fn cmd_start(idle_timeout_secs: Option<u64>) -> Result<()> {
    if try_connect().is_some() {
        let state = read_worker_state()?;
        println!(
            "[arcane] OCR worker is already running (PID {}, port {}).",
            state.pid, state.port
        );
        return Ok(());
    }

    let exe = std::env::current_exe().context("cannot determine arcane executable path")?;
    let log_path = crate::storage::filesystem::worker_log_path()?;
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("open worker log at {}", log_path.display()))?;
    let log_file2 = log_file.try_clone()?;

    let mut cmd = std::process::Command::new(&exe);
    cmd.arg("worker-serve");
    if let Some(secs) = idle_timeout_secs {
        cmd.arg("--idle-timeout-secs").arg(secs.to_string());
    }
    cmd.stdout(log_file).stderr(log_file2);

    // Detach the worker from the current console/terminal.
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW (0x08000000) | CREATE_NEW_PROCESS_GROUP (0x00000200)
        cmd.creation_flags(0x0800_0200);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Move child into its own process group so Ctrl-C won't kill it.
        cmd.process_group(0);
    }

    cmd.spawn().context("failed to spawn OCR worker process")?;

    println!("[arcane] Starting OCR worker (loading models, ~5 s the first time)…");
    for attempt in 0..40 {
        std::thread::sleep(Duration::from_millis(250));
        if try_connect().is_some() {
            let state = read_worker_state().unwrap_or(WorkerState {
                pid: 0,
                port: 0,
                started_at: String::new(),
                idle_timeout_secs: None,
            });
            println!(
                "[arcane] OCR worker ready (PID {}, port {}). Log: {}",
                state.pid,
                state.port,
                log_path.display()
            );
            return Ok(());
        }
        if attempt == 15 {
            println!("[arcane] Still waiting for models to load…");
        }
    }
    anyhow::bail!(
        "OCR worker did not respond within 10 s.\n\
         Check the log at {} for errors.\n\
         Ensure models are downloaded: arcane init-ocr",
        log_path.display()
    )
}

/// Stop the running OCR worker.
pub fn cmd_stop() -> Result<()> {
    let mut stream = try_connect().context("OCR worker is not running.")?;
    match transact(&mut stream, &OcrRequest::Stop)? {
        OcrResponse::Stopping => {
            println!("[arcane] OCR worker is shutting down.");
            Ok(())
        }
        other => anyhow::bail!("unexpected response from worker: {other:?}"),
    }
}

/// Print the status of the OCR worker.
pub fn cmd_status() -> Result<()> {
    match try_connect() {
        None => match read_worker_state() {
            Err(_) => println!("[arcane] OCR worker is not running."),
            Ok(state) => {
                println!(
                    "[arcane] OCR worker is NOT responding (stale state: PID {}, port {}).",
                    state.pid, state.port
                );
                println!("  Run `arcane ocr start` to launch a fresh worker.");
            }
        },
        Some(mut stream) => match transact(&mut stream, &OcrRequest::Ping)? {
            OcrResponse::Pong {
                pid,
                uptime_secs,
                requests_served,
            } => {
                let port = read_worker_state().map(|s| s.port).unwrap_or(0);
                println!("[arcane] OCR worker is running.");
                println!("  PID             : {pid}");
                println!("  Port            : {port}");
                println!("  Uptime          : {uptime_secs}s");
                println!("  Requests served : {requests_served}");
            }
            other => anyhow::bail!("unexpected response: {other:?}"),
        },
    }
    Ok(())
}

/// Stop then restart the OCR worker.
pub fn cmd_restart(idle_timeout_secs: Option<u64>) -> Result<()> {
    if try_connect().is_some() {
        println!("[arcane] Stopping existing OCR worker…");
        cmd_stop()?;
        for _ in 0..20 {
            std::thread::sleep(Duration::from_millis(250));
            if try_connect().is_none() {
                break;
            }
        }
    }
    cmd_start(idle_timeout_secs)
}

// ---------------------------------------------------------------------------
// Server loop (runs inside the detached worker process)
// ---------------------------------------------------------------------------

/// Main loop for the background worker process.
///
/// Called from `main.rs` when `arcane worker-serve` is invoked.  Binds a TCP
/// socket, writes the state file, then handles one request at a time until a
/// `Stop` is received or the optional idle timeout elapses.
pub fn serve_loop(idle_timeout_secs: Option<u64>) -> Result<()> {
    println!("[arcane-ocr-worker] Initialising OCR engine…");
    ocr::init_ocr_engine().context("failed to initialise OCR engine")?;
    println!("[arcane-ocr-worker] OCR engine ready.");

    let listener = TcpListener::bind("127.0.0.1:0").context("failed to bind worker socket")?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    let pid = std::process::id();

    let state = WorkerState {
        pid,
        port,
        started_at: chrono::Local::now().to_rfc3339(),
        idle_timeout_secs,
    };
    write_worker_state(&state)?;
    println!("[arcane-ocr-worker] Listening on 127.0.0.1:{port} (PID {pid})");

    let idle_timeout = idle_timeout_secs.map(Duration::from_secs);
    let start_time = Instant::now();
    let mut last_activity = Instant::now();
    let mut requests_served: u64 = 0;

    loop {
        match listener.accept() {
            Ok((mut stream, _peer)) => {
                last_activity = Instant::now();
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(300)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(300)));

                let should_stop = handle_one(&mut stream, start_time, requests_served);
                requests_served += 1;

                match should_stop {
                    Ok(true) => {
                        println!("[arcane-ocr-worker] Stop requested — shutting down.");
                        break;
                    }
                    Ok(false) => {}
                    Err(e) => eprintln!("[arcane-ocr-worker] request error: {e:#}"),
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if let Some(timeout) = idle_timeout {
                    if last_activity.elapsed() > timeout {
                        println!("[arcane-ocr-worker] Idle timeout — shutting down.");
                        break;
                    }
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                eprintln!("[arcane-ocr-worker] accept error: {e}");
                break;
            }
        }
    }

    remove_worker_state();
    println!("[arcane-ocr-worker] Shutdown complete.");
    Ok(())
}

/// Handle exactly one client connection. Returns `true` if the server should stop.
fn handle_one(stream: &mut TcpStream, start_time: Instant, requests_served: u64) -> Result<bool> {
    let payload = read_msg(stream)?;
    let req: OcrRequest =
        serde_json::from_slice(&payload).context("parse request JSON")?;

    let (resp, stop) = match req {
        OcrRequest::Ping => (
            OcrResponse::Pong {
                pid: std::process::id(),
                uptime_secs: start_time.elapsed().as_secs(),
                requests_served,
            },
            false,
        ),
        OcrRequest::Stop => (OcrResponse::Stopping, true),
        OcrRequest::ExtractText { pdf, pages, dpi } => {
            match ocr::extract_text_ocr_direct(Path::new(&pdf), &pages, dpi) {
                Ok(results) => (OcrResponse::Text { results }, false),
                Err(e) => (OcrResponse::Error { message: format!("{e:#}") }, false),
            }
        }
        OcrRequest::ExtractHeadings { pdf, pages, dpi } => {
            match ocr::extract_headings_ocr_direct(Path::new(&pdf), &pages, dpi) {
                Ok(results) => (OcrResponse::Headings { results }, false),
                Err(e) => (OcrResponse::Error { message: format!("{e:#}") }, false),
            }
        }
    };

    let resp_bytes = serde_json::to_vec(&resp).context("serialise response")?;
    write_msg(stream, &resp_bytes)?;
    Ok(stop)
}
