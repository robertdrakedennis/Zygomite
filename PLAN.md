# CS2 TypeScript Intellisense Plan

## Goal

Build full IDE intellisense for CS2/RuneScape 3 by reverse-engineering the data primitives
that CS2 bytecode references, then generating typed TypeScript exports so every reference
in a transpiled script is cmd+clickable to its definition.

## Architecture

```
RS3 Cache (binary)
  ├── archive 2  (config: items, npcs, objs, locs, seqs, spots, enums, params, structs)
  ├── archive 3  (interfaces: component layouts and dependencies)
  ├── archive 12 (client scripts: CS2 bytecode)
  └── data/      (opcode tables, enum lookups, names/scripts.txt)
        │
        ▼
  ResolverContext (Rust)
  ├── parsed_components: BTreeMap<u32, BTreeMap<u32, ComponentDeps>>
  ├── enums, varps, varbits, params, structs, scripts, configs...
        │
        ▼
  TypeScript Export (ts-export / transpile-scripts)
  ├── vars.ts, varbits.ts, enums.ts, params.ts, structs.ts
  ├── interfaces.ts (ComponentId UIDs + InterfaceId)
  ├── scripts.d.ts (named signatures from scripts.txt + return inference)
  ├── objs/npcs/locs/seqs/spots + named_objs/npcs/locs
  ├── dbtables.ts
  └── index.ts
        │
        ▼
  Transpiled Script (*.ts)
  └── Named exports, enum/DB/component refs where recoverable
```

## Data Primitives CS2 Can Reference

| Primitive | TS Export File | Current State |
|-----------|---------------|---------------|
| Vars (varps) | `vars.ts` | ✅ Named, typed |
| Varbits | `varbits.ts` | ✅ Named, typed |
| Enums | `enums.ts` | ✅ Map + `Enum_N.KEY` in transpile |
| Params | `params.ts` | ✅ ~8060 on 910 (`CONFIG_GROUP_PARAM`) |
| Structs | `structs.ts` | ✅ Named, typed |
| Interfaces | `interfaces.ts` | ✅ `ComponentId` full UIDs + fallback names |
| Scripts | `scripts.d.ts` | ✅ Named signatures (`names/scripts.txt`) |
| Arrays | per-script | ✅ `array_N: number[]` locals in transpile |
| DB tables/rows | `dbtables.ts` | ✅ `DB_TABLES.get(id)` in transpile |
| Objs / NPCs / Locs | `objs.ts` etc. + `named_*.ts` | ✅ ReadonlyMap + `Named*Ids` const aliases |
| Seqs / Spots / Invs | `seqs.ts`, `spots.ts`, `invs.ts` | ✅ Populated from cache |

## Remaining / follow-up (not blocking intellisense)

- Transpile control-flow cleanup (partial `goto`/`while` recovery)
- Param opcode → `PARAMS.get(id)` in structured expr recovery (vars/enums wired)
- Full-archive transpile CI (use `--all-scripts` locally only)
- 910 map decode parity with zwyz (out of intellisense scope)

## Commands

```bash
# Type definitions for IDE
cargo run --release -- --cache-dir $RS3_CACHE_DIR --data-dir $RS3_DATA_DIR \
  --build 910 --subbuild 0 ts-export --out-dir /tmp/ts-910

# Transpile subset
cargo run --release -- --cache-dir $RS3_CACHE_DIR --data-dir $RS3_DATA_DIR \
  --build 910 --subbuild 0 transpile-scripts --out-dir /tmp/transpile-910 \
  --filter-script bank_build --max-scripts 5
```

Integration tests: `tests/ts_export.rs` (910 fixture via `RS3_CACHE_DIR` / `RS3_DATA_DIR`).
