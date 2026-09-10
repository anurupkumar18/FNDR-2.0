# Local verification gates. `make test` is the full pass the skill's
# verification bar and CI both run; keep them in sync.

.PHONY: test lint test-rust test-ui bench run-app clean

test: lint test-rust test-ui

lint:
	scripts/workspace-lints.sh
	scripts/ui-lints.sh
	scripts/gen-agents-md.sh --check

test-rust:
	cargo fmt --all --check
	cargo clippy --workspace --all-targets -- -D warnings
	cargo test --workspace

test-ui:
	cd ui && npm run typecheck && npm test && npm run build
	scripts/check-tauri-build-output.sh

# FTS baseline on the sample corpus (format fixture, not an eval instrument;
# see bench/README.md). Real corpora and routes land with E05. Fails on any
# quality regression against the committed baseline.
bench:
	cargo run -q -p fndr-bench -- --corpus bench/corpus-sample \
		--baseline bench/baselines/corpus-sample.fts_baseline.json \
		--out target/bench-metrics.json

# Build the real app UI and launch the native FNDR host. This is "launch the
# app" in one command: builds ui/ with Vite into crates/fndr-shell/ui/workspace/,
# then runs the Tauri host, which opens both the trust window and the new
# workspace window (see the plan's "Known scope boundaries" for why both).
run-app:
	npm --prefix ui ci
	npm --prefix ui run build
	cargo run -p fndr-shell

# Debug build output is fully regenerable and untracked; nothing here prunes
# it, so repeated CARGO_BUILD_JOBS=1 rebuilds can grow target/ past 70 GiB
# (see lessons.md 2026-09-06). Run this whenever `du -sh target` looks large,
# especially before a build-heavy or disk-constrained session.
clean:
	cargo clean
