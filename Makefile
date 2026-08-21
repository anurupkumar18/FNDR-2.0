# Local verification gates. `make test` is the full pass the skill's
# verification bar and CI both run; keep them in sync.

.PHONY: test lint test-rust test-ui bench

test: lint test-rust test-ui

lint:
	scripts/workspace-lints.sh
	scripts/gen-agents-md.sh --check

test-rust:
	cargo fmt --all --check
	cargo clippy --workspace --all-targets -- -D warnings
	cargo test --workspace

test-ui:
	cd ui && npm run typecheck && npm test

bench:
	@echo "make bench: the fndr-bench harness is not implemented yet (E05)."
	@echo "Failing loudly so nothing mistakes this for a measured pass."
	@exit 2
