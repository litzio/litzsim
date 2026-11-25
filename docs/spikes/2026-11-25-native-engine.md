# 네이티브 엔진과 성능

2026-11-25 · 구현 후보와 벤치마크 제안

## 기술 후보

Windows 독립 실행과 Python 런타임 없는 배포를 우선한다. 논의의 추천 조합은 **C++·Qt 6 편집기 + 헤드리스 C++ 엔진**이다. 선행 GUI에서 확인한 기호와 편집 동작은 C++에서도 같은 Qt API로 구현할 수 있다. C ABI는 펌웨어 제어기를 같은 코드로 시험할 때 유용하다. Rust의 faer·Diffsol도 후보이며, 같은 정확도에서 C·C++·Rust 성능을 직접 비교한 결과는 아직 없다. [Qt C++](https://doc.qt.io/qt-6/cmake-get-started.html) · [Diffsol](https://docs.rs/diffsol/latest/diffsol/)

작은 상태공간·복소 행렬은 Eigen, 강성 ODE는 SUNDIALS CVODE, 대수 제약을 포함하는 회로는 IDA, 큰 희소 회로 행렬은 SuiteSparse/KLU를 비교한다. 스위치가 일정한 구간의 상태전파, 영교차 탐색, 제어 샘플 갱신은 회로와 사건 구조에 맞춰 설계한다. [Eigen](https://libeigen.gitlab.io/) · [SUNDIALS](https://sundials.readthedocs.io/en/latest/) · [KLU](https://github.com/DrTimothyAldenDavis/SuiteSparse/tree/dev/KLU)

제어 설계 화면은 유리 전달함수의 상태공간 실현과 주파수응답을 지원하는 방향이다. CLLLC의 주파수-전류 전달함수에는 배터리 전압과 전력 방향을 포함한 선형화 운전점을 붙인다. 20 µs 등 사용자가 정한 제어주기의 ZOH·계산 지연·포화를 시간영역 시험에 반영한다. PFC의 50 Hz 주기궤도와 100 Hz 링크 리플을 분석할 때는 단일 시점 선형화의 적용 범위를 밝힌다.

## CPU와 GPU를 비교할 실험

| 시험 | 비교할 항목 |
| --- | --- |
| 벅 + PI, 2상 PFC 기동 한 건 | CPU 사건 기반 해석과 GPU 상주 해석의 전체 시간, 동일 사건 시각·전류 피크·전압 저점 |
| 같은 벅 + 다단계 MPC | 예측 지평에 따른 후보 평가 비용, CPU 가지치기와 GPU 병렬 평가, 샘플별 게이트 결정 |
| 주파수·PI·부품 공차 탐색 | 동일 토폴로지 1·16·64·256·1,024건에서 멀티코어 CPU와 GPU 처리량, 전송·결과 회수 포함 시간 |
| 긴 파형의 표시 | 확대·이동 중 프레임 시간과 최소·최대 보존, 시뮬레이션 중 UI 응답 |

2상 PFC의 두 레그는 DC-Link를 공유하고 다음 제어 출력은 직전 상태에 의존한다. 한 실행에 독립 작업이 많은 MPC나 대규모 플랜트, 여러 설계 후보의 병렬 시험에 GPU를 우선 검토한다. 전체 파형을 모든 후보에 저장할 때의 메모리 사용량과 지표만 누적할 때의 비용을 함께 기록한다.

선행 Windows 실험(Ryzen 5 5600X, Numba 캐시 사용, 별도 프로세스)에서 CLLLC 20 ms는 스위칭 1.499 s, EDF 1.056 s의 엔진 시간이 걸렸다. 같은 8개 평균 신호를 20 µs마다 저장했다. 10 s EDF 계단 지령은 엔진 시간 9.886 s였다. 평탄한 구간의 이산 고정점 검사가 적용된 사례이므로 연속 램프나 다른 솔버에 이 수치를 외삽할 수 없다. 새 엔진에서는 빌드·준비, 계산, 저장, 표시를 분리하고 최대 메모리와 정확도를 함께 보고한다.

## 비교할 공개 프로젝트

[Pulsim](https://lgili.github.io/Pulsim/1.8.0/)은 회로 기술·스위칭 해석·후처리의 비교 대상이다. [GeckoCIRCUITS](https://github.com/geckocircuits/GeckoCIRCUITS)는 전력전자 회로와 제어 편집, [GSEIM](https://arxiv.org/abs/2204.12924)은 주기 정상상태 해석, [OpenModelica의 Buck 예제](https://build.openmodelica.org/Documentation/PVSystems.Examples.Application.BuckOpen.html)는 동일 듀티를 사용하는 스위칭·평균 모델 구성의 참고 사례다. 이들 프로젝트와의 속도 차이는 동일 회로·오차·저장 정책으로 별도 측정해야 한다.
