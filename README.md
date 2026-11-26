# LitzSim · 리츠심

사람과 AI가 같은 회로도를 편집하고 실험을 재현하는 오픈소스 전력전자 시뮬레이터를 개발합니다.

첫 예제는 양방향 온보드 충전기(OBC)입니다. 엔지니어가 회로에서 선택한 가지에 AI가 센서나 필터를 넣고, 변경한 회로에서 스위칭 모델과 평균 모델의 응답을 비교하는 것이 목표입니다. 첫 제품은 [Litzio](https://github.com/litzio/litzio) 플러그인입니다.

## 현재 상태

**Rust로 다시 만드는 중입니다.** Litzio 플러그인 화면에서 회로를 새로 만들거나 열어 편집하고, 저장한 뒤 실행·중지하고, 완료된 실행의 파형을 볼 수 있습니다. 재구축 기준은 [Rust 재구축](docs/rebuild.md)에 있습니다.

C++·Qt로 만든 1차 MVP(회로 편집기, 해석 엔진, 파형 창, 논문용 SVG/PDF 내보내기)는 [`archive/qt-cpp-mvp`](https://github.com/litzio/litzsim/tree/archive/qt-cpp-mvp) 브랜치와 `archive/qt-cpp-mvp-2026-11-26` 태그에 보관했습니다.

## 화면에서 회로 편집하기

Litzio에서 폴더를 연 뒤 작업 막대의 **LitzSim**을 엽니다. 화면은 빈 새 회로로 시작합니다. 왼쪽은 분류별 부품 라이브러리, 가운데는 회로도, 오른쪽은 선택한 소자의 속성 패널입니다. 왼쪽 아이콘 막대의 첫 단추로 라이브러리를 접고 펼칩니다.

- 라이브러리 항목을 누르면 빈 자리에, 회로도로 끌어 놓으면 놓은 격자에 배치합니다. 회로도 위에서 단축키를 누르면 포인터 위치에 배치합니다. 직류 전압원 `V`, 사인 전압원 `S`, 접지 `G`, 저항 `R`, 인덕터 `L`, 커패시터 `C`, 다이오드 `D`, N채널 MOSFET `M`, PWM 신호 `P`, 전류계 `A`, 전압계 `Shift+V`입니다. 단축키는 아직 바꿀 수 없습니다.
- 휠로 포인터 위치를 기준으로 확대·축소하고, 가운데 버튼으로 끌거나 `Space`를 누른 채 좌클릭으로 끌어 도면을 이동합니다. **전체 보기** 단추나 `Home`, `Ctrl+0`으로 회로 전체를 맞추며, 회로를 열 때와 새 회로에 첫 소자를 놓을 때도 전체 보기로 맞춥니다. 빈 곳의 우클릭 메뉴 **격자 보기**나 `Ctrl+'`로 격자를 켜고 끕니다. 확대 비율과 격자 표시는 회로 파일에 쓰지 않습니다.
- 빈 곳에서 좌클릭으로 끌면 사각 영역에 닿는 소자와 도선을 선택합니다. `Shift` 또는 `Ctrl`을 누른 채 클릭하면 선택에 추가하거나 제외하고, `Esc`로 선택을 해제합니다. 선택한 소자를 끌면 선택된 소자들이 같은 격자만큼 이동합니다. 놓을 때 한 번에 확정합니다. 단일 소자 선택에서는 `Shift+R` 회전, `F` 좌우 반전, `Shift+F` 상하 반전을 쓸 수 있습니다.
- 도선을 클릭해 선택하고 `Delete`, `Backspace` 또는 우클릭 메뉴 **도선 삭제**로 가지를 끊습니다. 여러 소자와 도선을 선택해 `Delete`를 누르면 한 번에 삭제합니다. 소자 속성 패널의 **연결 끊기**는 해당 단자만 네트에서 뺍니다. `Ctrl+Z` 한 번으로 직전 삭제 또는 이동을 되돌립니다.
- 단자 두 개를 차례로 누르면 연결합니다. 소자를 놓은 뒤 그 소자의 단자와 다른 소자의 단자가 1.5격자 안에 있고 둘 중 하나 이상이 미접속이면 연결을 추천하며 `Enter`로 연결하고 `Esc`로 취소합니다. 이미 연결된 두 네트는 추천으로 합치지 않습니다. 직류·사인 전압원이나 PWM의 두 단자가 같은 네트가 되는 연결은 확정 전에 확인을 받습니다.
- 배선에 포인터를 올리면 **이 가지에 전류계 끼우기 (A)**가 나타납니다. 단추나 `A`를 누르면 아래 `insertInBranch`로 전류계를 직렬로 끼웁니다.
- 속성 패널에서 소자 값을 `4.7k`, `10u`처럼 입력하고, 소자를 선택하지 않았을 때 해석 간격·종료 시각·저장 간격을 바꿉니다. `Ctrl+Z`로 되돌리고 `Ctrl+S`로 저장합니다.
- 열었거나 새로 만든 상태에서 회로가 바뀌면(해석 설정만 바꾼 경우 포함) 저장하지 않은 변경으로 표시하고, **새 회로**·**열기** 전에 버릴지 확인합니다.
- 새 회로는 폴더를 고르고 파일 이름을 정해 저장하며 기존 파일을 덮어쓰지 않습니다. 저장한 회로는 **실행**으로 회로 옆 `runs/`에 실행을 시작합니다. 실행 중에는 **실행**을 누를 수 없고 진행 시각과 **중지**가 표시됩니다. 저장한 회로에서 **파형 보기**를 누르면 [파형 창](#화면에서-파형-보기)으로 바뀝니다.

도면은 코어 `layout` 결과만 그립니다. 화면은 선택, 끌기 미리보기, 마우스를 올린 가지, 라이브러리 접힘, 확대 비율 같은 화면 상태만 가지며 회로 변경은 모두 상주 코어의 `apply`와 `undo`를 호출합니다. 전압원 단락 확인은 코어가 receipt의 `shortedSources`로 알려 주며, 화면은 이 진단이 있으면 확인을 받은 뒤에만 결과를 작업본에 반영하고 취소하면 버립니다. 선택·삭제와 단축키 규칙, 변경 표시·연결 추천 규칙, 단락 진단, Host 실행 폴더, 파형 창의 눈금·축 배정·확대 범위·설정 적용 규칙과 실행 기록·파형·커서 조회는 `cargo build --release` 뒤 `node --test plugin/rules.test.mjs`로 시험합니다.

## 화면에서 파형 보기

PSIM의 SIMVIEW처럼 실행은 모든 계기와 인덕터·다이오드 전류를 기록하고, 파형 창에서 볼 신호를 고릅니다. 회로 화면의 **파형 보기**를 누르면 같은 탭이 파형 창으로 바뀌고, 머리의 **← 회로**로 돌아갑니다. 돌아가면 회로도는 전체 보기로 맞춰지고 작업본과 되돌리기 기록은 그대로입니다.

- 머리의 실행 기록에서 회로 폴더 `runs/`의 완료된 실행을 고릅니다. 기본은 방금 완료한 실행, 없으면 가장 최근에 완료한 실행입니다. 현재 작업본과 revision이 다른 회로의 실행이면 머리에 표시합니다.
- **그래프 추가**로 빈 그래프를 만들고, 그래프마다 **신호 추가**에서 볼 신호를 고릅니다. 첫 신호와 같은 단위의 신호는 왼쪽 축, 다른 단위는 오른쪽 축에 놓이며 신호 옆 **좌**/**우**로 옮기고 **×**로 뺍니다. 축 하나에 신호가 하나면 눈금 글자를 그 신호 색으로 그립니다.
- 휠은 포인터 시각을 기준으로 모든 그래프의 시간축을 함께 확대·축소하고, 끌기는 이동, `Shift`+휠은 그 그래프의 세로축만 확대·축소합니다(그 축은 수동 범위가 됩니다). 그래프를 더블클릭하면 시간축 전체와 그 그래프의 자동 세로축으로, 머리의 **전체 보기**는 모든 그래프를 그렇게 되돌립니다. 그래프를 누른 뒤에는 `Home`이 전체 보기, `+`·`-`가 가운데 기준 확대·축소입니다.
- **커서**는 A·B 두 선을 놓습니다. 선을 끌어 옮기면 표에 신호별 A·B 시각의 원본 값(가장 가까운 원본 행), B−A, A–B 구간의 평균·RMS·최소·최대와 Δt(B−A), 1/|Δt|를 표시합니다. 이 값은 모두 코어 `measure`가 계산하고, 화면은 단위를 붙여 보여 줍니다.
- 그래프의 **축·격자**에서 주·보조 격자(보조는 주의 1/5), 시간 격자 간격(자동·지정), 왼쪽·오른쪽 축의 범위(자동은 보이는 구간의 최솟값·최댓값에 맞춤, 또는 수동)와 격자 간격을 정합니다. **모든 그래프에 적용**은 격자·시간 간격·자동 선택을 모든 그래프에, 직접 입력한 축 범위와 간격은 같은 신호를 보이는 축에만 적용합니다. 시간축은 모든 그래프가 함께 씁니다.
- 그래프·축·격자·커서는 화면 상태이며 회로 파일이나 실행 폴더에 쓰지 않습니다. 수식으로 새 신호를 만드는 기능(**수식 신호**, 비활성)은 [#63](https://github.com/litzio/litzsim/issues/63)에서 다룹니다.

그리는 데이터는 모두 상주 코어가 원본 행으로 만듭니다. 화면은 보이는 시간 범위와 그래프 폭(픽셀)으로 `wave`를 요청해 픽셀 구간별 최소·최대 원본 행을 선으로 잇고, 커서 값과 구간 통계는 `measure`로 받습니다. 입력이 오면 직전 결과를 새 범위에 옮겨 먼저 그리고, 그 범위의 응답이 오면 바로 다시 그립니다. 더 새로운 입력이 생긴 뒤 도착한 응답은 버립니다. 파형 선 색은 신호를 구분하기 위한 [Litzio 토큰](https://github.com/litzio/litzio/blob/main/DESIGN.md)의 예외로 `plugin/plot.js`의 `TRACE_COLORS`에 고정했습니다. 그 밖의 색·간격·글꼴은 토큰을 씁니다.

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
| `removeBranch` | `layout` 선분의 `terminals`, `from`, `to`. 선택한 선분에서 네트를 나누고, 단자가 하나만 남은 쪽은 미접속으로 만듭니다. [가지 끊기](docs/layout.md#가지-끊기) |

### 회로 파일과 상주 모드

`.litzsim` v1의 `parts`, `nets`, `layout`, `bends`, `simulation`을 읽고 안정된 순서와 숫자 표기로 저장합니다. `layout`의 `rotation`은 기존 0/90/180/270° 규약을 따릅니다. 선택 필드 `mirror`는 회전 후 적용할 반전 방향입니다. 양축 반전(`both`)은 180°를 더한 회전으로 정규화하고, 0° 회전과 반전 없음은 해당 필드를 생략합니다. 반전과 회전 후에도 단자 이름, 네트, 계기 극성은 유지합니다.

RC·RLC·벅 예제는 [examples](examples/)에 있습니다. PFC 예제는 지원 범위 밖 제어 설정이 있어 읽기를 거절합니다. 회로 파일에는 revision이나 Undo 기록을 저장하지 않습니다. 저장 중에는 시스템 임시 폴더의 경로별 OS 잠금을 사용합니다. 신규 저장은 하드 링크를 지원하는 파일시스템에서 완성된 파일을 원자적으로 게시합니다. 동일한 회로로 덮어쓰면 기존 바이트와 `diskRevision`을 유지합니다. 저항 기본값이 파일에 명시돼 있으면 저장할 때 그 키를 생략합니다.

Windows에서 저장 경로 해석이나 교체가 접근 거부 또는 공유 위반으로 막히면 각 단계에서 최대 250 ms의 누적 대기 동안 다시 시도합니다. 교체 시도마다 디스크 revision을 확인합니다. 다른 저장이 파일을 바꿨다면 현재 revision을 담은 `diskRevisionConflict`로 중단하며, 공유 경합이 계속되면 실패한 단계와 경로를 담은 `ioError`를 반환합니다.

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

결과 파일의 시간은 증가하며, 같은 시각의 두 행은 그 시각의 불연속입니다. 앞 행이 왼쪽 극한, 뒤 행이 오른쪽 극한이고, 한 시각에 세 행 이상은 두지 않습니다. 해석은 PWM 에지 시각을 기록할 때 그 직전 구간의 스위치 상태로 푼 행(왼쪽 극한) 뒤에 오른쪽 극한 행을 하나 더 기록합니다. 오른쪽 극한은 커패시터 전압과 인덕터 전류를 그대로 두고 다음 구간의 스위치 상태로 푼 값입니다. 예를 들어 벅의 하강 에지에서는 게이트 전압이 10 V에서 0 V로, 다이오드 전류가 0에서 인덕터 전류로 바뀌고 인덕터 전류와 출력 전압은 같습니다. 저장 간격으로 기록하지 않는 에지 시각에는 두 행 모두 남지 않습니다. 오른쪽 극한 행은 이후 계산에 쓰지 않으므로 다른 행의 값을 바꾸지 않습니다.

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

`runs DIR`은 회로 폴더의 실행을 최근 순으로 반환합니다. 각 항목은 `status`와 같은 필드와 시작 시각 `startedAt`(Unix ms)을 가지며, 완료된 실행에는 결과 파일 머리에서 읽은 행 수 `rows`, 시간 범위 `timeRange`, 시간을 뺀 신호 `channels`(`name`, `unit`)가 더해집니다.

```powershell
& $cli runs .
& $cli measure $run.runDirectory --channel VC_V --channel L1_A --from 0.0002 --to 0.0008
```

`measure`는 두 커서 시각 `from`, `to`(순서 무관)를 받아 신호별로 다음을 반환합니다.

- 각 시각에 가장 가까운 원본 행(`from`, `to`의 `row`, `t_s`, `value`)과 그 차이 `delta`(`to.value − from.value`). 같은 거리면 앞 행입니다.
- 두 시각 사이 구간에서 원본 행을 직선으로 이은 파형의 평균 `average`, RMS `rms`, 최솟값 `minimum`, 최댓값 `maximum`.
- 응답 머리의 `deltaTime_s`(`to − from`)와 `frequency_Hz`(1/|`to − from`|, 두 시각이 같으면 `null`).

구간 양 끝은 앞뒤 원본 행 사이를 선형 보간하고, 결과 시간 범위 밖은 잘라 냅니다. 구간이 결과 밖이면 통계는 `null`입니다. 같은 시각 두 행(불연속)은 잇지 않습니다.

- 구간은 시작 시각에서 오른쪽 극한, 끝 시각에서 왼쪽 극한을 씁니다.
- 두 행 사이는 폭이 0이라 적분에 더하지 않고, 최소·최대에는 두 값을 모두 넣습니다.
- 커서가 정확히 에지 시각이면 왼쪽 극한 행을, 에지 뒤면 오른쪽 극한 행을 가장 가까운 행으로 고릅니다. 두 시각이 같은 구간의 통계도 왼쪽 극한입니다.

`status.json`은 `running`, `stopping`, `completed`, `canceled`, `failed`를 기록합니다. `status`와 `stop`은 현재 상태, `simulatedTime`과 `stopTime`(초), `warnings` 배열, 실패 시 `error`를 반환합니다. `stop`은 `stopping`을 접수하고 해석 프로세스 종료를 확인한 뒤 `canceled`로 전이합니다. 완료된 실행에서만 `result.litzwave`를 조회할 수 있습니다.

파형 응답은 요청 범위 `timeRange`와 채널별 `name`, `unit`, `minimum`, `maximum`, `points`를 반환합니다. 각 점의 `row`는 파일의 0부터 시작하는 행 번호이며 `t_s`와 `value`는 원본값입니다. 조회는 양 끝 시각을 포함합니다. 첫 행과 마지막 행, 픽셀 구간별 최솟값·최댓값의 행, 양옆과 값이 다른 피크의 인접 행을 조회 범위 안에서 시간순으로 반환합니다. 같은 시각 두 행에 걸친 피크는 두 행 앞뒤의 행과 비교합니다. 범위에 행이 없으면 극값은 `null`, 점은 빈 배열입니다.

첫 조회는 결과 파일 전체를 메모리에 읽으므로 시간과 메모리가 파일 크기에 비례합니다. Windows 상주 모드에서 160 MB 벅 결과의 첫 조회는 228 ms였습니다.

상주 모드와 플러그인 Host는 아래 요청으로 같은 실행 연산을 호출합니다. 상주 CLI의 입력 파이프가 닫히면 해당 세션이 접수한 해석도 중지하고 종료를 확인합니다. 단독 `start`는 명령이 끝난 뒤에도 백그라운드에서 계속 실행됩니다.

| `op` | 필드 |
| --- | --- |
| `start` | 회로의 `directory`, 작업본 `document`, `expectedRevision`. |
| `status`, `stop` | `runId`. 다른 세션에서 시작했다면 회로의 `directory`도 전달합니다. |
| `runs` | 회로의 `directory` |
| `wave` | `runId`, 이름 배열 `channels`, `from_s`, `to_s`, `pixels`. 다른 세션에서 시작했다면 `directory`도 전달합니다. |
| `measure` | `runId`, 이름 배열 `channels`, 커서 시각 `from_s`(A), `to_s`(B). 다른 세션에서 시작했다면 `directory`도 전달합니다. |

플러그인 Host는 `start`, `status`, `stop`, `runs`, `wave`, `measure`에서 `directory` 대신 회로 `file`을 받으면 그 부모 폴더(드라이브 루트 포함)를 씁니다.

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
