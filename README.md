# LitzSim · 리츠심

사람과 AI가 같은 회로도를 편집하고 실험을 재현하는 오픈소스 전력전자 시뮬레이터를 개발합니다.

첫 예제는 양방향 온보드 충전기(OBC)입니다. 엔지니어가 회로에서 선택한 가지에 AI가 센서나 필터를 넣고, 변경한 회로에서 스위칭 모델과 평균 모델의 응답을 비교하는 것이 목표입니다. 첫 제품은 [Litzio](https://github.com/litzio/litzio) 플러그인입니다.

## 현재 상태

**Rust로 다시 만드는 중입니다.** 플러그인의 **코어 확인** 버튼에서 동봉된 CLI의 버전과 호출 왕복 시간을 볼 수 있습니다. 재구축 기준은 [Rust 재구축](docs/rebuild.md)에 있습니다.

C++·Qt로 만든 1차 MVP(회로 편집기, 해석 엔진, 파형 창, 논문용 SVG/PDF 내보내기)는 [`archive/qt-cpp-mvp`](https://github.com/litzio/litzsim/tree/archive/qt-cpp-mvp) 브랜치와 `archive/qt-cpp-mvp-2026-11-26` 태그에 보관했습니다.

## 외부 PC에서 설치하기

Windows x64에서 [rustup](https://rustup.rs/)으로 Windows GNU 툴체인(`stable-x86_64-pc-windows-gnu`)을 설치합니다. 기존 rustup을 쓰는 경우 아래처럼 이 PowerShell 세션에서 사용할 툴체인을 선택합니다.

```powershell
rustup toolchain install stable-x86_64-pc-windows-gnu --profile minimal
$env:RUSTUP_TOOLCHAIN = "stable-x86_64-pc-windows-gnu"
git clone https://github.com/litzio/litzsim.git
cd litzsim
cargo xtask plugin
```

1. Litzio에서 **설정 → 플러그인 → 플러그인 추가…**를 열고, 명령이 출력한 `target/plugin/litzsim/` 폴더를 선택합니다.
2. 목록에서 **LitzSim**을 켜고 **신뢰하고 켜기**를 선택합니다.
3. 작업 막대에서 **LitzSim**을 열고 **코어 확인**을 눌러 코어 버전과 왕복 시간을 확인합니다.

업데이트할 때는 같은 저장소에서 `git pull`과 `cargo xtask plugin`을 실행합니다. **다시 설치**가 표시되면 선택합니다. 이 버튼이 표시되지 않을 때는 연결한 Host를 지정해 `litzio plugins reload local.litzsim --host <Host 주소> --json`으로 다시 설치합니다.

[제품 목적](docs/purpose.md) · [문서 색인](docs/README.md) · [에이전트 작업 안내](AGENTS.md)
