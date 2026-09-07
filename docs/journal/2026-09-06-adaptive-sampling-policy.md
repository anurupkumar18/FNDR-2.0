# 2026-09-06: adaptive sampling policy

## Decision

T-308 starts with a pure cadence policy. Given an input-idle duration and time
since the last capture, it returns active, idle, deep-idle, or forced-capture
decisions without platform calls, threads, models, or persistence.

## What is verified

The default curve captures every 2 seconds when active, slows to 15 seconds
after one idle minute, pauses after five minutes idle, and forces a sparse
capture after two minutes of otherwise unchanged activity. The simulated
timeline test covers every state.

## Explicitly not done

No macOS CoreGraphics input-idle adapter, worker integration, or adaptive
sleep loop exists yet. The current fixed worker cadence remains unchanged.

## Landmines

Keep the idle probe at the shell boundary and use a supplied observation in
the engine policy. Do not add a polling thread or let capture cadence bypass
the existing privacy and shutdown guarantees.
