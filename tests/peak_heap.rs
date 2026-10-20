//! The chunk path's **peak live heap**, measured.
//!
//! Spec: [`docs/07-streaming-zst-decode-implementation-plan.md`](../docs/07-streaming-zst-decode-implementation-plan.md)
//! §1.1 and C5.
//!
//! # Why this file exists
//!
//! The whole justification for the streaming path is a number: the chunk
//! path's peak live heap goes from **6.1 MB to 0.2 MB** per active chunk,
//! which at `--io-threads 8` is ~64-97 MB of resident chunk buffers down to
//! ~1.6 MB. Without an instrument that number is a sentence in a document,
//! and the next change that reintroduces a buffer silently invalidates it.
//!
//! So this is the instrument: a counting allocator around the two shapes of
//! the chunk path, and a printout of what each one costs.
//!
//! # Why it is `#[ignore]`d
//!
//! Because it is a **measurement, not an assertion**. It prints two numbers
//! for a human to read; it does not fail when the number grows, because a
//! threshold on a heap figure is a flaky test on a machine nobody pinned.
//! The bridge is `#[ignore]`d for the same reason: it is expensive and it is
//! run deliberately.
//!
//! ```text
//! cargo test -p girpr --test peak_heap -- --ignored --nocapture
//! ```
//!
//! # What it measures
//!
//! **Peak live heap**, not a working set. The Windows heap satisfies a later
//! allocation from pages an earlier `free` left behind, so a "bytes
//! allocated" figure would flatter `decode_all`'s doubling: the buffer it
//! grows by doubling is allocated once and freed once, and the *peak* is what
//! the process actually held.
//!
//! The two shapes, over the same bytes:
//!
//! | shape | what it does |
//! |---|---|
//! | **streamed** | `sophon::stream_chunk_to` — the body is decoded as it arrives and the decoded bytes are written into the file |
//! | **buffered** | `resp.bytes()` → `zstd::decode_all` → the decoded buffer — what the tool used to do |
//!
//! The payload is the shape of the reference bundle's largest chunk: ~2.16 MB
//! of essentially incompressible bytes, so the compressed body and the
//! decoded body are the same size. A compressible payload would measure the
//! wrong thing — the compressed body would be small and the buffered path's
//! peak would be the decoded buffer alone.

#[path = "common/tempdir.rs"]
mod tempdir;

use std::alloc::{GlobalAlloc, Layout, System};
use std::future::Future;
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};

use girpr::sophon::{self, ChunkSink};

use tempdir::TempDir;

// ---------------------------------------------------------------------------
// the counting allocator
// ---------------------------------------------------------------------------

/// The live bytes, and the peak they reached.
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

/// A counting allocator: every byte the process holds is counted, and the
/// high-water mark is kept.
struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            bump(layout.size() as isize);
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        bump(-(layout.size() as isize));
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            bump(new_size as isize - layout.size() as isize);
        }
        p
    }
}

/// Move the live count and remember the peak.
fn bump(delta: isize) {
    let mut cur = LIVE.load(Ordering::Relaxed);
    loop {
        let next = (cur as isize + delta) as usize;
        match LIVE.compare_exchange_weak(cur, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => {
                // The peak is updated after the count is committed, so it is a
                // value the process really held rather than one a racing
                // thread only proposed.
                PEAK.fetch_max(next, Ordering::Relaxed);
                return;
            }
            Err(actual) => cur = actual,
        }
    }
}

/// The allocator of this test binary. It counts everything the process
/// allocates, including the test harness's own — which is why the numbers
/// below are *deltas* over a baseline rather than absolutes.
#[global_allocator]
static ALLOC: Counting = Counting;

/// The peak live heap a future costs, over the live bytes at its start.
///
/// The delta is the number that answers the question: how much heap did this
/// operation hold at its peak? The absolute number would answer a different
/// one (how much heap does the process hold), and the harness's own
/// allocations would be in it.
async fn measure<T, F: Future<Output = T>>(f: F) -> (usize, T) {
    let baseline = LIVE.load(Ordering::Relaxed);
    PEAK.store(baseline, Ordering::Relaxed);
    let out = f.await;
    let peak = PEAK.load(Ordering::Relaxed);
    (peak.saturating_sub(baseline), out)
}

// ---------------------------------------------------------------------------
// the sink: the decoded bytes go into a file, the way they go into the install
// ---------------------------------------------------------------------------

/// The chunk path's sink: the file the chunk belongs in.
struct FileSink {
    file: std::fs::File,
    written: u64,
}

impl FileSink {
    fn new(path: &std::path::Path) -> Self {
        Self {
            file: std::fs::File::create(path).expect("create the sink file"),
            written: 0,
        }
    }
}

impl Write for FileSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.file.write_all(buf)?;
        self.written += buf.len() as u64;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

impl ChunkSink for FileSink {
    fn restart(&mut self) -> std::io::Result<()> {
        self.written = 0;
        self.file.set_len(0)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// the body
// ---------------------------------------------------------------------------

/// Pseudo-random bytes.
///
/// A real chunk's payload is not compressible, and the measurement depends on
/// that: a compressible payload would make the compressed body small, and
/// the buffered path's peak would then be the decoded buffer alone — which
/// would understate the difference the streaming path makes.
fn incompressible(n: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(n + 8);
    let mut state: u64 = 0x243F_6A88_85A3_08D3;
    while out.len() < n {
        // xorshift64
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        out.extend_from_slice(&state.to_le_bytes());
    }
    out.truncate(n);
    out
}

/// A one-route server: the same body, as many times as it is asked for.
///
/// The body is written in pieces, so the client sees a **stream** and not one
/// buffer. That is what the real CDN does, and it is the whole point: the
/// streamed path's cost is one read window, and a server that delivered the
/// whole body at once would hand the measurement the thing it is measuring
/// away.
///
/// The connection is kept alive between requests, the way a CDN's is: the
/// client's buffers are then allocated once, in the warm-up, and belong to
/// the baseline rather than to the operation being measured.
async fn serve(body: Vec<u8>, piece: usize) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind the measurement server");
    let addr = listener.local_addr().expect("the server's address");
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let body = body.clone();
            tokio::spawn(async move {
                // Serve every request on this connection, until the client
                // closes it.
                loop {
                    let mut head = Vec::new();
                    loop {
                        let mut chunk = [0_u8; 1024];
                        let n = match socket.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => n,
                        };
                        head.extend_from_slice(&chunk[..n]);
                        if head.windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\n\r\n",
                        body.len()
                    );
                    socket.write_all(response.as_bytes()).await.ok();
                    for chunk in body.chunks(piece) {
                        socket.write_all(chunk).await.ok();
                        socket.flush().await.ok();
                        tokio::task::yield_now().await;
                    }
                }
            });
        }
    });
    format!("http://{addr}")
}

// ---------------------------------------------------------------------------
// the measurement
// ---------------------------------------------------------------------------

/// The size of the reference bundle's largest chunk, in bytes.
const CHUNK_BYTES: usize = 2_162_302;

#[tokio::test]
#[ignore = "a measurement, not an assertion: run it and read the numbers"]
async fn the_chunk_paths_peak_live_heap() {
    let payload = incompressible(CHUNK_BYTES);
    let frame = zstd::encode_all(&payload[..], 3).expect("encode the payload");
    let base = serve(frame.clone(), 16 * 1024).await;
    let client = reqwest::Client::new();
    let dir = TempDir::new("girpr_peak", "chunk");
    let path = dir.join("chunk.bin");

    // The warm-up: the connection's buffers are allocated once and stay live,
    // so they belong to the baseline and not to the operation.
    {
        let mut sink = FileSink::new(&path);
        sophon::stream_chunk_to(&client, &base, "chunk", payload.len() as u64, &mut sink)
            .await
            .expect("the warm-up fetch");
    }

    // The streamed path: the body is decoded as it arrives and the decoded
    // bytes are written into the file.
    let mut sink = FileSink::new(&path);
    let (streamed, digest) = measure(sophon::stream_chunk_to(
        &client,
        &base,
        "chunk",
        payload.len() as u64,
        &mut sink,
    ))
    .await;
    let digest = digest.expect("the streamed fetch");
    assert_eq!(digest.bytes, payload.len() as u64, "the whole chunk landed");
    assert_eq!(
        digest.md5,
        format!("{:x}", md5::compute(&payload)),
        "and it is the right chunk"
    );

    // The buffered path: what the tool used to do. The compressed body is
    // resident, and the decoded body is a buffer that grows by doubling.
    let (buffered, (compressed, decoded)) = measure(async {
        let resp = client
            .get(format!("{base}/chunk"))
            .send()
            .await
            .expect("send");
        let bytes = resp.bytes().await.expect("the body");
        let decoded = zstd::decode_all(&bytes[..]).expect("the decode");
        (bytes.len(), decoded.len())
    })
    .await;
    assert_eq!(decoded, payload.len(), "the same chunk, decoded");

    let mb = |b: usize| b as f64 / (1024.0 * 1024.0);
    println!(
        "chunk: {CHUNK_BYTES} B decoded, {compressed} B compressed\n\
         streamed peak live heap: {streamed} B ({:.2} MB)\n\
         buffered peak live heap: {buffered} B ({:.2} MB)\n\
         ratio: {:.1}x",
        mb(streamed),
        mb(buffered),
        if streamed > 0 {
            buffered as f64 / streamed as f64
        } else {
            f64::INFINITY
        },
    );
}
