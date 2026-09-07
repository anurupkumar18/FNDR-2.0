# FNDR alpha demo readiness · 2026-09-06

This checklist separates verified behavior from the deliberate, human-operated
Screen Recording rehearsal. It is not a release claim.

For a concise narrative that preserves those boundaries, use the
[presenter card](PRESENTER-CARD.md) with this checklist.

## Verified now

| Demo beat | Evidence | Boundary |
| --- | --- | --- |
| Local synthetic capture → OCR → durable FTS | `docs/demo/ALPHA-RUNBOOK.md` steps 1–3 were run on the checked-in fixture. The privacy-negative exits before OCR and leaves no database. | Synthetic fixture only; not live screen capture. |
| Authenticated local MCP | Existing MCP auth and durable-store tests pass in `make test`. | Do not record generated bearer tokens. |
| Desktop trust state | A real normal desktop launch stayed alive while leaving its explicit vault directory absent; the normal UI starts as `not_started`. | This does not prove a human saw the rendered window. |
| Permission preflight | The trust window calls Core Graphics' current Screen Recording preflight and shows `granted` or `not granted`; the check is separate from the permission-request API. | This is current status only, not a live deny/revoke test. |
| Explicit capture control | The window invokes `start_capture`; pause/resume waits for worker acknowledgement. Shell tests prove the non-capturing default and no ticks while paused. | The actual macOS permission interaction needs a human. |
| Permission failure guidance | A ScreenCaptureKit permission/tool failure becomes the content-free `screen_recording_or_capture_unavailable` tick reason; the trust window directs the presenter to macOS Screen Recording settings and the next capture opportunity. | This classifies an unavailable capture boundary, not a live TCC deny/revoke probe. |
| Private-browsing cue | A foreground title carrying the existing private/incognito cue becomes the content-free `private_browsing` skip reason before pixels are captured. | This is not a complete, browser-native private-window detector. |
| Owner audit viewer | The trust window can open the bounded local MCP audit ledger, showing only time, tool, outcome, and raw-text-release flag. A missing vault returns an honest empty view and is not created. | The ledger has no retention policy yet; a human still needs to review it in the rendered app. |
| Single-instance and clean exit code | Tauri single-instance callback compiles; the lifecycle owns a final drain on exit. | A live graphical handoff and hardware clean-exit observation remain human checks. |
| Retrieval | FTS CI baseline remains `1.0000` Recall@5/MRR@10 on the sample corpus. The opt-in real-Qwen vector smoke route measured `1.0000`/`1.0000`, p50/p95 `90.43/90.62 ms` on the local M1. | The vector route is not yet MCP/UI/fusion behavior and is not a tuned quality claim. |

`CARGO_BUILD_JOBS=1 make test` and `make bench` pass after these checks.

## Presenter-operated rehearsal

1. Run the runbook's `fndr-shell --doctor` command. It must report
   `ready_for_permission_rehearsal` and must not create the demo directory.
2. Launch the normal shell command. Show the `not_started` trust screen before
   selecting **Start capture**, including the Screen Recording preflight line.
3. Explain that the status window displays no pixels, OCR text, URLs, or model
   output. Only then select **Start capture** and deliberately handle the
   macOS Screen Recording prompt.
4. Before capture, select **Open audit log**. It should say that no MCP activity is recorded
   rather than create a vault. After a deliberate MCP rehearsal, use it to verify the
   content-free ledger.
5. Demonstrate **Pause capture**, then resume; use **Quit FNDR** to exercise
   the drain instead of force-quitting.
6. Run the local MCP command against the demo vault. Keep the emitted bearer
   token out of any recording and delete the named temporary demo directory
   after the process stops.

## Do not claim yet

- Live permission denial/revocation probing, egress counter, browser-native incognito
  coordination, or autostart.
- A real GUI single-instance handoff or an unattended hardware soak.
- Hybrid, RRF, reranked, temporal, or MCP-served vector retrieval.
