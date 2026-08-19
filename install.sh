#!/bin/sh
# claude-airou installer — downloads the latest release binary and wires it into Claude Code.
#
#   curl -fsSL https://raw.githubusercontent.com/hyungwookc07/claude-airou/main/install.sh | sh
#
# Re-run it any time to update: the binary is replaced, already-registered settings are left
# alone. Everything it touches lives in your home directory; nothing needs sudo.
#
# By default it registers the Claude Code hook, installs the /hatch-pet skill and starts the
# overlay. Start-at-login, the status line and the MCP server are switches in the menu bar
# 🐾 menu, so nothing you did not agree to gets registered behind your back.
#
# Options (curl … | sh -s -- --with-autostart):
#   --with-autostart   also start the overlay at login (LaunchAgent)
#   --with-statusline  also feed the usage gauge from your Claude Code status line
#   --with-mcp         also register the MCP server for the Claude desktop app
#   --no-setup         install the binary only, change no settings
#
# Environment:
#   CLAUDE_AIROU_INSTALL_DIR   where the binary goes (default ~/.local/bin)
#   CLAUDE_AIROU_VERSION       install a specific tag (default: the latest release)

set -eu

REPOSITORY="hyungwookc07/claude-airou"
BINARY_NAME="claude-airou"
ASSET_NAME="claude-airou-macos-universal.tar.gz"
INSTALL_DIR="${CLAUDE_AIROU_INSTALL_DIR:-$HOME/.local/bin}"
MINIMUM_MACOS_MAJOR_VERSION=14

setup_arguments=""
should_run_setup=1

for argument in "$@"; do
  case "$argument" in
    --no-setup) should_run_setup=0 ;;
    --with-autostart|--with-statusline|--with-mcp) setup_arguments="$setup_arguments $argument" ;;
    --minimal|--no-autostart) ;;  # the old spelling of today's default; accepted, no effect
    -h|--help) sed -n '2,20p' "$0" 2>/dev/null || true; exit 0 ;;
    *) echo "claude-airou: unknown option $argument" >&2; exit 2 ;;
  esac
done

say() { printf '%s\n' "$*"; }
fail() { printf 'claude-airou: %s\n' "$*" >&2; exit 1; }

# --- 1. environment ----------------------------------------------------------------------
[ "$(uname -s)" = "Darwin" ] || fail "this installer is macOS-only (the overlay is a native macOS app)."

macos_major_version="$(sw_vers -productVersion | cut -d. -f1)"
if [ "$macos_major_version" -lt "$MINIMUM_MACOS_MAJOR_VERSION" ]; then
  fail "macOS $MINIMUM_MACOS_MAJOR_VERSION or newer is required (found $(sw_vers -productVersion))."
fi
command -v curl >/dev/null 2>&1 || fail "curl is required."

# --- 2. resolve the version --------------------------------------------------------------
if [ -n "${CLAUDE_AIROU_VERSION:-}" ]; then
  release_tag="$CLAUDE_AIROU_VERSION"
else
  say "Looking up the latest claude-airou release…"
  release_tag="$(curl -fsSL "https://api.github.com/repos/$REPOSITORY/releases/latest" \
    | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)"
  [ -n "$release_tag" ] || fail "could not determine the latest release (GitHub API rate limit? set CLAUDE_AIROU_VERSION=vX.Y.Z)."
fi

download_base="https://github.com/$REPOSITORY/releases/download/$release_tag"
temporary_dir="$(mktemp -d)"
# shellcheck disable=SC2064
trap "rm -rf '$temporary_dir'" EXIT INT TERM

# --- 3. download and verify --------------------------------------------------------------
say "Downloading $release_tag…"
curl -fsSL "$download_base/$ASSET_NAME" -o "$temporary_dir/$ASSET_NAME" \
  || fail "download failed: $download_base/$ASSET_NAME"

if curl -fsSL "$download_base/$ASSET_NAME.sha256" -o "$temporary_dir/$ASSET_NAME.sha256" 2>/dev/null; then
  # Compare the hash itself rather than running `shasum -c`: the published file records
  # whatever path the release job hashed (e.g. "dist/<asset>"), which never matches the
  # temp directory the download lands in.
  expected_checksum="$(cut -d' ' -f1 < "$temporary_dir/$ASSET_NAME.sha256")"
  actual_checksum="$(shasum -a 256 "$temporary_dir/$ASSET_NAME" | cut -d' ' -f1)"
  [ -n "$expected_checksum" ] || fail "the published checksum file for $release_tag is empty."
  [ "$expected_checksum" = "$actual_checksum" ] \
    || fail "checksum mismatch — the download is corrupt or tampered with. Nothing was installed."
  say "Checksum verified."
else
  say "Note: no published checksum for $release_tag; skipping verification."
fi

tar -xzf "$temporary_dir/$ASSET_NAME" -C "$temporary_dir" || fail "could not unpack $ASSET_NAME."
[ -f "$temporary_dir/$BINARY_NAME" ] || fail "the archive did not contain $BINARY_NAME."

# --- 4. install the binary ---------------------------------------------------------------
# Only one overlay runs at a time (~/.claude-airou/overlay.lock), and a running one would
# keep the old build alive, so stop it before swapping the binary. setup restarts it.
if pgrep -f "$BINARY_NAME run" >/dev/null 2>&1; then
  say "Stopping the running overlay…"
  pkill -f "$BINARY_NAME run" >/dev/null 2>&1 || true
  sleep 1
  # The single-instance lock is a pid file that only the overlay itself cleans up on a
  # graceful quit; after a kill it lingers for a day and the next start would just print
  # "already running" and exit — leaving the user with no pet after an update.
  rm -f "${CLAUDE_AIROU_HOME:-$HOME/.claude-airou}/overlay.lock"
fi

mkdir -p "$INSTALL_DIR"
install -m 755 "$temporary_dir/$BINARY_NAME" "$INSTALL_DIR/$BINARY_NAME" \
  || fail "could not install into $INSTALL_DIR."
say "Installed $INSTALL_DIR/$BINARY_NAME ($("$INSTALL_DIR/$BINARY_NAME" version))"

# --- 5. PATH -----------------------------------------------------------------------------
path_needs_attention=0
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) path_needs_attention=1 ;;
esac

# --- 6. wire it into Claude Code ---------------------------------------------------------
if [ "$should_run_setup" -eq 1 ]; then
  # shellcheck disable=SC2086
  "$INSTALL_DIR/$BINARY_NAME" setup $setup_arguments

  # setup only starts the overlay when it registers the login item. Start it here otherwise:
  # without the menu bar icon there is nowhere to switch the remaining options on.
  case " $setup_arguments " in
    *" --with-autostart "*) ;;
    *)
      if ! pgrep -f "$BINARY_NAME run" >/dev/null 2>&1; then
        # nohup so closing the terminal that ran the installer does not take the pet with it.
        nohup "$INSTALL_DIR/$BINARY_NAME" run >/dev/null 2>&1 &
        sleep 1
        say ""
        say "The pet is on screen now. Everything else lives in the menu bar 🐾 menu."
      fi
      ;;
  esac
else
  say "Skipped setup (--no-setup). Run \`$INSTALL_DIR/$BINARY_NAME setup\` when you are ready."
fi

if [ "$path_needs_attention" -eq 1 ]; then
  say ""
  say "One more thing: $INSTALL_DIR is not on your PATH. Add it with"
  case "${SHELL:-}" in
    */zsh) say "  echo 'export PATH=\"$INSTALL_DIR:\$PATH\"' >> ~/.zshrc && exec zsh" ;;
    */bash) say "  echo 'export PATH=\"$INSTALL_DIR:\$PATH\"' >> ~/.bash_profile && exec bash -l" ;;
    *) say "  export PATH=\"$INSTALL_DIR:\$PATH\"   (add this to your shell profile)" ;;
  esac
  say "The pet works without it — this is only so you can type \`claude-airou\` yourself."
fi
