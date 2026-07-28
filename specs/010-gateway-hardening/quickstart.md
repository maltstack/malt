# Quickstart: Gateway Hardening

**Feature**: `specs/010-gateway-hardening/`

**Every scenario here must observe the property, not the status code.** That
is this feature's specific trap (research R7): each requirement has an easier
observable that passes while the defect is fully present.

| The easy assertion | Why it proves nothing |
|---|---|
| "429 after N requests" | Today's limiter already does this correctly. **Recovery** is what is broken, and a test that never waits will never see it |
| "413 for a large body" | Says nothing about whether the body was buffered first — which is the actual harm |
| "one map entry per client" | Says nothing about whether entries are ever released |
| "a `Retry-After` header exists" | Says nothing about whether its value works |

## Prerequisites

```bash
cargo build --workspace
```

```bash
./target/debug/malt daemon --port 7980
```

Requests need a token; two *distinct* clients need two *distinct valid
tokens*, because `client_id` is the bearer token itself (research R2). Two
arbitrary strings will not do.

---

## Scenario 1 (US1) — a throttled caller recovers

Exhaust the allowance:

```bash
for i in $(seq 1 200); do curl -s -o /dev/null -w "%{http_code} " -H "Authorization: Bearer $TOKEN" http://127.0.0.1:7980/sessions; done
```

**Expected**: 200s, then 429s.

Now wait one window and retry:

```bash
sleep 60 && curl -s -o /dev/null -w "%{http_code}\n" -H "Authorization: Bearer $TOKEN" http://127.0.0.1:7980/sessions
```

**Expected**: `200`.

**This is the whole feature.** Today it returns 429 forever — only a daemon
restart clears it. The assertion that matters is that **time alone** restored
service: no restart, and no code calling `refill`.

---

## Scenario 2 (US1) — one caller's exhaustion does not touch another

With client A exhausted from Scenario 1, using a second valid token:

```bash
curl -s -o /dev/null -w "%{http_code}\n" -H "Authorization: Bearer $TOKEN_B" http://127.0.0.1:7980/sessions
```

**Expected**: `200`, immediately.

---

## Scenario 3 (US1) — state does not accumulate

Issue requests from many distinct tokens, let their windows elapse, and
observe the daemon's memory return to roughly its prior level.

**Expected**: bounded. **Verified by measurement**, not by reasoning about the
data structure (SC-008) — an entry per token that is never released is the
current behaviour and looks identical from the outside until you measure.

If this cannot be observed reliably in a test, **say so and state what is
asserted instead.** Do not silently downgrade to counting map entries; that is
research R7 exactly.

---

## Scenario 4 (US2) — an oversized body is refused before it is buffered

```bash
head -c 100000000 /dev/urandom | base64 > /tmp/big.txt
```

```bash
curl -s -o /dev/null -w "%{http_code}\n" -X POST -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" --data-binary @/tmp/big.txt http://127.0.0.1:7980/sessions/1/exec
```

**Expected**: `413`.

**And**: the daemon's resident memory does not rise by the payload size while
this happens (SC-004). Watch it during the request. A 413 returned *after*
buffering 100 MB is the defect still present, and indistinguishable from the
fix by status code alone.

---

## Scenario 5 (US2) — the limit does not break ordinary use

Send a payload at the documented ceiling for normal commands.

**Expected**: `200`. A limit tight enough to reject legitimate work is a
regression, not hardening (FR-006, SC-005).

---

## Scenario 6 (US3) — a refusal is actionable

Trigger a 429 and read the response.

**Expected**: `Retry-After`, `X-RateLimit-*` headers, and a `cause` field.

**The check that matters**: a client that sleeps for exactly `Retry-After` and
retries **succeeds** (SC-006). A padded constant would pass a
"header is present" test and fail this one — which is the point.

---

## Scenario 7 (US3) — quota and ceiling are distinguishable

Drive enough distinct clients, each within its own allowance, to cross the
system-wide ceiling.

**Expected**: 429 with `cause: system_wide`, distinguishable by a caller
holding only the response (SC-007). A caller blameless in its own right needs
to know its back-off should differ.

---

## Scenario 8 — successful responses carry the budget

Any ordinary request.

**Expected**: `X-RateLimit-Remaining` decreasing across calls.

A limiter that only speaks when refusing teaches clients to find the limit by
hitting it.

---

## Gate check before completion

```bash
cargo test --workspace
```

```bash
cargo fmt --all -- --check
```

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

`cargo test --workspace` needs `MASH` set or the Smoosh test fails and reads
like a regression:

```powershell
$env:MASH = (Resolve-Path .\target\debug\mash.exe).Path
```

Linux via `bash scripts/wsl-mirror.sh`, not `/mnt/c`.

**Smoosh is not a gate here** — neither `mash` nor `malt-tools` is touched.
Stated so its absence is not read as an oversight. **macOS is not a target**
(ADR-0006).
