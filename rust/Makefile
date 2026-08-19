# Rust build of claude-airou — the same targets as the repo-root Makefile (Swift), so
# `make install` / `make autostart` work identically once you switch binaries.
PREFIX     ?= $(HOME)/.local
BIN_DIR    := $(PREFIX)/bin
BINARY     := target/release/claude-airou
LAUNCH_AGENT := $(HOME)/Library/LaunchAgents/dev.claude-airou.overlay.plist

.PHONY: build install run demo test autostart no-autostart clean

## Build a release binary (target/release/claude-airou)
build:
	cargo build --release

## Copy the binary to ~/.local/bin/claude-airou (replaces the Swift binary if it is there)
install: build
	mkdir -p $(BIN_DIR)
	install -m 755 $(BINARY) $(BIN_DIR)/claude-airou
	@echo "installed $(BIN_DIR)/claude-airou"

## Run the overlay from the build directory
run: build
	$(BINARY) run

## Cycle through every state so you can watch the pet react
demo: build
	$(BINARY) simulate demo

## Unit tests + the end-to-end battery
test:
	cargo test
	python3 integration_test.py

## Start the overlay at login (LaunchAgent) — same label/plist as the Swift Makefile
autostart: install
	mkdir -p $(dir $(LAUNCH_AGENT))
	printf '%s\n' \
	  '<?xml version="1.0" encoding="UTF-8"?>' \
	  '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">' \
	  '<plist version="1.0"><dict>' \
	  '  <key>Label</key><string>dev.claude-airou.overlay</string>' \
	  '  <key>ProgramArguments</key><array><string>$(BIN_DIR)/claude-airou</string><string>run</string></array>' \
	  '  <key>RunAtLoad</key><true/>' \
	  '  <key>KeepAlive</key><false/>' \
	  '</dict></plist>' > $(LAUNCH_AGENT)
	-launchctl bootout gui/$$(id -u) $(LAUNCH_AGENT) 2>/dev/null
	launchctl bootstrap gui/$$(id -u) $(LAUNCH_AGENT)
	@echo "overlay will start at login ($(LAUNCH_AGENT))"

## Stop starting the overlay at login
no-autostart:
	-launchctl bootout gui/$$(id -u) $(LAUNCH_AGENT) 2>/dev/null
	rm -f $(LAUNCH_AGENT)

clean:
	cargo clean
