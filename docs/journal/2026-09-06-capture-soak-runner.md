# 2026-09-06: a soak runner, and the thing I did not run

## Decision

Two consecutive handoffs have said the same thing: nothing in the repo owns
`start_real_capture_worker`. The composition — ScreenCaptureKit, Vision OCR,
the privacy gate, SQLite, the queued embedder, Lance — has never run for
longer than a single test tick, and T-310's soak has been open since T-302
landed with no instrument to perform it.

`cargo run -p fndr-shell --example capture_soak` is that instrument. It owns
the real worker for a bounded number of minutes, prints per-outcome tick
counts and the shutdown drain, and exits non-zero if zero ticks occurred,
because a soak that captured nothing is a finding rather than a pass.

It is a CLI, not a desktop lifecycle. T-901 still owes the real one, and the
ledger says so rather than letting a green soak read as "auto-capture
works".

## RSS, because that is what the ticket is about

The first version just proved the process survived. That misses the point:
T-310's AC asks for an RSS trend specifically because the
`screencapturekit` crate's issue history is leaks and stalled callbacks. A
runner that cannot see a leak is not a soak instrument, so it now samples
its own resident size every fifteen seconds and reports start, end, peak,
and growth.

A failed sample is skipped rather than recorded as zero, since a zero would
fake a downward trend — the exact wrong answer for a leak hunt. The output
is a series a human reads, not an assertion: one short run cannot separate
a leak from a warm cache, and the runner says so when it has too few
samples to support a call.

## What I deliberately did not do

I did not run it. This tool captures the operator's screen for the whole
run, needs a Screen Recording grant, and stores whatever is visible. The
session that built it is unattended, and starting real screen capture on
someone's machine while they are away is not mine to do — not for a
permission prompt they did not see, and not for whatever happens to be on
the screen.

So this slice ships verified-as-code and unverified-as-soak: it compiles,
its argument handling and failure paths are exercised by the type system
and the existing worker tests, and the actual multi-day run belongs to a
person who consents to it. The ledger row states that boundary rather than
implying the soak is done.

## Landmines

Do not wire this into CI. It is interactive by nature and captures whatever
is on screen; an automated soak would be recording a build agent's display
into a store nobody reads.

`--block-app` and `--block-domain` exist and should be used. The blocklist
is the only thing standing between a long soak and a stored copy of
whatever the operator opened during it.
