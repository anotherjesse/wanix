# Makefile for the Rust-native Wanix workspace.

BIN ?= wanix
LINK_BIN ?= /usr/local/bin

.DEFAULT_GOAL := help

build: ## Build the Wanix CLI
	cargo build --locked --package wanix-cli
.PHONY: build

test: ## Run the full workspace test suite
	cargo test --workspace --locked
.PHONY: test

check: ## Run the workspace check gate
	cargo check --workspace --locked
.PHONY: check

workbench: ## Build the browser cockpit extension bundle
	cd workbench && npm install && npm run compile-web
.PHONY: workbench

link: build ## Link the debug Wanix CLI into LINK_BIN
	ln -fs "$(shell pwd)/target/debug/$(BIN)" "$(LINK_BIN)/$(BIN)"
.PHONY: link

clean: ## Remove Rust and workbench build artifacts
	cargo clean
	rm -rf workbench/dist workbench/node_modules
.PHONY: clean

help: ## Show available rules
	@awk 'BEGIN {FS = ":.*## "; print "Available rules:"} /^[a-zA-Z0-9_-]+:.*## / {printf "  %-12s %s\n", $$1, $$2}' $(MAKEFILE_LIST)
.PHONY: help
