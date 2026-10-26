//! The HTTP fetcher and the request log that makes a capture auditable.
//!
//! **M1.** [`Fetcher`] owns retries, the timeout, the `User-Agent`, and the
//! provenance record that becomes `api/requests.ndjson`. A bundle that records
//! *what it fetched* is a bundle someone can re-fetch and diff; one that records
//! only its *results* is a claim.
//!
//! [`probe`] answers a different question — which tags will the API serve at
//! all — and the difference is the fact `diff_tags` does not carry.
//!
//! ## What is recorded, and why each field earns its place
//!
//! | field | why |
//! |---|---|
//! | `kind` | so "what did the manifest fetch cost?" is a filter, not a string match on a URL |
//! | `elapsed_ms` | total across attempts, so a slow retry is visible as slowness |
//! | `attempts` | a 200 after three tries is a different fact from a 200 first time |
//! | `redirected` / `final_url` | a CDN that redirects is a CDN that can change what it serves |
//! | `resp_headers` | `etag`, `content-md5`, `x-cache` are how the CDN's claims get checked |
//! | `bytes_in` | the only way to price a manifest without re-downloading it |
//!
//! `bytes_in` counts **every** body received, including the body of a failed
//! request. A throttled run that was retried three times paid for four responses
//! and a summary that said `0` for three of them would be a cost figure that
//! flatters itself exactly when the cost was highest.
//!
//! ## Retries
//!
//! Retried: connection and timeout errors, `408`, `429`, and any `5xx`.
//! **Not** retried: every other `4xx` — a `404` on a chunk means the blob is
//! gone, and retrying it three times only makes the failure take longer and look
//! like a network problem. Backoff is exponential from `--retries`' base and
//! capped; there is no jitter, because a capture that adds randomness to its own
//! request log is harder to diff against a previous one.

#[cfg(test)]
use std::collections::BTreeMap;
#[cfg(test)]
use std::sync::{Arc, Mutex};
#[cfg(test)]
use std::time::{Duration, Instant};

#[cfg(test)]
use crate::exit::ExitCode;

pub mod error;
pub mod fetcher;
pub mod log;
pub mod negative;
pub mod probe;
pub mod record;
pub mod transport;

pub use error::ApiError;
pub(crate) use fetcher::shared_req_headers;
pub use fetcher::{Destination, Fetched, Fetcher, Observation, Streamed};

#[cfg(test)]
use fetcher::{body_failure, client_default_headers};
pub use log::{RequestLog, now_rfc3339};
pub use probe::TagProbe;
pub(crate) use record::header_map;
pub use record::{ACCEPT, ACCEPT_ENCODING, Kind, RequestRecord};
pub use transport::{
    DEFAULT_MAX_RESPONSE_BYTES, RealSleep, RecordingSleep, RetryPolicy, Sleep, SleepFuture,
    Transport,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::biz::{Biz, BranchParams};
    use crate::cli::Biz as BizId;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[test]
    fn the_record_claims_exactly_the_headers_the_client_is_configured_with() {
        // **The defect in one assertion.** A record that names headers the client
        // was not configured with makes a false claim about its own request, in
        // the one field a reader would use the log for. Both sides are now built
        // from one list, so this cannot drift by editing one of them — asserted
        // rather than assumed, because the whole failure mode was two lists that
        // nobody compared.
        let ua = "test-agent/1.0";
        let recorded = RequestRecord::default_req_headers(ua, None);
        let configured = client_default_headers(ua).expect("header map");

        let mut from_client: BTreeMap<String, String> = BTreeMap::new();
        for (name, value) in configured.iter() {
            from_client.insert(
                name.as_str().to_string(),
                value.to_str().expect("ascii header").to_string(),
            );
        }
        assert_eq!(
            recorded, from_client,
            "the bundle's claim about the request and the request must be the same"
        );
        assert!(recorded.contains_key("accept"), "accept is sent");
        assert!(recorded.contains_key("user-agent"), "user-agent is sent");
    }

    #[test]
    fn the_record_does_not_claim_a_compression_it_cannot_decode() {
        // Naming `gzip` on the wire without reqwest's `gzip` feature is not a
        // no-op: the CDN would answer compressed and reqwest, unable to
        // decompress, would hand back the compressed bytes — so every manifest
        // digest in the bundle would be a digest of compressed JSON. This is the
        // half of the choice that a "just send gzip" fix would get wrong.
        assert_eq!(
            ACCEPT_ENCODING, "identity",
            "the tool offers no compression, so it must not claim any"
        );
        let headers = RequestRecord::default_req_headers("ua", None);
        assert_eq!(headers["accept-encoding"], "identity");
    }

    #[test]
    fn a_range_is_recorded_and_is_not_part_of_the_shared_set() {
        // `Range` is the one per-request header, so it is the one the shared
        // list must not carry — a client-wide `Range` would make every request a
        // byte range.
        assert!(
            !shared_req_headers("ua").contains_key("range"),
            "a client-wide Range would break every request but the first"
        );
        let with = RequestRecord::default_req_headers("ua", Some("bytes=0-1023"));
        assert_eq!(with["range"], "bytes=0-1023");
    }

    // ---------------------------------------------------------------- URLs --

    #[test]
    fn the_measured_global_urls_are_reproduced_exactly() {
        let b = Biz::resolve(BizId::Hk4eGlobal);
        assert_eq!(
            b.bases()
                .hyp_url(&b, crate::biz::HypEndpoint::GameConfigs, "en-us"),
            "https://sg-hyp-api.hoyoverse.com/hyp/hyp-connect/api/getGameConfigs\
             ?launcher_id=VYTpXlbWo8&language=en-us&game_ids[]=gopR6Cufr3"
        );
        let p = BranchParams {
            package_id: "ScSYQBFhu9".into(),
            password: "bDL4JUHL625x".into(),
            branch: "main".into(),
            tag: "7.1.0".into(),
            diff_tags: vec![],
        };
        assert_eq!(
            p.get_build_url(b.sophon_base(), Some("7.0.0")),
            "https://sg-downloader-api.hoyoverse.com/downloader/sophon_chunk/api/getBuild\
             ?branch=main&package_id=ScSYQBFhu9&password=bDL4JUHL625x&tag=7.0.0"
        );
    }

    // --------------------------------------------------------------- retry --

    #[test]
    fn the_backoff_is_exponential_from_the_second_attempt_and_capped() {
        let p = RetryPolicy {
            attempts: 10,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(10),
        };
        assert_eq!(p.delay_before(1), Duration::ZERO);
        assert_eq!(p.delay_before(2), Duration::from_millis(500));
        assert_eq!(p.delay_before(3), Duration::from_millis(1000));
        assert_eq!(p.delay_before(4), Duration::from_millis(2000));
        assert_eq!(p.delay_before(5), Duration::from_millis(4000));
        assert_eq!(p.delay_before(6), Duration::from_millis(8000));
        assert_eq!(p.delay_before(7), Duration::from_secs(10), "capped");
        // A shift overflow would panic or wrap; it must clamp instead.
        assert_eq!(p.delay_before(64), Duration::from_secs(10));
    }

    #[test]
    fn at_least_one_attempt_is_always_made() {
        assert_eq!(RetryPolicy::new(0).attempts, 1);
        assert_eq!(RetryPolicy::new(1).attempts, 1);
        assert_eq!(RetryPolicy::new(5).attempts, 5);
    }

    /// The whole retry policy in one test: a `404` is not worth retrying, a
    /// `503` is, and a timeout is.
    #[test]
    fn only_overload_and_server_faults_are_retryable() {
        for s in [408, 429, 500, 502, 503, 504, 599] {
            assert!(RetryPolicy::should_retry_status(s), "{s} should retry");
        }
        for s in [200, 204, 206, 301, 302, 400, 401, 403, 404, 410, 451] {
            assert!(!RetryPolicy::should_retry_status(s), "{s} must not retry");
        }
    }

    // ----------------------------------------------------------------- log --

    #[test]
    fn sequence_numbers_are_reserved_at_issue_time_and_stay_unique() {
        let log = RequestLog::new();
        let a = log.reserve();
        let b = log.reserve();
        assert_eq!((a, b), (1, 2));
        // Reserving before pushing is what keeps concurrent fetches in issue
        // order; the log stays sorted even if records arrive out of order.
        let mk = |seq, url: &str| RequestRecord {
            seq,
            ts: now_rfc3339(),
            kind: Kind::Chunk,
            method: "GET",
            url: url.to_string(),
            range: None,
            status: 200,
            final_url: url.to_string(),
            redirected: false,
            elapsed_ms: 1,
            req_headers: BTreeMap::new(),
            resp_headers: BTreeMap::new(),
            bytes_in: 10,
            stored_as: None,
            attempts: 1,
        };
        log.push(mk(b, "/second"));
        log.push(mk(a, "/first"));
        // Pushed out of order, read back in **seq** order: a `--jobs 8` capture
        // lands its chunk fetches whenever the network finishes, so completion
        // order and `seq` order differ in exactly the run that has the most of
        // them.
        let records = log.records();
        let seqs: Vec<u64> = records.iter().map(|r| r.seq).collect();
        assert_eq!(seqs, vec![a, b], "records come back in seq order");
        let urls: Vec<&str> = records.iter().map(|r| r.url.as_str()).collect();
        assert_eq!(
            urls,
            vec!["/first", "/second"],
            "so issue order is recoverable"
        );
        assert_eq!(log.bytes_in(), 20);
        assert_eq!(log.len(), 2);
        assert!(!log.is_empty());
    }

    #[test]
    fn stored_as_is_filled_in_by_sequence_number() {
        let log = RequestLog::new();
        let seq = log.reserve();
        log.push(RequestRecord {
            seq,
            ts: now_rfc3339(),
            kind: Kind::Manifest,
            method: "GET",
            url: "https://x/m".into(),
            range: None,
            status: 200,
            final_url: "https://x/m".into(),
            redirected: false,
            elapsed_ms: 1,
            req_headers: BTreeMap::new(),
            resp_headers: BTreeMap::new(),
            bytes_in: 5,
            stored_as: None,
            attempts: 1,
        });
        log.set_stored_as(seq, "manifests/7.1.0/game.manifest.zst");
        assert_eq!(
            log.records()[0].stored_as.as_deref(),
            Some("manifests/7.1.0/game.manifest.zst")
        );
        // An unknown seq is ignored rather than appended: a typo must not create
        // a phantom record.
        log.set_stored_as(999, "nowhere");
        assert_eq!(log.len(), 1);
    }

    /// `set_stored_as` reaches the right row **whatever order the rows landed
    /// in** — which is the property the `by_seq` index buys, and the one a
    /// `--jobs 8` capture actually depends on.
    ///
    /// A linear scan that returned the *first* match rather than the one with
    /// the matching `seq` would pass the single-row test above and fail this.
    #[test]
    fn stored_as_finds_its_row_among_rows_that_landed_out_of_order() {
        let log = RequestLog::new();
        let mk = |seq: u64, url: &str| RequestRecord {
            seq,
            ts: now_rfc3339(),
            kind: Kind::Chunk,
            method: "GET",
            url: url.to_string(),
            range: None,
            status: 200,
            final_url: url.to_string(),
            redirected: false,
            elapsed_ms: 1,
            req_headers: BTreeMap::new(),
            resp_headers: BTreeMap::new(),
            bytes_in: 7,
            stored_as: None,
            attempts: 1,
        };
        let seqs: Vec<u64> = (0..64).map(|_| log.reserve()).collect();
        // Land them backwards, as a saturated chunk fetcher would.
        for &s in seqs.iter().rev() {
            log.push(mk(s, &format!("/chunk/{s}")));
        }
        assert_eq!(log.len(), 64);

        for &s in seqs.iter() {
            log.set_stored_as(s, format!("chunks/blobs/{s}.zst"));
        }

        for &s in seqs.iter() {
            let row = log
                .records()
                .into_iter()
                .find(|r| r.seq == s)
                .expect("the row is there");
            assert_eq!(
                row.stored_as.as_deref(),
                Some(format!("chunks/blobs/{s}.zst").as_str()),
                "seq {s} got someone else's path"
            );
        }
    }

    #[test]
    fn the_log_renders_as_ndjson_with_every_field_present() {
        let log = RequestLog::new();
        let seq = log.reserve();
        log.push(RequestRecord {
            seq,
            ts: "2026-10-14T10:15:02.117Z".into(),
            kind: Kind::Api,
            method: "GET",
            url: "https://x/getGameConfigs".into(),
            range: Some("bytes=0-9".into()),
            status: 200,
            final_url: "https://x/getGameConfigs".into(),
            redirected: false,
            elapsed_ms: 418,
            req_headers: BTreeMap::from([(
                "user-agent".to_string(),
                "UnityPlayer/2019.4.40f1 (UnityWebRequest/1.0, libcurl/7.80.0-DEV)".to_string(),
            )]),
            resp_headers: BTreeMap::from([("etag".to_string(), "\"313EA8\"".to_string())]),
            bytes_in: 1529,
            stored_as: Some("api/getGameConfigs.json".into()),
            attempts: 2,
        });
        let text = log.to_ndjson();
        assert_eq!(text.lines().count(), 1);
        let v: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        for key in [
            "seq",
            "ts",
            "kind",
            "method",
            "url",
            "range",
            "status",
            "final_url",
            "redirected",
            "elapsed_ms",
            "req_headers",
            "resp_headers",
            "bytes_in",
            "stored_as",
            "attempts",
        ] {
            assert!(v.get(key).is_some(), "missing {key} in {text}");
        }
        assert_eq!(v["kind"], "api");
        assert_eq!(v["seq"], 1);
        assert_eq!(v["attempts"], 2);
        assert_eq!(v["resp_headers"]["etag"], "\"313EA8\"");
        assert!(text.ends_with('\n'), "ndjson must be newline-terminated");
    }

    /// A writer that announces it has entered `write`, then blocks there until
    /// the test releases it — a rendezvous, so the lock's state can be observed
    /// with no timing assumption.
    ///
    /// Only the first `write` call takes part: the pair is `take`n, so later calls
    /// pass straight through. A single release therefore cannot deadlock if
    /// `write_ndjson_to` calls `write` more than once. The release wait still has
    /// a timeout as a deadlock guard, so a panicking test fails rather than hangs.
    struct SlowWrite {
        rendezvous: Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>,
    }

    impl std::io::Write for SlowWrite {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if let Some((entered, release)) = self.rendezvous.take() {
                let _ = entered.send(());
                // The test is alive and sends after its `try_lock`; the timeout
                // only stops a failing test from hanging the runner.
                let _ = release.recv_timeout(std::time::Duration::from_secs(5));
            }
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// A writer that accepts `ok` bytes, then fails.
    struct FailsAfter {
        left: usize,
    }

    impl std::io::Write for FailsAfter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if self.left == 0 {
                return Err(std::io::Error::other("no space left on device"));
            }
            let n = self.left.min(buf.len());
            self.left -= n;
            Ok(n)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// A disk that fills mid-write must **not** be reported as a successful log.
    ///
    /// The log is streamed, which introduces a hazard a whole-document write does not
    /// have: a partially written `requests.ndjson` is a file that exists and is
    /// short. `write_ndjson_to` propagates the write error instead of returning a
    /// count, so the run fails loudly — and the count it *would* have returned is
    /// exactly the number of records that made it to disk, which is why the error
    /// must not be swallowed.
    #[test]
    fn a_write_that_fails_mid_stream_is_an_error_not_a_short_log() {
        let log = RequestLog::new();
        for _ in 0..20 {
            let seq = log.reserve();
            log.push(RequestRecord {
                seq,
                ts: now_rfc3339(),
                kind: Kind::Chunk,
                method: "GET",
                url: "/x".into(),
                range: None,
                status: 200,
                final_url: "/x".into(),
                redirected: false,
                elapsed_ms: 1,
                req_headers: BTreeMap::new(),
                resp_headers: BTreeMap::new(),
                bytes_in: 1,
                stored_as: None,
                attempts: 1,
            });
        }

        // Enough for a few rows, nowhere near all twenty.
        let mut w = FailsAfter { left: 40 };
        let err = log
            .write_ndjson_to(&mut w)
            .expect_err("a failing writer must not look like a short success");
        assert!(
            err.to_string().contains("no space left"),
            "the OS error must survive, not be replaced: {err}"
        );

        // And a sink that never fails still gets everything.
        let mut ok: Vec<u8> = Vec::new();
        assert_eq!(log.write_ndjson_to(&mut ok).unwrap(), 20);
        assert_eq!(ok.iter().filter(|&&b| b == b'\n').count(), 20);
    }

    /// The write happens **outside** the lock.
    ///
    /// A byte-for-byte test cannot see this — moving the `write_all` calls inside
    /// the locked scope produces identical output — and it matters because the log
    /// is shared with the fetchers: a slow disk holding the log lock would stall
    /// every in-flight request's bookkeeping.
    ///
    /// So it is checked directly. The writer signals from inside `write` and then
    /// blocks there until the test releases it; if the lock were held across that
    /// window, `try_lock` could not succeed while the writer waits. Correct code
    /// releases the lock before writing, so `try_lock` succeeds on the first
    /// attempt — and the rendezvous means that is observed, not assumed.
    #[test]
    fn the_write_happens_with_the_log_lock_released() {
        use std::sync::mpsc;

        let log = RequestLog::new();
        for _ in 0..4 {
            let seq = log.reserve();
            log.push(RequestRecord {
                seq,
                ts: now_rfc3339(),
                kind: Kind::Chunk,
                method: "GET",
                url: "/x".into(),
                range: None,
                status: 200,
                final_url: "/x".into(),
                redirected: false,
                elapsed_ms: 1,
                req_headers: BTreeMap::new(),
                resp_headers: BTreeMap::new(),
                bytes_in: 1,
                stored_as: None,
                attempts: 1,
            });
        }

        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let writer_log = log.clone();
        let handle = std::thread::spawn(move || {
            let mut w = SlowWrite {
                rendezvous: Some((entered_tx, release_rx)),
            };
            writer_log.write_ndjson_to(&mut w).expect("write");
        });

        entered_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the writer should have entered write()");

        // The writer is inside `write` and parked there, on the other thread.
        let free = log.0.try_lock().is_ok();
        release_tx.send(()).expect("the writer is waiting");
        handle.join().expect("writer thread");

        assert!(
            free,
            "the log lock is held across the write, so a slow disk would stall \
         every fetch that is still logging"
        );
    }

    #[test]
    fn an_empty_log_is_an_empty_file() {
        let log = RequestLog::new();
        assert!(log.is_empty());
        assert_eq!(log.to_ndjson(), "");
        assert_eq!(log.bytes_in(), 0);
    }

    /// `write_ndjson_to` sorts indices rather than sorting cloned records, so it
    /// has to reproduce `records()`'s ordering **exactly**.
    ///
    /// Two properties, and the second is the one a mutation would break:
    ///
    /// 1. Records land out of order (a saturated chunk fetcher) and come out in
    ///    `seq` order — an unsorted writer passes on a log pushed in order, which
    ///    is why the push order here is deliberately reversed.
    /// 2. A **duplicate `seq` loses no row.** `records()` refuses to read through
    ///    `by_seq` for exactly this reason; sorting indices keeps that property,
    ///    and a writer that deduplicated by `seq` would quietly drop a row from
    ///    the audit log this tool exists to produce. Three rows share `seq` 2, so
    ///    a length assertion alone is not enough — every row is matched by its
    ///    URL too.
    #[test]
    fn the_streaming_writer_orders_by_seq_and_drops_nothing() {
        let log = RequestLog::new();
        let mk = |seq: u64, url: &str| RequestRecord {
            seq,
            ts: now_rfc3339(),
            kind: Kind::Chunk,
            method: "GET",
            url: url.to_string(),
            range: None,
            status: 200,
            final_url: url.to_string(),
            redirected: false,
            elapsed_ms: 1,
            req_headers: BTreeMap::new(),
            resp_headers: BTreeMap::new(),
            bytes_in: 7,
            stored_as: None,
            attempts: 1,
        };
        // Push order is the reverse of seq order, plus a three-way seq collision.
        for (seq, url) in [(3_u64, "/c"), (2, "/a"), (1, "/z"), (2, "/b"), (2, "/d")] {
            log.push(mk(seq, url));
        }

        let mut out: Vec<u8> = Vec::new();
        let n = log.write_ndjson_to(&mut out).unwrap();
        assert_eq!(n, 5, "every row must be written, duplicates included");

        let got = String::from_utf8(out).expect("utf8");
        let rows: Vec<serde_json::Value> = got
            .lines()
            .map(|l| serde_json::from_str(l).expect("a row"))
            .collect();
        assert_eq!(rows.len(), 5);

        // Sorted by seq...
        let seqs: Vec<u64> = rows.iter().map(|r| r["seq"].as_u64().unwrap()).collect();
        assert_eq!(seqs, vec![1, 2, 2, 2, 3], "rows must come out in seq order");

        // ...and stable within a collision, so all three seq-2 rows survive with
        // distinct URLs. A dedup-by-seq writer fails here, not on `n`.
        let dup_urls: Vec<&str> = rows
            .iter()
            .filter(|r| r["seq"] == 2)
            .map(|r| r["url"].as_str().unwrap())
            .collect();
        assert_eq!(dup_urls, vec!["/a", "/b", "/d"]);

        // And it agrees with the document `records()` would have produced.
        let expected: String = log
            .records()
            .iter()
            .map(|r| serde_json::to_string(r).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(got, format!("{expected}\n"));
    }

    /// `write_ndjson` goes through `BufWriter`, so its output has to equal
    /// `to_ndjson`'s — a buffer boundary inside the file would be the kind of
    /// thing only a byte comparison catches.
    #[test]
    fn the_buffered_file_write_equals_the_string() {
        let log = RequestLog::new();
        for _ in 0..300 {
            let seq = log.reserve();
            log.push(RequestRecord {
                seq,
                ts: now_rfc3339(),
                kind: Kind::Chunk,
                method: "GET",
                url: format!("/chunk/{seq}/{}", "x".repeat(900)),
                range: Some(format!("bytes={seq}-")),
                status: 200,
                final_url: format!("/chunk/{seq}"),
                redirected: false,
                elapsed_ms: 3,
                req_headers: BTreeMap::new(),
                resp_headers: BTreeMap::new(),
                bytes_in: 1000 + seq,
                stored_as: None,
                attempts: 1,
            });
        }
        // 300 rows of ~950 B is ~285 KB, past the 256 KB BufWriter, so the
        // write really does cross a flush boundary mid-document.
        let dir = girpr_tempdir::TempDir::new("gircap-http-ndjson", "crosses_a_flush");
        let path = dir.join("api").join("requests.ndjson");
        let n = log.write_ndjson(&path).expect("write");
        assert_eq!(n, 300);
        assert_eq!(
            std::fs::read(&path).expect("read"),
            log.to_ndjson().into_bytes(),
            "the buffered file must be byte-identical to the string"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ------------------------------------------------------------- sleeper --

    /// Exercises the retry loop against a real socket, with the waiting removed.
    ///
    /// These bind `127.0.0.1:0` and are therefore local-only; nothing here
    /// reaches a CDN.
    mod local_server {
        use super::*;
        use std::io::{BufRead, BufReader, Write};
        use std::net::{TcpListener, TcpStream};

        /// A throwaway HTTP server that answers with a scripted sequence and
        /// records the request lines it saw.
        pub struct Mock {
            pub port: u16,
            pub hits: Arc<AtomicU32>,
            pub requests: Arc<Mutex<Vec<String>>>,
        }

        pub fn spawn(statuses: Vec<(u16, &'static [u8])>, extra_body: &'static str) -> Mock {
            spawn_answers(
                statuses
                    .into_iter()
                    .map(|(status, body)| Answer {
                        status,
                        body,
                        claim: None,
                        omit_length: false,
                    })
                    .collect(),
                extra_body,
            )
        }

        /// One scripted answer, and how much of it to actually deliver.
        ///
        /// `claim` is the `Content-Length` the server announces. Setting it
        /// larger than `body.len()` makes the server promise more than it sends
        /// and then hang up — **a truncated response**, which is the one failure
        /// a real CDN produces and the only way to exercise the retry path from
        /// *mid-body*. Every other mock failure happens before a single body byte
        /// is written, so it never reaches the code that has to cope with a
        /// partial write.
        pub struct Answer {
            pub status: u16,
            pub body: &'static [u8],
            pub claim: Option<usize>,
            /// Send **no** Content-Length, so the body is delimited by the
            /// connection closing. The one shape a Content-Length pre-check
            /// cannot see, and therefore the only way to test the running
            /// total on its own.
            pub omit_length: bool,
        }

        pub fn spawn_answers(answers: Vec<Answer>, extra_body: &'static str) -> Mock {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            let port = listener.local_addr().unwrap().port();
            let hits = Arc::new(AtomicU32::new(0));
            let requests = Arc::new(Mutex::new(Vec::new()));
            let (h, r) = (hits.clone(), requests.clone());
            std::thread::spawn(move || {
                for a in answers {
                    let Ok((stream, _)) = listener.accept() else {
                        return;
                    };
                    h.fetch_add(1, Ordering::SeqCst);
                    serve_one(
                        stream,
                        a.status,
                        a.body,
                        a.claim,
                        a.omit_length,
                        extra_body,
                        &r,
                    );
                }
            });
            Mock {
                port,
                hits,
                requests,
            }
        }

        fn serve_one(
            mut stream: TcpStream,
            status: u16,
            body: &'static [u8],
            claim: Option<usize>,
            omit_length: bool,
            extra_body: &'static str,
            requests: &Arc<Mutex<Vec<String>>>,
        ) {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).ok();
            // Drain headers; record the ones that matter.
            let mut headers = Vec::new();
            loop {
                let mut h = String::new();
                if reader.read_line(&mut h).unwrap_or(0) == 0 {
                    break;
                }
                if h.trim().is_empty() {
                    break;
                }
                headers.push(h);
            }
            requests.lock().unwrap().push(format!(
                "{line}{}",
                headers
                    .iter()
                    .filter(|h| h.to_ascii_lowercase().starts_with("range:"))
                    .map(String::as_str)
                    .collect::<String>()
            ));

            let head = format!(
                "HTTP/1.1 {status} X\r\n{}Content-Type: application/octet-stream\r\nETag: \"abc\"\r\nServer: MockOSS\r\n{extra_body}Connection: close\r\n\r\n",
                if omit_length {
                    String::new()
                } else {
                    format!("Content-Length: {}\r\n", claim.unwrap_or(body.len()))
                }
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(body);
            let _ = stream.flush();
        }

        pub fn url(mock: &Mock, path: &str) -> String {
            format!("http://127.0.0.1:{}{path}", mock.port)
        }
    }

    use local_server::*;

    fn fetcher(retries: u32, sleeper: Arc<dyn Sleep>) -> Fetcher {
        fetcher_with(retries, sleeper, DEFAULT_MAX_RESPONSE_BYTES)
    }

    /// [`fetcher`] with an explicit body ceiling, for the ceiling's own tests.
    fn fetcher_with(retries: u32, sleeper: Arc<dyn Sleep>, max_response_bytes: u64) -> Fetcher {
        Fetcher::with_sleeper(
            Transport {
                retries,
                timeout: Duration::from_secs(5),
                max_response_bytes,
            },
            RequestLog::new(),
            sleeper,
        )
        .expect("client builds")
    }

    /// The provenance row a fetch logged, for asserting what was **recorded**.
    ///
    /// `Fetched` carries only a `seq`, so the assertions read the log instead —
    /// which is the thing that actually reaches the bundle.
    ///
    /// Returns the **last** row, because a retried request logs every attempt:
    /// the row carrying the final `attempts` count is the last one.
    fn recorded(f: &Fetcher) -> RequestRecord {
        let v = f.log().records();
        assert!(!v.is_empty(), "nothing was recorded");
        v.into_iter().next_back().expect("at least one row")
    }

    /// A panic in one fetch must not take the log — or the run — with it.
    ///
    /// Poisoning was genuinely reachable, not theoretical: the log's lock sits
    /// inside the concurrent chunk fetcher, so one task panicking while it holds
    /// this lock poisons it for every other task. The old
    /// `.expect("request log is not poisoned")` at eight sites then made every
    /// later writer panic too, so a single panic anywhere turned into a run that
    /// could not even record its own failure — while `Store::one` was explicitly
    /// built to tolerate a panicking fetch task and carry on.
    #[test]
    fn a_poisoned_request_log_still_takes_records() {
        let other = RequestLog::new();
        let log = RequestLog::new();
        assert_eq!(log.reserve(), 1, "issue order starts at 1");

        // Poison it exactly the way a panicking fetch task would.
        let panicker = log.clone();
        let died = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _held = panicker.0.lock().expect("held only in order to poison");
            panic!("a fetch task died holding the log");
        }));
        assert!(died.is_err(), "the panic is the point");
        assert!(log.0.is_poisoned(), "and it did poison the lock");

        // Which is the whole claim: the next writer carries on.
        let seq = log.reserve();
        assert_eq!(seq, 2, "a poisoned log still numbers");
        let r = RequestRecord {
            seq,
            ts: "2026-01-01T00:00:00.000Z".into(),
            kind: Kind::Chunk,
            method: "GET",
            url: "https://example.invalid/x".into(),
            range: None,
            status: 200,
            final_url: String::new(),
            redirected: false,
            elapsed_ms: 1,
            req_headers: BTreeMap::new(),
            resp_headers: BTreeMap::new(),
            bytes_in: 0,
            stored_as: None,
            attempts: 1,
        };
        log.push(r);
        log.set_stored_as(seq, "somewhere");
        assert_eq!(log.len(), 1, "and still records");
        assert_eq!(log.bytes_in(), 0);
        let back = log.records();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].stored_as.as_deref(), Some("somewhere"));
        // Poisoning is per-lock, not global: an unrelated log is unaffected.
        assert_eq!(other.len(), 0);
    }
    #[tokio::test]
    async fn waiting_for_a_retry_yields_the_runtime_rather_than_blocking_it() {
        // `#[tokio::test]` runs a current-thread runtime: one worker, and
        // everything on it. A *synchronous* sleep in the retry backoff would hold
        // that worker for its whole duration — which is what makes `--jobs 64` on
        // a 4-core box stall with every worker asleep.
        //
        // The margin is still wide relative to the work: blocked, this takes
        // >= 100 ms; yielding, it takes ~10 ms. A timing assertion is only
        // meaningful if the two outcomes are far apart rather than close.
        let waiter = tokio::spawn(async {
            RealSleep.sleep(Duration::from_millis(100)).await;
        });
        let started = Instant::now();
        tokio::time::sleep(Duration::from_millis(10)).await;
        let elapsed = started.elapsed();
        waiter.await.expect("the waiter finished");

        assert!(
            elapsed < Duration::from_millis(60),
            "a 10 ms sleep took {elapsed:?} while one task waited 100 ms: \
             the waiter is holding the only worker"
        );
    }

    #[tokio::test]
    async fn a_first_attempt_success_records_attempts_one_and_no_waiting() {
        let mock = spawn(vec![(200, b"hello")], "");
        let sleeper = Arc::new(RecordingSleep::default());
        let f = fetcher(3, sleeper.clone());
        let got = f
            .get(&url(&mock, "/x"), Kind::Chunk, None)
            .await
            .expect("200");

        assert_eq!(got.body, b"hello");
        assert_eq!(recorded(&f).attempts, 1);
        assert_eq!(recorded(&f).status, 200);
        assert_eq!(recorded(&f).bytes_in, 5);
        assert_eq!(recorded(&f).method, "GET");
        assert_eq!(recorded(&f).kind, Kind::Chunk);
        assert!(!recorded(&f).redirected);
        assert_eq!(
            recorded(&f).req_headers["user-agent"],
            crate::logging::user_agent()
        );
        assert_eq!(
            sleeper.delays(),
            Vec::<Duration>::new(),
            "no backoff on success"
        );
        assert_eq!(mock.hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_server_fault_is_retried_and_the_recovered_record_says_so() {
        let mock = spawn(
            vec![(503, b"busy"), (503, b"busy"), (200, b"recovered")],
            "",
        );
        let sleeper = Arc::new(RecordingSleep::default());
        let f = fetcher(3, sleeper.clone());
        let got = f
            .get(&url(&mock, "/x"), Kind::Api, None)
            .await
            .expect("third attempt succeeds");

        assert_eq!(got.body, b"recovered");
        assert_eq!(
            recorded(&f).attempts,
            3,
            "the provenance row must show the retries"
        );
        assert_eq!(mock.hits.load(Ordering::SeqCst), 3);
        assert_eq!(
            sleeper.delays(),
            vec![
                RetryPolicy::new(3).delay_before(2),
                RetryPolicy::new(3).delay_before(3),
            ],
            "exactly one wait between attempts, exponential, none before the first"
        );

        // Every attempt is logged, including the two that failed. A capture that
        // hit a throttle and recovered must show the 429s.
        let records = f.log().records();
        assert_eq!(records.len(), 3, "the failed attempts are part of the log");
        assert_eq!(
            records.iter().map(|r| r.status).collect::<Vec<_>>(),
            vec![503, 503, 200]
        );
        assert_eq!(
            records.iter().map(|r| r.seq).collect::<Vec<_>>(),
            vec![1, 2, 3],
            "seq is reserved at issue time, so the log stays in order"
        );
    }

    #[tokio::test]
    async fn a_404_is_not_retried_because_retrying_it_changes_nothing() {
        let mock = spawn(vec![(404, b"gone"), (200, b"never reached")], "");
        let sleeper = Arc::new(RecordingSleep::default());
        let f = fetcher(5, sleeper.clone());
        let err = f
            .get(&url(&mock, "/x"), Kind::Chunk, None)
            .await
            .expect_err("404 is fatal");

        match &err {
            ApiError::Status {
                status, attempts, ..
            } => {
                assert_eq!(*status, 404);
                assert_eq!(*attempts, 1);
            }
            other => panic!("expected a status error, got {other:?}"),
        }
        assert_eq!(
            mock.hits.load(Ordering::SeqCst),
            1,
            "the second answer is unused"
        );
        assert!(sleeper.delays().is_empty());
        assert_eq!(err.exit_code(), ExitCode::Network);
        assert_eq!(f.log().len(), 1, "the failure is still recorded");
    }

    #[tokio::test]
    async fn exhausting_the_attempts_fails_and_logs_every_one() {
        let mock = spawn(vec![(503, b"a"), (503, b"b"), (503, b"c")], "");
        let sleeper = Arc::new(RecordingSleep::default());
        let f = fetcher(3, sleeper.clone());
        let err = f
            .get(&url(&mock, "/x"), Kind::Api, None)
            .await
            .expect_err("all three failed");

        match &err {
            ApiError::Status { attempts, .. } => assert_eq!(*attempts, 3),
            other => panic!("expected a status error, got {other:?}"),
        }
        assert_eq!(mock.hits.load(Ordering::SeqCst), 3);
        assert_eq!(sleeper.delays().len(), 2, "no wait after the last attempt");
        assert_eq!(f.log().len(), 3);
    }

    #[tokio::test]
    async fn a_transport_failure_is_retried_and_reported_as_transport() {
        // Port 0 is not connectable, so every attempt is refused immediately:
        // on Windows a closed high port would instead wait ~2 s for its SYN
        // retry. The error under test is still a transport failure, not a status.
        let sleeper = Arc::new(RecordingSleep::default());
        let f = fetcher(2, sleeper.clone());
        let err = f
            .get("http://127.0.0.1:0/x", Kind::Api, None)
            .await
            .expect_err("nothing is listening");

        match &err {
            ApiError::Transport { attempts, .. } => {
                assert_eq!(*attempts, 2);
            }
            other => panic!("expected a transport error, got {other:?}"),
        }
        assert_eq!(sleeper.delays().len(), 1);
        let records = f.log().records();
        assert_eq!(records.len(), 2);
        assert!(
            records.iter().all(|r| r.status == 0),
            "no response means no status: {:?}",
            records.iter().map(|r| r.status).collect::<Vec<_>>()
        );
        assert!(err.url().unwrap().contains("127.0.0.1:0"));
    }

    /// A body to stream: big enough to arrive in several chunks, binary enough to
    /// catch a text-mode accident, and with a zstd magic at the front so it is a
    /// plausible blob rather than an arbitrary blob.
    fn streamable_body() -> Vec<u8> {
        let mut v = b"\x28\xb5\x2f\xfd".to_vec();
        for i in 0..60_000u32 {
            v.push((i.wrapping_mul(31) >> 7) as u8);
            v.extend_from_slice(b"payload ");
        }
        v
    }

    /// The mock serves &'static [u8] from its own thread, so a generated body has
    /// to outlive the test. Leaking is the honest way to say so in a test; a
    /// Cow would borrow instead and not compile.
    fn leak(v: &[u8]) -> &'static [u8] {
        Box::leak(v.to_vec().into_boxed_slice())
    }

    fn scratch(name: &str) -> girpr_tempdir::TempDir {
        girpr_tempdir::TempDir::new("gircap-stream", name)
    }

    /// A body the mock will send, `n` bytes of it.
    fn big_body(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i % 251) as u8).collect()
    }

    #[tokio::test]
    async fn observe_refuses_a_body_over_the_ceiling_rather_than_recording_an_empty_shape() {
        // The probe path is the one a hostile origin can most cheaply provoke — it is
        // a single unauthenticated GET — and its reads were unbounded. Over the
        // ceiling it must **fail**, not record an empty shape: an empty shape in
        // `api/error_shapes.json` would be a falsehood about what the origin did.
        //
        // Contrast with an *unreadable* body, which `observe` deliberately turns into
        // an empty body rather than an error. That asymmetry is the point, so it is
        // what the assertion pins.
        let body = big_body(4096);
        let mock = spawn(vec![(200, leak(&body))], "");
        let f = fetcher_with(
            1,
            Arc::new(RecordingSleep::default()),
            body.len() as u64 - 1,
        );

        let err = f
            .observe(&url(&mock, "/shape"), Kind::Probe)
            .await
            .expect_err("over the ceiling is a failure, not a shape");
        assert!(err.to_string().contains("ceiling"), "{err}");
    }

    #[tokio::test]
    async fn a_body_over_the_ceiling_is_refused_rather_than_buffered() {
        // **The whole point of the ceiling.** Without one, a broken or hostile
        // origin that keeps sending fills RAM until the process dies. A timeout
        // does not help: a server streaming a gigabyte steadily for ten minutes
        // never trips one.
        let body = big_body(4096);
        let mock = spawn(vec![(200, leak(&body)), (200, leak(&body))], "");
        let f = fetcher_with(
            1,
            Arc::new(RecordingSleep::default()),
            body.len() as u64 - 1,
        );

        let err = f
            .get(&url(&mock, "/big"), Kind::Chunk, None)
            .await
            .expect_err("a body over the ceiling must not be buffered");
        let text = err.to_string();
        assert!(
            text.contains("ceiling"),
            "the message must name the ceiling: {text}"
        );
        assert!(
            text.contains("--max-response-bytes"),
            "and how to raise it: {text}"
        );
        // **Not retried.** An origin that sent too much will send too much again,
        // and the mock had a second answer that must not have been consumed.
        assert_eq!(
            mock.hits.load(Ordering::SeqCst),
            1,
            "a `Shape` is not retried, so one attempt is all it gets"
        );
    }

    #[tokio::test]
    async fn a_body_under_the_ceiling_is_untouched() {
        // The other half, or the ceiling would be a tool that fetches nothing.
        // Byte-for-byte equality, so this also pins that routing the buffered path
        // through `drain_capped` did not alter what a caller receives.
        let body = big_body(4096);
        let mock = spawn(vec![(200, leak(&body))], "");
        let f = fetcher_with(1, Arc::new(RecordingSleep::default()), body.len() as u64);

        let got = f
            .get(&url(&mock, "/ok"), Kind::Chunk, None)
            .await
            .expect("a body exactly at the ceiling is allowed");
        assert_eq!(got.body, body, "the bytes must be exactly what was served");
        assert_eq!(got.body.len() as u64, body.len() as u64);
    }

    #[tokio::test]
    async fn a_declared_length_over_the_ceiling_is_refused_without_reading_it() {
        // The cheap half of the two checks: an origin that *says* it is sending
        // 10 GiB is refused before a byte is transferred, rather than after this
        // tool has spent the time and the bandwidth receiving it.
        //
        // Driven with `Content-Length` set to more than the body actually sent —
        // which is what `local_server::Answer::claim` is for, and is also the
        // shape of a lying origin.
        let body = big_body(64);
        let mock = spawn_answers(
            vec![Answer {
                status: 200,
                body: leak(&body),
                claim: Some(body.len() * 1000),
                omit_length: false,
            }],
            "",
        );
        let f = fetcher_with(1, Arc::new(RecordingSleep::default()), body.len() as u64);

        let err = f
            .get(&url(&mock, "/lying"), Kind::Chunk, None)
            .await
            .expect_err("a declared length over the ceiling must be refused");
        assert!(
            err.to_string().contains("ceiling"),
            "the message must name the ceiling: {err}"
        );
    }

    #[tokio::test]
    async fn the_file_destination_is_capped_too() {
        // A ceiling enforced in the buffered path and not the file path is a
        // ceiling with a hole in it — and the file path is the one that fetches
        // blobs, so it is the hole that matters.
        let body = big_body(2048);
        let mock = spawn(vec![(200, leak(&body))], "");
        let dir = scratch("file-cap");
        let path = dir.join("blob.bin");
        let mut out = crate::hash::HashingFile::create(&path).expect("create");
        let f = fetcher_with(1, Arc::new(RecordingSleep::default()), 16);

        let err = f
            .get_to_file(&url(&mock, "/big"), Kind::Chunk, &mut out)
            .await
            .expect_err("the file destination must be capped as well");
        assert!(err.to_string().contains("ceiling"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_body_over_the_ceiling_is_stopped_even_with_no_content_length() {
        // **The second check, on its own.** Found by mutation: with an accurate
        // `Content-Length` the pre-check always fires first, so disabling the running
        // total changed nothing — all 37 http tests passed. An origin that omits the
        // header is the only shape the pre-check cannot see, and without the running
        // total it is unbounded exactly as before this change.
        let body = big_body(4096);
        let mock = spawn_answers(
            vec![Answer {
                status: 200,
                body: leak(&body),
                claim: None,
                omit_length: true,
            }],
            "",
        );
        let f = fetcher_with(
            1,
            Arc::new(RecordingSleep::default()),
            body.len() as u64 - 1,
        );

        let err = f
            .get(&url(&mock, "/no-length"), Kind::Chunk, None)
            .await
            .expect_err("with no Content-Length the running total is the only defence");
        assert!(
            err.to_string().contains("ceiling"),
            "the message must name the ceiling: {err}"
        );
    }

    #[tokio::test]
    async fn observe_records_a_server_error_once_and_never_retries() {
        // `kind: "probe"` covers two paths with different retry behaviour: the tag
        // probe goes through `get` and the retry policy, the negative probes go
        // through `observe`, which sends once and records `attempts: 1`
        // unconditionally.
        //
        // All 8 probe rows in the reference bundle have `attempts = 1`, so a reader
        // cannot tell the two apart from the log — which is why that is documented
        // rather than split into two kinds. What *can* be pinned is the asymmetry the
        // documentation asserts, and this is it: a `500`, which the retry policy
        // treats as retryable, is still recorded once by `observe`.
        let mock = spawn(
            vec![(503, b"<Error><Code>Busy</Code></Error>"), (200, b"unused")],
            "",
        );
        let f = fetcher(3, Arc::new(RecordingSleep::default()));

        let o = f
            .observe(&url(&mock, "/shape"), Kind::Probe)
            .await
            .expect("a 5xx is data for a probe, not a failure");
        assert_eq!(o.status, 503, "the status is recorded as received");
        assert!(o.text().contains("Busy"), "and so is the body");
        assert_eq!(
            mock.hits.load(Ordering::SeqCst),
            1,
            "observe must not retry: the mock only had one answer for it to consume"
        );

        let rows: Vec<_> = f.log().records();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].attempts, 1,
            "`attempts: 1` is hardcoded, and is the claim"
        );
        assert_eq!(rows[0].kind, Kind::Probe);
    }

    #[tokio::test]
    async fn a_failed_request_counts_the_body_it_threw_away() {
        // `bytes_in` must price every request, not only the successful ones: a
        // `429` carrying a few KB of XML envelope is transferred, and the requests
        // it would under-report are the retried ones — the expensive ones.
        let envelope = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Error>\n  \
                         <Code>TooManyRequests</Code>\n  <Message>Slow down.</Message>\n\
                         </Error>\n";
        let mock = spawn(vec![(429, envelope)], "");
        let f = fetcher(1, Arc::new(RecordingSleep::default()));

        let err = f
            .get(&url(&mock, "/blob"), Kind::Chunk, None)
            .await
            .expect_err("429 is not a success");
        assert!(
            matches!(err, ApiError::Status { status: 429, .. }),
            "{err:?}"
        );

        let rows: Vec<_> = f.log().records();
        assert_eq!(rows.len(), 1, "the failure must still be recorded");
        assert_eq!(
            rows[0].bytes_in,
            envelope.len() as u64,
            "bytes_in must count the body that arrived, not report the failure as free"
        );
        assert_eq!(rows[0].status, 429);
    }

    #[tokio::test]
    async fn a_rejected_request_is_priced_at_every_attempt_it_cost() {
        // The shape the module doc promises: a run that was throttled and retried
        // paid for every response it received, and the summary's total has to say
        // so. Two `429`s and a `200` is a run that cost three bodies.
        let envelope = b"<Error><Code>TooManyRequests</Code></Error>";
        let payload = b"the blob, at last";
        let mock = spawn_answers(
            vec![
                Answer {
                    status: 429,
                    body: envelope,
                    claim: None,
                    omit_length: false,
                },
                Answer {
                    status: 503,
                    body: envelope,
                    claim: None,
                    omit_length: false,
                },
                Answer {
                    status: 200,
                    body: leak(payload),
                    claim: None,
                    omit_length: false,
                },
            ],
            "",
        );
        let f = fetcher(3, Arc::new(RecordingSleep::default()));

        let got = f
            .get(&url(&mock, "/blob"), Kind::Chunk, None)
            .await
            .expect("the third attempt succeeds");

        assert_eq!(got.body, payload, "the successful body is the third one");
        assert_eq!(mock.hits.load(Ordering::SeqCst), 3, "it retried twice");

        // Every attempt is logged — a run that recovered from a throttle must
        // show the 429s, or `attempts` is a claim nobody can check.
        let rows: Vec<_> = f.log().records();
        assert_eq!(rows.len(), 3, "all three attempts are recorded");
        let failed: u64 = rows[..2].iter().map(|r| r.bytes_in).sum();
        assert_eq!(
            failed,
            envelope.len() as u64 * 2,
            "both throttled responses must be priced, not reported as free"
        );
        assert_eq!(rows[2].bytes_in, payload.len() as u64);
        assert_eq!(
            f.log().bytes_in(),
            envelope.len() as u64 * 2 + payload.len() as u64,
            "the run's total is every body it received"
        );
    }

    #[tokio::test]
    async fn a_streamed_fetch_writes_the_body_and_reports_the_digests_of_those_bytes() {
        // The buffered path is the reference: this asserts the streamed path
        // produced the same bytes on disk and the same digests and the same
        // `bytes_in`, so nothing downstream — `chunks/index.json`'s md5, A6's four
        // checks, the size in `MANIFEST.json` — can tell the two apart.
        let body = streamable_body();
        // Two mocks, because the server answers exactly as many requests as it
        // was given answers and this test makes two.
        let buffered_mock = spawn(vec![(200, leak(&body))], "");
        let streamed_mock = spawn(vec![(200, leak(&body))], "");
        let dir = scratch("same-as-buffered");
        let f = fetcher(1, Arc::new(RecordingSleep::default()));

        let buffered = f
            .get(&url(&buffered_mock, "/blob"), Kind::Chunk, None)
            .await
            .expect("buffered");

        let path = dir.join("streamed.bin");
        let mut out = crate::hash::HashingFile::create(&path).expect("create");
        let streamed = f
            .get_to_file(&url(&streamed_mock, "/blob"), Kind::Chunk, &mut out)
            .await
            .expect("streamed");
        let digests = out.finish().expect("finish");

        let on_disk = std::fs::read(&path).expect("read back");
        assert_eq!(on_disk.len(), body.len(), "length moved");
        assert!(
            on_disk == body,
            "the bytes on disk are not the bytes served"
        );
        assert_eq!(streamed.bytes_in, buffered.body.len() as u64);
        assert_eq!(streamed.status, buffered.status);
        assert_eq!(digests, crate::hash::md5_sha256(&body));
        assert_eq!(digests.bytes, body.len() as u64);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn the_digests_of_a_retried_blob_cover_only_the_body_that_landed() {
        // **The failure this change had to get right, and the one the reference
        // bundle cannot check.** A healthy capture never retries, so the 91-of-97
        // gate says nothing about what happens when a body is truncated half way
        // through and the next attempt succeeds.
        //
        // The first answer under-delivers: it promises a length and sends less,
        // so the client has *already written part of it* when the read fails. The
        // digests must then describe the second answer alone — not the
        // concatenation, and not a prefix of the first.
        let first = streamable_body();
        let second = b"<?xml version=\"1.0\"?><Error><Code>NoSuchKey</Code></Error>".to_vec();
        let mock = spawn_answers(
            vec![
                Answer {
                    status: 200,
                    // Announce the whole length, send a third of it, then hang
                    // up — a truncated response. `claim` is what the header says
                    // and `body` is what goes on the wire, so they must differ.
                    body: leak(&first[..first.len() / 3]),
                    claim: Some(first.len()),
                    omit_length: false,
                },
                Answer {
                    status: 200,
                    body: leak(&second),
                    claim: None,
                    omit_length: false,
                },
            ],
            "",
        );
        let dir = scratch("retry-digests");
        let path = dir.join("blob.bin");
        let mut out = crate::hash::HashingFile::create(&path).expect("create");

        // Two attempts allowed, and a sleeper that does not sleep: the backoff is
        // not what is under test and a real one would cost a second per retry.
        let f = fetcher(2, Arc::new(RecordingSleep::default()));
        let got = f
            .get_to_file(&url(&mock, "/blob"), Kind::Chunk, &mut out)
            .await
            .expect("the retry should succeed");
        let digests = out.finish().expect("finish");

        assert_eq!(
            mock.hits.load(Ordering::SeqCst),
            2,
            "it should have retried"
        );
        assert_eq!(
            digests,
            crate::hash::md5_sha256(&second),
            "digests must be the second body"
        );
        assert_eq!(digests.bytes, second.len() as u64);
        assert_eq!(got.bytes_in, second.len() as u64);

        let on_disk = std::fs::read(&path).expect("read back");
        assert_eq!(
            on_disk.len(),
            second.len(),
            "the file must not still hold the tail of the truncated attempt"
        );
        assert!(on_disk == second, "the file holds the wrong bytes");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_write_failure_is_not_retried_as_if_it_were_the_network() {
        // A disk that will not take the bytes must stop the run, not spend the
        // retry budget rediscovering that a full disk stays full. Reported as a
        // `Transport` it would be retried, because a transport failure is
        // retryable by policy.
        //
        // **Tested at the classifier rather than through a real full disk.** A
        // portable way to make one is not available, and the decision is a
        // two-line branch: what matters is that the two causes land on different
        // variants, and that the policy then treats them differently.
        let read = body_failure("http://x/blob", 2, true, "connection reset");
        assert!(
            matches!(read, ApiError::Transport { .. }),
            "a read failure is the network's fault and must be retryable, got {read:?}"
        );

        let write = body_failure("http://x/blob", 2, false, "no space left on device");
        assert!(
            matches!(write, ApiError::Shape { .. }),
            "a write failure is the disk's fault and must not be retried, got {write:?}"
        );

        // The two halves of the claim: the policy keys off the variant, and the
        // variants differ. (A `Transport` retries; a `Shape` does not — see
        // `run`.)
        let retryable = |e: &ApiError| match e {
            ApiError::Transport { .. } => true,
            ApiError::Shape { .. } => false,
            ApiError::Status { .. } => true,
        };
        assert!(retryable(&read));
        assert!(!retryable(&write));
        assert!(
            write.to_string().contains("no space left on device"),
            "the local cause must survive into the message a reader sees: {write}"
        );
    }

    #[tokio::test]
    async fn a_range_request_sends_the_literal_header_and_records_it() {
        let mock = spawn(
            vec![(206, b"0123456789")],
            "Content-Range: bytes 0-9/100\r\n",
        );
        let sleeper = Arc::new(RecordingSleep::default());
        let f = fetcher(1, sleeper.clone());
        let got = f
            .get(&url(&mock, "/m"), Kind::Manifest, Some("bytes=0-9"))
            .await
            .expect("206");

        assert_eq!(got.body, b"0123456789");
        assert_eq!(recorded(&f).status, 206, "a partial response is a success");
        assert_eq!(recorded(&f).range.as_deref(), Some("bytes=0-9"));
        assert_eq!(recorded(&f).req_headers["range"], "bytes=0-9");
        assert_eq!(recorded(&f).resp_headers["content-range"], "bytes 0-9/100");
        assert_eq!(recorded(&f).bytes_in, 10);

        let seen = mock.requests.lock().unwrap().clone();
        assert!(
            seen[0].to_ascii_lowercase().contains("range: bytes=0-9"),
            "the header never reached the server: {seen:?}"
        );
    }

    #[tokio::test]
    async fn a_response_without_a_range_sends_no_range_header() {
        let mock = spawn(vec![(200, b"whole")], "");
        let f = fetcher(1, Arc::new(RecordingSleep::default()));
        f.get(&url(&mock, "/m"), Kind::Manifest, None)
            .await
            .unwrap();
        let got = recorded(&f);
        assert_eq!(got.range, None);
        assert!(!got.req_headers.contains_key("range"));
        assert!(
            !mock.requests.lock().unwrap()[0]
                .to_ascii_lowercase()
                .contains("range:"),
            "an absent --range must not be sent as an empty header"
        );
    }

    #[tokio::test]
    async fn response_headers_are_lowercased_and_kept_verbatim() {
        let mock = spawn(vec![(200, b"x")], "");
        let f = fetcher(1, Arc::new(RecordingSleep::default()));
        f.get(&url(&mock, "/x"), Kind::Api, None).await.unwrap();
        let got = recorded(&f);
        assert!(
            got.resp_headers
                .keys()
                .all(|k| k.chars().all(|c| !c.is_ascii_uppercase()))
        );
        assert_eq!(got.resp_headers["server"], "MockOSS");
        assert_eq!(got.resp_headers["etag"], "\"abc\"", "quotes are kept");
    }

    #[tokio::test]
    async fn a_zero_retcode_is_accepted_and_a_nonzero_one_is_exit_two() {
        let mock = spawn(vec![(200, br#"{"retcode":0,"data":{"tag":"7.1.0"}}"#)], "");
        let f = fetcher(1, Arc::new(RecordingSleep::default()));
        let (body, v) = f.get_json(&url(&mock, "/g"), Kind::Api).await.unwrap();
        assert_eq!(v["data"]["tag"], "7.1.0");
        assert!(body.starts_with(b"{"));

        // The documented way this API says "no such tag": HTTP 200, retcode -202.
        let mock2 = spawn(
            vec![(
                200,
                br#"{"data":null,"message":"not found","retcode":-202}"#,
            )],
            "",
        );
        let f2 = fetcher(1, Arc::new(RecordingSleep::default()));
        let err = f2
            .get_json(&url(&mock2, "/g?tag=0.0.1"), Kind::Api)
            .await
            .expect_err("retcode -202");
        assert_eq!(err.exit_code(), ExitCode::Network);
        assert!(err.to_string().contains("-202"), "{err}");
        assert!(err.to_string().contains("not found"), "{err}");
    }

    /// A response that is not a wrapper fails, and is **not** retried: a `200` that
    /// is an HTML error page is a wrong answer, and asking again makes it a
    /// slower wrong answer.
    #[tokio::test]
    async fn a_body_that_is_not_a_wrapper_is_an_error_not_a_silent_pass() {
        let mock = spawn(vec![(200, b"<html>404</html>")], "");
        let f = fetcher(1, Arc::new(RecordingSleep::default()));
        let err = f.get_json(&url(&mock, "/g"), Kind::Api).await.unwrap_err();
        assert!(err.to_string().contains("retcode"), "{err}");
        assert_eq!(mock.hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn the_log_is_written_to_disk_as_ndjson() {
        let dir = girpr_tempdir::TempDir::new("gircap-http", "written_to_disk_as_ndjson");
        let path = dir.join("api").join("requests.ndjson");

        let mock = spawn(vec![(200, b"body")], "");
        let f = fetcher(1, Arc::new(RecordingSleep::default()));
        f.get(&url(&mock, "/x"), Kind::Api, None).await.unwrap();
        let n = f.log().write_ndjson(&path).unwrap();

        assert_eq!(n, 1);
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 1);
        let v: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        assert_eq!(v["kind"], "api");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn kinds_are_wire_spelled() {
        assert_eq!(Kind::Api.as_str(), "api");
        assert_eq!(Kind::Manifest.to_string(), "manifest");
        assert_eq!(Kind::Chunk.to_string(), "chunk");
        assert_eq!(Kind::Probe.to_string(), "probe");
        assert_eq!(serde_json::to_string(&Kind::Probe).unwrap(), "\"probe\"");
    }

    #[test]
    fn every_api_error_maps_to_exit_two() {
        for e in [
            ApiError::Transport {
                url: "u".into(),
                attempts: 1,
                detail: "d".into(),
            },
            ApiError::Status {
                url: "u".into(),
                status: 500,
                attempts: 1,
            },
            ApiError::shape("s", "d"),
        ] {
            assert_eq!(e.exit_code(), ExitCode::Network);
            assert!(!e.to_string().is_empty());
        }
    }
}
