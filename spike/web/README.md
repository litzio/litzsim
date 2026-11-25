# LitzSim 웹 스파이크

Windows Release 빌드에서 생성한 `litzsim-cli.exe`와 같은 폴더의 Qt 런타임을 사용한다. RC와 벅 예제는 쓰기 가능한 별도 시험 폴더에 복사한다. 장기 실행용 복사본 `rc-long.litzsim`, `buck-long.litzsim`에는 각각 `stopTime_s` 10000, 100을 지정하고 `outputStep_s`를 `timeStep_s`의 1000배로 설정한다.

## 브라우저

```powershell
npm install --prefix spike/web --ignore-scripts
node spike/web/server.mjs build/litzsim-cli.exe <fixture-directory> cold
node spike/web/server.mjs build/litzsim-cli.exe <fixture-directory> persistent
```

각 서버는 loopback의 빈 포트를 사용한다. 실행 결과에 표시된 주소를 열고 정리할 때 Ctrl+C를 누른다. 이미 esbuild, React와 React DOM이 설치된 환경에서는 그 폴더의 `package.json` 절대 경로를 `SPIKE_BROWSER_PACKAGE`로 지정할 수 있다.

## Litzio

Litzio Host에서 접근이 허용된 별도 폴더에 `plugin/`을 복사한다. 실제 Host 주소를 지정해 `plugins validate`와 `plugins install`을 실행한다. 시험 플러그인 설정의 `executable`에 `litzsim-cli.exe` 절대 경로를 넣고 `persistentCore`를 켜거나 끈다. 설정 JSON은 artifact 밖에 둔다. Litzio에서 시험 회로 폴더를 열고 작업 막대의 LitzSim을 선택한다. 시험 종료 후 같은 Host에서 `plugins unregister local.litzsim-spike`를 실행하고 목록에서 제거를 확인한다.

## 반복 측정

Windows에서 재사용 persistent 프로필을 사용한다. `measure.js`의 CONFIG 주석을 `const config = { host: true, benchmark: true, count: 50 };`로 교체한 사본을 `playwright-cli run-code --filename=<file>`에 전달한다. 브라우저 단독판은 `host: false`다. RC를 열어 저항을 추가하고 드롭과 단자 연결을 각각 50회 수행한다. 한 번의 편집마다 Undo로 기준 상태를 복원한다. `flow: true`는 RC와 벅 저장·실행·파형·중지를 검사하므로 원본에서 복사한 시험 회로를 사용한다. `capture: true`는 플러그인 화면 영역의 PNG를 만든다.

`node spike/web/verify-flow.mjs build/litzsim-cli.exe <fixture-directory>`로 UI 저장본을 명시적인 v1 예상값과 CLI 배치 결과에 대조한다. 완료 파형은 직접 CLI 실행 결과와 바이트 단위로 대조하고, 중지한 실행은 스냅샷과 상태를 읽는다.
