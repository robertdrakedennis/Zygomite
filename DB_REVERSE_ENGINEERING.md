# RS3 Cache Database Reverse Engineering

The RS3 cache (build 910) contains an embedded database system in archive 2,
config groups 40 (table schemas) and 41 (row data). This is Jagex's proprietary
SQLite-like storage for item definitions, NPC stats, and other game data.

## Summary

| Metric | Count |
|--------|-------|
| Total tables | 361 |
| Total rows | 18,077 |
| Tables with column schemas | ~180 |
| Tables with only row data (no schema) | ~180 |

## Key Tables

### Table 163 — Item Definitions (5,237 rows, 32 cols)

The primary item definition table. Each row defines a single item.

| Col | Type(s) | Default | Meaning |
|-----|---------|---------|---------|
| 0 | [0] int | — | `itemId` — unique item identifier |
| 1 | [0] int | — | `parentId` — parent item or category |
| 2 | [36] string | — | `name` — display name (e.g. "Ceremonial Djinn robes") |
| 3 | [36] string | — | `description` — examine text |
| 4 | [33] param | — | `paramId` — linked param config entry |
| 5 | [0] int | — | `typeId` — item type/category |
| 6 | [0] int | 99 | `value` — shop/GE price |
| 7 | [0] int | 268435454 | `flags` — bitfield |
| 8 | [0] int | 1 | `stackable` — stackability flag |
| 9 | [36] string | — | `description2` — secondary description |
| 10 | [36] string | — | `description3` — tertiary description |
| 11 | [1] bool | 0 | `membersOnly` — members-only flag |
| 12 | [74] | — | Unknown reference type |
| 13 | [0] int | — | `categoryId` — item category |
| 14 | [0] int | — | Unknown |
| 15 | [0] int | — | Unknown |
| 16 | [39] | — | Unknown reference type |
| 17 | [0] int | — | Unknown |
| 18 | [3] | — | Unknown reference type |
| 19 | [0] int | 2 | Unknown |
| 20 | [73] | — | Unknown reference |
| 21 | [0] int | 0 | Unknown |
| 22 | [0] int | 0 | Unknown |
| 23 | [0] int | — | `modelId` — primary 3D model |
| 24 | [0] int | — | `modelId2` — secondary 3D model |
| 25 | [0] int | 2147483647 | Unknown large value |
| 26 | [209] | — | `color` — RGBA tint value |
| 27 | [0] int | 1 | Unknown |
| 28 | [0] int | 0 | Unknown |
| 29 | [36] string | — | Unknown string |
| 30 | [0,0,0,0,0,0] int[6] | — | `equipmentOverrides` — only 2 items use this. Indices: stab/slash/crush/magic/range/strength bonus |
| 31 | [73] | — | `soundId` — sound effect |

### Table 29 — NPC Stats (105 rows, 46 cols)

NPC combat and behavior definitions.

| Col | Type | Meaning |
|-----|------|---------|
| 0 | int | `npcId` |
| 1-3 | int | Model IDs |
| 5 | string | `name` (e.g. "Kandarin cow") |
| 6 | string | `description` |
| 7 | int | `size` — tile size in game units |
| 9 | int | `combatLevel` |
| 10 | int | `hitpoints` |
| 14 | int | `attack` — attack level |
| 17 | int | `defence` — defence level |
| 18 | int | `accuracy` — combat accuracy |
| 29 | [33,0] param+int | Param reference |
| 39-41 | [73] | Sound/graphic IDs |
| 42 | [26,26] | Attack/defence animation IDs |
| 47 | [33] param | Additional param |

### Table 4 — Item Categories (83 rows, 12 cols)

| Col | Type | Meaning |
|-----|------|---------|
| 0 | int | Category ID |
| 1 | string | Name (e.g. "Junk") |
| 2 | string | Description |
| 4 | [23] | Model ID |
| 5 | [57] | Icon ID |
| 6 | int | Value/weight |
| 7 | int | Flag |

### Table 5 — Item Sets/Outfits (160 rows, 16 cols)

| Col | Type | Meaning |
|-----|------|---------|
| 0 | int | Set ID |
| 1 | string | Name (e.g. "Sentinel outfit piece") |
| 2 | string | Description |
| 5 | [23] | Representative item/model ID |

### Table 7 — Clue Scroll Locations (62 rows, 4 cols)

| Col | Type | Meaning |
|-----|------|---------|
| 0 | int | Location ID |
| 1 | int | Difficulty tier (1-5) |
| 2 | string | Location description (e.g. "Inside the shed in Lumbridge Swamp") |
| 11 | int | Additional flag |

### Table 72 — Ability Definitions (836 rows, 24 cols)

Combat ability definitions.

| Col | Type | Meaning |
|-----|------|---------|
| 3 | string | Description/instructions |
| 6 | [9] large int | Ability ID reference |
| 7 | int | Flag (-1 = N/A) |
| 8 | int | Type indicator |

### Table 84 — Slayer Categories (180 rows, 13 cols)

| Col | Type | Meaning |
|-----|------|---------|
| 0 | int | Category ID |
| 4 | string | Name (e.g. "Rabbits") |
| 13 | [33] param | Linked param |

### Table 85 — Perks/Effects (168 rows, 13 cols)

| Col | Type | Meaning |
|-----|------|---------|
| 3 | string | Description (e.g. "Absorbs 20% of any damage...") |
| 5-6 | [33] param | Linked param references |
| 10 | int | Numeric value (e.g. 20 for 20%) |

### Table 158 — NPC Drop Tables (789 rows, 13 cols)

| Col | Type | Meaning |
|-----|------|---------|
| 0 | string | Drop description |

### Table 285 — Boss Encounters (97 rows, 34 cols)

| Col | Type | Meaning |
|-----|------|---------|
| 0 | int | Encounter ID |
| 2-3 | int | Boss NPC IDs |
| 7 | string | Boss name (e.g. "New Boss: Flesh-hatcher Mhekarnahz") |
| 9 | string | Description |

## Equipment/Weapon Stats

Most weapon stats (stab/slash/crush/magic/range attack/defense, strength,
prayer) are **computed client-side** from item tier and category, not stored
per-item in the database.

Column 30 of table 163 is an "equipment override" field (6 integers) that
only 2 of 5,237 items use:

| Item ID | Name | col 30 values |
|---------|------|---------------|
| 4504 | Moonlight halo | [-3, 185, 0, 0, 0, 70] |
| 4505 | Eclipse halo | [-3, 185, 0, 0, 0, 70] |

These are special cosmetic items with unusual stat combinations that
override the client-side computation.

## Column Type IDs

The `tupleTypes` arrays reference type IDs from the game's type system:

| Type ID | Rust | Meaning |
|---------|------|---------|
| 0 | i32 | 32-bit signed integer |
| 1 | i32 | Boolean (0=false, 1=true) |
| 3 | ? | Unknown scalar |
| 9 | ? | Large composite |
| 22 | ? | 64-bit number |
| 23 | i32 | Model/animation reference |
| 26 | i32 | Animation ID |
| 33 | i32 | Param/config reference (paired with 0) |
| 36 | String | Text string |
| 39 | ? | Unknown reference |
| 57 | i32 | Icon/sprite reference |
| 73 | i32 | Sound/graphic reference |
| 74 | i32 | Unknown large reference |
| 209 | i32 | Color value (RGBA) |

## Data Format

Each `DbRowEntry` belongs to a table (`table` field) and contains one or more
`DbRowColumn` entries. Each column has:
- `column`: column index (u8)
- `tupleTypes`: array of type IDs defining the value format
- `rows`: array of value tuples, each matching the `tupleTypes` signature

The corresponding `DbTableEntry` defines the schema:
- `column`: column index
- `tupleTypes`: expected type signature
- `defaults`: default tuples when a row omits the column
