# OpenResearch Postudio contract

Base: `cb75d884e76f06ff69cb6a29b9dae44f5f03b4a6` (OpenResearch 0.2.27, MIT). Versioned changes are in `patches/openresearch/postudio-contract.patch`.

## Expected commit

Postudio run requests pass `expected_commit` in `ExpRunArgs`. Native selects the experiment branch SHA once, rejects a different expected SHA before archive or run insertion, then archives that selected SHA. API v2 reports mismatch as HTTP 409; callers must request a fresh snapshot and approval. Legacy callers may omit the field.

## Handshake and snapshot routes

`GET /api/v2/hello?nonce=...` keeps `instance`, `proto`, `version`, and authenticated discovery `proof`; response advertises `postudioContract: 1` and capabilities `expectedCommit`, `snapshot`, `workspace`, `privatePorts`.

Authenticated `GET /api/v2/projects/{project}/experiments/{experiment}/snapshot` returns `{headCommit,uncommittedCount,worktreePath,branch}`. `uncommittedCount` is null when no native Postudio worktree exists. `POST .../workspace` explicitly creates an attached worktree on the experiment branch and returns the same fields plus `cwd`; existing dirty worktrees are reused without reset.

## Trackio private port

`ORX_POSTUDIO_TRACKIO_PORT` is optional. When set, it must be a valid nonzero TCP port; Trackio attempts only that loopback port and reports an error if unavailable, never allocating a fallback. The owner-only `server.json` descriptor records this port in `privatePorts` before lazy Trackio startup. Old descriptors deserialize with an empty list. The descriptor lease is exposed to the owner/runtime for port denial across restart.

## Reconnect

`POST /api/v2/runs/{run}/reconnect` resolves the stored native handle. SSH/Slurm handles use the existing authenticated host retry, including its “Connect required” check, followed by a bounded engine observation. Local handles use a bounded engine observation. The returned phase and contact come from the engine, never from an assumed successful reconnect. Terminal runs remain unchanged. SSH interruption/resumption has not been validated on hardware.

## Workspace identity

Snapshot requests reject switched or detached workspaces and a different Git common directory. Blocking snapshot work runs outside the async executor. Worktree creation disables repository hooks and preserves existing dirty files. A timeout for every underlying Git subprocess remains a hardening item.
