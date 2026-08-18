PREFIX     ?= $(HOME)/.local
BIN_DIR    := $(PREFIX)/bin
BINARY     := .build/release/claude-pet
SKILL_DIR  := $(HOME)/.claude/skills/hatch-pet
LAUNCH_AGENT := $(HOME)/Library/LaunchAgents/dev.claude-pet.overlay.plist

.PHONY: build install hooks skill uninstall run demo render-all autostart no-autostart clean

## Build a release binary (.build/release/claude-pet)
build:
	swift build -c release

## Copy the binary to ~/.local/bin/claude-pet
install: build
	mkdir -p $(BIN_DIR)
	install -m 755 $(BINARY) $(BIN_DIR)/claude-pet
	@echo "installed $(BIN_DIR)/claude-pet"

## Register the hook in ~/.claude/settings.json (backup is written first)
hooks: install
	$(BIN_DIR)/claude-pet install-hooks

## Install the /hatch-pet skill for Claude Code (~/.claude/skills/hatch-pet)
skill:
	mkdir -p $(SKILL_DIR)
	cp skills/hatch-pet/SKILL.md $(SKILL_DIR)/SKILL.md
	@echo "installed $(SKILL_DIR)/SKILL.md"

## Remove hooks, binary, skill and launch agent (keeps ~/.claude-pet)
uninstall:
	-$(BIN_DIR)/claude-pet uninstall-hooks
	-launchctl bootout gui/$$(id -u) $(LAUNCH_AGENT) 2>/dev/null
	rm -f $(LAUNCH_AGENT) $(BIN_DIR)/claude-pet
	rm -rf $(SKILL_DIR)

## Run the overlay from the build directory
run: build
	$(BINARY) run

## Cycle through every state so you can watch the pet react
demo: build
	$(BINARY) simulate demo

## Render every built-in pet to render/<id>/sheet.png
render-all: build
	@for f in Sources/ClaudePet/Resources/pets/*.json; do \
		$(BINARY) render $$f --out render/$$(basename $$f .json) --scale 10; \
	done

## Start the overlay at login (LaunchAgent)
autostart: install
	mkdir -p $(dir $(LAUNCH_AGENT))
	printf '%s\n' \
	  '<?xml version="1.0" encoding="UTF-8"?>' \
	  '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">' \
	  '<plist version="1.0"><dict>' \
	  '  <key>Label</key><string>dev.claude-pet.overlay</string>' \
	  '  <key>ProgramArguments</key><array><string>$(BIN_DIR)/claude-pet</string><string>run</string></array>' \
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
	swift package clean
	rm -rf render
