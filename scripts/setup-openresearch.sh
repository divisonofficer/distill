#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != Darwin ]]; then
  printf 'This setup script builds the macOS OpenResearch runtime.\n' >&2
  exit 1
fi

repo="$(cd "$(dirname "$0")/../../OpenResearch" && pwd)"
expected_commit=cb75d884e76f06ff69cb6a29b9dae44f5f03b4a6
patch_file="$(cd "$(dirname "$0")/.." && pwd)/patches/openresearch/postudio-contract.patch"
actual_commit="$(git -C "$repo" rev-parse HEAD)"
if [[ "$actual_commit" != "$expected_commit" ]]; then
  printf 'OpenResearch checkout must be %s; found %s\n' "$expected_commit" "$actual_commit" >&2
  exit 1
fi
if [[ -z "$(git -C "$repo" status --porcelain)" ]]; then
  git -C "$repo" apply --check "$patch_file"
  git -C "$repo" apply "$patch_file"
else
  if ! git -C "$repo" diff --cached --quiet; then
    printf 'OpenResearch index has staged changes; refusing to build.\n' >&2
    exit 1
  fi
  git -C "$repo" apply --reverse --check "$patch_file" 2>/dev/null || {
    printf 'OpenResearch changes are not the known Postudio patch.\n' >&2
    exit 1
  }
  actual_files="$(git -C "$repo" status --porcelain | sed 's/^...//' | sort)"
  expected_files="$(printf '%s\n' src/commands/remote_host.rs src/commands/up.rs src/commands/up/api_v2.rs src/commands/up/api_v2/tests.rs src/compute.rs src/local/trackio.rs src/main.rs | sort)"
  if [[ "$actual_files" != "$expected_files" ]]; then
    printf 'OpenResearch has changes outside the known Postudio patch.\n' >&2
    exit 1
  fi
fi
if [[ "$(rustc +1.100.0 --version | awk '{print $2}')" != 1.100.0 ]]; then
  printf 'Rust 1.100.0 is required. Install it with rustup toolchain install 1.100.0.\n' >&2
  exit 1
fi
if [[ "$(node --version | sed 's/^v//' | cut -d. -f1)" -lt 22 ]]; then
  printf 'Node.js 22 or newer is required for the upstream UI build.\n' >&2
  exit 1
fi

cd "$repo"
npm exec --yes --package=pnpm@12.16.1 -- pnpm --dir ui install --frozen-lockfile
npm exec --yes --package=pnpm@12.16.1 -- pnpm --dir ui build
rustup target add --toolchain 1.100.0 aarch64-apple-darwin x86_64-apple-darwin
RUSTUP_TOOLCHAIN=1.100.0 npm exec --yes --package=pnpm@12.16.1 -- node scripts/build-host.mjs --target aarch64-apple-darwin --target x86_64-apple-darwin
ORX_EMBED_HOST_DIR="$repo/target/host-binaries" cargo +1.100.0 build --release --locked --bin orx
printf 'Built %s\n' "$repo/target/release/orx"
