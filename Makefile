.PHONY: build release test check fmt clippy clean help all

help:
	@echo "mcdu - Makefile targets:"
	@echo ""
	@echo "  make build      - Build debug binary"
	@echo "  make release    - Build optimized release binary"
	@echo "  make test       - Run all tests"
	@echo "  make check      - Run cargo check"
	@echo "  make fmt        - Format code with rustfmt"
	@echo "  make clippy     - Run clippy linter"
	@echo "  make clean      - Remove build artifacts"
	@echo "  make all        - Run check, fmt, clippy, and test"
	@echo ""

build:
	cargo build

release:
	cargo build --release

test:
	cargo test --release

check:
	cargo check --all-targets

fmt:
	cargo fmt

fmt-check:
	cargo fmt -- --check

clippy:
	cargo clippy --all-targets -- -D warnings

clean:
	cargo clean

all: check fmt-check clippy test
	@echo "✓ All checks passed!"

# Development targets
dev:
	RUST_LOG=debug cargo run

install:
	cargo install --path .

uninstall:
	cargo uninstall mcdu
