# claude-airou 🐾

[English README](README.md)

Claude Code용 데스크톱 펫. (이름은 몬스터헌터의 동료 고양이 아이루에서 — 퀘스트에 따라다니니까.) OpenAI Codex 앱의 펫처럼 화면 구석에 떠 있는 픽셀 캐릭터가
Claude가 **생각 중 / 작업 중 / 승인 대기 / 입력 대기 / 완료 / 에러** 인지 실시간으로 보여준다.

- 네이티브 macOS 오버레이 (Swift, AppKit + SwiftUI). 외부 의존성 0, 바이너리 하나.
- Claude Code **hooks**로 상태를 받는다 → 터미널 CLI, 데스크톱 앱, IDE 확장 어디서 돌려도 동작.
- 항상 위에 떠 있고, 포커스를 뺏지 않고, 모든 Space/전체화면 위에 보인다. 드래그로 옮기고 위치를 기억한다.
- 여러 세션을 동시에 추적. 승인이 필요한 세션이 있으면 그 세션을 우선 표시.
- 펫은 JSON 픽셀아트 팩. 내장 펫 + `~/.claude-airou/pets/*.json` 커스텀 펫. `/hatch-pet` 스킬로 Claude에게 새 펫을 만들게 할 수 있다 (Codex의 `/hatch` 대응).

![states](docs/states.png)

내장 펫 8종 — **Airou**(마스코트: 아이루풍 사냥 고양이, `docs/make_airou.py`) · Mochi(고양이) · Quackers(오리) · Boo(유령) · Jelly(슬라임) · Bolt(로봇) · Inky(문어) · **Clawd**(Claude Code 시작 화면의 그 주황 마스코트, `docs/make_clawd.py`로 생성):

![pets](docs/pets.png)

## 설치

요구사항: macOS 14+, Swift 5.9+ 툴체인 (Xcode 또는 Command Line Tools).

```bash
git clone https://github.com/hyungwookc07/claude-airou.git && cd claude-airou
make install      # → ~/.local/bin/claude-airou
make hooks        # ~/.claude/settings.json 에 hook 등록 (백업 파일을 먼저 만든다)
make statusline   # (선택) 배터리 게이지에 Claude Code 상태줄 데이터를 공급 (아래 참고)
make skill        # (선택) /hatch-pet 스킬을 ~/.claude/skills 에 설치
claude-airou        # 오버레이 실행 (메뉴바 🐾 아이콘 + 펫)
```

이미 열려 있던 Claude Code 세션은 hook 설정을 다시 읽지 않으므로 **새 세션을 시작**해야 펫이 반응한다.

로그인 시 자동 실행: `make autostart` (LaunchAgent `dev.claude-airou.overlay`; 해제는 `make no-autostart`).
오버레이는 한 번에 하나만 뜬다 — 두 번째 `claude-airou run`은 "already running"만 찍고 종료한다 (`~/.claude-airou/overlay.lock`).

## 동작 원리

```
Claude Code ──hook 이벤트(stdin JSON)──▶ claude-airou hook ──▶ ~/.claude-airou/state/<session>.json
                                                                       ▲
                                                 claude-airou (오버레이) ─┘ 0.3초마다 읽어서 스프라이트/말풍선 갱신
```

| Claude Code hook 이벤트 | 펫 상태 | 표시 |
|---|---|---|
| `SessionStart` (startup/resume/clear) | hello | 👋 인사 (몇 초 후 idle) |
| `UserPromptSubmit`, `PostToolUse`, `PostToolBatch`, `PreCompact`, `SessionStart(compact)` | thinking | 생각 점 |
| `PreToolUse`, `SubagentStart` | working | ⚙️ + "Reading foo.swift" 같은 도구 요약 말풍선 |
| `PermissionRequest`, `Notification(permission_prompt)` | waiting_approval | 🔴 빨간 시계 (펄스) — **승인 필요** |
| `PreToolUse(AskUserQuestion / ExitPlanMode)`, `Notification(agent_needs_input / elicitation_dialog)`, `Elicitation` | needs_input | 🟠 물음표 — 당신 차례 |
| `Stop`, `Notification(agent_completed)` | done | ✅ 초록 체크 + 점프 (몇 초 후 idle) |
| `PostToolUseFailure`, `StopFailure` | error | ❗ 흔들림 |
| `Notification(idle_prompt)` | idle | (Claude가 응답을 마치고 60초 지남 — 막힌 상태를 정리) |
| `SessionEnd` | (세션 제거) | |

hook 바이너리는 stdout에 아무것도 쓰지 않고 항상 exit 0 으로 끝난다 (Claude Code는 일부 이벤트의 hook stdout을 모델 컨텍스트에 넣기 때문). 무슨 일이 있었는지는 `~/.claude-airou/hook.log`에 남는다.

병렬 도구 호출·서브에이전트도 같은 `session_id`로 hook을 쏘기 때문에 hook은 단순 덮어쓰기가 아니라 병합한다:
승인/질문을 기다리는 동안 형제 도구의 `PostToolUse`나 서브에이전트 이벤트는 무시되고, 기다리던 그 도구(`tool_use_id`)가 끝나거나 `PostToolBatch`/`Stop`/`UserPromptSubmit`이 오면 풀린다 (`Hook/HookMergePolicy.swift`).

### 알려진 한계

- **승인 후에도 잠시 시계가 남는다.** Claude Code에는 "사용자가 승인했다"는 hook이 없다. 승인하면 도구가 실행되고 끝날 때(`PostToolUse`) 풀리므로, 오래 걸리는 명령을 승인하면 그동안 빨간 시계가 유지된다.
- **거부/Esc는 이벤트가 없다.** `Stop`은 인터럽트 때 안 오고 거부는 `PostToolUseFailure`를 내지 않는다. 다음 이벤트(`PostToolBatch`, 새 프롬프트, 60초 후 `idle_prompt`)가 정리하고, 그래도 안 오면 20분(대기)·15분(작업) 뒤 자동으로 idle 이 된다.
- 상태 파일 위치를 바꾸려면 `CLAUDE_AIROU_HOME=/path` (오버레이와 hook 양쪽에 같은 값이 보여야 한다).

## 사용법

```
claude-airou                       오버레이 실행
claude-airou simulate demo         상태를 순서대로 돌려보기 (hook 없이 확인)
claude-airou simulate waiting_approval --message "Approve? git push"
claude-airou status                오버레이가 보고 있는 세션 목록
claude-airou pets                  사용 가능한 펫 목록
claude-airou validate FILE.json    펫 JSON 검증
claude-airou render PET --out DIR  모든 프레임을 PNG로 (sheet.png 포함)
claude-airou preview PET           ASCII 미리보기
claude-airou snapshot --out a.png  실행 중인 오버레이를 PNG로 저장 (화면 녹화 권한 불필요)
claude-airou install-hooks [--print]   / uninstall-hooks
```

펫을 **클릭**하면 쓰다듬기(하트 + 대사), **드래그**하면 이동, **우클릭**하거나 메뉴바 🐾를 누르면 메뉴:
펫 선택 · 크기(Small/Medium/Large) · 세션(고정) · 게이지 항목 · 세션 전부 펼쳐 보기 · 말풍선 숨기기 · 클릭 통과 · 펫 숨기기 · 위치 초기화 · hook 설치 · 로그 열기.

펫 아래에는 **배터리 게이지**(기본: 컨텍스트 창 잔량, 메뉴 → Gauge에서 5시간/7일 한도 잔량이나 끄기로 전환)와 **상태 아이콘이 붙은 세션 라벨**(빨간 시계 = 승인 대기, ⚙️ 작업 중, ✅ 완료 …)이 있다.

### 배터리 게이지

Claude Code는 상태줄 명령에 `context_window.used_percentage`, `rate_limits.five_hour / seven_day`, `cost`가 담긴 JSON을 넘긴다. `make statusline`(또는 `claude-airou install-statusline`)은 `settings.statusLine`을 `claude-airou statusline`으로 바꾸는데, 이 명령은 그 수치를 세션별로 기록한 뒤 **원래 쓰던 상태줄 명령을 같은 stdin으로 그대로 실행**한다 — 터미널 상태줄은 전과 똑같이 보인다. 원래 설정은 `~/.claude-airou/statusline-passthrough.json`에 보관되고 `claude-airou uninstall-statusline`으로 복원된다.

상태줄이 돌지 않는 세션(일부 데스크톱 앱 세션 등)도 컨텍스트 게이지는 나온다: hook이 transcript의 마지막 assistant 메시지 토큰 사용량으로 추정한다. 한도(rate limit)는 상태줄로만 알 수 있다.

### 세션이 여러 개일 때

접힌 상태에선 가장 중요한 세션(승인 대기 > 작업 중 > 최근) 하나만 `project +N` 배지와 함께 보인다 — 배지에 빨간 점이 있으면 *다른* 세션이 당신을 기다리는 중. **펫을 클릭하면 세션이 좌우로 펼쳐진다**: 지금 세션은 가운데 원래 크기, 나머지는 좌우에 70% 크기로 각자 표정 · 상태 배지 · 프로젝트명을 달고 늘어선다.

![sessions](docs/sessions.png)

- 옆 펫을 클릭하면 그 세션이 가운데로 **고정**된다 (자동 규칙보다 우선; 메뉴 "Sessions → Automatic"으로 해제).
- 가운데 펫을 다시 클릭하면 접힌다. 세션이 하나만 남으면 저절로 접힌다.
- 메뉴 → "Show all sessions side by side"를 켜면 항상 펼쳐진 상태.
- 펼쳐지고 접힐 때 가운데 펫은 화면에서 움직이지 않는다 (창만 좌우로 늘었다 줄어듦).

설정은 `~/.claude-airou/config.json`.

## 커스텀 펫 만들기

Claude Code에서:

```
/hatch-pet a sleepy axolotl who thinks every build will pass
```

스킬이 `~/.claude-airou/pets/<id>.json`을 만들고 `claude-airou validate` / `render`로 확인한 뒤 결과 시트를 보여준다.
메뉴바 🐾 → Pet → 새 펫 선택 (이미 실행 중이면 "Reload pets").

직접 만들려면 [skills/hatch-pet/SKILL.md](skills/hatch-pet/SKILL.md)의 포맷을 따르면 된다. 요약:

```json
{
  "id": "nori-axolotl", "name": "Nori", "species": "axolotl", "fps": 3,
  "palette": { "k": "#3a2a2a", "p": "#f6a7c1", "w": "#ffffff", "e": "#222222" },
  "phrases": { "pet": ["blub."] },
  "frames": {
    "idle":             [ ["..kk..", ".kppk.", "..kk.."], ["..kk..", ".kppk.", "..kk.."] ],
    "thinking":         [ ["..."] ],
    "working":          [ ["..."] ],
    "waiting_approval": [ ["..."] ],
    "needs_input":      [ ["..."] ],
    "done":             [ ["..."] ],
    "error":            [ ["..."] ],
    "hello":            [ ["..."] ]
  }
}
```

- 팔레트 키는 한 글자, `.`/공백은 투명. 모든 프레임은 같은 크기(16×16~24×24 권장).
- 없는 상태는 자동 폴백 (`working→thinking→idle`, `hello→done→idle`, ...).
- 상태 아이콘(빨간 시계, 초록 체크)은 오버레이가 그리므로 스프라이트는 표정만 바꾸면 된다.

내장 펫 소스는 `Sources/ClaudeAirou/Resources/pets/`에 있고 빌드 시 바이너리에 임베드된다.

## 문제 해결

- 펫이 반응하지 않음 → `~/.claude/settings.json`에 `hooks`가 있는지 (`claude-airou install-hooks --print`로 기대 형태 확인), Claude Code 세션을 새로 시작했는지, `~/.claude-airou/hook.log`에 줄이 찍히는지 확인.
- 특정 세션이 승인 대기에 멈춰 보임 → 세션이 죽었을 수 있다. 30분 뒤 자동으로 idle, `claude-airou status`로 확인, `rm ~/.claude-airou/state/<id>.json`.
- 오버레이가 화면 밖 → 메뉴바 🐾 → Reset position.
- 클릭 통과를 켜서 펫을 클릭할 수 없게 됐다면 메뉴바 🐾에서 끈다.

## 제거

```bash
make uninstall          # hooks 제거(백업 생성), 바이너리/스킬/LaunchAgent 삭제
rm -rf ~/.claude-airou    # 설정·펫·상태까지 지우려면
```

## 개발

```bash
swift build && .build/debug/claude-airou run
make render-all         # 내장 펫 전부 렌더 → render/<id>/sheet.png
```

구조: `Sources/ClaudeAirou/{Hook,State,Pets,UI,Install,CLI}`. hook 이벤트 → 상태 매핑은 `Hook/HookEventMapper.swift` 한 곳에 있다.

## 라이선스

MIT — [LICENSE](LICENSE) 참고.
