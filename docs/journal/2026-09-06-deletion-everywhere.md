# T-206 deletion-everywhere handoff

## Shipped

- `fndr_store::DeleteScope` supports `RecordIds`, an inclusive capture-time
  range, a domain (including subdomains), and `All`.
- `delete_everywhere` resolves record IDs from SQLite first, deletes matching
  derived Lance rows, then deletes SQLite truth in one transaction. It returns
  record and indexed-chunk counts.
- A missing Lance table is a successful no-op: there cannot be indexed data
  before the first flush.
- Domain deletion uses the parsed-host, label-boundary contract shared with
  the privacy blocklist. `bank.com` includes `online.bank.com`; it cannot
  delete `burbank.com`. Stored URLs remain structurally sanitized.

## Failure semantics

- If Lance deletion fails, SQLite is not changed. The caller receives the
  failure and can retry; owner content was not falsely reported deleted.
- If SQLite deletion fails after Lance succeeds, SQLite remains the durable
  source of truth and `LanceWriter::rebuild` can restore the derived index.
  The index cannot expose the already-removed rows in that interval.

## Verification

- `CARGO_BUILD_JOBS=1 cargo test -p fndr-privacy -p fndr-store` passed:
  20 privacy tests, 12 store unit tests, 6 Lance flush/deletion tests, and 2
  rebuild tests. The serial setting keeps the 8 GB laptop within its RAM
  budget.

## Explicitly not done

- There is no owner-facing command, IPC, MCP tool, or vault UI delete action
  yet. This is the safety-critical engine boundary they will share.
- Retrieval and graph are not implemented, so their post-delete absence proof
  cannot be added honestly yet.
- Retention (T-207) must call this path with a controlled clock; it is not
  implemented as an independent SQL cleanup job.
