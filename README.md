# LitzSim · 리츠심

사람과 AI가 같은 회로도를 편집하고 실험을 재현하는 오픈소스 전력전자 시뮬레이터를 개발합니다.

첫 예제는 양방향 온보드 충전기(OBC)입니다. 엔지니어가 회로에서 선택한 가지에 AI가 센서나 필터를 넣고, 변경한 회로에서 스위칭 모델과 평균 모델의 응답을 비교하는 것이 목표입니다. 첫 제품은 [Litzio](https://github.com/litzio/litzio) 플러그인입니다.

## 현재 상태

**Rust로 다시 만드는 중입니다.** 플러그인의 **코어 확인** 버튼에서 동봉된 CLI의 버전과 호출 왕복 시간을 볼 수 있습니다. 재구축 기준은 [Rust 재구축](docs/rebuild.md)에 있습니다.

C++·Qt로 만든 1차 MVP(회로 편집기, 해석 엔진, 파형 창, 논문용 SVG/PDF 내보내기)는 [`archive/qt-cpp-mvp`](https://github.com/litzio/litzsim/tree/archive/qt-cpp-mvp) 브랜치와 `archive/qt-cpp-mvp-2026-11-26` 태그에 보관했습니다.

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
