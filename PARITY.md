# rs3-cache-rs Parity Audit

Run audit:

```sh
python3 tools/parity_audit.py --java-root ../rs3-cache --rust-root .
```

Current result (2026-05-20):

- `full_parity: True`
- Java config unpackers: `59`
- Rust config features: `59`
- Missing config features: `0`
- Missing top-level unpack targets: `0`
- Missing defaults targets: `0` (`graphics/audio/wearpos/worldmap/title`)

Parity goal:

1. Keep `full_parity: True` in CI.
2. Keep config unpacker coverage at `59/59`.
3. Keep top-level unpack target coverage at `0 missing`.
