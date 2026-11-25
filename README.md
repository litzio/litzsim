# LitzSim · 리츠심

사람과 AI가 같은 회로도를 편집하고 실험을 재현하는 오픈소스 전력전자 시뮬레이터를 개발합니다.

첫 예제는 양방향 온보드 충전기(OBC)입니다. 엔지니어가 회로에서 선택한 가지에 AI가 센서나 필터를 넣고, 변경한 회로에서 스위칭 모델과 평균 모델의 응답을 비교하는 것이 목표입니다. 첫 제품은 [Litzio](https://github.com/litzio/litzio) 플러그인입니다.

## 현재 상태

**Rust로 다시 만드는 중입니다.** 플러그인의 **코어 확인** 버튼에서 동봉된 CLI의 버전과 호출 왕복 시간을 볼 수 있습니다. 재구축 기준은 [Rust 재구축](docs/rebuild.md)에 있습니다.

C++·Qt로 만든 1차 MVP(회로 편집기, 해석 엔진, 파형 창, 논문용 SVG/PDF 내보내기)는 [`archive/qt-cpp-mvp`](https://github.com/litzio/litzsim/tree/archive/qt-cpp-mvp) 브랜치와 `archive/qt-cpp-mvp-2026-11-26` 태그에 보관했습니다.

## CLI로 회로 편집하기

Rust 1.89 이상에서 `cargo build --release`로 CLI를 빌드합니다. 아래 명령으로 작업 전 파일을 복사하고, JSON 편집 배치를 적용하고, 복사본으로 되돌릴 수 있습니다.

```powershell
$cli = ".\target\release\litzsim.exe"
Copy-Item examples/buck.litzsim buck.litzsim
Copy-Item buck.litzsim previous.litzsim
$before = & $cli inspect buck.litzsim --json | ConvertFrom-Json
'[{"op":"mirror","id":"Q1","axis":"horizontal"},{"op":"setParameter","id":"R1","key":"resistance_ohm","value":47}]' |
    Set-Content -Encoding utf8NoBOM ops.json
$edited = & $cli apply buck.litzsim --expect $before.revision --ops ops.json | ConvertFrom-Json
& $cli undo buck.litzsim --expect $edited.receipt.afterRevision --before $edited.receipt.beforeRevision --previous previous.litzsim
```

- `inspect`는 회로 문서, 정규화한 문서의 `revision`, 읽은 파일 바이트의 `diskRevision`, 단자별 네트와 격자 좌표, 계기 기준 방향을 반환합니다. 미접속 단자는 `missingPins`에서 확인할 수 있습니다.
- `apply`와 `undo`는 문서와 receipt를 반환하고 파일을 교체합니다. receipt는 작업 전후 revision, 생성·수정·삭제된 소자 ID, 변경된 네트별 전후 단자 목록을 담습니다. 실패하면 JSON 오류와 0이 아닌 종료 코드를 반환합니다. 배치 연산 오류에는 1부터 시작하는 연산 번호와 대상 ID가 들어갑니다.
- `undo`는 현재 문서와 스냅샷의 revision을 각각 receipt의 `afterRevision`, `beforeRevision`과 대조합니다. 스냅샷이 다르면 `previousMismatch`를 반환합니다.

| `ops.json`의 `op` | 필드 |
| --- | --- |
| `add` | `type`, 선택 `grid: [x,y]`(기본 `[0,0]`). 새 ID는 `resistor1`처럼 형식 이름에 사용 가능한 번호를 붙입니다. |
| `move` | `id`, `grid: [x,y]` |
| `rotate` | `id`. 현재 자세에서 시계 방향으로 90° 회전합니다. |
| `mirror` | `id`, `axis`(`horizontal`은 좌우, `vertical`은 상하). 회로도 축을 기준으로 반전합니다. |
| `setParameter` | `id`, `key`(예: `resistance_ohm`), `value` |
| `connect` | `a`, `b`(`Q1.g`처럼 소자 ID와 단자명) |
| `disconnect` | `pin` |
| `delete` | `id` |

### 회로 파일과 상주 모드

`.litzsim` v1의 `parts`, `nets`, `layout`, `bends`, `simulation`을 읽고 안정된 순서와 숫자 표기로 저장합니다. `layout`의 `rotation`은 기존 0/90/180/270° 규약을 따릅니다. 선택 필드 `mirror`는 회전 후 적용할 반전 방향입니다. 양축 반전(`both`)은 180°를 더한 회전으로 정규화하고, 0° 회전과 반전 없음은 해당 필드를 생략합니다. 반전과 회전 후에도 단자 이름, 네트, 계기 극성은 유지합니다.

RC·RLC·벅 예제는 [examples](examples/)에 있습니다. PFC 예제는 지원 범위 밖 제어 설정이 있어 읽기를 거절합니다. 회로 파일에는 revision이나 Undo 기록을 저장하지 않습니다. 저장 중에는 시스템 임시 폴더의 경로별 OS 잠금을 사용합니다. 신규 저장은 하드 링크를 지원하는 파일시스템에서 완성된 파일을 원자적으로 게시합니다.

`litzsim serve`는 요청 한 줄마다 JSON 한 줄을 반환합니다. `ping`은 버전을, `new`는 빈 v1 문서와 revision을 반환합니다. 실패 응답의 `error` 객체에는 `code`와 `message`가 들어갑니다. 편집과 파일 요청의 필드는 다음과 같습니다.

| `op` | 필드 |
| --- | --- |
| `inspect` | `file` 또는 `document` |
| `apply` | `document`, `expectedRevision`, `ops` |
| `undo` | `document`, `expectedAfter`, `expectedBefore`, `previous`(편집 전 문서) |
| `save` | `file`, `document`, 기존 파일의 `expectedDiskRevision`. revision을 생략하면 신규 저장을 요청합니다. |

## 외부 PC에서 설치하기

Windows x64에 Rust가 없다면 [rustup](https://rustup.rs/)을 설치합니다. PowerShell에서 저장소를 복제하고 기본 Rust 툴체인으로 플러그인을 빌드합니다.

```powershell
git clone https://github.com/litzio/litzsim.git
cd litzsim
cargo xtask plugin
```

1. Litzio에서 **설정 → 플러그인 → 플러그인 추가…**를 열고, 명령이 출력한 `target/plugin/litzsim/` 폴더를 선택합니다.
2. 목록에서 **LitzSim**을 켜고 **신뢰하고 켜기**를 선택합니다.
3. 작업 막대에서 **LitzSim**을 열고 **코어 확인**을 눌러 코어 버전과 왕복 시간을 확인합니다.

### `link.exe`를 찾을 수 없을 때

빌드가 `link.exe`를 찾을 수 없다는 오류로 실패하면 Windows GNU 툴체인을 설치하고 현재 PowerShell 세션에서만 선택합니다. 이 툴체인으로 만든 설치 폴더를 Litzio Desktop에 등록해 코어 응답을 확인했습니다.

```powershell
rustup toolchain install stable-x86_64-pc-windows-gnu --profile minimal
$env:RUSTUP_TOOLCHAIN = "stable-x86_64-pc-windows-gnu"
cargo xtask plugin
```

### 업데이트하기

1. 등록할 때 사용한 저장소 폴더에서 `git pull`과 `cargo xtask plugin`을 실행합니다.
2. Litzio Desktop의 **설정 → 플러그인**에서 **LitzSim 설정**을 열고 **업데이트 확인**을 누릅니다.
3. 새 내용이 감지되면 **업데이트**를 누릅니다. 작업 막대에서 **LitzSim**을 열고 **코어 확인**을 눌러 응답을 확인합니다.

[제품 목적](docs/purpose.md) · [문서 색인](docs/README.md) · [에이전트 작업 안내](AGENTS.md)
