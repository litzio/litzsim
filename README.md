# LitzSim · 리츠심

사람과 AI가 같은 회로도를 편집하고 실험을 재현하는 오픈소스 전력전자 시뮬레이터를 개발합니다.

첫 예제는 양방향 온보드 충전기(OBC)입니다. 엔지니어가 회로에서 선택한 가지에 AI가 센서나 필터를 넣고, 변경한 회로에서 스위칭 모델과 평균 모델의 응답을 비교하는 것이 목표입니다.

현재 C++ 공통 코어에서 JSON 회로 파일을 검사하고 복사할 수 있습니다. 예제는 [RC](examples/rc.litzsim)와 [벅](examples/buck.litzsim)입니다. 단자 이름은 `소자ID.핀` 형식이며 전압·전류 프로브의 양의 방향은 p에서 n입니다. 연결은 nets, 격자 위치는 layout, 적분 간격과 종료 시각은 simulation에서 관리합니다.

Qt 6 개발 환경, C++17 컴파일러와 CMake로 빌드합니다. 아래는 Windows의 단일 구성 Release 빌드 예입니다.

```powershell
cmake -S . -B build -G Ninja -DCMAKE_BUILD_TYPE=Release
cmake --build build
ctest --test-dir build --output-on-failure
./build/litzsim-cli inspect examples/rc.litzsim
```

[제품 목적](docs/purpose.md) · [에이전트 작업 안내](AGENTS.md)
