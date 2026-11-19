# Shortcuts for development (Linux/macOS; on Windows use the cargo commands).
#   make run              build the release binary and start it
#   make run ARGS="--config /tmp/test.toml"
#   make dev              start a debug build (faster to build)
#   make test | lint | check | fmt
#   make test-vcsim VCSIM=/path/to/vcsim   vSphere simulator tests (TC-133)

CARGO ?= cargo
BIN   := target/release/virt-interact
ARGS  ?=
VCSIM ?=

.PHONY: all release run dev test test-vcsim lint fmt check

all: release

# Always rebuilds what changed (cargo decides), so `make run` is current.
release:
	$(CARGO) build --release --locked -p vi-gui

run: release
	./$(BIN) $(ARGS)

dev:
	$(CARGO) run --locked -p vi-gui -- $(ARGS)

test:
	$(CARGO) test --workspace --locked

test-vcsim:
	@test -n "$(VCSIM)" || { echo "set VCSIM=/path/to/vcsim"; exit 1; }
	VI_VCSIM="$(VCSIM)" $(CARGO) test --locked -p vi-vsphere -p vi-backend

lint:
	$(CARGO) fmt --check
	$(CARGO) clippy --workspace --all-targets --locked -- -D warnings

fmt:
	$(CARGO) fmt

check: lint test
