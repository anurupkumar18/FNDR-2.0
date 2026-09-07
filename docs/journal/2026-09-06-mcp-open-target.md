# 2026-09-06: fndr.open_target, and a struct left alone

## Decision

`fndr.search` finds a memory and `fndr.source_evidence` explains it;
neither could get the user back to it. `fndr.open_target` resolves a
`record_id` to its sanitized URL, or failing that to the app bundle it was
captured from, or failing that to an explicit `unavailable` kind carrying
the reason.

No new store method. The tool reads through the existing
`Store::record_evidence`.

## What is verified

One test walks all three resolutions: a record with a URL resolves to
`url`, a record with only a bundle identifier resolves to `app`, and a
record with neither resolves to `unavailable` with a populated `reason`.
An unknown `record_id` is a typed refusal, matching `fndr.source_evidence`.

Returned URLs are whatever the write path stored, and that path runs them
through `fndr_privacy::sanitize_url_for_storage` first, so credentials,
query strings, and fragments were never persisted to hand back. Reopening
a target gives the page, not the session.

## The struct I did not extend

`Store::capture_metadata` returns `CaptureMetadata { bundle_id, url }` and
would have been the natural fit, needing only `app_name` for a useful
label. Three tests across `fndr-shell` and `fndr-memory` assert that struct
equals exactly those two fields; that assertion is the T-306 privacy claim
that a capture retains only explicit metadata. Adding fields to satisfy a
new caller would have quietly diluted a deliberate privacy test into a
shape assertion. Reusing `record_evidence` costs one discarded chunk read
per call, on a path that runs once per user action, and leaves the
assertion intact.

## Explicitly not done

No file-path targets: nothing in the capture pipeline records a document
path today, so `open_target` cannot resolve one, and inventing a guess from
a window title would be worse than the honest `unavailable`. No actual
opening either; this returns a target, and whoever acts on it (UI, agent)
owns that side effect.
