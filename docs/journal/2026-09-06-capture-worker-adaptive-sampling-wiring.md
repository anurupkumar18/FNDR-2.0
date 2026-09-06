# 2026-09-06: capture worker adaptive sampling wiring

## Decision

T-308 closes: `fndr-shell::capture_worker` now consumes the pure
`SamplingPolicy` and the macOS `InputIdleSource` seams instead of running a
fixed-cadence loop. `RealCaptureWorkerConfig::capture_interval` becomes
`sampling: SamplingPolicy`; `start_real_capture_worker` supplies
`MacOSInputIdle` as the concrete idle probe.

## What is verified

`run_capture_loop` computes an idle observation and time-since-last-capture
each iteration and asks `SamplingPolicy::decide` for the next action:
`CaptureNow` ticks the scheduler and re-checks the shutdown channel with a
non-blocking `try_recv`; `Wait(d)` blocks on `command_rx.recv_timeout(d)`;
`DeepIdle` blocks on `recv_timeout(idle_interval)` so the worker still wakes
periodically to re-observe idleness without capturing or busy-looping. The
2-second active-interval floor is still enforced before the scheduler opens.
Tests (`crates/fndr-shell/src/capture_worker.rs`) cover an immediate first
capture, a real wait between active-cadence captures, and a fixed idle input
above `deep_idle_after` producing zero capture ticks with a fast shutdown.

## Explicitly not done

No background polling thread, no model load, and no change to
`fndr-capture::sampling`'s pure decision logic. `MacOSInputIdle` remains
infallible (returns `Duration::ZERO` on non-macOS); that pre-existing
fallback is a known boundary, not something this slice changes. T-310's
long-running ScreenCaptureKit soak and fresh-permission run remain
unverified, as they were before this slice.

## Landmines

The shutdown channel must stay responsive under every `SamplingDecision`
arm: never call a blocking `recv_timeout` longer than `idle_interval`, and
never let a `CaptureNow` streak skip a channel check between ticks.
