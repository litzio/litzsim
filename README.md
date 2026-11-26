# LitzSim · 리츠심

사람과 AI가 같은 회로도를 편집하고 실험을 재현하는 오픈소스 전력전자 시뮬레이터를 개발합니다.

첫 예제는 양방향 온보드 충전기(OBC)입니다. 엔지니어가 회로에서 선택한 가지에 AI가 센서나 필터를 넣고, 변경한 회로에서 스위칭 모델과 평균 모델의 응답을 비교하는 것이 목표입니다. 첫 제품은 [Litzio](https://github.com/litzio/litzio) 플러그인입니다.

## 현재 상태

**Rust로 다시 만드는 중입니다.** Litzio 플러그인 화면에서 회로를 새로 만들거나 열어 편집하고, 저장한 뒤 실행·중지할 수 있습니다. 파형 창은 아직 없습니다. 재구축 기준은 [Rust 재구축](docs/rebuild.md)에 있습니다.

C++·Qt로 만든 1차 MVP(회로 편집기, 해석 엔진, 파형 창, 논문용 SVG/PDF 내보내기)는 [`archive/qt-cpp-mvp`](https://github.com/litzio/litzsim/tree/archive/qt-cpp-mvp) 브랜치와 `archive/qt-cpp-mvp-2026-11-26` 태그에 보관했습니다.

## 화면에서 회로 편집하기

Litzio에서 폴더를 연 뒤 작업 막대의 **LitzSim**을 엽니다. 화면은 빈 새 회로로 시작합니다. 왼쪽은 분류별 부품 라이브러리, 가운데는 회로도, 오른쪽은 선택한 소자의 속성 패널입니다. 왼쪽 아이콘 막대의 첫 단추로 라이브러리를 접고 펼칩니다.

- 라이브러리 항목을 누르면 빈 자리에, 회로도로 끌어 놓으면 놓은 격자에 배치합니다. 회로도 위에서 단축키를 누르면 포인터 위치에 배치합니다. 직류 전압원 `V`, 사인 전압원 `S`, 접지 `G`, 저항 `Shift+R`, 인덕터 `L`, 커패시터 `C`, 다이오드 `D`, N채널 MOSFET `M`, PWM 신호 `P`, 전류계 `A`, 전압계 `Shift+V`입니다. 단축키는 아직 바꿀 수 없습니다.
- 휠로 포인터 위치를 기준으로 확대·축소하고, 빈 곳을 끌거나 `Space`를 누른 채 끌어 이동합니다. **전체 보기** 단추나 `Home`, `Ctrl+0`으로 회로 전체를 맞추며, 회로를 열 때와 새 회로에 첫 소자를 놓을 때도 전체 보기로 맞춥니다. 확대 비율은 회로 파일에 쓰지 않습니다.
- 소자를 끌면 미리보기만 움직이고 놓을 때 이동을 확정합니다. 선택한 소자는 `R` 회전, `F` 좌우 반전, `Shift+F` 상하 반전, `Delete` 삭제이며 우클릭 메뉴와 속성 패널에도 같은 명령이 있습니다.
- 단자 두 개를 차례로 누르면 연결합니다. 소자를 놓은 뒤 그 소자의 단자와 다른 소자의 단자가 1.5격자 안에 있고 둘 중 하나 이상이 미접속이면 연결을 추천하며 `Enter`로 연결하고 `Esc`로 취소합니다. 이미 연결된 두 네트는 추천으로 합치지 않습니다. 직류·사인 전압원이나 PWM의 두 단자가 같은 네트가 되는 연결은 확정 전에 확인을 받습니다.
- 배선에 포인터를 올리면 **이 가지에 전류계 끼우기 (A)**가 나타납니다. 단추나 `A`를 누르면 아래 `insertInBranch`로 전류계를 직렬로 끼웁니다.
- 속성 패널에서 소자 값을 `4.7k`, `10u`처럼 입력하고, 소자를 선택하지 않았을 때 해석 간격·종료 시각·저장 간격을 바꿉니다. `Ctrl+Z`로 되돌리고 `Ctrl+S`로 저장합니다.
- 열었거나 새로 만든 상태에서 회로가 바뀌면(해석 설정만 바꾼 경우 포함) 저장하지 않은 변경으로 표시하고, **새 회로**·**열기** 전에 버릴지 확인합니다.
- 새 회로는 폴더를 고르고 파일 이름을 정해 저장하며 기존 파일을 덮어쓰지 않습니다. 저장한 회로는 **실행**으로 회로 옆 `runs/`에 실행을 시작합니다. 실행 중에는 **실행**을 누를 수 없고 진행 시각과 **중지**가 표시됩니다. **파형 보기**는 다음 단계에서 제공합니다.

도면은 코어 `layout` 결과만 그립니다. 화면은 선택, 끌기 미리보기, 마우스를 올린 가지, 라이브러리 접힘, 확대 비율 같은 화면 상태만 가지며 회로 변경은 모두 상주 코어의 `apply`와 `undo`를 호출합니다. 전압원 단락 확인은 코어가 receipt의 `shortedSources`로 알려 주며, 화면은 이 진단이 있으면 확인을 받은 뒤에만 결과를 작업본에 반영하고 취소하면 버립니다. 변경 표시·연결 추천 규칙, 단락 진단, Host 실행 폴더는 `cargo build --release` 뒤 `node --test plugin/rules.test.mjs`로 상주 코어와 함께 시험합니다.

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
& $cli layout buck.litzsim --json
```

- `inspect`는 회로 문서, 정규화한 문서의 `revision`, 읽은 파일 바이트의 `diskRevision`, 단자별 네트와 격자 좌표, 계기 기준 방향을 반환합니다. 미접속 단자는 `missingPins`에서 확인할 수 있습니다.
- `apply`와 `undo`는 문서와 receipt를 반환하고 파일을 교체합니다. receipt는 작업 전후 revision, 생성·수정·삭제된 소자 ID, 변경된 네트별 전후 단자 목록, 이 작업으로 두 단자가 새로 같은 네트가 된 전압원(직류·사인 전압원, PWM) ID 목록 `shortedSources`를 담습니다. 실패하면 JSON 오류와 0이 아닌 종료 코드를 반환합니다. 배치 연산 오류에는 1부터 시작하는 연산 번호와 대상 ID가 들어갑니다.
- `undo`는 현재 문서와 스냅샷의 revision을 각각 receipt의 `afterRevision`, `beforeRevision`과 대조합니다. 스냅샷이 다르면 `previousMismatch`를 반환합니다.
- `layout`은 같은 문서 revision의 기호·단자·직각 배선·접속점·교차 점프·라벨과 겹침 진단을 반환합니다. 배선 기하는 네트의 단자 배열 순서와 무관하며, 라벨은 소유 기호 주변에 배치합니다. 좌표와 라벨 치수는 [회로도 기하 규약](docs/layout.md)을 따릅니다.

| `ops.json`의 `op` | 필드 |
| --- | --- |
| `add` | `type`, 선택 `grid: [x,y]`(기본 `[0,0]`). 새 ID는 종류별 접두어에 쓰이지 않은 가장 작은 번호를 붙입니다. 직류·사인 전압원 `V`, 저항 `R`, 인덕터 `L`, 커패시터 `C`, 다이오드 `D`, N채널 MOSFET `Q`, PWM `PWM`, 전압계 `VM`, 전류계 `AM`, 접지 `GND`입니다(예: `R1`, `GND1`). 기존 파일의 ID는 바꾸지 않습니다. |
| `move` | `id`, `grid: [x,y]` |
| `rotate` | `id`. 현재 자세에서 시계 방향으로 90° 회전합니다. |
| `mirror` | `id`, `axis`(`horizontal`은 좌우, `vertical`은 상하). 회로도 축을 기준으로 반전합니다. |
| `setParameter` | `id`, `key`(예: `resistance_ohm`), `value` |
| `connect` | `a`, `b`(`Q1.g`처럼 소자 ID와 단자명) |
| `disconnect` | `pin` |
| `delete` | `id` |
| `setSimulation` | `key`(`timeStep_s`, `stopTime_s`, `outputStep_s`), `value` |
| `insertInBranch` | `type`(두 단자 소자, 예: `ammeter`), `layout` 선분의 `terminals`, `from`, `to`. 선분을 끊고 소자를 직렬로 넣습니다. 첫 단자(전류계 `p`)는 `terminals[0]` 쪽에 연결하므로 전류계는 `terminals[0]` 쪽에서 `terminals[1]` 쪽으로 흐르는 전류를 잽니다. 3격자 이하에서 전류계와 이웃 기호의 획이 겹치면 연산을 거절합니다. [가지에 끼우기](docs/layout.md#가지에-끼우기) |

### 회로 파일과 상주 모드

`.litzsim` v1의 `parts`, `nets`, `layout`, `bends`, `simulation`을 읽고 안정된 순서와 숫자 표기로 저장합니다. `layout`의 `rotation`은 기존 0/90/180/270° 규약을 따릅니다. 선택 필드 `mirror`는 회전 후 적용할 반전 방향입니다. 양축 반전(`both`)은 180°를 더한 회전으로 정규화하고, 0° 회전과 반전 없음은 해당 필드를 생략합니다. 반전과 회전 후에도 단자 이름, 네트, 계기 극성은 유지합니다.

RC·RLC·벅 예제는 [examples](examples/)에 있습니다. PFC 예제는 지원 범위 밖 제어 설정이 있어 읽기를 거절합니다. 회로 파일에는 revision이나 Undo 기록을 저장하지 않습니다. 저장 중에는 시스템 임시 폴더의 경로별 OS 잠금을 사용합니다. 신규 저장은 하드 링크를 지원하는 파일시스템에서 완성된 파일을 원자적으로 게시합니다. 동일한 회로로 덮어쓰면 기존 바이트와 `diskRevision`을 유지합니다. 저항 기본값이 파일에 명시돼 있으면 저장할 때 그 키를 생략합니다.

`litzsim serve`는 요청 한 줄마다 JSON 한 줄을 반환합니다. `ping`은 버전을, `new`는 빈 v1 문서와 revision을 반환합니다. 성공한 `apply`와 `undo` 응답에는 `document`, `receipt`, 같은 revision의 `layout`이 함께 들어갑니다. 실패 응답의 `error` 객체에는 `code`와 `message`가 들어갑니다. 편집과 파일 요청의 필드는 다음과 같습니다.

| `op` | 필드 |
| --- | --- |
| `inspect` | `file` 또는 `document` |
| `layout` | `file` 또는 `document` |
| `apply` | `document`, `expectedRevision`, `ops` |
| `undo` | `document`, `expectedAfter`, `expectedBefore`, `previous`(편집 전 문서) |
| `save` | `file`, `document`, 기존 파일의 `expectedDiskRevision`. revision을 생략하면 신규 저장을 요청합니다. |

## CLI로 회로 해석하기

`litzsim run FILE OUT`은 RC·RLC·이상 벅의 과도응답을 계산하고 지정한 새 파일에 원본 파형을 저장합니다. 성공하면 행 수, 채널 이름과 단위, 종료 시각, `warnings` 배열을 JSON으로 출력합니다. 실행 예시는 `litzsim run examples/rc.litzsim rc.litzwave`입니다. 출력 경로는 하드 링크를 지원하는 파일시스템의 기존 폴더에 지정합니다. 기존 파일은 보존합니다.

`timeStep_s`는 적분 간격입니다. PWM 에지가 스텝 사이에 있으면 그 시각에서 스텝을 나눕니다. `outputStep_s`를 생략하면 PWM 에지를 포함한 모든 적분 결과를 저장합니다. 적분 간격의 정수배를 지정하면 해당 시간 격자에서만 저장합니다. 두 경우 모두 시각 0과 `stopTime_s`의 표본을 포함합니다. 전압계는 p−n, 전류계는 p→n 방향을 사용합니다. 인덕터 전류는 p→n, 다이오드 전류는 애노드에서 캐소드 방향입니다. 미접속 단자와 지원 범위 밖 모델은 실행 전에 진단합니다.

커패시터의 `esr_ohm` 기본값은 1 µΩ입니다. [Simscape Capacitor](https://www.mathworks.com/help/simscape/ref/capacitor.html)와 같이 작은 직렬 저항을 두어 전원에 직결한 커패시터도 해석할 수 있게 합니다. 직류·사인 전압원의 `internalResistance_ohm` 기본값은 0 Ω으로 이상 전원을 유지합니다. 두 파라미터는 0 이상이며, 생략하면 기본값을 적용합니다. 커패시터의 `initialVoltage_V`는 ESR 안쪽의 초기 전압입니다.

돌입 전류 시험에서는 실제 커패시터 ESR과 전원 내부 저항을 입력하고 커패시터와 직렬로 전류계를 넣습니다. 충전 시정수(경로의 직렬 저항 합 × 용량)보다 작은 `timeStep_s`를 지정하고, 스텝을 줄여 피크가 수렴하는지 확인합니다. 전원과 커패시터가 직결되거나 분기 없는 저항·전류계로 이어진 경로에서 시정수가 스텝의 1/10보다 짧으면 `inrushTimeStep` 경고를 냅니다. 경고의 `partIds`, `timeConstant_s`, `timeStep_s`는 관련 소자와 시간(초)을 나타냅니다. 이때 돌입 전류 피크가 time-step에 좌우되며 해석은 계속합니다.

저항이 0 Ω인 경로에서 DC 전원과 커패시터의 초기 전압이 단자 극성까지 일치하면 커패시터 전류를 0 A로 초기화합니다. 초기 전압이 다르면 `singularCircuit`에 ESR 또는 전원 내부 저항을 추가하라는 안내가 나옵니다. 이상 전압 제약의 루프, 초기 인덕터 전류에 귀로가 없는 회로, 열린 스위치로 고립된 노드도 특이 행렬을 일으킬 수 있습니다. 구조적 원인을 확인하면 `targetId`에 관련 소자 하나를, `message`에 실패 시각과 원인 및 관련 소자 ID를 표시합니다.

`.litzwave` 파일은 8바이트 서명 `LITZWV02`, 행 수(u64), 채널 메타데이터의 바이트 길이(u32), UTF-8 JSON 채널 배열, 원본 데이터 순서로 저장합니다. 채널 배열의 첫 항목은 `{"name":"time","unit":"s"}`입니다. 이어지는 항목은 `VC_V` 같은 채널 이름과 `V` 또는 `A` 단위를 담습니다. 원본 데이터는 시간순 고정 폭 행으로 구성되며, 각 행에 채널 배열 순서대로 IEEE 754 binary64 값이 들어갑니다. 모든 바이너리 숫자는 리틀엔디언입니다.

### 백그라운드 실행과 파형 조회

`start`는 회로 옆 `runs/<runId>/`에 `circuit.litzsim` 스냅샷을 저장하고 별도 감독 프로세스를 띄운 뒤 바로 반환합니다. 감독 프로세스가 이어서 해석 프로세스를 시작합니다. 응답의 `runDirectory`를 후속 CLI 호출에 넘깁니다. 같은 회로 폴더에 진행 중인 실행이 있으면 `runInProgress`를 반환합니다.

`runs/active.lock`, `runs/admission.lock`과 실행 폴더의 `status.lock`은 실행이 끝나도 잠금에 다시 쓸 파일로 남습니다.
Windows의 `runDirectory`는 `\\?\`로 시작하는 verbatim 경로로 반환됩니다. 표시용으로 일반 경로가 필요하면 화면에서 변환할 수 있습니다.

```powershell
$run = & $cli start buck.litzsim | ConvertFrom-Json
& $cli status $run.runDirectory
# 상태가 completed에 도달하면 파형을 조회합니다.
& $cli wave $run.runDirectory --channel L1_A --channel VC_V --from 0 --to 0.001 --pixels 800
# 긴 실행을 중지하려면 다음 명령을 사용합니다.
& $cli stop $run.runDirectory
```

`status.json`은 `running`, `stopping`, `completed`, `canceled`, `failed`를 기록합니다. `status`와 `stop`은 현재 상태, `simulatedTime`과 `stopTime`(초), `warnings` 배열, 실패 시 `error`를 반환합니다. `stop`은 `stopping`을 접수하고 해석 프로세스 종료를 확인한 뒤 `canceled`로 전이합니다. 완료된 실행에서만 `result.litzwave`를 조회할 수 있습니다.

파형 응답은 요청 범위 `timeRange`와 채널별 `name`, `unit`, `minimum`, `maximum`, `points`를 반환합니다. 각 점의 `row`는 파일의 0부터 시작하는 행 번호이며 `t_s`와 `value`는 원본값입니다. 조회는 양 끝 시각을 포함합니다. 첫 행과 마지막 행, 픽셀 구간별 최솟값·최댓값의 행, 양옆과 값이 다른 피크의 인접 행을 조회 범위 안에서 시간순으로 반환합니다. 범위에 행이 없으면 극값은 `null`, 점은 빈 배열입니다.

첫 조회는 결과 파일 전체를 메모리에 읽으므로 시간과 메모리가 파일 크기에 비례합니다. Windows 상주 모드에서 160 MB 벅 결과의 첫 조회는 228 ms였습니다.

상주 모드와 플러그인 Host는 아래 요청으로 같은 실행 연산을 호출합니다. 상주 CLI의 입력 파이프가 닫히면 해당 세션이 접수한 해석도 중지하고 종료를 확인합니다. 단독 `start`는 명령이 끝난 뒤에도 백그라운드에서 계속 실행됩니다.

| `op` | 필드 |
| --- | --- |
| `start` | 회로의 `directory`, 작업본 `document`, `expectedRevision`. 플러그인 Host는 `directory` 대신 회로 `file`을 받으면 그 부모 폴더(드라이브 루트 포함)를 씁니다. |
| `status`, `stop` | `runId`. 다른 세션에서 시작했다면 회로의 `directory`도 전달합니다. |
| `wave` | `runId`, 이름 배열 `channels`, `from_s`, `to_s`, `pixels`. 다른 세션에서 시작했다면 `directory`도 전달합니다. |

## 외부 PC에서 설치하기

Windows x64에 Rust가 없다면 [rustup](https://rustup.rs/)을 설치합니다. PowerShell에서 저장소를 복제하고 기본 Rust 툴체인으로 플러그인을 빌드합니다.

```powershell
git clone https://github.com/litzio/litzsim.git
cd litzsim
cargo xtask plugin
```

1. Litzio에서 **설정 → 플러그인 → 플러그인 추가…**를 열고, 명령이 출력한 `target/plugin/litzsim/` 폴더를 선택합니다.
2. 목록에서 **LitzSim**을 켜고 **신뢰하고 켜기**를 선택합니다.
3. 작업 막대에서 **LitzSim**을 열고 빈 새 회로와 부품 라이브러리가 표시되는지 확인합니다. 회로를 열거나 저장하려면 Litzio에서 회로 폴더를 먼저 엽니다.

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
3. 새 내용이 감지되면 **업데이트**를 누릅니다. 작업 막대에서 **LitzSim**을 열어 화면이 표시되는지 확인합니다.

[제품 목적](docs/purpose.md) · [문서 색인](docs/README.md) · [에이전트 작업 안내](AGENTS.md)
