.DEFAULT_GOAL := help

NAME    := tmux-agent-watch
PREFIX  ?= /usr/local
BINDIR  ?= $(PREFIX)/bin

.PHONY: help build release test fmt fmt-check clippy check run once install uninstall clean refresh-manifests

help: ## Show this help
	@awk 'BEGIN {FS = ":.*## "} /^[a-z-]+:.*## / {printf "  %-18s %s\n", $$1, $$2}' $(MAKEFILE_LIST)

build: ## Debug build
	cargo build

release: ## Optimized build
	cargo build --release

test: ## Run all tests
	cargo test

fmt: ## Format sources
	cargo fmt

fmt-check: ## Check formatting without writing
	cargo fmt --check

clippy: ## Lint with warnings as errors
	cargo clippy --all-targets -- -D warnings

check: fmt-check clippy test ## Full validation: format check + lint + tests

run: ## Run the TUI (debug build)
	cargo run

once: ## One scan, plain-text tree
	cargo run -- --once

install: release ## Install into $(PREFIX)/bin (default /usr/local; e.g. make install PREFIX=~/.local)
	install -d $(DESTDIR)$(BINDIR)
	install -m 755 target/release/$(NAME) $(DESTDIR)$(BINDIR)/$(NAME)

uninstall: ## Remove the installed binary from $(PREFIX)/bin
	rm -f $(DESTDIR)$(BINDIR)/$(NAME)

clean: ## Remove build artifacts
	cargo clean

refresh-manifests: ## Re-vendor detection manifests from ../herdr (or HERDR=path)
	scripts/refresh-manifests.sh $(or $(HERDR),../herdr)
	cargo test
