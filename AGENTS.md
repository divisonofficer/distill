# AGENTS.md — DistillCustom

## Build hygiene (disk)

Parallel agents once filled this laptop's disk (98%, ~150 GB of Cargo output). For every Cargo run:

- Build dirs in this checkout: only `target/` (full workspace: `distill`, `distill-shell`, `postudio-probe`) and `target-node/` (`-p distill-node` only; see `plan/research-base/EXECUTION.md`). Never create another `CARGO_TARGET_DIR` per task. If Cargo waits on the build-directory lock, wait.
- In a separate worktree (e.g. `~/.distill/worktrees/...`), build into that worktree's own `target/` and delete it when the task ends.
- Incremental compilation is off repo-wide (`.cargo/config.toml` `[build] incremental = false`). Do not set `CARGO_INCREMENTAL=1` in agent runs.
- Before a long build, check free space (`df -h ~`). Under 20 GB, run `scripts/clean-build-cache.sh --apply` (it removes only caches no process is using) and report what it freed.
- Never delete `target/release/distill` (`~/.local/bin/distill` links to it) or the binary of a running process.
