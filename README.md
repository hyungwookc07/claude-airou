# claude-pet 🐾

Claude Code용 데스크톱 펫. OpenAI Codex 앱의 펫처럼 화면 구석에 떠 있는 픽셀 캐릭터가
Claude가 **생각 중 / 작업 중 / 승인 대기 / 입력 대기 / 완료 / 에러** 인지 실시간으로 보여준다.

- 네이티브 macOS 오버레이 (Swift, AppKit + SwiftUI). 외부 의존성 0, 바이너리 하나.
- Claude Code **hooks**로 상태를 받는다 → 터미널 CLI, 데스크톱 앱, IDE 확장 어디서 돌려도 동작.
- 항상 위에 떠 있고, 포커스를 뺏지 않고, 모든 Space/전체화면 위에 보인다. 드래그로 옮기고 위치를 기억한다.
- 여러 세션을 동시에 추적. 승인이 필요한 세션이 있으면 그 세션을 우선 표시.
- 펫은 JSON 픽셀아트 팩. 내장 펫 + `~/.claude-pet/pets/*.json` 커스텀 펫. `/hatch-pet` 스킬로 Claude에게 새 펫을 만들게 할 수 있다 (Codex의 `/hatch` 대응).

![states](docs/states.png)

## 설치

요구사항: macOS 14+, Swift 5.9+ 툴체인 (Xcode 또는 Command Line Tools).

```bash
git clone <this repo> && cd claude-pet
make install      # → ~/.local/bin/claude-pet
make hooks        # ~/.claude/settings.json 에 hook 등록 (백업 파일을 먼저 만든다)
make skill        # (선택) /hatch-pet 스킬을 ~/.claude/skills 에 설치
claude-pet        # 오버레이 실행 (메뉴바 🐾 아이콘 + 펫)
```

이미 열려 있던 Claude Code 세션은 hook 설정을 다시 읽지 않으므로 **새 세션을 시작**해야 펫이 반응한다.

로그인 시 자동 실행: `make autostart` (해제는 `make no-autostart`).

## 동작 원리

```
Claude Code ──hook 이벤트(stdin JSON)──▶ claude-pet hook ──▶ ~/.claude-pet/state/<session>.json
                                                                       ▲
                                                 claude-pet (오버레이) ─┘ 0.3초마다 읽어서 스프라이트/말풍선 갱신
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

hook 바이너리는 stdout에 아무것도 쓰지 않고 항상 exit 0 으로 끝난다 (Claude Code는 일부 이벤트의 hook stdout을 모델 컨텍스트에 넣기 때문). 무슨 일이 있었는지는 `~/.claude-pet/hook.log`에 남는다.

병렬 도구 호출·서브에이전트도 같은 `session_id`로 hook을 쏘기 때문에 hook은 단순 덮어쓰기가 아니라 병합한다:
승인/질문을 기다리는 동안 형제 도구의 `PostToolUse`나 서브에이전트 이벤트는 무시되고, 기다리던 그 도구(`tool_use_id`)가 끝나거나 `PostToolBatch`/`Stop`/`UserPromptSubmit`이 오면 풀린다 (`Hook/HookMergePolicy.swift`).

### 알려진 한계

- **승인 후에도 잠시 시계가 남는다.** Claude Code에는 "사용자가 승인했다"는 hook이 없다. 승인하면 도구가 실행되고 끝날 때(`PostToolUse`) 풀리므로, 오래 걸리는 명령을 승인하면 그동안 빨간 시계가 유지된다.
- **거부/Esc는 이벤트가 없다.** `Stop`은 인터럽트 때 안 오고 거부는 `PostToolUseFailure`를 내지 않는다. 다음 이벤트(`PostToolBatch`, 새 프롬프트, 60초 후 `idle_prompt`)가 정리하고, 그래도 안 오면 20분(대기)·15분(작업) 뒤 자동으로 idle 이 된다.
- 상태 파일 위치를 바꾸려면 `CLAUDE_PET_HOME=/path` (오버레이와 hook 양쪽에 같은 값이 보여야 한다).

## 사용법

```
claude-pet                       오버레이 실행
claude-pet simulate demo         상태를 순서대로 돌려보기 (hook 없이 확인)
claude-pet simulate waiting_approval --message "Approve? git push"
claude-pet status                오버레이가 보고 있는 세션 목록
claude-pet pets                  사용 가능한 펫 목록
claude-pet validate FILE.json    펫 JSON 검증
claude-pet render PET --out DIR  모든 프레임을 PNG로 (sheet.png 포함)
claude-pet preview PET           ASCII 미리보기
claude-pet snapshot --out a.png  실행 중인 오버레이를 PNG로 저장 (화면 녹화 권한 불필요)
claude-pet install-hooks [--print]   / uninstall-hooks
```

펫을 **클릭**하면 쓰다듬기(하트 + 대사), **드래그**하면 이동, **우클릭**하거나 메뉴바 🐾를 누르면 메뉴:
펫 선택 · 크기(Small/Medium/Large) · 말풍선 숨기기 · 클릭 통과 · 펫 숨기기 · 위치 초기화 · hook 설치 · 로그 열기.

설정은 `~/.claude-pet/config.json`.

## 커스텀 펫 만들기

Claude Code에서:

```
/hatch-pet a sleepy axolotl who thinks every build will pass
```

스킬이 `~/.claude-pet/pets/<id>.json`을 만들고 `claude-pet validate` / `render`로 확인한 뒤 결과 시트를 보여준다.
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

내장 펫 소스는 `Sources/ClaudePet/Resources/pets/`에 있고 빌드 시 바이너리에 임베드된다.

## 문제 해결

- 펫이 반응하지 않음 → `~/.claude/settings.json`에 `hooks`가 있는지 (`claude-pet install-hooks --print`로 기대 형태 확인), Claude Code 세션을 새로 시작했는지, `~/.claude-pet/hook.log`에 줄이 찍히는지 확인.
- 특정 세션이 승인 대기에 멈춰 보임 → 세션이 죽었을 수 있다. 30분 뒤 자동으로 idle, `claude-pet status`로 확인, `rm ~/.claude-pet/state/<id>.json`.
- 오버레이가 화면 밖 → 메뉴바 🐾 → Reset position.
- 클릭 통과를 켜서 펫을 클릭할 수 없게 됐다면 메뉴바 🐾에서 끈다.

## 제거

```bash
make uninstall          # hooks 제거(백업 생성), 바이너리/스킬/LaunchAgent 삭제
rm -rf ~/.claude-pet    # 설정·펫·상태까지 지우려면
```

## 개발

```bash
swift build && .build/debug/claude-pet run
make render-all         # 내장 펫 전부 렌더 → render/<id>/sheet.png
```

구조: `Sources/ClaudePet/{Hook,State,Pets,UI,Install,CLI}`. hook 이벤트 → 상태 매핑은 `Hook/HookEventMapper.swift` 한 곳에 있다.
