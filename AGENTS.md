# Repository instructions

## Local Rust builds

- Reuse the checkout's default `target/` for all Cargo commands. Do not create task-specific `target-*` directories or override `CARGO_TARGET_DIR`/`--target-dir` just to isolate a build. If Cargo waits on the build-directory lock, wait rather than starting a separate build.
- This repository disables incremental compilation in `.cargo/config.toml` to limit disk use across parallel agents. Use `CARGO_INCREMENTAL=0` for one-off verification and deployment. Avoid changing profiles, feature sets, or `RUSTFLAGS` without a reason; Cargo may retain multiple copies of the artifacts.
- Before deploying a persistent server, copy the executables out of Cargo's `target/`. Keep runtime state separate from build output. Never remove a binary in use or an installed CLI symlink target.
- Before long builds, check free space with `df -h .`. If space falls below 20 GB, inspect `scripts/clean-build-cache.sh` in dry-run mode before deciding what to delete. Do not clean other checkouts or the global Cargo cache as a side effect of routine builds.
