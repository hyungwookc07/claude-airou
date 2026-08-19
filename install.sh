#!/bin/sh
# claude-airou installer — downloads the latest release binary and wires it into Claude Code.
#
#   curl -fsSL https://raw.githubusercontent.com/hyungwookc07/claude-airou/main/install.sh | sh
#
# Re-run it any time to update: the binary is replaced, already-registered settings are left
# alone. Everything it touches lives in your home directory; nothing needs sudo.
#
# Options (curl … | sh -s -- --minimal):
#   --minimal          hooks + /hatch-pet skill only, no start-at-login
#   --no-autostart     skip start-at-login
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
    --minimal|--no-autostart|--with-statusline|--with-mcp) setup_arguments="$setup_arguments $argument" ;;
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
  # The published file names the asset by its bare name, so verify from inside the temp dir.
  ( cd "$temporary_dir" && shasum -a 256 -c "$ASSET_NAME.sha256" >/dev/null ) \
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
