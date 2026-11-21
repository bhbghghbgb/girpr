use super::*;
use crate::biz::{Biz, BranchParams};
use crate::cli::Biz as BizId;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

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

// ----------------------------------------------------------------- log --

// --------------------------------------------------------- mock servers --

// Two test servers with deliberately different jobs.
//
// `canned` feeds `reqwest` a scripted status/body pair through the
// dependency-free loopback leaf, which owns the listener, the invocation
// counts and the request journal. `malformed` stays a small raw-socket
// helper, because a well-behaved mock server cannot announce a
// `Content-Length` larger than the body it sends, or omit the header
// entirely — and those two shapes are the whole point of those tests.

/// Canned replies, served on a dependency-free loopback server.
mod canned {
    use super::*;
    use girpr_loopback_mock::{RecordedRequest, Reply, ScriptedServer};

    /// A canned server. The loopback leaf owns the listener, the reply writer
    /// and the request journal; this module keeps gircap's own scripting
    /// contract (which headers are sent and which counts are pinned).
    pub struct Mock {
        server: ScriptedServer,
    }

    impl Mock {
        /// How many requests reached the server.
        pub async fn hits(&self) -> usize {
            self.server.hits()
        }

        /// The requests recorded, for asserting what went on the wire.
        pub async fn requests(&self) -> Vec<RecordedRequest> {
            self.server.requests()
        }

        /// `http://127.0.0.1:<port>`.
        pub fn uri(&self) -> String {
            self.server.uri()
        }
    }

    /// Serve `statuses` in order, one reply per request, on any path.
    ///
    /// `extra` are response headers the product records (`content-range`,
    /// `set-cookie`, ...) but that are not part of the canned body. Every
    /// reply also carries the `content-type` and `etag` the tests pin.
    pub async fn spawn(
        statuses: Vec<(u16, &'static [u8])>,
        extra: &[(&'static str, &'static str)],
    ) -> Mock {
        let expected = statuses.len() as u64;
        spawn_expect(statuses, expected, extra).await
    }

    /// [`spawn`] with an explicit expected request count. Used by the tests
    /// that deliberately script an answer they must **not** consume: the
    /// count is verified when the server is dropped.
    pub async fn spawn_expect(
        statuses: Vec<(u16, &'static [u8])>,
        expect: u64,
        extra: &[(&'static str, &'static str)],
    ) -> Mock {
        let replies = statuses
            .into_iter()
            .map(|(status, body)| {
                let mut reply = Reply::new(status, body)
                    .header("content-type", "application/octet-stream")
                    .header("etag", "\"abc\"");
                for (name, value) in extra {
                    reply = reply.header(name, value);
                }
                reply
            })
            .collect();
        Mock {
            server: ScriptedServer::start_expecting(replies, expect),
        }
    }
}

/// The one raw-socket helper that stays: malformed responses.
mod malformed {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::{TcpListener, TcpStream};

    /// One scripted answer, and how much of it is actually delivered.
    ///
    /// `claim` is the `Content-Length` the server announces. Setting it
    /// larger than `body.len()` makes the server promise more than it sends
    /// and then hang up — **a truncated response**, the one failure a real
    /// CDN produces and the only way to exercise the retry path from
    /// *mid-body*. `omit_length` sends no header at all, the one shape a
    /// `Content-Length` pre-check cannot see.
    pub struct Answer {
        pub status: u16,
        pub body: &'static [u8],
        pub claim: Option<usize>,
        pub omit_length: bool,
    }

    pub struct Mock {
        pub port: u16,
        hits: Arc<AtomicU32>,
    }

    impl Mock {
        pub async fn hits(&self) -> usize {
            self.hits.load(Ordering::SeqCst) as usize
        }
    }

    pub fn spawn_answers(answers: Vec<Answer>) -> Mock {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        let hits = Arc::new(AtomicU32::new(0));
        let h = Arc::clone(&hits);
        std::thread::spawn(move || {
            for a in answers {
                let Ok((stream, _)) = listener.accept() else {
                    return;
                };
                h.fetch_add(1, Ordering::SeqCst);
                serve_one(stream, a.status, a.body, a.claim, a.omit_length);
            }
            // The script is exhausted. A well-behaved client stops here, but a
            // regression that makes one more request must fail fast rather than
            // hang until the client's timeout: keep accepting and close any
            // extra connection immediately so the caller sees EOF at once
            // instead of an unanswered socket sitting in the listen backlog.
            while let Ok((stream, _)) = listener.accept() {
                drop(stream);
            }
        });
        Mock { port, hits }
    }

    fn serve_one(
        mut stream: TcpStream,
        status: u16,
        body: &'static [u8],
        claim: Option<usize>,
        omit_length: bool,
    ) {
        // Read the request off the socket so the client does not see a response
        // before it has written one. Only the status line is needed here;
        // the malformed tests do not assert on the request journal.
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).ok();

        let head = format!(
            "HTTP/1.1 {status} X\r\n{}Content-Type: application/octet-stream\r\nETag: \"abc\"\r\nServer: MockOSS\r\nConnection: close\r\n\r\n",
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
}

use canned::{spawn, spawn_expect};
use malformed::{Answer, spawn_answers};

/// The base URL of either mock, so one `url` helper serves both.
trait BaseUrl {
    fn base(&self) -> String;
}

impl BaseUrl for canned::Mock {
    fn base(&self) -> String {
        self.uri()
    }
}

impl BaseUrl for malformed::Mock {
    fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

fn url<M: BaseUrl>(mock: &M, path: &str) -> String {
    format!("{}{path}", mock.base())
}

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

#[tokio::test]
async fn a_404_is_not_retried_because_retrying_it_changes_nothing() {
    let mock = spawn_expect(vec![(404, b"gone")], 1, &[]).await;
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
    assert_eq!(mock.hits().await, 1, "a 404 is not retried");
    assert!(sleeper.delays().is_empty());
    assert_eq!(err.exit_code(), ExitCode::Network);
    assert_eq!(f.log().len(), 1, "the failure is still recorded");
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
    let mock = spawn(vec![(200, leak(&body))], &[]).await;
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
async fn the_file_destination_is_capped_too() {
    // A ceiling enforced in the buffered path and not the file path is a
    // ceiling with a hole in it — and the file path is the one that fetches
    // blobs, so it is the hole that matters.
    let body = big_body(2048);
    let mock = spawn(vec![(200, leak(&body))], &[]).await;
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
    let mock = spawn_expect(vec![(503, b"<Error><Code>Busy</Code></Error>")], 1, &[]).await;
    let f = fetcher(3, Arc::new(RecordingSleep::default()));

    let o = f
        .observe(&url(&mock, "/shape"), Kind::Probe)
        .await
        .expect("a 5xx is data for a probe, not a failure");
    assert_eq!(o.status, 503, "the status is recorded as received");
    assert!(o.text().contains("Busy"), "and so is the body");
    assert_eq!(
        mock.hits().await,
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

/// The same accounting on the destination that fetches chunk blobs.
///
/// `Attempt` has two failure closures — one per destination — and the bug
/// was hard-coded in both. A test of one would not pin the other.
#[tokio::test]
async fn a_truncated_success_body_is_priced_at_what_arrived_for_a_file_too() {
    let sent = b"\x28\xb5\x2f\xfdpartial frame that stops";
    let full = b"\x28\xb5\x2f\xfdthe whole frame on the retry";
    let mock = spawn_answers(vec![
        Answer {
            status: 200,
            body: sent,
            claim: Some(sent.len() * 10),
            omit_length: false,
        },
        Answer {
            status: 200,
            body: full,
            claim: None,
            omit_length: false,
        },
    ]);
    let dir = scratch("truncated-file");
    let path = dir.join("blob.bin");
    let mut out = crate::hash::HashingFile::create(&path).expect("create");
    let f = fetcher(3, Arc::new(RecordingSleep::default()));

    let got = f
        .get_to_file(&url(&mock, "/blob"), Kind::Chunk, &mut out)
        .await
        .expect("the retry succeeds and restarts the file");
    assert_eq!(got.bytes_in, full.len() as u64);
    assert_eq!(
        std::fs::read(&path).expect("the stored file"),
        full,
        "the retry's body replaced the partial one"
    );

    let rows = f.log().records();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0].bytes_in,
        sent.len() as u64,
        "the file destination's failed attempt is priced the same way"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_streamed_fetch_writes_the_body_and_reports_the_digests_of_those_bytes() {
    // The buffered path is the reference: this asserts the streamed path
    // produced the same bytes on disk and the same digests and the same
    // `bytes_in`, so nothing downstream — `chunks/index.json`'s md5, A6's four
    // checks, the size in `MANIFEST.json` — can tell the two apart.
    let body = streamable_body();
    // Two servers, each with `expect(1)`: this test makes two requests, one
    // buffered and one streamed, and neither should hit the other's script.
    let buffered_mock = spawn(vec![(200, leak(&body))], &[]).await;
    let streamed_mock = spawn(vec![(200, leak(&body))], &[]).await;
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
    let mock = spawn_answers(vec![
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
    ]);
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

    assert_eq!(mock.hits().await, 2, "it should have retried");
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
async fn a_zero_retcode_is_accepted_and_a_nonzero_one_is_exit_two() {
    let mock = spawn(vec![(200, br#"{"retcode":0,"data":{"tag":"7.1.0"}}"#)], &[]).await;
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
        &[],
    )
    .await;
    let f2 = fetcher(1, Arc::new(RecordingSleep::default()));
    let err = f2
        .get_json(&url(&mock2, "/g?tag=0.0.1"), Kind::Api)
        .await
        .expect_err("retcode -202");
    assert_eq!(err.exit_code(), ExitCode::Network);
    assert!(err.to_string().contains("-202"), "{err}");
    assert!(err.to_string().contains("not found"), "{err}");
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
