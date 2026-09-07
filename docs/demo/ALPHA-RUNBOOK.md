# Alpha demo runbook: local memory, visible privacy, authenticated MCP

## Scope and truthfulness

This runbook demonstrates the current alpha walking skeleton, not the final
product. It uses a checked-in synthetic PNG fixture and prints that fact in
the narration. It proves a real path through Vision OCR, local SQLite FTS,
and the authenticated MCP server; it does not claim continuous capture,
semantic retrieval, UI onboarding, a real model benchmark, or Connected
Planner execution.

The runbook satisfies the alpha outcome in `docs/PRD.md` and is the baseline
for later beta and final rehearsal scripts.

## Preconditions

- macOS with the Vision framework available.
- Rust toolchain and repository dependencies installed.
- Run from the FNDR-2.0 repository root.
- Do not use real captures, databases, or credentials in the demo workspace.

## QA gate before presenting

```sh
cargo test -p fndr-privacy
cargo test -p fndr-store skeleton::tests::file_backed_store_survives_reopen -- --exact
cargo test -p fndr-mcp
```

All tests must pass. The MCP suite includes the named adversarial checks for
unauthenticated loopback and a web origin carrying a valid bearer token.

## Demo sequence

### 1. Persisted local capture-to-search path

Use a temporary database so no demonstration data remains after the run:

```sh
alpha_tmp=$(mktemp -d /tmp/fndr-alpha.XXXXXX)
alpha_db="$alpha_tmp/memory.sqlite3"
cargo run -p fndr-mcp --example skeleton -- \
  --image crates/fndr-ocr/tests/fixtures/skeleton_fixture.png \
  --store "$alpha_db" --query "quick brown fox"
cargo run -p fndr-mcp --example skeleton -- \
  --image crates/fndr-ocr/tests/fixtures/skeleton_fixture.png \
  --store "$alpha_db" --query "quick brown fox"
```

Expected evidence:

- both runs report real OCR block and confidence data;
- the first reports `total records: 1` and the second `total records: 2`;
- the second search returns two FTS hits containing the fixture text; and
- the raw-PNG negative test confirms that the fixture bytes are absent from
  the SQLite database, WAL, and SHM artifacts after OCR text is stored.

Narrate: "This is a synthetic fixture. The point is that the same local
SQLite-backed memory survives a process restart and is returned by the MCP
search engine."

### 2. Privacy negative before OCR

```sh
cargo run -p fndr-mcp --example skeleton -- \
  --image crates/fndr-ocr/tests/fixtures/skeleton_fixture.png \
  --app 1Password --store "$alpha_db" --query "quick brown fox"
```

Expected evidence: the command exits with code `3` and prints
`capture skipped before OCR: PasswordManager`. It must not print OCR metadata,
`stored 1 record`, or a search hit for this attempted capture.

Narrate: "The fixture never reaches OCR when its app context is a password
manager. This is a visible policy decision, not a silent skip."

### 3. Owner-configured blocklist negative

```sh
cargo run -p fndr-mcp --example skeleton -- \
  --image crates/fndr-ocr/tests/fixtures/skeleton_fixture.png \
  --url https://docs.example.com/fndr --block-domain example.com \
  --store "$alpha_db" --query "quick brown fox"
```

Expected evidence: the command exits with code `3` and prints
`capture skipped before OCR: UserBlocklist`. Use this to explain that the
same policy has safe suffix-domain matching; a blocked `example.com` covers
its subdomains, not unrelated strings that merely contain that text.

### 4. Authenticated MCP surface

Start the example without `--query` and follow its printed connection snippet:

```sh
cargo run -p fndr-mcp --example skeleton -- \
  --image crates/fndr-ocr/tests/fixtures/skeleton_fixture.png \
  --store "$alpha_db"
```

Expected evidence: it prints a loopback endpoint and a new bearer token. The
test suite already proves missing bearer tokens and cross-origin requests are
rejected. Never paste the generated token into a screen recording or commit
it to a document. An authenticated agent can also call `fndr.privacy_status`
to see the local-default flag, planner-disabled flag, and configured
blocklist counts without receiving the blocklist entries.

### 5. Desktop preflight, lifecycle, and durable MCP host

This is the current alpha desktop path. It opens a trust/status window and a
menu-bar icon, but a normal launch does not request Screen Recording or start
capture. The window shows generated lifecycle codes only, never screen
content, OCR text, URLs, or model output. First show the visible
`not_started` state and privacy explanation. Only then select **Start
capture** when the human presenter intends to grant Screen Recording. Once
active, closing the window leaves the capture host running in the menu bar,
where **Show FNDR** restores it and **Quit FNDR** starts the shutdown drain.
**Pause capture** in the window and **Pause / Resume Capture** in the menu bar
wait for the worker acknowledgement before showing a paused state; they do not
interrupt an in-flight local write:

The trust window also displays the current non-prompting Screen Recording
preflight. `Granted` means macOS currently reports access; `Not granted` is
not a prompt and does not start capture. Only **Start capture** can begin the
macOS permission flow.

**Open audit log** reads a bounded local MCP ledger only when the owner asks.
It shows time, tool, outcome, and whether raw capture text was released—never
the query, record ID, URL, or captured content. With no existing vault it
reports no recorded MCP activity and does not create one.

If the first capture tick reports **screen recording or capture unavailable**,
the window directs the presenter to FNDR's Screen Recording setting in macOS
System Settings. This is a stable content-free failure class, not proof that
macOS denied a particular request; after granting or re-granting access, wait
for the next capture opportunity and confirm its new lifecycle status.

When foreground metadata itself carries the existing private/incognito title
cue, FNDR reports **private browsing** and withholds pixels before capture.
This is a visible safety cue, not a claim of complete browser-native private
window detection; do not demonstrate it as a universal incognito guarantee.

First, run the non-capturing doctor. It returns `3` when a supplied model is
missing or the data directory cannot be prepared; it does not create the data
directory, launch the window, or request a macOS permission:

```sh
cargo run -p fndr-shell --bin fndr-shell -- --doctor --data-dir "$alpha_tmp/desktop" \
  --model models/Qwen3-Embedding-0.6B-Q8_0.gguf
```

Run the first command in one terminal. Leave it running, then run the second
in another terminal:

```sh
cargo run -p fndr-shell --bin fndr-shell -- --data-dir "$alpha_tmp/desktop" \
  --model models/Qwen3-Embedding-0.6B-Q8_0.gguf
```

```sh
cargo run -p fndr-mcp -- --store "$alpha_tmp/desktop/vault.sqlite3"
```

Use the second command's printed bearer-token snippet to connect the MCP
client. For a bounded capture lifecycle rehearsal, add `--run-seconds 60` to
the first command: this explicitly CLI-authorizes capture to start and then
requests the clean exit drain. It is not a substitute for deliberate
permission and hardware verification.

### 6. Cleanup

After the process stops, remove only the temporary directory created above:

```sh
find "$alpha_tmp" -type f -delete
find "$alpha_tmp" -type d -empty -delete
```

## Failure handling

| Symptom | Interpretation | Action |
| --- | --- | --- |
| Vision OCR unavailable | Environment is not a supported alpha demo machine. | Use the documented macOS machine or stop the demo; do not replace OCR with a mock. |
| Privacy negative stores a record | Blocking privacy regression. | Do not demo; file a regression test before a fix. |
| Search returns no fixture result | Capture/OCR/store boundary failed. | Run the named tests, inspect typed output, and repair the failing stage. |
| MCP client cannot connect | Treat bearer token and host/origin checks as required, not optional. | Use the generated loopback snippet; run `cargo test -p fndr-mcp`. |

## Evidence to retain

Record the commit SHA, three QA command results, the two-run output, the
privacy-negative output, the machine/OS version, and any failure ticket. Do
not retain a generated bearer token or the temporary database.
