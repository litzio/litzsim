---
name: litzsim-orchestrator
description: Orchestrate several LitzSim GitHub issues end to end by planning waves, launching Paseo worker agents in isolated worktrees, reviewing risky PRs, verifying each merge result locally with CMake/CTest, and merging into main. Use only when the user explicitly invokes litzsim-orchestrator inside the litzio/litzsim repository.
disable-model-invocation: true
---

# LitzSim Orchestrator (Claude Code entry)

이 파일은 Claude Code용 진입점이다. 절차의 원본은 Codex와 함께 쓰는 `.agents/skills/litzsim-orchestrator/SKILL.md` 하나다.

1. 저장소 루트의 `.agents/skills/litzsim-orchestrator/SKILL.md`를 읽고 그대로 따른다.
2. 그 문서의 `references/`, `scripts/` 경로는 `.agents/skills/litzsim-orchestrator/` 기준이다.
3. 이 진입점에 절차를 복사하거나 따로 고치지 않는다. 바꿀 내용은 원본에만 반영한다.
