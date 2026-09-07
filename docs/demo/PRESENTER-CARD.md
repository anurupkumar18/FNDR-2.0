# FNDR alpha presenter card

Use this alongside the full [alpha runbook](ALPHA-RUNBOOK.md) and the
[readiness checklist](DEMO-READINESS.md). It is a speaking guide, not new
product behavior.

## Open with the trust boundary

1. Run `fndr-shell --doctor` using the exact demo paths from the runbook.
   Show the model/data-path result and the Screen Recording **preflight**.
   Say: “This checked the current macOS status. It did not request permission,
   start Tauri, capture a frame, or create the vault.”
2. Launch FNDR normally. Show `not started` before selecting anything.
   Say: “Opening FNDR does not collect context. **Start capture** is the only
   normal action that can begin the macOS Screen Recording flow.”
3. Point out the window’s own content boundary: lifecycle codes, not pixels,
   OCR text, URLs, model output, queries, or record IDs.
4. Show **Open audit log**. Before any MCP call it must say no activity is
   recorded and leave the vault absent. After a deliberate authenticated MCP
   call, it shows only time, tool, outcome, and raw-text-release flag.

## Demonstrate the safe operating controls

- **Pause capture** waits for the capture worker to acknowledge, then stops new
  opportunities without discarding an in-flight local write.
- The menu-bar **Show FNDR** restores a hidden window; **Quit FNDR** performs
  the durable shutdown drain.
- A metadata-detected private/incognito title cue reports `private browsing`
  and withholds pixels before capture. Do not claim this detects every private
  window in every browser.
- A ScreenCaptureKit permission/tool failure reports
  `screen recording or capture unavailable` without exposing the system error.
  The window directs the operator to System Settings; it does not claim to
  distinguish denial from revocation.

## Deliver the evidence loop

Use the checked-in synthetic fixture for the deterministic OCR → local SQLite
FTS → authenticated MCP demonstration. Narrate it plainly: “This fixture is
synthetic. It proves the local path, not continuous live capture.” Use the
privacy-negative examples in the runbook to show password-manager and
owner-blocklist exclusion before OCR.

The MCP bearer token is a live secret. Keep it out of any recording, notes, or
commit. Remove only the named temporary demo directory after the demo process
has stopped.

## Claims to avoid

- A human-visible GUI rehearsal, live permission denial/revocation behavior,
  real screen capture, or a hardware soak has not been performed by this
  unattended workflow.
- No live egress counter, audit retention policy, browser-native incognito
  coordination, autostart, or graphical single-instance handoff is complete.
- `fndr.search` now serves a real vector route, but only when the MCP server
  is launched with `--model`/`--index-dir`; results are merged with keyword
  hits, not fused or ranked together, so there is still no hybrid, RRF, or
  reranking. The UI still has no search surface at all.
