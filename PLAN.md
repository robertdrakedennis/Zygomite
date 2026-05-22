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
  └── data/      (opcode tables, enum lookups)
        │
        ▼
  ResolverContext (Rust)
  ├── parsed_components: BTreeMap<u32, BTreeMap<u32, ComponentDeps>>
  ├── enums: BTreeMap<u32, EnumEntry>
  ├── varps_by_domain: HashMap<VarDomain, BTreeMap<u32, VarEntry>>
  ├── varbits: BTreeMap<u32, VarBitEntry>
  ├── params: BTreeMap<u32, ParamEntry>
  ├── structs: BTreeMap<u32, StructEntry>
  ├── scripts: BTreeMap<u32, Vec<u8>>
  └── (items, npcs, objs, locs, seqs, spots) ← NOT YET LOADED
        │
        ▼
  TypeScript Export (ts-export / transpile-scripts)
  ├── vars.ts          ✅ Rich types
  ├── varbits.ts       ✅ Rich types
  ├── enums.ts         ✅ Basic types (needs named values)
  ├── params.ts        ✅ Rich types
  ├── structs.ts       ✅ Rich types
  ├── interfaces.ts    ⚠️  Flat ID array only (needs names + component info)
  ├── scripts.ts       ⚠️  Barrel only (needs function signatures)
  ├── invs.ts          ❌ Stub
  ├── objs.ts          ❌ Stub
  ├── npcs.ts          ❌ Stub
  ├── locs.ts          ❌ Stub
  ├── seqs.ts          ❌ Stub
  ├── spots.ts         ❌ Stub
  └── index.ts         ✅ Re-exports all types
        │
        ▼
  Transpiled Script (script_N.ts)
  ├── Before: UI.create(5)           ← raw number
  └── After:  UI.create(ComponentId.CHAT_BOX)  ← cmd+clickable
```

## Data Primitives CS2 Can Reference

| Primitive | Opcodes | TS Export File | Current State |
|-----------|---------|---------------|---------------|
| Vars (varps) | `push_var`, `pop_var` | `vars.ts` | ✅ Named, typed |
| Varbits | `push_varbit`, `pop_varbit` | `varbits.ts` | ✅ Named, typed |
| Enums | `push_constant_string` (Int) | `enums.ts` | ⚠️ Map only, no named values |
| Params | param ops | `params.ts` | ✅ Named, typed |
| Structs | struct param ops | `structs.ts` | ✅ Named, typed |
| Interfaces | `cc_*`, `if_*` (60+ opcodes) | `interfaces.ts` | ⚠️ Flat ID array |
| Scripts | `gosub_with_params` | `scripts.ts` | ⚠️ Barrel only |
| Arrays | `define_array`, `push_array_int` | (none) | ❌ No export |
| DB tables/rows | `db_*` ops | (none) | ❌ No export |
| Items | `inv_*`, `obj_*` | `invs.ts`, `objs.ts` | ❌ Stubs |
| NPCs | `npc_*` | `npcs.ts` | ❌ Stub |
| Locations | `loc_*` | `locs.ts` | ❌ Stub |
| Animations | `seq_*` | `seqs.ts` | ❌ Stub |
| Spotanims | `spot_*` | `spots.ts` | ❌ Stub |

## Phase Plan

### Phase 1: Named Interface Constants

**Data**: `ctx.parsed_components` already has component names, types, and dependency graphs.

**Changes**:
1. Flatten `parsed_components` into `HashMap<u32, ComponentDeps>` (component_id → info)
2. Generate `interfaces.ts` with:
   - `ComponentId` const object mapping names → IDs
   - `ComponentInfo` interface with type, name, and dependency lists
   - `ALL_COMPONENTS: ReadonlyMap<number, ComponentInfo>` for runtime lookup
3. Update `expr_recovery.rs` to emit named constants:
   - `cc_create(Int(5))` → `UI.create(ComponentId.CHAT_BOX)`
   - `if_gettext(Int(5))` → `UI.getText(ComponentId.CHAT_BOX)`
4. Add `import { ComponentId } from './interfaces'` to script output when needed

**Naming rules**: RS3 interface names use snake_case. We expose them as-is
(`ComponentId.chat_box`) for fidelity to the source data.

### Phase 2: Named Enum Values

**Data**: `ctx.enums` has key→value pairs. Many enums have string values (e.g., skill names).

**Changes**:
1. Generate per-enum namespaces with typed constants
2. Update `expr_recovery` to emit enum member references

### Phase 3: Script Function Signatures

**Data**: `CompiledScript` has argument counts/types. Return type from body analysis.

**Changes**:
1. Generate `scripts.d.ts` with `export function script_N(...): ReturnType;`
2. Use `gosub_with_params` targets to emit typed cross-script calls

### Phase 4: Config Type Completion

**Data**: Items, NPCs, objects, locations, sequences, spotanims from archive 2.

**Changes**:
1. Load config archives into `ResolverContext`
2. Generate typed exports for each config type
3. Link config IDs to CS2 opcodes that reference them
