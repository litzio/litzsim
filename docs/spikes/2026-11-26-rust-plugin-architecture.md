# Rust 기반 Litzio 플러그인 1차 아키텍처

2026-11-26 · [이슈 #32](https://github.com/litzio/litzsim/issues/32) 구현을 위한 설계안. 아래 명령과 서명은 구현 목표다.

RC·RLC·이상 벅을 Litzio에서 새로 만들고 편집·저장한 뒤 실행하고 일반 크기의 파형을 탐색한다. **플러그인에 Rust CLI 하나를 동봉하고, 편집용으로 한 번 기동하며 해석마다 같은 실행 파일의 별도 자식을 띄운다.** 사용자의 회로와 결과는 사용자 폴더의 `.litzsim`과 `runs/`에 둔다. Rust 코어는 CLI만으로 동작하며, React는 얇은 어댑터로만 Litzio를 호출한다. 비서 없이 전 과정을 끝낼 수 있어야 한다. [제품 목적](../purpose.md#litzio-플러그인-우선)

## 1. 코어를 어디서 실행할 것인가

| 후보 | 편집 왕복과 긴 해석·중지 | 외부 PC 빌드와 로컬 설치 | 판정 |
| --- | --- | --- | --- |
| 동봉한 네이티브 실행 파일 | 상주 편집 자식에 JSON Lines로 한 번 왕복한다. 별도 해석 자식의 종료를 기다리는 동안 편집 큐는 비어 있다. | 동작이 확인된 네이티브 Cargo 경로를 쓰고 완성한 exe를 폴더에 복사한다. CLI와 같은 바이너리다. | **추천**. 실제 Host에서 비교 가능한 선례가 있다. |
| Node 안의 WASM | 편집 인스턴스를 유지하고 긴 해석은 전용 worker thread에서 실행한다. 중지는 worker 종료와 부분 결과 폐기로 처리한다. Rust의 복사·직렬화 포함 p95는 미측정이다. | Rust WASM target과 JS 메모리·파일 입출력 경계, 별도 네이티브 CLI가 필요하다. wasm 파일을 미리 준비한다. | 보류. 네이티브 왕복이 실제 병목일 때 비교한다. |
| Node 네이티브 모듈 | 직접 호출 비용은 작을 수 있으나 동기 해석은 이벤트 루프를 막는다. 중지 가능한 별도 작업과 취소 확인이 필요하다. | OS·아키텍처와 Node-API에 맞는 .node 및 바인딩 빌드가 필요하다. 설치 중 컴파일할 수 없다. | 보류. CLI 외에 바인딩 하나를 더 검증할 만큼의 이득이 확인되지 않았다. |

[플러그인 스파이크](2026-11-26-litzio-plugin-spike.md)의 Windows C++ RC 시험에서 매회 기동은 입력→확정 프레임 p95 약 188–463 ms, 상주는 약 35 ms였다. 실제 Host의 드롭·연결은 각각 매회 기동 201.0·463.3 ms, 상주 36.0·35.3 ms였다. `spike/25-litzio-plugin`의 `spike/web/plugin/host.mjs`는 편집 호출을 직렬화하고 해석 자식의 close를 추적한다. 이 수명 분리를 계승한다. 외부 실행 파일 경로 설정과 기동 방식 토글은 동봉으로 없어진다.

Litzio 근거는 checkout `cbb6d85f` 기준이다. 플러그인 child는 Host의 `process.execPath`로 시작하며 Desktop에서는 Electron Node 모드다. 소스의 Node 요구 범위와 패키지 시험의 내장 Node 기준은 모두 **≥24.22.3, <25**, Desktop 의존성은 Electron 45.4.1이다. 이 Node에서 표준 WebAssembly API를 쓰는 설계는 가능하다. SDK가 WASM 로더나 Rust 바인딩을 제공한다는 전제는 두지 않는다. 외부 PC 설치판의 패치 버전과 WASM 기동은 미측정이다. [plugin-process.ts](https://github.com/litzio/litzio/blob/cbb6d85f/apps/editor/server/plugins/plugin-process.ts) · [package.json](https://github.com/litzio/litzio/blob/cbb6d85f/package.json) · [Desktop 의존성](https://github.com/litzio/litzio/blob/cbb6d85f/apps/desktop/package.json) · [패키지 Node 시험](https://github.com/litzio/litzio/blob/cbb6d85f/apps/desktop/test/packaged/cli-check.ts)

## 2. 소유 범위

| 배치 | 지금 필요한 책임 | 판정 |
| --- | --- | --- |
| Cargo workspace의 litzsim-core 라이브러리 | circuit은 v1 입출력·단자·측정 방향·편집·Undo, layout은 격자 기하·라벨·충돌, solver는 회로에서 방정식 조립·시간 적분, waves는 결과 쓰기·범위 조회를 맡는다. | **모듈로 구분**. GUI 없이 같은 예제로 각각 시험할 수 있다. 별도 엔진·도면·파형 크레이트는 연기한다. |
| litzsim CLI 바이너리 | 파일과 표준 입출력, 상주 편집 루프, 실행 스냅샷과 해석 자식 모드. 사람이 쓰는 명령과 플러그인이 같은 라이브러리를 링크한다. | 한 실행 파일을 패키징한다. 별도 Rust 플러그인 바인딩은 연기한다. |
| plugin/의 screen.jsx·editor.jsx·host.mjs | editor는 선택·드래그 미리보기·도면·파형 표시. screen의 어댑터가 파일 선택·invoke·UI 부품을 전달한다. host는 설치본 기준 exe 탐색, 자식과 요청 수명만 관리한다. | 시제품 screen과 browser.jsx가 공유하던 Editor 경계를 유지한다. 단독 브라우저 진입점은 연기한다. |

보관 태그 `archive/qt-cpp-mvp-2026-11-26`의 `src/circuit.{h,cpp}`에서 소자 ID·명명된 핀·네트·편집 규약을, `src/solver.cpp`에서 MNA·후진 오일러·PWM 에지 분리·이상 스위치 판정을 대조한다. `src/schematic.cpp`의 우회·점프 자리 확보·라벨 영역 판정을 순수 기하로 옮긴다. Qt 장면과 마우스 이벤트는 옮기지 않는다. `src/waves.{h,cpp}`의 시간순 원본행과 극값의 행 위치는 유지하되 대용량 인덱스 최적화는 연기한다. [보관 소스](https://github.com/litzio/litzsim/tree/archive/qt-cpp-mvp-2026-11-26/src)

## 3. 파일·편집·도면·실행 계약

| 경계 | 입력 → 성공 결과 | 판정 |
| --- | --- | --- |
| 문서 | new() → 빈 v1, open(file) → {document, revision, diskRevision}, save(file, document, expectedDiskRevision 또는 신규) → {diskRevision} | **v1 유지**. parts·nets·layout·bends·simulation과 단위·부호를 보존한다. 지원 밖 모델은 명시적으로 거절하여 원본을 보존한다. |
| 편집 | apply(document, expectedRevision, ops[]) → {document, receipt}, undo(current, expectedAfter, expectedBefore, previous) → {document, receipt} | 추가·이동·회전·값·단자 연결·삭제를 복사본에 적용하고 모두 유효할 때 한 번 확정한다. 미접속 편집본은 저장 가능하며 실행 전에 누락 핀을 알린다. |
| 도면 | layout(document) → {revision, symbols, pins, segments, junctions, jumps, labels, issues} | 편집 응답에서 apply와 layout을 한 왕복으로 합친다. 동일 입력에서 동일 배치가 나오도록 격자·탐색 순서·라벨 치수를 고정한다. |
| 실행 | start(directory, document, expectedRevision) → {runId, revision, state}, status(runId)·stop(runId) → {runId, state, simulatedTime, stopTime, error?} | 작업본·초기값·설정 그대로 runs/ID/circuit.litzsim에 고정하고 접수한다. 계산 종료까지 RPC를 붙들지 않는다. |
| 파형 | wave(runId, channels, from_s, to_s, pixels) → {channels와 단위, timeRange, 채널별 (row,t_s,value)[]} | 완료된 결과만 조회한다. 구간별 최소·최대 원본행을 시간순으로 전달하고 화면은 가장 최근 요청의 범위만 그린다. |

[지난 설계](2026-11-26-web-editor-architecture.md)의 receipt에는 before/after revision, 생성·수정·삭제 ID, 바뀐 네트별 전후 핀 목록이 있다. 이를 유지한다. 작업 revision은 코어가 정규화한 문서 내용으로 계산하고 expected와 대조한다. Undo는 전 문서와 직전 receipt를 요청자가 보관하고 코어가 현재 문서와 after, 전 문서와 before를 대조한다. 화면에서 revision 문자열만 비교하던 시제품을 대체한다. 실패한 연산 번호·대상 ID·이유를 돌려주며 실패 전 작업본은 유지한다.

소유자 결정(2026-11-26)에 따라 좌우·상하 반전을 같은 편집 계약에 넣는다. v1 `layout`의 선택 필드 `mirror`는 기존 `rotation`을 적용한 뒤 회로도 축을 기준으로 적용한다. 양축 반전은 180° 회전으로 합치고, 0° 회전과 반전 없음은 해당 필드를 생략한다. [CLI 연산과 파일 규약](../../README.md#cli로-회로-편집하기)

디스크 revision은 읽었던 파일 바이트 기준이다. CLI와 플러그인 저장은 같은 파일별 잠금 안에서 revision 검사와 임시 파일 교체를 수행한다. 신규 저장은 기존 파일이 있으면 거절한다. 충돌 응답은 기대·관측 revision을 담고 탭은 미저장 문서를 유지한다. revision·Undo는 회로 파일에 쓰지 않는다. [v1 예제](https://github.com/litzio/litzsim/tree/archive/qt-cpp-mvp-2026-11-26/examples)

도면에서 같은 네트의 분기는 점, 다른 네트의 교차는 두 net ID와 좌표가 있는 점프 반원이다. 연결 명령은 격자 위치에서 추론하지 않고 단자 쌍을 받는다. 전압은 p−n, 전류는 정의된 기준 화살표를 표시한다. 기호·ID·값·핀 라벨 상자를 라우팅 장애물로 취급하고 남은 겹침은 두 대상 ID와 영역으로 반환한다. React는 IEEE 315 기호와 검정 선을 그리고 격자 위에서 드래그한 뒤 드롭 때만 확정한다. [공동 편집](2026-11-25-circuit-editor.md) · [도면 품질 조사](2026-11-25-schematic-publication-audit.md)

host는 실행마다 편집 자식과 독립된 해석 자식을 소유한다. CLI 단독 start도 같은 자식 모드로 실행한다. 해석 자식은 run 폴더의 중지 요청을 적분 구간 사이에 확인하고 진행 상태를 원자적으로 교체한다. stop은 stopping 접수 후 반환하며 자식 종료 확인 뒤 canceled를 표시한다. 플러그인 해제 때는 자식들을 중지하고 응답하지 않는 자식은 강제 종료 후 close를 회수한다. 플러그인 소유 자식은 부모 파이프 EOF도 감시한다. 완료는 결과 임시 파일 교체 성공 뒤에만 기록한다. 실패·취소된 결과는 조회에서 제외한다. Host 일반 RPC 한계는 10초다. [실행 계약](https://github.com/litzio/litzio/blob/cbb6d85f/docs/design/plugin-stage1-v1.md)

## 4. 외부 PC 빌드와 설치

| 방법 | 빌드 결과와 설치 | 판정 |
| --- | --- | --- |
| clone 후 저장소 루트에서 cargo xtask plugin | Cargo alias가 작은 Rust 패키징 작업을 실행한다. 현재 Windows target의 Release CLI를 빌드하고 target/plugin/litzsim/ 아래에 litzio-plugin.json, host.mjs, screen.jsx, editor.jsx, bin/litzsim.exe를 실제 파일로 모은다. | **추천**. 이 폴더를 Litzio [플러그인 추가…]로 등록하고 처음 켤 때 신뢰한다. 빌드 결과는 커밋하지 않는다. |

manifest의 entry와 screen은 각각 host.mjs와 screen.jsx다. host는 import.meta.url 기준으로 설치본의 bin을 찾는다. 로컬 등록은 manifest를 검증하고 폴더를 Host 전용 위치로 복사한다. .git은 제외하고 실제 node_modules는 있으면 복사하며 링크는 거부한다. 로컬 설치·다시 읽기에는 npm 다운로드나 네이티브 빌드가 없다. GitHub 출처의 lockfile 설치와 구별한다. [로컬 설치·의존성 규칙](https://github.com/litzio/litzio/blob/cbb6d85f/docs/design/plugin-stage1-v1.md)

화면은 artifact 내부 상대 import, 주입된 React와 ui, 앱의 토큰·글꼴만 사용한다. Host의 esbuild가 로컬 JSX를 처리한다. 외부 모듈 연결은 react, react/jsx-runtime, react-dom만 허용한다. 시제품 browser.jsx의 react-dom/client 진입점은 배포 대상에서 뺀다. 호스트용 공개 SDK의 루트와 /node는 화면 라이브러리가 아니다. 이 구성에는 별도 Node 설치·npm·MSYS2가 필요하지 않도록 설계한다. 새 파일은 어댑터에서 폴더 선택 후 파일명을 받아 신규 저장하며 기존 파일은 확장자로 골라 연다. [화면 계약](https://github.com/litzio/litzio/blob/cbb6d85f/docs/design/plugin-screens-v1.md) · [번들 구현](https://github.com/litzio/litzio/blob/cbb6d85f/apps/editor/server/plugins/screen-code.ts) · [ADR-0022](https://github.com/litzio/litzio/blob/cbb6d85f/docs/adr/0022-plugin-screen-code.md) · [SDK exports](https://github.com/litzio/litzio/blob/cbb6d85f/packages/plugin-sdk/package.json)

## 5. AI가 읽고 쓰는 경로

| 경로 | 명령 형태와 읽을 결과 | 판정 |
| --- | --- | --- |
| CLI | litzsim inspect FILE --json → revision·소자 값/단위·핀별 net·프로브 극성. litzsim apply FILE --expect REV --ops OPS.json → 저장된 변경과 receipt. litzsim undo FILE --expect AFTER --before BEFORE --previous SNAPSHOT → 복원 receipt. | **같은 코어를 쓰는 CLI만 제공**. 변경 전 파일을 보관한 호출자가 Undo할 수 있다. |
| 같은 CLI의 도면·실행 조회 | litzsim layout FILE --json → 예컨대 gate와 input의 비접속 교차 좌표, G 라벨과 배선이 겹치면 대상·영역. litzsim start FILE → run 폴더, litzsim status RUN·stop RUN → 진행·종료 상태, litzsim wave RUN --channel VC --from T0 --to T1 --pixels W → 시간·값. | GUI와 같은 ID·진단·스냅샷을 출력한다. 비서 등록·스킬·Litzio 문서 연동은 연기한다. |

## 6. 구현 PR 순서와 완료 판정

추천 순서는 아래와 같다. 각 행을 따로 병합한다. 2와 3은 1 뒤에 독립 시험할 수 있고 CLI 명령 등록·workspace manifest 같은 공통 파일은 번호 순으로 합친다. 4는 3, 5는 2와 4가 필요하다. 수치 허용범위는 결과를 보기 전에 보관 시험과 해석해로 고정한다. CPU·Windows·Release·측정 구간과 제외 시간을 함께 기록한다. [재구축 인수표](../rebuild.md) · [기준해·조건](../reviews/pfc-validation.md)

| 순서 | 납품 단위 | 통과 조건 |
| --- | --- | --- |
| 선행 스파이크 | 외부 PC의 깨끗한 clone에서 단일 명령 → 로컬 등록·활성 → 화면 버튼으로 동봉 exe 왕복 | 시스템 Node·pacman·절대 경로 설정 없이 설치 복사본에서 응답하고 비활성화 뒤 자식 0개. 하나라도 실패하면 설치 경로·도구 의존성을 분리해 기록하고 패키징 결정을 재검토한다. |
| 1 | 문서·편집과 파일 CLI | RC·RLC·벅 v1 재열기, 빈 회로부터 핀 연결·삭제·Undo, 중간 실패 시 일괄 원복, 오래된 저장 거절. 저장→재열기 후 ID·단위·극성·배치 일치. |
| 2 | 도면 배치와 layout CLI | 세 예제에서 직각 경로·격자·분기점·점프·라벨 상자 검사. 벅 gate 교차는 비접속, 접속점에는 같은 네트만 있다. 이동·회전 뒤 겹침 진단과 Undo 후 원래 배치 대조. |
| 3 | 해석과 원본 파형 쓰기 | 5 V 계단·0–5 ms·초기 0인 RC/RLC에서 10→5→2.5 µs마다 최대 전압 오차가 거의 절반. 벅 5 V 입력·3 V 고정 출력·1 mH·10 kHz·듀티 0.5·0.37 µs 스텝에서 초기 0.2/0 A별 구간 기준해와 피크 0.30/0.10 A, DCM 첫 영전류 83.33 µs를 한 스텝 이내로 대조. |
| 4 | CLI 실행 수명·파형 조회 | 실행 중 원본을 수정해도 스냅샷 유지. 10초 넘는 실행의 접수·조회·중지 각각 10초 미만, 자식 종료 및 부분 결과 비공개. 일반 크기 파형에서 확대·이동한 범위의 단일 피크·시간 순서를 원본행과 대조. |
| 5 | React 편집·파형, 플러그인 어댑터와 패키징 | 외부 PC 등록 후 새 회로 작성→저장→재열기→실행→파형 탐색. 실제 Windows 창에서 핀 극성·화살표·점프·라벨, 축·범례·피크 확인. 실행 중 편집·중지·종료까지 4의 조건을 Host에서 반복한다. |

5의 지연 시험은 선행 스파이크와 같은 RC 배치에서 드롭과 두 번째 핀 클릭 각각 50회, 매회 Undo 복원으로 한다. 입력부터 해당 revision의 확정 프레임까지 연산별 p95 ≤100 ms다. 왕복 시간과 프레임 대기를 분리하고 파형 응답 뒤 불필요한 프레임 예약을 더하지 않는다. 느린 옛 조회 응답이 최신 확대 범위를 덮지 않는지도 검사한다. [편집 측정 조건](2026-11-26-litzio-plugin-spike.md) · [파형 스파이크](2026-11-26-web-waveform-spike.md)

## 7. 이후 선택을 열어 두는 최소 조건

| 지금 보존할 것 | 나중에 판정할 것 |
| --- | --- |
| 명명된 핀·단위·초기조건과 실행별 스냅샷 | PFC·폐루프 제어기·CLLLC 모델과 검증 |
| 원본 시간·행 순서와 범위 조회, 물리적 연결과 독립된 도면 기하 | 1,000만 행 인덱스·성능, 논문 폭에서 글자·선·접속점 치수와 벡터 SVG/PDF |
| Litzio 무의존 CLI와 화면 어댑터 | 단독판 껍데기. 1차에는 확장 트레이트·외부 플러그인 체계를 추가하지 않는다. |

## 소유자가 정할 것

| 제품 선택 | 근거 | 추천 |
| --- | --- | --- |
| 실행 중 다시 실행을 누르면 기존 실행을 유지하거나 병렬 실행을 허용 | 1차 목표는 한 회로를 편집하면서 현재 실행을 관찰·중지하는 흐름이다. | 현재 실행을 유지하고 중지 후 다시 실행하도록 표시한다. |

## 미검증 가정

- 외부 PC에서 rustup·cargo run이 동작한다는 소유자 확인까지 있다. 실제 의존성 취득·Release 링크·설치본의 자식 실행과 해제는 선행 스파이크 대상이다.
- 35 ms는 보관된 C++ 상주 코어 측정이다. Rust 직렬화·배선 계산·Windows 실제 글꼴로 표시한 라벨 경계·실행 중 편집 지연은 새 구현으로 측정해야 한다.
- 비정상 종료 때 Windows 자식의 EOF 감지와 중지 지연, 실제 설치판 Node에서의 WASM worker 성능은 검증하지 않았다.
