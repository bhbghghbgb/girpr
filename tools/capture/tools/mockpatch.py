#!/usr/bin/env python3
"""
Patch `tests/common/mod.rs` so the loopback mock can be told to misbehave.

Three scenarios the default world cannot produce, and which are exactly the three
places the artifact contract changed for degraded input:

  arm_fail  - answer a matching path with a status and a *body*, `n` times.
              Exercises: a non-2xx on the retryable path, so `bytes_in` on a
              failed attempt (844f5c2) and `attempts` (527833c) are observable.

  arm_body  - answer a matching path 200 with these bytes instead of the real
              ones.  Serving bytes that are not a zstd frame is the mis-served
              chunk 54ff12d is about.

The patch is textual and anchored, and it refuses to apply if an anchor has
moved rather than half-applying.  It is idempotent.

It changes the mock only, never the tool, and the happy path is untouched: an
unarmed mock behaves exactly as the tree's own.  The control for that is to
fingerprint a patched-mock run against an unpatched one.
"""
import sys
from pathlib import Path

MARK = "// ---- gircap-probe: hostile-input instrumentation ----"

HEADER = MARK + """
use std::sync::OnceLock as _ProbeOnceLock;

/// One armed misbehaviour: a path substring, and what to do instead.
#[derive(Clone)]
pub struct _ProbeRule {
    pub needle: String,
    pub status: u16,
    pub body: Vec<u8>,
    pub remaining: usize,
    pub max: usize,
}

static _PROBE_RULES: _ProbeOnceLock<std::sync::Mutex<Vec<_ProbeRule>>> = _ProbeOnceLock::new();

fn _probe_take(path: &str) -> Option<(u16, Vec<u8>, bool)> {
    let cell = _PROBE_RULES.get()?;
    let mut rules = cell.lock().expect("probe rules are not poisoned");
    // Longest needle first, so a narrow rule beats a broad one on the same path.
    let idx = rules
        .iter()
        .enumerate()
        .filter(|(_, r)| path.contains(r.needle.as_str()))
        .max_by_key(|(_, r)| r.needle.len())
        .map(|(i, _)| i)?;
    let r = &mut rules[idx];
    if r.remaining == 0 {
        return None;
    }
    r.remaining -= 1;
    Some((r.status, r.body.clone(), true))
}

/// Re-arm every rule. The probe captures twice and must give both the same
/// world; without this the second capture sees a mock that has already spent
/// its one armed answer, and the determinism guard rejects the commit for a
/// reason that has nothing to do with the tool.
pub fn _probe_reset() {
    if let Some(cell) = _PROBE_RULES.get() {
        let mut rules = cell.lock().expect("probe rules are not poisoned");
        for r in rules.iter_mut() {
            r.remaining = r.max;
        }
    }
}

pub fn _probe_arm(needle: &str, status: u16, body: Vec<u8>, times: usize) {
    let cell = _PROBE_RULES.get_or_init(|| std::sync::Mutex::new(Vec::new()));
    let mut rules = cell.lock().expect("probe rules are not poisoned");
    rules.retain(|r| r.needle != needle);
    rules.push(_ProbeRule {
        needle: needle.to_string(),
        status,
        body,
        remaining: times,
        max: times,
    });
}
"""

ANCHOR_IMPL = "impl Mock {"
IMPL_ADD = """
    /// gircap-probe: answer `needle` with `status` and `body`, `times` times.
    pub fn arm_fail(self, needle: &str, status: u16, body: &[u8], times: usize) -> Mock {
        _probe_arm(needle, status, body.to_vec(), times);
        self
    }

    /// gircap-probe: answer `needle` 200 with `body`, `times` times.
    pub fn arm_body(self, needle: &str, body: &[u8], times: usize) -> Mock {
        _probe_arm(needle, 200, body.to_vec(), times);
        self
    }
"""

ANCHOR_ALLOW = "#![allow(dead_code)]"
# Anchors are whole lines, and are checked as whole lines.  A prefix is not an
# anchor: a truncated prefix matches fewer characters than intended and produces
# a brace mismatch far from the edit, which is not a useful error message.

# The mock's `serve` had two shapes across the range.  428fc8e made it speak
# keep-alive, which replaced the `let Some(bytes) = respond(..) else` early return
# with an `outcome` match.  Both anchors are tried; neither found is an error.
#
# Each block is INSERTED BEFORE its anchor, never in place of it: the original
# line has to survive, or the construct it opens never closes.  Getting that
# wrong produced a brace mismatch 50 lines later and a very confusing error.
# The needle is matched against the full request target, path *and* query: the
# negative probes share their paths with the metadata capture
# (`/sophon/getBuild`, `/hyp/getGameConfigs`), so a path-only match cannot tell
# "the capture asked for this" from "the probe asked for this on purpose".
# What distinguishes them is in the query.
RESPOND_VARIANTS = [
    (
        "        let outcome = match respond(table, &path, &query, tag.as_deref())",
        """        // gircap-probe: an armed rule overrides whatever the world says.
        if let Some((status, body, _fired)) = _probe_take(&format!("{path}?{query}")) {
            let ct = if status == 200 { "application/octet-stream" } else { "text/plain" };
            let outcome = write_response(&mut stream, status, ct, &body, keep_alive);
            outcome?;
            if !keep_alive {
                return Ok(());
            }
            continue;
        }
""",
    ),
    (
        "    let Some(bytes) = respond(table, &path, &query, tag.as_deref()) else {",
        """    // gircap-probe: an armed rule overrides whatever the world says.
    if let Some((status, body, _fired)) = _probe_take(&format!("{path}?{query}")) {
        let ct = if status == 200 { "application/octet-stream" } else { "text/plain" };
        return write_response(&mut stream, status, ct, &body);
    }
""",
    ),
]


def _lines_matching(lines, anchor):
    """Indices of lines that *begin* with `anchor`.

    Prefix-on-a-line, not substring: the HEAD shape writes
    `... tag.as_deref()) {` and the earlier one writes `... else {`, so the two
    anchors deliberately omit whatever follows.  Anything looser than a line
    boundary would match inside a comment.
    """
    return [i for i, ln in enumerate(lines) if ln.rstrip().startswith(anchor)]


def apply(path):
    p = Path(path)
    text = p.read_text(encoding="utf-8")
    if MARK in text:
        return "already patched"
    lines = text.splitlines()
    nl = "\r\n" if "\r\n" in text else "\n"

    # Required anchors: each must match exactly one line.  A *shorter* prefix
    # than intended silently drops a `]` and produces a syntax error fifty lines
    # later.
    # (anchor, block, where).  `where` matters: an inner attribute must stay first
    # in the file and an `impl` header must stay first in its block, so their
    # blocks go *after*.  Only the `serve` dispatch goes *before* its anchor,
    # because that anchor opens a construct the block returns out of.
    plan = []
    for anchor, block, where in (
        (ANCHOR_ALLOW, HEADER, "after"),
        (ANCHOR_IMPL, IMPL_ADD, "after"),
    ):
        hits = _lines_matching(lines, anchor)
        if len(hits) != 1:
            raise SystemExit(
                f"refusing to patch {p}: anchor {anchor!r} matches {len(hits)} lines, "
                f"expected 1.\nThe mock's shape moved; the patch would half-apply."
            )
        plan.append((hits[0], block, where))

    # The `serve` dispatch has two shapes across this range, so exactly one of
    # the variants must match - not all of them, and not none.
    matched = [(a, b, _lines_matching(lines, a)) for a, b in RESPOND_VARIANTS]
    matched = [(a, b, h) for a, b, h in matched if h]
    if len(matched) != 1 or len(matched[0][2]) != 1:
        detail = ", ".join(f"{a!r}->{len(h)}" for a, _, h in matched)
        raise SystemExit(
            f"refusing to patch {p}: expected exactly one `serve` dispatch anchor "
            f"to match exactly one line; got {len(matched)} variants matching ({detail}).\n"
            f"The mock's shape moved; the patch would half-apply."
        )
    a, b, h = matched[0]
    plan.append((h[0], b, "before"))

    # Splice from the bottom up so earlier indices stay valid.  "after" inserts
    # at at+1; writing at at:at+1 would *replace* the anchor and silently drop a
    # `{` - which the brace guard below is there to catch.
    for at, block, where in sorted(plan, key=lambda t: t[0], reverse=True):
        body = block.rstrip("\n").split("\n")
        at = at + 1 if where == "after" else at
        lines[at:at] = body

    out = nl.join(lines) + ("\r\n" if nl == "\r\n" else "\n")
    d_open = out.count("{") - text.count("{")
    d_close = out.count("}") - text.count("}")
    if d_open != d_close:
        raise SystemExit(
            f"refusing to write {p}: the inserted block adds {d_open} `{{` and "
            f"{d_close} `}}`. It is malformed."
        )
    for anchor in (ANCHOR_ALLOW, ANCHOR_IMPL, a):
        if len(_lines_matching(out.splitlines(), anchor)) != 1:
            raise SystemExit(f"refusing to write {p}: anchor {anchor!r} did not survive")
    p.write_text(out, encoding="utf-8")
    return "patched"


if __name__ == "__main__":
    for a in sys.argv[1:]:
        print(f"{a}: {apply(a)}")