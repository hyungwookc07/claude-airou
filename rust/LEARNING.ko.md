# 이 코드베이스로 러스트 공부하기

이 레포는 러스트 학습 교재로 꽤 유리한 조건을 갖고 있다: **같은 프로그램이 Swift와
러스트(`rust/src/`)로 1:1 대응되게 존재**한다. 이미 아는 쪽(동작)을 기준으로 낯선 쪽(문법·개념)을
읽을 수 있고, 모든 모듈에 테스트가 있어서 "고쳐보고 `cargo test`"가 즉시 가능하다.

> Swift 원본(`Sources/ClaudeAirou/`)은 v1.0에서 레포에서 삭제됐다. 아래에서 `Models/PetState.swift`처럼
> Swift 파일을 가리키는 곳은 모두 git 히스토리의 커밋 `3037817`을 기준으로 한다 — 나란히 놓고
> 읽으려면 `git show 3037817:Sources/ClaudeAirou/Models/PetState.swift` 로 꺼내 보거나
> `git worktree add ../claude-airou-swift 3037817` 으로 그 시점을 통째로 체크아웃하면 된다.

읽는 순서는 쉬운 것 → 어려운 것, 그리고 각 단계에서 새로 등장하는 러스트 개념을 하나씩만
추가하도록 짰다.

## 준비

```bash
cd rust
cargo test            # 전부 초록인 상태에서 시작
cargo doc --open      # 내 크레이트 문서를 브라우저로 (doc 주석이 그대로 API 문서가 된다)
```

수정 → `cargo test` → 되돌리기(git checkout)를 반복하는 게 가장 빠른 학습 루프다.
일부러 코드를 망가뜨렸을 때 **컴파일러가 뭐라고 하는지** 읽는 것 자체가 공부다 —
러스트 컴파일러의 에러 메시지는 사실상 튜토리얼이다.

## 1단계 — `model.rs` ↔ `Models/PetState.swift`: enum, Option, serde

첫 파일로 이만한 게 없다. Swift 파일을 왼쪽에, 러스트를 오른쪽에 놓고 대응시켜 보자.

| Swift | Rust | 눈여겨볼 것 |
|---|---|---|
| `enum PetState: String, Codable` | `#[derive(Serialize, Deserialize)] enum PetState` + `#[serde(rename_all = "snake_case")]` | 러스트 enum은 원시값이 없다. 문자열 표현은 serde 속성이나 `raw()` 같은 메서드로 별도 제공 |
| `var isBusy: Bool { self == .thinking … }` | `pub fn is_busy(self) -> bool { matches!(self, …) }` | 계산 프로퍼티가 없어서 그냥 메서드. `matches!` 매크로는 `switch` 한 줄 축약 |
| `TimeInterval?` (Optional) | `Option<f64>` | 개념이 1:1이다. `nil` = `None`, 강제 언래핑 `!` 는 `.unwrap()` (그리고 우리 코드에선 I/O 경로에서 금지) |
| `switch state { case .hello: … }` | `match state { PetState::Hello => … }` | `match`는 **모든 경우를 강제**한다 — Swift의 exhaustive switch와 같은 철학 |
| `struct SessionSnapshot: Codable` | `#[serde(rename_all = "camelCase")] struct SessionSnapshot` | Swift Codable의 "프로퍼티명 = JSON 키" 규칙을 serde 속성으로 재현한 것. **두 언어가 같은 파일을 읽고 쓰는 비결이 이 속성 몇 줄이다** |

연습: `transient_duration_secs`에서 `Hello`의 4.0을 지워보라. 컴파일 에러가 아니라 테스트
실패(`effective_state_decays`)가 난다. 반대로 `match`에서 가지 하나를 지우면 **컴파일**이
거부된다. "타입으로 막을 수 있는 버그"와 "테스트로 잡는 버그"의 경계를 체감할 수 있다.

## 2단계 — `state_store.rs` ↔ `State/SessionStateStore.swift`: Result, ?, 소유권 입문

- Swift의 `throws` / `try` → 러스트의 `Result<T, E>` / `?`. `write()`의
  `crate::paths::ensure_dir(&self.directory)?` 에서 `?`는 "에러면 즉시 반환"이다.
  Swift에서 `try`가 하던 일을 값 수준에서 한다 — 예외가 아니라 반환값이라서,
  **에러를 무시하려면 무시한다고 코드에 써야 한다** (`let _ = …`).
- `load_all(&self)` 의 `&self`: 빌림(borrow). Swift는 참조/값 의미를 컴파일러가 알아서
  처리하지만, 러스트는 "읽기만 빌린다(`&`)/고쳐 쓰게 빌린다(`&mut`)/아예 가져간다(move)"를
  호출부 표기로 구분한다. 이 파일은 전부 `&self` — 저장소를 읽기만 하는 메서드들이기 때문.
- `write_atomic`: temp 파일 + rename. Foundation의 `.atomic` 옵션이 하던 일을 직접 짠 것.
  라이브러리 마법이 사라지면 어떤 일이 벌어지는지 보여주는 좋은 예.

연습: `sanitize_session_id`의 `.take(80)`을 `.take(8)`로 바꾸고 어떤 테스트가 왜 깨지는지
확인해 보라 (테스트가 파일명 규격의 계약서 역할을 한다).

## 3단계 — `hook_mapper.rs` ↔ `Hook/HookEventMapper.swift`: 패턴 매칭의 본편

이 포트에서 가장 "러스트다운" 파일. Swift의 거대한 `switch` 두 개가 러스트 `match`로
어떻게 번역되는지 본다.

- 연관값 있는 enum: Swift `case update(state:message:toolName:)` ↔ 러스트
  `MappingResult::Update { state, message, tool_name }`. 필드 이름 있는 variant는 사실상
  경량 구조체다.
- `if let interaction = mapUserInteractionTool(input) { return interaction }` ↔
  `if let Some(interaction) = map_user_interaction_tool(input) { return interaction; }` —
  Optional 언래핑 패턴이 그대로 대응된다.
- 문자열 처리: `truncate()`가 바이트가 아니라 **문자 수**로 자르는 이유를 주석과 테스트에서
  확인하라. 러스트 `String`은 UTF-8 바이트 벡터라서 `text[..100]` 같은 바이트 슬라이스는
  한글/이모지 중간을 자르면 **panic**한다. `chars()`를 세는 코드가 그 방어다.
  (Swift `String.count`는 grapheme 단위라 또 미묘하게 다르다 — 두 언어의 "문자"가
  다르다는 것 자체가 배울 점.)

## 4단계 — `hook.rs`, `cli_commands.rs`: I/O, 클로저, 테스트 설계

- `hook.rs`의 `process(input_data, store, log_path)` — I/O(stdin 읽기)와 로직을 분리해서
  테스트가 로직만 주입 호출한다. Swift 쪽 `HookCommand.run`과 비교하면 "테스트 가능하게
  자르는 위치"가 어디인지 보인다.
- `catch_unwind`: 러스트의 panic은 Swift의 fatalError에 가깝다. "hook은 절대 실패하면
  안 된다"는 계약을 지키기 위해 panic조차 흡수하는 방어가 어떻게 생겼는지 봐두면 좋다.
- `cli_commands.rs`의 `run_simulate_impl(…, sleep: &dyn Fn(f64))` — 클로저를 인자로 받아
  테스트에서는 sleep을 no-op으로 주입한다. Swift에서 함수 타입 파라미터 넘기던 것과 같은
  발상, 문법만 `&dyn Fn`.

## 5단계 — `mcp.rs`: 스레드, Mutex, Arc

Swift 쪽은 GCD(`DispatchQueue`, `DispatchSourceTimer`)로 동시성을 처리했다. 러스트 쪽은:

- `Arc<Mutex<ServerState>>` — 공유 상태의 러스트 정석. `Arc`는 참조 카운트(=Swift 클래스
  인스턴스의 ARC와 같은 원리를 명시적으로 쓴 것), `Mutex`는 잠금.
- 핵심 차이: **러스트는 잠그지 않고 공유 데이터를 만지는 코드가 컴파일되지 않는다.**
  Swift에서 `stateLock.lock()`을 깜빡해도 컴파일은 되는 것과 대비된다. `lock()`이 돌려주는
  가드가 스코프를 벗어나면 자동 해제되는 것(RAII)도 `defer { unlock() }`과 비교해 볼 것.
- 워치독: `std::thread::spawn` + 30초 루프. GCD 타이머의 저수준 번역이다.

## 6단계 — `pets.rs`, `render.rs`, `install.rs`: 컬렉션, 수명, 실전 직렬화

- `frames_for(&self, state) -> &[Vec<String>]` — 반환 타입의 `&`가 수명(lifetime) 개념의
  입구다. "self 안의 데이터를 복사 없이 빌려서 돌려준다"는 뜻이고, 컴파일러가 그 참조가
  self보다 오래 살지 못하게 감시한다. Swift에선 배열이 COW라 그냥 값을 돌려주면 됐던 자리.
- `render.rs`의 픽셀 버퍼(`Vec<u8>` + chunks_exact_mut) — CoreGraphics가 해주던 일을
  손으로 하는 코드. 인덱스 계산과 클리핑을 러스트가 얼마나 깐깐하게 다루는지 보인다.
- `install.rs` — `serde_json::Value`로 "타입을 모르는 JSON"을 다루는 법. 1~2단계의
  정적 타입 직렬화와 대비되는, 동적 JSON 트리 편집이다 (남의 설정 파일을 건드리니까
  모르는 키는 그대로 보존해야 한다 — 그래서 Value).

## 7단계 — `overlay/` (macOS): cfg, unsafe, FFI의 경계

- `#[cfg(target_os = "macos")]` — 플랫폼 분기가 파일/모듈 단위로 어떻게 걸리는지.
  `main.rs`의 `mod overlay;` 선언부터 따라가 보라.
- `present_macos.rs` — 이 크레이트의 `unsafe`가 모두 모여 있는 곳(CALayer 프레젠터,
  NSWindow/NSScreen/NSAlert 브리지). 러스트의 안전 보장이 어디까지고, OS API(Objective-C
  런타임)를 만나는 지점에서 어떻게 좁은 상자 안에 가두는지 보여준다. "unsafe는 금지가
  아니라 격리"라는 감각.
- `row_layout.rs` / `logic.rs` / `animation.rs` / `placement.rs`는 창 없이 단위 테스트되는
  순수 로직(Swift의 `RowLayout`, `PetViewModel`, SwiftUI 애니메이션, `OverlayPanel` 배치
  규칙의 포팅). "뷰 모델을 순수 함수로 짜고, 창(window.rs)은 그 결과를 그리기만 한다"는
  구조를 보기 좋다.
- `draw.rs`/`text.rs`는 의존성 거의 없이 소프트웨어 렌더링을 하는 순수 로직이라, GUI에
  관심 없어도 배열 다루기 연습 자료로 좋다.

## 개념 대응표 (요약)

| Swift | Rust |
|---|---|
| `Optional<T>` / `nil` / `??` | `Option<T>` / `None` / `.unwrap_or(…)` |
| `throws` → `do/try/catch` | `Result<T, E>` → `?` / `match` |
| `protocol` + 확장 | `trait` + `impl Trait for Type` |
| `Codable` | `serde` (`Serialize`/`Deserialize` derive) |
| `struct`(값) vs `class`(참조) | 기본이 값(move). 참조 공유가 필요하면 명시적으로 `Rc`/`Arc` |
| ARC (자동 참조 카운트) | 소유권 + 빌림 (컴파일 타임). 참조 카운트는 opt-in(`Arc`) |
| `defer` | RAII — 값이 스코프를 벗어나면 `Drop` 자동 실행 (MutexGuard 등) |
| `guard let x = … else { return }` | `let Some(x) = … else { return; };` (let-else) |
| GCD (`DispatchQueue`) | `std::thread` + 채널/`Mutex` (또는 async 런타임 — 이 크레이트는 안 씀) |
| `fatalError` / 크래시 | `panic!` — 그리고 그것조차 계약이 있으면 `catch_unwind`로 방어 |
| SwiftPM `Package.swift` | `Cargo.toml` (+ `Cargo.lock`, `cargo test`가 SwiftPM보다 훨씬 중심에 있다) |

## 추천 진행법

1. 위 순서로 하루 한 모듈씩, 항상 Swift 원본(커밋 `3037817`)과 나란히 읽는다.
2. 모듈마다 테스트를 하나 골라 일부러 깨뜨리고, 컴파일러/테스트가 각각 무엇을 잡아주는지 본다.
3. 작은 기능을 직접 추가해 본다. 좋은 첫 과제들:
   - `claude-airou pets --json` 옵션 (serde 직렬화 연습)
   - `simulate`에 `--count N` 반복 옵션 (CLI 파싱 연습)
   - 말풍선 이모지 렌더링 (Apple Color Emoji는 비트맵 폰트라 아직 빈칸 — 로드맵의 열린 항목이자 실전 기여)
4. 막히면 `cargo doc --open`과 러스트 공식 책(The Book, https://doc.rust-lang.org/book/)의
   해당 장을 찾아 읽는 식으로. 이 코드에 등장하는 개념은 대부분 4·6·8·10·13·16장에 있다.
