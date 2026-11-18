# Postudio 백로그 — 연구 앱과 확장 (v0.4 기준)

> 지금은 [`PLAN.md`](PLAN.md)의 MVP(멀티 호스트 + 폰)에 집중한다.
> 아래는 결정된 사항과 설계 메모를 보존하는 문서다. 순서는 확정되지 않았다.
> 원칙: 이 앱들이 쓸 데이터는 **처음부터 append-only 이벤트로 기록**해서 P3 원장에 그대로 복제되게 한다.

공통 개념 — **계보(provenance)**: `dataset@ver → run → checkpoint → figure/table → paper`

---

## B1. Experiments — 학습 실행·관리

**확정된 사실**: PyTorch + TensorBoard. 하네스에게 학습 스크립트와 TensorBoard 실행을 지시하는 방식.
→ 별도 job 시스템을 강요하지 않는다. **에이전트가 띄운 프로세스를 "실험"으로 승격**하는 방향으로 간다.

| 기능 | 내용 |
|---|---|
| 지속성 | 에이전트가 띄운 학습을 `systemd-run --user` scope(또는 tmux)로 감싸 에이전트·node 재시작과 무관하게 유지 (MVP는 에이전트 백그라운드 작업에 의존) |
| 실험 레코드 | `id, host, gpus, git commit + dirty patch 해시, command, config 스냅샷, env lock 해시, 입력 dataset@ver, 출력 경로, TensorBoard logdir, status, 시작/종료, exit code` |
| TensorBoard | logdir를 node가 인식 → Studio에서 run별 TB 열기(MVP의 포트 프록시 재사용). 여러 run은 TB 하나로 모아보기 |
| 지표 요약 | TB event 파일에서 스칼라를 node가 읽어 run 비교표·푸시 요약 생성 |
| GPU 현황 | NVML: 3090(24GB)·8000(48GB) 사용률, VRAM, 온도, 프로세스 |
| 배치 | VRAM 요구량 기준으로 어느 서버에 띄울지 에이전트가 제안 (스케줄러는 단순 큐부터) |
| 감시자 | 완료·실패·NaN·loss 급등·GPU idle(멈춤 의심) → 푸시 + 요약 |
| 재현 | commit + patch + env + data로 "다시 실행" |
| 장애 점검 대응 | 서버 점검 공지 시 실행 중 실험 목록과 마지막 체크포인트 위치를 알림 |
| 툴 | `exp.launch / status / logs / compare / kill*` (* 승인) |

## B2. Data & NAS

**확정된 사실**: NAS는 SMB/NFS를 모두 지원하지만 **설정 변경 불가, 이미 마운트된 경로만 사용**.
→ NAS 스냅샷·Docker·쿼터 API·NAS 측 스캐너를 쓸 수 없다. 모든 처리는 호스트의 마운트를 통해서 한다.

| 기능 | 내용 |
|---|---|
| 경로 추상화 | `nas://<share>/<path>` 논리 URI ↔ 호스트별 마운트(Ubuntu `/mnt/...`, mac `/Volumes/...`, Windows 드라이브/UNC) 매핑 |
| 데이터셋 레지스트리 | `name@version, URI, 크기, 파일 수, manifest 지문, 출처·라이선스, derived_from, used_by` |
| 지문 | TB급은 (경로, 크기, mtime) manifest + 샘플 해시. 전체 BLAKE3는 선택 |
| 용량 보기 | 전체 사용률은 마운트의 `statvfs`, 세부는 **내 디렉터리만** 증분 스캔 (공유 NAS라면 타인 영역은 스캔하지 않음). 인덱스는 로컬 SQLite |
| 정리 후보 | 실패·대체된 run의 체크포인트, 보존 정책 초과 중간 체크포인트, 중복 파일, 캐시류, 레지스트리에 연결되지 않은 출력 |
| 정리 실행 | NAS 스냅샷이 없으므로 더 보수적으로: 계획 → 승인 → 내 영역의 `.trash/`로 이동 → 30일 후 삭제(다시 승인) |
| 보호 | 데이터셋은 에이전트에게 읽기 전용, `immutable` 데이터셋 쓰기 거부, 삭제 계열은 항상 승인 |
| 성능 | SMB/NFS 메타데이터 스캔이 느릴 수 있음 → GPU 서버(NAS와 가까운 쪽)에서 야간 증분 스캔 |
| 툴 | `data.find / register / preview`, `nas.usage / cleanup_plan / trash*` |

## B3. Library — Potero 편입

**확정된 사실**: POSTECH GenAI 미사용. **서지관리 AI 기능은 Distill이 담당**.

| 층 | 방식 |
|---|---|
| 레포 | `services/potero`에 git subtree (이력 보존) |
| 런타임 | Ktor(:18080) + GROBID를 GPU 서버 한 대에서 node가 감독, `/svc/library/*` 프록시 |
| PDF 저장 | NAS (`nas://library/`) |
| **AI 기능 → Distill** | Potero의 `PostechLLMService` 등 LLM 계층을 Distill로 교체. ① 단발성 작업(메타데이터 정제, 태그 추천, 요약, narrative) → node의 내부 LLM 엔드포인트(Distill 계정·라우팅, Gemini OAuth 포함) ② 논문 대화·related work처럼 툴이 필요한 작업 → **Distill 세션**(`lib.*` 툴 + 논문 컨텍스트) |
| UI | Potero 화면(Library, PdfViewer, Inspector, Notes)을 Studio 라우트로 이식 (같은 스택) |
| 에이전트 툴 | `lib.search / get / add / cite / related / figures` → 코딩·Writing 세션에서 사용 |
| Potero 우선순위 유지 | 리더 내 AI UX(Auto Highlight, Quick Summary, Smart Citation…)를 Distill 기반으로 계속 |
| 원장 (후기) | Potero repository 계층에 이벤트 발행 → 각 호스트 Potero DB를 프로젝션으로 |

## B4. Writing — Overleaf를 하네스로

**확정된 사실**: Overleaf Git 연동 가능 플랜.

| 구성 | 내용 |
|---|---|
| 동기화 | 논문 = git repo(호스트 귀속, 코딩 모드와 동일) ↔ Overleaf Git 브리지. 공저자 실시간 협업은 Overleaf에 그대로 |
| 편집 | CodeMirror 6 + LaTeX, 에이전트 편집은 hunk 단위 수락/거절 |
| 컴파일 | TeX Live 고정 컨테이너 + `latexmk` (GPU 서버 또는 로컬) |
| 미리보기 | Potero PdfViewer + SyncTeX 정·역방향 검색 |
| 컴파일 루프 | 로그 파싱 → 자동 수정 → 재컴파일 |
| 인용 | Library 검색 → `\cite` 삽입 → `.bib` 동기화, 누락·미사용 참조 경고 |
| **결과 동기화** | 실험 레코드 → `results.tex` 매크로(`\oursPSNR` 등) 자동 생성. 본문 숫자가 run에 묶임 |
| 그림 계보 | 그림 스크립트 → PDF 그림 + sidecar(run id, commit) |
| 제출 | 학회 템플릿 점검, double-blind 익명화 검사, arXiv 패키징, 마감 추적 |
| 툴 | `tex.build / errors / results_sync / overleaf_push*` |

## B5. 그래픽스 뷰어

이미지 비교(줌 동기화, A/B 슬라이더, 차이 히트맵, FLIP), EXR/HDR(node에서 톤매핑 프리뷰 + 픽셀값 조회), 시퀀스·비디오, 3D(glTF/OBJ/PLY, 3DGS splat), 에이전트용 `present(uri | compare[a,b])`.

## B6. Antigravity `agy` 어댑터

v0.3에서 "Antigravity 연결"은 **Distill의 Gemini OAuth**(PLAN §12.1)로 정리됐다.
`agy` 하위 프로세스 어댑터는 다른 하네스 비교용 선택 기능으로 보존한다:
- `agy -p --input-format stream-json --output-format stream-json`, `--conversation <id>` 재개
- 대화 저장: `~/.gemini/antigravity-cli/conversations/<uuid>.db`
- node MCP를 `agy mcp add`로 등록하면 연구 툴 공유 가능

## B7. 워크스페이스 초대

- 원장 엔트리를 `author_host`와 `author_user`로 분리하고, topic에 `workspace_id` 부여, 워크스페이스별 키
- 네트워크: Tailscale 노드 공유
- OWUI 부품 라이선스: 30일 사용자 50명 이하 유지
