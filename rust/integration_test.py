#!/usr/bin/env python3
"""Integration battery for the claude-airou Rust binary.

Drives the real binary as subprocesses against a throwaway CLAUDE_AIROU_HOME — nothing
under your real ~/.claude-airou or ~/.claude is touched. Covers the hook lifecycle
(incl. the approval merge policy), the transcript estimator, the status line
passthrough, a full MCP conversation (hatch_pet returns a real PNG), all three
installers against fixture files, Swift-formatted file interop, and robustness
(garbage input, hostile session ids, 20 concurrent hooks).

    make test        (from the repo root: cargo test + this file)
    cd rust && cargo build --release && python3 integration_test.py

Override the binary with CLAUDE_AIROU_BIN=/path/to/claude-airou.
Works the same on macOS and Linux (the overlay UI itself is not covered — run
`claude-airou run` and `claude-airou simulate demo` by hand for that).
"""
import base64, hashlib, json, os, shutil, subprocess, sys, tempfile, time

BIN = os.environ.get("CLAUDE_AIROU_BIN") or os.path.join(
    os.path.dirname(os.path.abspath(__file__)), "target", "release", "claude-airou"
)
if not os.path.exists(BIN):
    sys.exit(f"binary not found at {BIN} — run `cargo build --release` first "
             f"(or set CLAUDE_AIROU_BIN)")
SANDBOX = tempfile.mkdtemp(prefix="claude-airou-itest-")
AIROU = os.path.join(SANDBOX, "airou-home")
STATE = os.path.join(AIROU, "state")
ENV = {**os.environ, "CLAUDE_AIROU_HOME": AIROU}

passed, failed = [], []

def check(name, condition, detail=""):
    if condition:
        passed.append(name)
    else:
        failed.append((name, detail))
        print(f"FAIL {name}  {detail}")

def hook(payload):
    return subprocess.run([BIN, "hook"], input=json.dumps(payload).encode(),
                          env=ENV, capture_output=True)

def state_of(session):
    path = os.path.join(STATE, f"{session}.json")
    if not os.path.exists(path):
        return None
    with open(path) as f:
        return json.load(f)

def usage_of(session):
    path = os.path.join(STATE, f"{session}.usage.json")
    if not os.path.exists(path):
        return None
    with open(path) as f:
        return json.load(f)

os.makedirs(AIROU, exist_ok=True)

# ---------- A. Claude Code session lifecycle through the hook ----------
S = "itest-lifecycle"
hook({"hook_event_name": "SessionStart", "session_id": S, "cwd": "/tmp/myproj"})
st = state_of(S)
check("A1 SessionStart→hello", st and st["state"] == "hello" and st["message"] == "Hi! Ready when you are" and st["cwd"] == "/tmp/myproj", str(st))

hook({"hook_event_name": "UserPromptSubmit", "session_id": S, "cwd": "/tmp/myproj"})
check("A2 UserPromptSubmit→thinking", state_of(S)["state"] == "thinking")

hook({"hook_event_name": "PreToolUse", "session_id": S, "cwd": "/tmp/myproj",
      "tool_name": "Read", "tool_input": {"file_path": "/a/b/main.rs"}, "tool_use_id": "t1"})
st = state_of(S)
check("A3 PreToolUse Read→working", st["state"] == "working" and st["message"] == "Reading main.rs" and st.get("toolName") == "Read", str(st))

hook({"hook_event_name": "PermissionRequest", "session_id": S, "cwd": "/tmp/myproj",
      "tool_name": "Bash", "tool_input": {"command": "git push"}, "tool_use_id": "t9"})
st = state_of(S)
check("A4 PermissionRequest→waiting_approval+pendingId", st["state"] == "waiting_approval"
      and st["message"] == "Approve? Running: git push" and st.get("pendingToolUseId") == "t9", str(st))

hook({"hook_event_name": "PostToolUse", "session_id": S, "cwd": "/tmp/myproj",
      "tool_name": "Read", "tool_use_id": "t1"})
check("A5 sibling PostToolUse kept", state_of(S)["state"] == "waiting_approval")

hook({"hook_event_name": "PostToolUse", "session_id": S, "cwd": "/tmp/myproj",
      "tool_name": "Grep", "tool_use_id": "t2", "agent_id": "subagent-123"})
check("A6 subagent event kept", state_of(S)["state"] == "waiting_approval")

hook({"hook_event_name": "PostToolUse", "session_id": S, "cwd": "/tmp/myproj",
      "tool_name": "Bash", "tool_use_id": "t9"})
check("A7 awaited tool finished→thinking", state_of(S)["state"] == "thinking")

hook({"hook_event_name": "Stop", "session_id": S, "cwd": "/tmp/myproj"})
st = state_of(S)
check("A8 Stop→done", st["state"] == "done" and st["message"] == "Done!")

hook({"hook_event_name": "Notification", "notification_type": "idle_prompt", "session_id": S, "cwd": "/tmp/myproj"})
st = state_of(S)
check("A9 idle_prompt→idle", st["state"] == "idle" and st["message"] == "")

hook({"hook_event_name": "SessionEnd", "session_id": S, "cwd": "/tmp/myproj", "reason": "exit"})
check("A10 SessionEnd removes file", state_of(S) is None)

with open(os.path.join(AIROU, "hook.log")) as f:
    log_text = f.read()
check("A11 merge-policy log lines", "kept (sibling tool t1 finished while waiting on t9)" in log_text
      and "kept (subagent PostToolUse while waiting_approval)" in log_text, log_text[-400:])

# ---------- B. Transcript context estimate ----------
S2 = "itest-estimator"
transcript = os.path.join(SANDBOX, "transcript.jsonl")
with open(transcript, "w") as f:
    f.write(json.dumps({"type": "user", "message": {"content": "hi"}}) + "\n")
    f.write(json.dumps({"type": "assistant", "isSidechain": False, "message": {"usage": {
        "input_tokens": 1000, "cache_creation_input_tokens": 9000,
        "cache_read_input_tokens": 150000, "output_tokens": 50}}}) + "\n")
hook({"hook_event_name": "PostToolUse", "session_id": S2, "cwd": "/tmp/x",
      "tool_name": "Read", "transcript_path": transcript})
u = usage_of(S2)
check("B1 transcript estimate", u and u["source"] == "transcript" and u["contextTokens"] == 160000
      and abs(u["contextUsedPercentage"] - 80.0) < 0.01, str(u))

# ---------- C. Status line recording + passthrough ----------
sl_json = json.dumps({
    "session_id": "itest-sl",
    "context_window": {"used_percentage": 42.4, "context_window_size": 200000},
    "rate_limits": {"five_hour": {"used_percentage": 10.0}, "seven_day": {"used_percentage": 3.0}},
    "cost": {"total_cost_usd": 1.23}, "model": {"display_name": "Opus"},
}).encode()
r = subprocess.run([BIN, "statusline", "--then", "printf PASSTHRU; exit 5"],
                   input=sl_json, env=ENV, capture_output=True)
check("C1 passthrough stdout+exit code", r.stdout == b"PASSTHRU" and r.returncode == 5,
      f"stdout={r.stdout!r} rc={r.returncode} err={r.stderr!r}")
u = usage_of("itest-sl")
check("C2 usage recorded", u and u["source"] == "status_line" and u["contextUsedPercentage"] == 42.4
      and u["fiveHourUsedPercentage"] == 10.0 and u["totalCostUSD"] == 1.23, str(u))
subprocess.run([BIN, "simulate", "working", "--session", "itest-sl", "--cwd", "/tmp/slproj"],
               env=ENV, capture_output=True)
r = subprocess.run([BIN, "status"], env=ENV, capture_output=True, text=True)
check("C3 status shows usage suffix", "[ctx 42% 5h 10% 7d 3% status_line]" in r.stdout, r.stdout)
subprocess.run([BIN, "simulate", "clear", "--session", "itest-sl"], env=ENV, capture_output=True)

# ---------- D. MCP conversation ----------
mcp = subprocess.Popen([BIN, "mcp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, env=ENV)

def rpc(obj):
    mcp.stdin.write((json.dumps(obj) + "\n").encode())
    mcp.stdin.flush()

def read_reply():
    line = mcp.stdout.readline()
    return json.loads(line) if line else None

rpc({"jsonrpc": "2.0", "id": 1, "method": "initialize",
     "params": {"protocolVersion": "2025-06-18", "clientInfo": {"name": "claude-ai", "version": "1"}}})
reply = read_reply()
check("D1 initialize result", reply["id"] == 1 and reply["result"]["protocolVersion"] == "2025-06-18"
      and reply["result"]["serverInfo"]["name"] == "claude-airou"
      and "pet_status" in reply["result"]["instructions"], json.dumps(reply)[:200])
chat_session = f"claude-chat-{mcp.pid}"
st = state_of(chat_session)
check("D2 hello snapshot with Claude Chat label", st and st["state"] == "hello" and st["cwd"] == "Claude Chat", str(st))

rpc({"jsonrpc": "2.0", "method": "notifications/initialized"})
rpc({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})
reply = read_reply()
names = [tool["name"] for tool in reply["result"]["tools"]]
check("D3 tools/list four tools", names == ["pet_status", "list_pets", "preview_pet", "hatch_pet"], str(names))

rpc({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
     "params": {"name": "pet_status", "arguments": {"state": "done", "message": "한글 테스트"}}})
reply = read_reply()
st = state_of(chat_session)
check("D4 pet_status Korean round-trip", reply["result"]["isError"] is False
      and st["state"] == "done" and st["message"] == "한글 테스트", str(st))

definition = {"id": "testy-blob", "name": "Testy", "species": "blob", "fps": 3,
              "palette": {"k": "#222222", "b": "#66ccff"},
              "frames": {"idle": [["kkkk", "kbbk", "kbbk", "kkkk"], ["kkkk", "kbbk", "kbbk", "kkkk"]]}}
rpc({"jsonrpc": "2.0", "id": 4, "method": "tools/call",
     "params": {"name": "hatch_pet", "arguments": {"definition": definition}}})
reply = read_reply()
content = reply["result"]["content"]
image_items = [c for c in content if c.get("type") == "image"]
png_ok = bool(image_items) and base64.b64decode(image_items[0]["data"])[:8] == b"\x89PNG\r\n\x1a\n"
pet_file = os.path.join(AIROU, "pets", "testy-blob.json")
check("D5 hatch_pet saves + returns real PNG", reply["result"]["isError"] is False and png_ok
      and os.path.exists(pet_file), json.dumps(content)[:200])
check("D6 post-tool thinking write", state_of(chat_session)["state"] == "thinking")

rpc({"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {"name": "preview_pet", "arguments": {"id": "testy-blob"}}})
reply = read_reply()
check("D7 preview_pet returns image", any(c.get("type") == "image" for c in reply["result"]["content"]))

rpc({"jsonrpc": "2.0", "id": 6, "method": "tools/call", "params": {"name": "nope", "arguments": {}}})
reply = read_reply()
check("D8 unknown tool -32602", reply.get("error", {}).get("code") == -32602, str(reply))

mcp.stdin.write(b"this is not json\n"); mcp.stdin.flush()
reply = read_reply()
check("D9 parse error -32700 id null", reply.get("error", {}).get("code") == -32700 and reply.get("id") is None, str(reply))

rpc([{"jsonrpc": "2.0", "id": 10, "method": "ping"}, {"jsonrpc": "2.0", "id": 11, "method": "ping"}])
first, second = read_reply(), read_reply()
check("D10 batch two pings", {first["id"], second["id"]} == {10, 11}, f"{first} {second}")

rpc({"jsonrpc": "2.0", "id": 12, "method": "wat/isthis"})
reply = read_reply()
check("D11 unknown method -32601", reply.get("error", {}).get("code") == -32601, str(reply))

rpc({"jsonrpc": "2.0", "id": 13, "method": "tools/call", "params": {"name": "list_pets", "arguments": {}}})
reply = read_reply()
text = reply["result"]["content"][0]["text"]
check("D12 list_pets shows hatched custom pet", "testy-blob — Testy the blob (4x4, custom)" in text, text[:300])

mcp.stdin.close()
rc = mcp.wait(timeout=10)
check("D13 EOF: exit 0 + session removed", rc == 0 and state_of(chat_session) is None, f"rc={rc}")

# ---------- E. Installers against fixture files ----------
settings = os.path.join(SANDBOX, "settings.json")
foreign = {"model": "opus",
           "statusLine": {"type": "command", "command": "printf ORIGINAL"},
           "hooks": {"PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "other-tool hook"}]}]}}
with open(settings, "w") as f:
    json.dump(foreign, f)

r = subprocess.run([BIN, "install-hooks", "--settings", settings], env=ENV, capture_output=True, text=True)
data = json.load(open(settings))
events = set(data["hooks"].keys())
pre = data["hooks"]["PreToolUse"]
check("E1 install-hooks adds 17 events, keeps foreign", r.returncode == 0 and len(events) == 17
      and data["model"] == "opus" and any("other-tool" in json.dumps(g) for g in pre)
      and any("claude-airou" in json.dumps(g) for g in pre), f"rc={r.returncode} events={len(events)}")

digest_before = hashlib.sha256(open(settings, "rb").read()).hexdigest()
backups_before = [p for p in os.listdir(SANDBOX) if "backup" in p]
subprocess.run([BIN, "install-hooks", "--settings", settings], env=ENV, capture_output=True)
digest_after = hashlib.sha256(open(settings, "rb").read()).hexdigest()
backups_after = [p for p in os.listdir(SANDBOX) if "backup" in p]
check("E2 reinstall is byte-identical no-op", digest_before == digest_after and backups_before == backups_after)

subprocess.run([BIN, "uninstall-hooks", "--settings", settings], env=ENV, capture_output=True)
data = json.load(open(settings))
check("E3 uninstall keeps only foreign", list(data["hooks"].keys()) == ["PreToolUse"]
      and "claude-airou" not in json.dumps(data) and data["model"] == "opus", json.dumps(data)[:200])

desktop = os.path.join(SANDBOX, "claude_desktop_config.json")
with open(desktop, "w") as f:
    json.dump({"mcpServers": {"other": {"command": "/bin/other"}}}, f)
subprocess.run([BIN, "install-mcp", "--config", desktop], env=ENV, capture_output=True)
data = json.load(open(desktop))
check("E4 install-mcp adds exec-form entry, keeps foreign",
      data["mcpServers"]["claude-airou"]["args"] == ["mcp"] and "other" in data["mcpServers"], json.dumps(data))
subprocess.run([BIN, "uninstall-mcp", "--config", desktop], env=ENV, capture_output=True)
data = json.load(open(desktop))
check("E5 uninstall-mcp removes only ours", "claude-airou" not in data.get("mcpServers", {}) and "other" in data["mcpServers"])

r = subprocess.run([BIN, "install-statusline", "--settings", settings], env=ENV, capture_output=True, text=True)
data = json.load(open(settings))
our_line = json.dumps(data.get("statusLine", {}))
r2 = subprocess.run([BIN, "statusline", "--settings", settings], input=sl_json, env=ENV, capture_output=True)
check("E6 install-statusline + stashed passthrough runs ORIGINAL",
      "claude-airou" in our_line and "statusline" in our_line and r2.stdout == b"ORIGINAL",
      f"line={our_line} out={r2.stdout!r} err={r2.stderr!r}")
subprocess.run([BIN, "uninstall-statusline", "--settings", settings], env=ENV, capture_output=True)
data = json.load(open(settings))
check("E7 uninstall-statusline restores original", data.get("statusLine") == foreign["statusLine"], json.dumps(data.get("statusLine")))

# ---------- F. Swift-formatted files read fine ----------
now = int(time.time())
swift_state = {"cwd": "/x/swiftproj", "lastEventName": "Stop", "message": "Done!",
               "sessionId": "swift-sess", "state": "done", "toolName": "Bash",
               "updatedAtEpochSeconds": now, "pendingToolUseId": "toolu_1"}
os.makedirs(STATE, exist_ok=True)
with open(os.path.join(STATE, "swift-sess.json"), "w") as f:
    json.dump(swift_state, f, sort_keys=True)  # Swift: sortedKeys + integer double
swift_usage = {"sessionId": "swift-sess", "source": "status_line", "updatedAtEpochSeconds": now,
               "contextUsedPercentage": 55, "totalCostUSD": 2}
with open(os.path.join(STATE, "swift-sess.usage.json"), "w") as f:
    json.dump(swift_usage, f, sort_keys=True)
r = subprocess.run([BIN, "status"], env=ENV, capture_output=True, text=True)
check("F1 Swift-format state+usage readable", "swift-sess\tswiftproj\tdone" in r.stdout
      and "ctx 55%" in r.stdout, r.stdout)
for suffix in (".json", ".usage.json"):
    os.remove(os.path.join(STATE, "swift-sess" + suffix))

# ---------- G. Robustness ----------
r = subprocess.run([BIN, "hook"], input=os.urandom(5 * 1024 * 1024), env=ENV, capture_output=True)
check("G1 5MB garbage to hook exits 0", r.returncode == 0 and r.stdout == b"")

hook({"hook_event_name": "SessionStart", "session_id": "../../evil id!", "cwd": "/tmp"})
check("G2 hostile session id sanitized", os.path.exists(os.path.join(STATE, "evilid.json"))
      and not os.path.exists(os.path.join(SANDBOX, "evil id!.json")))
os.remove(os.path.join(STATE, "evilid.json"))

procs = [subprocess.Popen([BIN, "hook"], stdin=subprocess.PIPE, env=ENV) for _ in range(20)]
for i, p in enumerate(procs):
    p.stdin.write(json.dumps({"hook_event_name": "PreToolUse", "session_id": "race-sess",
                              "cwd": "/tmp", "tool_name": "Read",
                              "tool_input": {"file_path": f"/f/{i}.rs"}}).encode())
    p.stdin.close()
codes = [p.wait(timeout=15) for p in procs]
st = state_of("race-sess")
check("G3 20 parallel hooks: all exit 0, state file intact", all(c == 0 for c in codes)
      and st is not None and st["state"] == "working", f"codes={set(codes)} st={st}")

mcp2 = subprocess.Popen([BIN, "mcp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, env=ENV)
mcp2.stdin.write(b"x" * (2 * 1024 * 1024) + b"\n"); mcp2.stdin.flush()
first = json.loads(mcp2.stdout.readline())
mcp2.stdin.write(json.dumps({"jsonrpc": "2.0", "id": 1, "method": "ping"}).encode() + b"\n"); mcp2.stdin.flush()
second = json.loads(mcp2.stdout.readline())
mcp2.stdin.close(); mcp2.wait(timeout=10)
check("G4 mcp survives 2MB garbage line", first.get("error", {}).get("code") == -32700
      and second.get("id") == 1 and second.get("result") == {}, f"{first} {second}")

print(f"\n{'='*50}\n{len(passed)} passed, {len(failed)} failed")
for name, detail in failed:
    print(f"  FAILED: {name}")
if failed:
    print(f"sandbox kept for inspection: {SANDBOX}")
else:
    shutil.rmtree(SANDBOX, ignore_errors=True)
sys.exit(1 if failed else 0)
