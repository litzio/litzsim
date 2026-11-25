# LitzSim 작업 안내

- [제품 목적](docs/purpose.md)과 [문서 색인](docs/README.md)에서 작업에 관련된 문서를 먼저 읽는다.
- 구현 언어는 Rust다. C++·Qt 1차 MVP는 `archive/qt-cpp-mvp`에 보관했고, 새 구현의 인수 기준은 [Rust 재구축](docs/rebuild.md)을 따른다.
- 1차 제품은 Litzio 플러그인이다. [제품 목적](docs/purpose.md#litzio-플러그인-우선)의 네 경계(코어는 Litzio 무의존·CLI로 동작, 화면은 어댑터로만 Litzio 호출, 데이터는 LitzSim 파일, 핵심 기능은 비서 무의존)를 지킨다.
- 회로의 단자·연결·측정 방향과 편집 연산은 공통 코어가 소유한다. GUI와 AI는 같은 연산을 호출한다.
- 모델 변경은 같은 회로·제어·초기조건에서 검증한다. 정확도와 실행시간을 보고할 때 비교 조건과 미검증 범위를 남긴다.
- 요청을 만족하는 작은 변경을 우선한다. 설계 탐색은 `docs/spikes/`에 기록하고 새 문서를 색인에 연결한다.
- AI가 작성한 커밋에는 실제 작성 도구와 모델을 `AI-Agent`, `AI-Model` 트레일러로 기록한다.
