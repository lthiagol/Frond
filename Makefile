# frond — build/test/clean
# Read-only Obsidian-style terminal viewer. See SPEC.md.

# Cargo: use the one on PATH, else rustup's default location. A shell
# whose PATH lacks ~/.cargo/bin (a fresh login shell, a terminal launched
# from a desktop file, a CI image) would otherwise fail every target with
# "cargo: command not found". Override with `make CARGO=... <target>` or
# an exported CARGO.
CARGO ?= $(shell command -v cargo 2>/dev/null || echo $(HOME)/.cargo/bin/cargo)

.PHONY: help build release test smoke fmt clippy check run clean clean-fixtures

help:
	@echo "make build          - debug build"
	@echo "make release        - release build (binary: target/release/frond)"
	@echo "make test           - unit + integration tests (smoke suites excluded)"
	@echo "make smoke          - pty smoke suites (slow, uses uv + pyte)"
	@echo "make fmt / clippy   - formatting / lints"
	@echo "make check          - fmt + clippy + test + smoke"
	@echo "make run            - launch the TUI (opens on the vault picker)"
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
	uv run --with pyte $(CARGO) test -p frond --test smoke -- --ignored
	uv run --with pyte $(CARGO) test -p frond --test smoke_m2 -- --ignored
	uv run --with pyte $(CARGO) test -p frond --test smoke_m3 -- --ignored
	uv run --with pyte $(CARGO) test -p frond --test smoke_m4 -- --ignored
	uv run --with pyte $(CARGO) test -p frond --test smoke_m5 -- --ignored
	uv run --with pyte $(CARGO) test -p frond --test smoke_m6 -- --ignored
	uv run --with pyte $(CARGO) test -p frond --test smoke_m7 -- --ignored

fmt:
	$(CARGO) fmt

clippy:
	$(CARGO) clippy --workspace --all-targets -- -D warnings

check: fmt clippy test smoke

run:
	$(CARGO) run --release -p frond --

# Removes ALL build artifacts. snapshots/ and docs/ are never touched.
clean:
	$(CARGO) clean
	rm -rf target

# Removes smoke-test leftovers: the private per-user scratch root the smoke
# scripts use (scripts/smoke_tmp.py), plus the /tmp paths of manual probes.
clean-fixtures:
	rm -rf "$${FROND_SMOKE_TMP:-$${TMPDIR:-/tmp}/frond-smoke-$$(id -u)}"
	rm -rf /tmp/vault-copy* /tmp/gitprobe* /tmp/probe-* /tmp/frond-ui*.md
