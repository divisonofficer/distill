#!/usr/bin/env bash
# Report (default) or delete (--apply) Cargo build caches that AGENTS.md says should not pile up:
#   1. incremental/ dirs under this checkout's target dirs (incremental is off repo-wide)
#   2. target-* dirs in this checkout other than target-node/
#   3. target*/ dirs inside Distill worktrees ($DISTILL_HOME/worktrees/<repo>/<worktree>/)
# Never touched: target/ and target-node/ themselves, so target/release/distill
# (~/.local/bin/distill) and binaries of running leaders/nodes stay.
# A dir is skipped while cargo/rustc builds into it, a process runs from it, or a process has its cwd inside it.
# No pipefail on purpose: `grep -q`/`head` closing a pipe early would SIGPIPE the writer and turn a
# match into a failure (an in-use dir reported free). Membership checks below read here-strings instead.
set -eu

apply=0
case "${1:-}" in
  --apply) apply=1 ;;
  "" ) ;;
  *) echo "usage: $0 [--apply]" >&2; exit 2 ;;
esac

repo="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
distill_home="${DISTILL_HOME:-$HOME/.distill}"

# Newline-separated lists (bash 3.2 on macOS: no mapfile, empty arrays break `set -u`).
commands="$(ps -Aww -o command= 2>/dev/null || true)"
# Absolute paths of every running executable (and its cwd): catches binaries started by relative path,
# e.g. ./target-node/debug/postudio-node, which the command line alone would miss.
open_paths="$(lsof -d txt,cwd -Fn 2>/dev/null | sed -n 's/^n//p' || true)"
busy=""
for pid in $(pgrep -x cargo 2>/dev/null; pgrep -x rustc 2>/dev/null); do
  cwd="$(lsof -a -p "$pid" -d cwd -Fn 2>/dev/null | sed -n 's/^n//p' || true)"
  cwd="${cwd%%$'\n'*}"
  dir="$(ps eww -o command= -p "$pid" 2>/dev/null | tr ' ' '\n' | sed -n 's/^CARGO_TARGET_DIR=//p' || true)"
  dir="${dir%%$'\n'*}"
  if [[ -n "$dir" ]]; then
    [[ "$dir" == /* ]] || dir="$cwd/$dir"
    busy="$busy"$'\n'"$dir"
  elif [[ -n "$cwd" ]]; then
    busy="$busy"$'\n'"$cwd/target"
    [[ "$cwd" == "$repo"* ]] && busy="$busy"$'\n'"$repo/target"
  fi
done

# $1 = target root a cargo build would lock; $2 = dir about to be deleted.
in_use() {
  local root="$1" dir="$2"
  if grep -qxF -- "$root" <<<"$busy"; then return 0; fi
  if grep -qF -- "$dir/" <<<"$commands"; then return 0; fi
  if awk -v d="$dir" '$0 == d || index($0, d "/") == 1 { found = 1 } END { exit !found }' <<<"$open_paths"; then
    return 0
  fi
  return 1
}

total_kb=0
consider() {
  local root="$1" dir="$2" why="$3" kb
  [[ -d "$dir" ]] || return 0
  kb="$(du -sk "$dir" 2>/dev/null | cut -f1)"
  if in_use "$root" "$dir"; then
    printf 'skip  (in use)  %6s MB  %s\n' "$((kb / 1024))" "$dir"
    return 0
  fi
  total_kb=$((total_kb + kb))
  if [[ "$apply" == 1 ]]; then
    rm -rf -- "$dir"
    printf 'removed         %6s MB  %s  [%s]\n' "$((kb / 1024))" "$dir" "$why"
  else
    printf 'would remove    %6s MB  %s  [%s]\n' "$((kb / 1024))" "$dir" "$why"
  fi
}

shopt -s nullglob
for t in "$repo"/target "$repo"/target-*; do
  [[ -d "$t" ]] || continue
  case "$(basename "$t")" in
    target|target-node)
      for inc in "$t"/*/incremental; do consider "$t" "$inc" "incremental cache"; done ;;
    *) consider "$t" "$t" "unsanctioned target dir" ;;
  esac
done
for t in "$distill_home"/worktrees/*/*/target "$distill_home"/worktrees/*/*/target-*; do
  consider "$t" "$t" "worktree build dir"
done

verb="would free"; [[ "$apply" == 1 ]] && verb="freed"
printf '%s %s MB. Disk: %s free.\n' "$verb" "$((total_kb / 1024))" "$(df -h "$repo" | awk 'NR==2 {print $4}')"
[[ "$apply" == 1 ]] || echo "Dry run. Re-run with --apply to delete."
