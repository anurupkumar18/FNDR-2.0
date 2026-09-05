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
- the current skeleton passes OCR text, not image bytes, into its store call.

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

### 3. Authenticated MCP surface

Start the example without `--query` and follow its printed connection snippet:

```sh
cargo run -p fndr-mcp --example skeleton -- \
  --image crates/fndr-ocr/tests/fixtures/skeleton_fixture.png \
  --store "$alpha_db"
```

Expected evidence: it prints a loopback endpoint and a new bearer token. The
test suite already proves missing bearer tokens and cross-origin requests are
rejected. Never paste the generated token into a screen recording or commit
it to a document.

### 4. Cleanup

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
