PREFIX     ?= $(HOME)/.local
BIN_DIR    := $(PREFIX)/bin
CARGO_MANIFEST := rust/Cargo.toml
BINARY     := rust/target/release/claude-airou
SKILL_DIR  := $(HOME)/.claude/skills/hatch-pet
LAUNCH_AGENT := $(HOME)/Library/LaunchAgents/dev.claude-airou.overlay.plist
# Pre-rename artefacts (the project used to be "claude-pet"); cleaned up by install/uninstall.
LEGACY_LAUNCH_AGENT := $(HOME)/Library/LaunchAgents/dev.claude-pet.overlay.plist
LEGACY_BINARY := $(BIN_DIR)/claude-pet

.PHONY: build test install setup hooks statusline mcp skill uninstall run demo render-all autostart no-autostart clean

## Build a release binary (rust/target/release/claude-airou)
build:
	cargo build --release --manifest-path $(CARGO_MANIFEST)

## Unit tests + the end-to-end battery (drives the release binary in a throwaway sandbox)
test: build
	cargo test --manifest-path $(CARGO_MANIFEST)
	python3 rust/integration_test.py

## Copy the binary to ~/.local/bin/claude-airou
install: build
	mkdir -p $(BIN_DIR)
	install -m 755 $(BINARY) $(BIN_DIR)/claude-airou
	@if [ -f $(LEGACY_LAUNCH_AGENT) ]; then \
		launchctl bootout gui/$$(id -u) $(LEGACY_LAUNCH_AGENT) 2>/dev/null; rm -f $(LEGACY_LAUNCH_AGENT); \
		echo "removed legacy launch agent dev.claude-pet.overlay"; fi
	@if [ -f $(LEGACY_BINARY) ]; then rm -f $(LEGACY_BINARY); echo "removed legacy $(LEGACY_BINARY)"; fi
	@echo "installed $(BIN_DIR)/claude-airou"

## One-shot wiring — hooks + /hatch-pet skill + start at login (what the installer runs)
setup: install
	$(BIN_DIR)/claude-airou setup

## Register the hook in ~/.claude/settings.json (backup is written first)
hooks: install
	$(BIN_DIR)/claude-airou install-hooks

## Feed the usage gauge from the Claude Code status line (your own status line keeps running)
statusline: install
	$(BIN_DIR)/claude-airou install-statusline

## Register the MCP server in the Claude desktop app, so Claude chat can drive the pet
mcp: install
	$(BIN_DIR)/claude-airou install-mcp

## Install the /hatch-pet skill for Claude Code (~/.claude/skills/hatch-pet)
skill:
	mkdir -p $(SKILL_DIR)
	cp skills/hatch-pet/SKILL.md $(SKILL_DIR)/SKILL.md
	@echo "installed $(SKILL_DIR)/SKILL.md"

## Remove hooks, MCP entry, binary, skill and launch agent (keeps ~/.claude-airou)
uninstall:
	-$(BIN_DIR)/claude-airou uninstall
	-launchctl bootout gui/$$(id -u) $(LEGACY_LAUNCH_AGENT) 2>/dev/null
	rm -f $(LAUNCH_AGENT) $(LEGACY_LAUNCH_AGENT) $(BIN_DIR)/claude-airou $(LEGACY_BINARY)
	rm -rf $(SKILL_DIR)

## Run the overlay from the build directory
run: build
	$(BINARY) run

## Cycle through every state so you can watch the pet react
demo: build
	$(BINARY) simulate demo

## Render every built-in pet to render/<id>/sheet.png
render-all: build
	@for f in pets/*.json; do \
		$(BINARY) render $$f --out render/$$(basename $$f .json) --scale 10; \
	done

## Start the overlay at login (LaunchAgent)
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
	cargo clean --manifest-path $(CARGO_MANIFEST)
	rm -rf render
