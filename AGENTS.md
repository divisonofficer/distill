# Repository instructions

## Local Rust builds

- Reuse the checkout's default `target/` for `cargo build`, `cargo test`, `cargo check`, and `cargo clippy`. Do not create task-specific `target-*` directories or override `CARGO_TARGET_DIR`/`--target-dir` just to isolate a build. If a worktree is unnecessary, reuse the existing checkout too.
- For one-off verification and deployment builds, prefix the Cargo command with `CARGO_INCREMENTAL=0`. Keep the normal incremental defaults for repeated local development. Avoid varying profiles, feature sets, and `RUSTFLAGS` without a reason, since Cargo may retain multiple copies of the artifacts.
- Before starting or updating a persistent server, copy the built executables to a deployment directory outside Cargo's `target/` and run them from there. Keep deployment binaries and runtime state separate from build output.
- If disk space becomes tight, inspect `du -sh target` and check for active builds and running binaries before cleaning. Never delete a build directory used by a running service. Do not clean other checkouts or the global Cargo cache as a side effect of routine builds.
