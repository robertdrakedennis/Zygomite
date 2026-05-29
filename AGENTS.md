# AGENTS.md for `/Users/robert/projects/alerion/tools/rs3-cache-rs`

## Documentation

**Start here:** [../../docs/workflows/cs2-cache.md](../../docs/workflows/cs2-cache.md) · **Revision model:** [../../docs/workflows/revision-model.md](../../docs/workflows/revision-model.md) · **Logging:** [../../docs/workflows/logging.md](../../docs/workflows/logging.md)

## Runtime

- Build with `cargo build --release`.
- Main entrypoint is `src/cli.rs`; keep command behavior deterministic and file-oriented.
- Runtime build context comes from `--build`, `--subbuild`, `--cache-dir`, and `--data-dir`; log those fields on command summary events.
- Prefer runtime overlay proof flow from [../../docs/workflows/cs2-cache.md](../../docs/workflows/cs2-cache.md) before trusting donor `947` ids.

## Logging

- Log for query, not prose grep. New operational logs must be structured events with stable field names.
- Unit of work is command invocation or long-running command stage. Emit one canonical summary event per command, plus stage events only for expensive phases.
- Keep `stdout` for declared command output. Keep human progress on `stderr` terse. If command becomes automation surface, add explicit `--json` mode instead of growing ad-hoc text.
- Canonical event must answer: `event`, `command`, `outcome`, `duration_ms`, `build`, `subbuild`, `cache_dir`, `data_dir`, and exact ids such as `script_id`, `interface_group`, `archive`, or `group_id`.
- High-cardinality ids are feature, not bug. Preserve exact script, archive, group, config, map, and build ids.
- Do not emit one line per file or group by default on large scans. Emit counts, durations, sampled examples, and single failure summary unless user asked for trace mode.
- Do not log full cache payloads or giant blobs. Log paths, sizes, hashes, ids, and bounded previews instead.
- Log failure once at boundary that owns command outcome. Inner helpers return typed errors; command boundary emits canonical failure event.

## Git

- Do not cosign commits; use global git config identity.
