# 2026-09-06: fndr.active_focus, and what "current" is allowed to mean

## Decision

ADR-007 describes `fndr.active_focus` as "current app/window/project/task
inference". The available data is the newest `memory_record`. The gap
between those two things is the whole design problem: a record captured
three hours ago is a fact, but reporting it as what someone is *currently*
doing is a fabrication, and an agent handed a bare app name will phrase it
exactly that way.

So the tool returns a typed `status` — `active`, `stale`, or `none` — with
the observation's `age_ms` alongside it. `none` means nothing was ever
captured. `stale` means there is an observation, but it is older than the
caller's own tolerance. Neither can be misread as a claim about now.

The default tolerance is five minutes, matching `fndr-capture`'s
`deep_idle_after`: past that much idleness the sampler itself stops
believing the screen represents what someone is doing, so the read side
should not believe it either.

## What is verified

Three statuses, three assertions: an empty store reports `none` and no app
name; the fixture record (captured at epoch 42, decades stale) reports
`stale` with a positive age; the same record reports `active` once the
caller passes a tolerance wide enough to cover it. A negative
`stale_after_ms` is a typed refusal.

The store read is one line — `latest_record_id` — deliberately kept apart
from `record_evidence` so this composes the existing read instead of
growing a second "read a record" path.

## The coverage test earned its keep

Adding this ninth tool made `every_registered_tool_writes_an_audit_entry`
fail, because the new tool was registered but not exercised there. That is
precisely the friction it was built for: the failure said a tool had
appeared without its audit coverage being considered. Written this morning,
useful by the afternoon.

## Explicitly not done

Project and task inference, both named in ADR-007's entry. Neither has a
data model, and inferring a "project" from a window title would be a guess
presented as an observation, which is the failure mode this whole tool is
shaped to avoid.

Also: staleness is measured against capture time, not against whether the
capture worker is actually running. A stopped worker looks identical to an
idle user here. The pipeline health surface (T-1004) is where "capture is
not running" belongs, and this tool should eventually cite it rather than
re-deriving it.
