# frond — build/test/clean
# Read-only Obsidian-style terminal viewer. See SPEC.md.

CARGO ?= cargo

.PHONY: help build release test smoke fmt clippy check run clean clean-fixtures

help:
	@echo "make build          - debug build"
	@echo "make release        - release build (binary: target/release/app)"
	@echo "make test           - unit + integration tests (smoke suites excluded)"
	@echo "make smoke          - pty smoke suites (slow, uses uv + pyte)"
	@echo "make fmt / clippy   - formatting / lints"
	@echo "make check          - fmt + clippy + test + smoke"
	@echo "make run            - launch the TUI (prompts for the vault path)"
	@echo "make clean          - remove all build artifacts"
	@echo "make clean-fixtures - remove /tmp test fixtures"

build:
	$(CARGO) build

release:
	$(CARGO) build --release

test:
	$(CARGO) test --workspace

# Smokes drive the app via python3 + pyte; `uv run --with pyte` provides pyte
# on the PATH without a manual install.
smoke:
	uv run --with pyte $(CARGO) test -p app --test smoke -- --ignored
	uv run --with pyte $(CARGO) test -p app --test smoke_m2 -- --ignored
	uv run --with pyte $(CARGO) test -p app --test smoke_m3 -- --ignored
	uv run --with pyte $(CARGO) test -p app --test smoke_m4 -- --ignored

fmt:
	$(CARGO) fmt

clippy:
	$(CARGO) clippy --workspace --all-targets -- -D warnings

check: fmt clippy test smoke

run:
	$(CARGO) run --release -p app --

# Removes ALL build artifacts. snapshots/ and docs/ are never touched.
clean:
	$(CARGO) clean
	rm -rf target

# Removes smoke-test leftovers from /tmp (fixtures are recreated by each run).
clean-fixtures:
	rm -rf /tmp/frond-m2-vault /tmp/frond-m3-vault /tmp/frond-m4-vault \
	       /tmp/watchtest.md /tmp/vault-copy* /tmp/gitprobe* /tmp/probe-* \
	       /tmp/frond-ui*.md
