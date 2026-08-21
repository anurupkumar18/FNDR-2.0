# Local verification gates. `make test` is the full pass the skill's
# verification bar and CI both run; keep them in sync.

.PHONY: test lint test-rust test-ui bench

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
	cd ui && npm run typecheck && npm test

# FTS baseline on the sample corpus (format fixture, not an eval instrument;
# see bench/README.md). Real corpora and routes land with E05. Fails on any
# quality regression against the committed baseline.
bench:
	cargo run -q -p fndr-bench -- --corpus bench/corpus-sample \
		--baseline bench/baselines/corpus-sample.fts_baseline.json \
		--out target/bench-metrics.json
