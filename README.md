# LitzSim · 리츠심

사람과 AI가 같은 회로도를 편집하고 실험을 재현하는 오픈소스 전력전자 시뮬레이터를 개발합니다.

첫 예제는 양방향 온보드 충전기(OBC)입니다. 엔지니어가 회로에서 선택한 가지에 AI가 센서나 필터를 넣고, 변경한 회로에서 스위칭 모델과 평균 모델의 응답을 비교하는 것이 목표입니다.

Qt 편집기에서 [RC](examples/rc.litzsim), [RLC](examples/rlc.litzsim), [벅](examples/buck.litzsim), [2상 토템폴 PFC](examples/pfc.litzsim) 예제를 열고 실행할 수 있습니다. 단자 이름은 `소자ID.핀` 형식이며 전압·전류 프로브의 기준 방향은 p에서 n입니다. 연결은 nets, 격자 위치는 layout, 적분 간격과 종료 시각은 simulation에서 관리합니다.

Qt 6 Core·Widgets·Test와 SVG 아이콘 플러그인, C++17 컴파일러와 CMake로 빌드합니다. 아래는 Windows의 단일 구성 Release 빌드 예입니다. MSYS2 UCRT64 환경에서는 해당 환경의 bin 디렉터리가 PATH에 있어야 합니다.

```powershell
cmake -S . -B build -G Ninja -DCMAKE_BUILD_TYPE=Release
cmake --build build
ctest --test-dir build --output-on-failure
./build/litzsim-cli inspect examples/rc.litzsim
./build/litzsim-cli run examples/buck.litzsim build/buck.csv
./build/litzsim examples/pfc.litzsim
```

라이브러리에서 부품을 누르거나 캔버스에 놓고, 두 단자를 차례로 눌러 연결합니다. 소자를 끌어 이동하고 선택 후 R로 회전합니다. 값을 더블클릭해 수정하고 Ctrl+Z로 되돌립니다. F5는 현재 탭을 실행하고 Shift+F5는 중지합니다. 회로 옆 `runs` 폴더에 실행 시점의 회로·설정과 결과를 보관합니다. 결과 창에서 휠로 시간축을, Shift+휠로 세로축을 확대하고 끌어서 이동합니다. 빈 plot을 추가한 뒤 신호 겹치기에서 실행·X·Y·오른쪽 축을 선택할 수 있습니다.

![2상 토템폴 PFC 실제 편집 화면](docs/reviews/assets/pfc-editor.png)

해석 설정에서 결과 저장 간격을 time-step의 정수 배로 지정할 수 있습니다. 예를 들어 time-step이 1 µs이고 저장 간격이 7 µs이면 0, 7, 14 µs 순으로 기록하며 종료 시각의 표본도 보관합니다. JSON 필드는 `simulation.outputStep_s`입니다. 비워 두면 기존 파일처럼 해석기가 내보내는 모든 표본을 저장합니다.

[검증 조건과 측정값](docs/reviews/pfc-validation.md) · [리치오 화면 비교](docs/reviews/desktop-shell.md)

[제품 목적](docs/purpose.md) · [에이전트 작업 안내](AGENTS.md)
