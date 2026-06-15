use crate::animator::decode as decode_animator_controller;
use crate::audio::{AudioKind, inspect_audio_file};
use crate::cache::FlatCache;
use crate::config::{
    parse_achievement, parse_area, parse_bas, parse_billboard, parse_bugtemplate, parse_category,
    parse_controller, parse_cursor, parse_dbrow, parse_dbtable, parse_enum, parse_gamelogevent,
    parse_headbar, parse_hitmark, parse_hunt, parse_idk, parse_inv, parse_itemcode, parse_light,
    parse_loc, parse_material, parse_mel, parse_mesanim, parse_msi, parse_npc, parse_obj,
    parse_overlay, parse_param, parse_particle_effector, parse_particle_emitter, parse_quest,
    parse_quickchatcat, parse_quickchatphrase, parse_seq, parse_seqgroup, parse_skybox, parse_spot,
    parse_struct, parse_stylesheet, parse_texture, parse_underlay, parse_var_client_string,
    parse_var_npc_bit, parse_var_shared, parse_var_shared_string, parse_water, parse_worldarea,
};
use crate::constants::{
    ARCHIVE_ACHIEVEMENTS, ARCHIVE_ANIMATOR, ARCHIVE_BILLBOARDS, ARCHIVE_BINARY,
    ARCHIVE_CHUNK_INSTANCES, ARCHIVE_CLIENTSCRIPTS, ARCHIVE_CONFIG, ARCHIVE_CUTSCENE2D,
    ARCHIVE_DEFAULTS, ARCHIVE_ENUM_CONFIG, ARCHIVE_FONTMETRICS, ARCHIVE_INTERFACES,
    ARCHIVE_LOC_CONFIG, ARCHIVE_MAPSQUARES, ARCHIVE_MATERIALS, ARCHIVE_MODELS_RT7,
    ARCHIVE_NPC_CONFIG, ARCHIVE_OBJ_CONFIG, ARCHIVE_PARTICLES, ARCHIVE_QUICKCHAT_CONFIG,
    ARCHIVE_SEQ_CONFIG, ARCHIVE_SPOT_CONFIG, ARCHIVE_STRUCT_CONFIG, ARCHIVE_STYLESHEETS,
    ARCHIVE_TEXTURES, ARCHIVE_TTF, ARCHIVE_UI_ANIM, ARCHIVE_VFX, ARCHIVE_WORLDMAP, AUDIO_ARCHIVES,
    BUILD, CONFIG_GROUP_ACHIEVEMENT_ARCHIVE57, CONFIG_GROUP_AREA, CONFIG_GROUP_BAS,
    CONFIG_GROUP_BILLBOARD_ARCHIVE29, CONFIG_GROUP_BUGTEMPLATE, CONFIG_GROUP_CATEGORY,
    CONFIG_GROUP_CONTROLLER, CONFIG_GROUP_CURSOR, CONFIG_GROUP_DBROW, CONFIG_GROUP_DBTABLE,
    CONFIG_GROUP_GAMELOGEVENT, CONFIG_GROUP_HEADBAR, CONFIG_GROUP_HITMARK, CONFIG_GROUP_HUNT,
    CONFIG_GROUP_IDK, CONFIG_GROUP_INV, CONFIG_GROUP_ITEMCODE, CONFIG_GROUP_LIGHT,
    CONFIG_GROUP_LOC_LEGACY, CONFIG_GROUP_MATERIAL_ARCHIVE26, CONFIG_GROUP_MEL,
    CONFIG_GROUP_MESANIM, CONFIG_GROUP_MSI, CONFIG_GROUP_NPC_LEGACY, CONFIG_GROUP_OBJ_LEGACY,
    CONFIG_GROUP_OVERLAY, CONFIG_GROUP_PARAM, CONFIG_GROUP_PARTICLE_EFFECTOR_ARCHIVE27,
    CONFIG_GROUP_PARTICLE_EMITTER_ARCHIVE27, CONFIG_GROUP_QUEST,
    CONFIG_GROUP_QUICKCHATCAT_ARCHIVE24, CONFIG_GROUP_QUICKCHATPHRASE_ARCHIVE24, CONFIG_GROUP_SEQ,
    CONFIG_GROUP_SEQGROUP, CONFIG_GROUP_SKYBOX, CONFIG_GROUP_SPOT, CONFIG_GROUP_STRUCT,
    CONFIG_GROUP_UNDERLAY, CONFIG_GROUP_VAR_BIT, CONFIG_GROUP_VAR_CLAN,
    CONFIG_GROUP_VAR_CLAN_SETTING, CONFIG_GROUP_VAR_CLIENT, CONFIG_GROUP_VAR_CLIENT_STRING,
    CONFIG_GROUP_VAR_CONTROLLER, CONFIG_GROUP_VAR_GLOBAL, CONFIG_GROUP_VAR_NPC,
    CONFIG_GROUP_VAR_NPC_BIT, CONFIG_GROUP_VAR_OBJECT, CONFIG_GROUP_VAR_PLAYER,
    CONFIG_GROUP_VAR_PLAYER_GROUP, CONFIG_GROUP_VAR_REGION, CONFIG_GROUP_VAR_SHARED,
    CONFIG_GROUP_VAR_SHARED_STRING, CONFIG_GROUP_VAR_WORLD, CONFIG_GROUP_WATER,
    CONFIG_GROUP_WORLDAREA, DEFAULTS_GROUP_AUDIO, DEFAULTS_GROUP_GRAPHICS, DEFAULTS_GROUP_TITLE,
    DEFAULTS_GROUP_WEARPOS, DEFAULTS_GROUP_WORLDMAP, SUBBUILD,
};
use crate::cutscene2d::decode as decode_cutscene2d;
use crate::dep_tree::{EntityRef, EntityType, ResolverContext, build_tree};
use crate::fixture::{default_tar_path, ensure_archive_complete, open_cache};
use crate::interface::render_interface_group;
use crate::map::{decode_chunk_instance_stream, decode_map_square, decode_map_square_best_effort};
use crate::model::Model;
use crate::script::{
    CompiledScript, Instruction, MIN_SCRIPT_BUILD, OpcodeBook, Operand, VarBitRef, VarRef,
    decode_script, encode_script, parse_cs2_asm,
};
use crate::transpile::{
    REVERSIBLE_FORMAT_VERSION, ReverseCompileContext, ScriptCatalog, ScriptCatalogBuilder,
    Transpiler, enum_pair_property_name, is_reversible_source, lower_structured_script,
    parse_reversible_source, parse_structured_typescript, render_reversible_source,
    structured_digest,
};
use crate::vars::{VarDomain, parse_var, parse_varbit};
use crate::vfx::decode as decode_vfx;
use anyhow::{Context, Result, bail, ensure};
use clap::{Parser, Subcommand, ValueEnum};
use image::{ImageBuffer, Rgb};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write;
use std::fs;
use std::io::{BufWriter, Write as IoWrite};
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Parser, Debug)]
#[command(name = "rs3-cache-rs")]
#[command(about = "Rust CLI for RS3 cache extraction and parsing")]
pub struct Cli {
    #[arg(long)]
    pub cache_dir: Option<PathBuf>,
    #[arg(long)]
    pub cache_tar: Option<PathBuf>,
    #[arg(long, default_value = "data")]
    pub data_dir: PathBuf,
    #[arg(long, default_value_t = BUILD)]
    pub build: u32,
    #[arg(long, default_value_t = SUBBUILD)]
    pub subbuild: u32,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    Interfaces {
        #[arg(long)]
        out_dir: Option<PathBuf>,
    },
    /// Decode / preview / rasterize / diff 910 bitmap fonts and 948 modern (TTF)
    /// fonts. Reads the runtime `.js5` pack directly (no flat cache needed).
    Font {
        #[command(subcommand)]
        command: crate::font::cli::FontCommand,
    },
    /// Explain an interface: a per-component table (`index/type/textfont/colour/
    /// ops/bounds/text`) plus its upward dependency closure (`requires:` fonts,
    /// sprites, scripts, enums, …, child interfaces). Reads the runtime
    /// interfaces `.js5` pack (no flat cache needed), or a raw group `.dat`.
    ///
    /// Example:
    /// ```bash
    /// cd tools/rs3-cache-rs
    /// cargo run --release -- explain-interface 691
    /// ```
    #[command(name = "explain-interface")]
    ExplainInterface {
        /// Interface group id.
        id: u32,
        /// Emit the explanation as JSON instead of the human table.
        #[arg(long)]
        json: bool,
        /// Runtime pack root holding `client.interfaces.js5`. READ-ONLY. When
        /// the interface is absent here (a donor-only id), falls back to the
        /// donor pack (`cache/rs3-cache/948-all/pack`) automatically.
        #[arg(long, default_value = crate::explain::DEFAULT_PACK_ROOT)]
        pack_root: PathBuf,
        /// Decode a raw interface-group `.dat` (JS5 raw group) instead of the
        /// runtime pack. The interface id is still taken from `id`.
        #[arg(long)]
        raw_dat: Option<PathBuf>,
        /// Build number to decode interface components at.
        #[arg(long, default_value_t = BUILD)]
        decode_build: u32,
        /// Also report the FULL transitive clientscript closure of the
        /// interface's component-bound scripts and the count MISSING from the 910
        /// base (the splice burden). Walks the donor (948) script call graph from
        /// the flat cache at `--scripts-cache` (decoded at `--scripts-build`) and
        /// compares against the 910-base roster at `--base-pack-root`.
        #[arg(long)]
        transitive: bool,
        /// Flat cache dir holding the donor clientscripts (archive 12) to walk for
        /// `--transitive`. Defaults to the global `--cache-dir`.
        #[arg(long)]
        scripts_cache: Option<PathBuf>,
        /// Build to decode the donor scripts at for `--transitive`. Defaults to
        /// the global `--build`.
        #[arg(long)]
        scripts_build: Option<u32>,
        /// 910-base pack root holding the pristine `client.scripts.js5` roster used
        /// to compute the splice burden for `--transitive`. READ-ONLY.
        #[arg(long, default_value = crate::explain::DEFAULT_BASE_PACK_ROOT)]
        base_pack_root: PathBuf,
    },
    /// Interface-group tooling. `interface transcode` downcodes a donor (948)
    /// interface group's components to the 910 client wire format so a
    /// newer-than-691 donor interface (e.g. the ritual selection UI 1224) mounts
    /// on the 910 client instead of crashing.
    Interface {
        #[command(subcommand)]
        command: InterfaceCommand,
    },
    /// Explain a loc: resolve its multivar surface (gating varbit/varp → per-value
    /// child loc → ops), list each option, and reverse-match the candidate
    /// interfaces each option opens (ranked by op/name text overlap plus reads of
    /// the gating varbit's feature varp block). The loc→interface open is
    /// server-side, so candidates are suggestions, summarised with their
    /// `explain-interface` closure. Accepts a multivar parent or a child loc id.
    /// Runs against the flat cache (`--cache-dir` / `--build`) since the gated
    /// interface's onload/refresh scripts live in the donor cache.
    ///
    /// Example:
    /// ```bash
    /// cd tools/rs3-cache-rs
    /// cargo run --release -- --cache-dir ../../cache/unpacked/948 --build 948 \
    ///   --subbuild 1 explain-loc 115416
    /// ```
    #[command(name = "explain-loc")]
    ExplainLoc {
        /// Loc id (a multivar parent or one of its child locs).
        id: u32,
        /// Maximum number of candidate interfaces to report.
        #[arg(long, default_value_t = 10)]
        max_candidates: usize,
        /// Emit the explanation as JSON instead of the human report.
        #[arg(long)]
        json: bool,
    },
    /// Pretty-print any group in any known cache format, reading the runtime
    /// `.js5` packs (replaces ad-hoc byte-probes). `--format auto` infers from
    /// the archive id.
    ///
    /// Example:
    /// ```bash
    /// cd tools/rs3-cache-rs
    /// cargo run --release -- decode --archive 13 --group 26
    /// ```
    Decode {
        /// Cache archive id (used for `auto` inference + reporting).
        #[arg(long)]
        archive: u32,
        /// Group id within the archive. For config formats packed by config
        /// group (dbtable/dbrow/param) this is overridden by the canonical
        /// config group, so any value works there.
        #[arg(long)]
        group: u32,
        /// Output format: auto|sprite|fontmetrics|fontmetrics2|ttf|interface|
        /// dbtable|dbrow|enum|struct|param|npc|obj. `auto` infers from the
        /// 948 flat-cache archive id (e.g. 18→npc, 19→obj, 40→dbtable).
        #[arg(long, default_value = "auto")]
        format: String,
        /// Runtime pack root holding the `client.*.js5` files. READ-ONLY. When
        /// the group is absent here, falls back to the donor pack
        /// (`cache/rs3-cache/948-all/pack`) automatically.
        #[arg(long, default_value = crate::decode::DEFAULT_PACK_ROOT)]
        pack_root: PathBuf,
        /// Emit the dump as JSON instead of a human summary.
        #[arg(long)]
        json: bool,
    },
    Varps {
        #[arg(long)]
        out_file: Option<PathBuf>,
        #[arg(long, default_value = "all")]
        domain: VarDomainArg,
    },
    Varbits {
        #[arg(long)]
        out_file: Option<PathBuf>,
    },
    Configs {
        #[arg(long)]
        out_dir: Option<PathBuf>,
    },
    /// Dump every clientscript (legacy: `cs2 --out-dir DIR --out-file FILE`), or
    /// run a CS2 build-time subcommand (`cs2 lint-splice …`).
    Cs2 {
        /// Optional CS2 subcommand. When omitted, runs the legacy dump using the
        /// `--out-file` / `--out-dir` flags below.
        #[command(subcommand)]
        command: Option<Cs2Command>,
        #[arg(long)]
        out_file: Option<PathBuf>,
        #[arg(long)]
        out_dir: Option<PathBuf>,
    },
    /// Config-format tooling. `config transcode` re-encodes a donor config group
    /// from its wire format to the base client's (DBTABLETYPE + DbTableIndex).
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// The semantic port layer's representability dry-run. `port plan --interface N
    /// --from 948 --to 910` enumerates the full 948→910 delta the port will hit —
    /// proc collisions, missing opcode families (cc_list/cc_radiogroup/dropdown),
    /// arity drift, the stylesheet-colour stub, and the modern-font gap — each with
    /// the named lowering that handles it or `Unhandled`.
    Port {
        #[command(subcommand)]
        command: PortCommand,
    },
    Models {
        #[arg(long)]
        out_file: Option<PathBuf>,
        #[arg(long)]
        out_dir: Option<PathBuf>,
        #[arg(long)]
        sample_only: bool,
    },
    Audio {
        #[arg(long)]
        out_dir: Option<PathBuf>,
        #[arg(long)]
        max_files: Option<usize>,
    },
    Unpack {
        #[arg(long)]
        out_dir: PathBuf,
        #[arg(long)]
        sample_models: bool,
        #[arg(long)]
        skip_audio: bool,
        /// Continue maps export when individual map square decodes fail.
        #[arg(long)]
        best_effort_maps: bool,
        #[arg(long)]
        max_audio_files: Option<usize>,
    },
    DepTreeInterface {
        #[arg(long)]
        id: u32,
        #[arg(long, default_value_t = 50)]
        max_depth: u32,
        #[arg(long)]
        out_file: PathBuf,
    },
    DepTreeScript {
        #[arg(long)]
        id: u32,
        #[arg(long, default_value_t = 50)]
        max_depth: u32,
        #[arg(long)]
        out_file: PathBuf,
    },
    DepTreeVarp {
        #[arg(long)]
        id: u32,
        #[arg(long)]
        domain: VarDomainArg,
        #[arg(long, default_value_t = 50)]
        max_depth: u32,
        #[arg(long)]
        out_file: PathBuf,
    },
    DepTreeVarbit {
        #[arg(long)]
        id: u32,
        #[arg(long, default_value_t = 50)]
        max_depth: u32,
        #[arg(long)]
        out_file: PathBuf,
    },
    DepTreeConfig {
        #[arg(long)]
        kind: ConfigKindArg,
        #[arg(long)]
        id: u32,
        #[arg(long, default_value_t = 50)]
        max_depth: u32,
        #[arg(long)]
        out_file: PathBuf,
    },
    TsExport {
        #[arg(long)]
        out_dir: PathBuf,
    },
    TranspileScripts {
        #[arg(long)]
        out_dir: PathBuf,
        #[arg(long)]
        filter_script: Option<String>,
        /// Generated script style. `high-ts` tries cleaner control flow first and falls back after byte gate.
        #[arg(long, value_enum, default_value_t = TranspileOutputStyle::HighTs)]
        output_style: TranspileOutputStyle,
        #[arg(long, default_value_t = 100)]
        max_scripts: usize,
        /// Transpile every script in the cache (ignores max-scripts cap).
        #[arg(long)]
        all_scripts: bool,
        /// Guard compiled script byte size during transpile (`0` disables limit).
        #[arg(long, default_value_t = crate::transpile::DEFAULT_MAX_TRANSPILE_SCRIPT_BYTES)]
        max_script_bytes: usize,
        /// Guard decoded instruction count during transpile (`0` disables limit).
        #[arg(long, default_value_t = crate::transpile::DEFAULT_MAX_TRANSPILE_INSTRUCTIONS)]
        max_script_instructions: usize,
        /// Guard generated TypeScript size during transpile (`0` disables limit).
        #[arg(long, default_value_t = crate::transpile::DEFAULT_MAX_TRANSPILE_GENERATED_BYTES)]
        max_generated_bytes: usize,
    },
    MigrateCheck {
        #[arg(long)]
        interface_group: u32,
        #[arg(long)]
        out_file: PathBuf,
        /// Optional directory for split audit artifacts.
        #[arg(long)]
        audit_dir: Option<PathBuf>,
        #[arg(long)]
        source_cache_tar: Option<PathBuf>,
        #[arg(long, default_value_t = 947)]
        source_build: u32,
        #[arg(long, default_value_t = 1)]
        source_subbuild: u32,
        /// Enable ID remap planning for conflicted entities.
        #[arg(long)]
        remap: bool,
        /// Buffer above target's max ID for allocating free IDs (default 10000).
        #[arg(long, default_value_t = 10000)]
        remap_buffer: u32,
        /// Validate migrated dependency bundle against target build by rewriting and encoding scripts.
        #[arg(long)]
        validate_target: bool,
        /// Allow heuristic dependency sites during target validation instead of blocking them.
        #[arg(long)]
        allow_heuristic_sites: bool,
    },
    MigrateScript {
        #[arg(long)]
        script_id: u32,
        #[arg(long)]
        out_file: PathBuf,
        /// Optional directory for split audit artifacts.
        #[arg(long)]
        audit_dir: Option<PathBuf>,
        #[arg(long)]
        source_cache_tar: Option<PathBuf>,
        #[arg(long, default_value_t = 947)]
        source_build: u32,
        #[arg(long, default_value_t = 1)]
        source_subbuild: u32,
        #[arg(long)]
        remap: bool,
        #[arg(long, default_value_t = 10000)]
        remap_buffer: u32,
        /// Validate migrated dependency bundle against target build by rewriting and encoding scripts.
        #[arg(long)]
        validate_target: bool,
        /// Allow heuristic dependency sites during target validation instead of blocking them.
        #[arg(long)]
        allow_heuristic_sites: bool,
    },
    /// Validate a CS2 script's bytecode against the target build.
    ValidateScript {
        #[arg(long)]
        script_id: u32,
        #[arg(long)]
        out_file: Option<PathBuf>,
        /// Emit the validation report as JSON to stdout
        #[arg(long)]
        json: bool,
    },
    /// Assemble reversible or pragma-annotated CS2 TypeScript back to CS2 binary.
    #[command(name = "assemble-script")]
    AssembleScript {
        /// Path to reversible .ts or pragma ASM file
        #[arg(long)]
        input: PathBuf,
        /// Path to write the compiled .cs2 binary
        #[arg(long)]
        output: PathBuf,
        /// Cache build version (default: source metadata or CLI build)
        #[arg(long)]
        build: Option<u32>,
        /// Cache sub-build version (default: source metadata or CLI subbuild)
        #[arg(long)]
        subbuild: Option<u32>,
        /// Disable embedded ASM fallback and require structured recompilation
        #[arg(long)]
        strict_structured: bool,
        /// Skip post-compile verification (byte round-trip + stack validation)
        #[arg(long)]
        no_verify: bool,
        /// Emit a structured JSON completion event to stdout
        #[arg(long)]
        json: bool,
    },
    /// Assemble multiple pragma ASM scripts in one process.
    #[command(name = "assemble-script-batch")]
    AssembleScriptBatch {
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long)]
        out_dir: PathBuf,
    },
    /// Dump a lossless raw-flat cache tree for JS5 repacking.
    #[command(name = "dump-raw-flat")]
    DumpRawFlat {
        /// Output directory for the raw flat cache
        #[arg(long)]
        out_dir: PathBuf,
        /// Comma-separated archive IDs to dump (default: all)
        #[arg(long)]
        archives: Option<String>,
    },
    /// Dump config dependency references for the cache overlay workflow.
    #[command(name = "dump-refs")]
    DumpRefs {
        /// Output directory (writes refs/{obj,npc,loc,...}.json)
        #[arg(long)]
        out_dir: PathBuf,
    },
    /// Extract the canonical CS2 command registry from the client sources and data files.
    ///
    /// Reads `ScriptRunner.executeCommand` plus `ClientScriptCommand` and the
    /// `opcodes-*`/`stack-effects`/`opcode-aliases` data files, writes a
    /// name-keyed registry JSON and a discrepancy report. Reads only; never
    /// touches the cache or any existing input file.
    ///
    /// Example:
    /// ```bash
    /// cd tools/rs3-cache-rs
    /// cargo run --release -- --data-dir data extract-cs2-registry
    /// ```
    #[command(name = "extract-cs2-registry")]
    ExtractCs2Registry {
        /// Root of the client checkout (holds `client/src/main/java/...`).
        #[arg(long, default_value = "../../client")]
        client_root: PathBuf,
        /// Registry output path (default: `<data-dir>/cs2/registry-910.json`).
        #[arg(long)]
        out_file: Option<PathBuf>,
        /// Report output path (default: `<out-file dir>/registry-910.report.json`).
        #[arg(long)]
        report_file: Option<PathBuf>,
    },
    /// Generate the mechanical CS2 Java tables from the extracted registry.
    ///
    /// Emits `ClientScriptCommand.java`, `Cs2Dispatch.java`, and
    /// `data/cs2/categories-910.json`. With `--check`, writes nothing and exits
    /// with code 3 if any generated output differs from what is on disk.
    ///
    /// Example:
    /// ```bash
    /// cd tools/rs3-cache-rs
    /// cargo run --release -- generate-cs2-java --check
    /// ```
    #[command(name = "generate-cs2-java")]
    GenerateCs2Java {
        /// Registry JSON path (default: `<data-dir>/cs2/registry-910.json`).
        #[arg(long)]
        registry: Option<PathBuf>,
        /// Root of the client checkout (holds `client/src/main/java/...`).
        #[arg(long, default_value = "../../client")]
        client_root: PathBuf,
        /// Java source root (default: `<client-root>/client/src/main/java`).
        #[arg(long)]
        out_dir: Option<PathBuf>,
        /// Compare-only mode: write nothing; exit 3 on any difference.
        #[arg(long)]
        check: bool,
    },
    /// Regenerate the build-910 opcode txt files as views of the registry.
    ///
    /// Emits `opcodes-910.txt`, `opcodes-large-910.txt`, and
    /// `opcode-aliases-910.txt` from `cs2/registry-910.json`. `opcodes-948.txt`
    /// (and every other build) is left untouched — it carries donor-only opcodes
    /// the 910-anchored registry cannot represent. With `--check`, writes nothing
    /// and exits with code 3 if any generated view differs from disk.
    ///
    /// Example:
    /// ```bash
    /// cd tools/rs3-cache-rs
    /// cargo run --release -- --data-dir data generate-cs2-data --check
    /// ```
    #[command(name = "generate-cs2-data")]
    GenerateCs2Data {
        /// Registry JSON path (default: `<data-dir>/cs2/registry-910.json`).
        #[arg(long)]
        registry: Option<PathBuf>,
        /// Output directory for the views (default: `<data-dir>`).
        #[arg(long)]
        out_dir: Option<PathBuf>,
        /// Compare-only mode: write nothing; exit 3 on any difference.
        #[arg(long)]
        check: bool,
    },
    /// Gate every CS2 opcode used by the runtime pack against the registry.
    ///
    /// Reads the runtime pack's single-file `client.scripts.js5`, decodes every
    /// clientscript group with the 910 opcode book, and verifies each used
    /// opcode maps to a registry command with a real dispatch handler. Writes a
    /// coverage report and exits 4 when error-severity findings exist.
    ///
    /// Example:
    /// ```bash
    /// cd tools/rs3-cache-rs
    /// cargo run --release -- --data-dir data cs2-coverage
    /// ```
    #[command(name = "cs2-coverage")]
    Cs2Coverage {
        /// Runtime pack root (holds `client.scripts.js5`).
        #[arg(long, default_value = "../../server/data/pack-910-base-948-overlay")]
        pack_root: PathBuf,
        /// Override for the clientscript pack file (default: `<pack-root>/client.scripts.js5`).
        #[arg(long)]
        pack_file: Option<PathBuf>,
        /// Registry JSON path (default: `<data-dir>/cs2/registry-910.json`).
        #[arg(long)]
        registry: Option<PathBuf>,
        /// Report output path (default: `<data-dir>/cs2/coverage-910.report.json`).
        #[arg(long)]
        out_file: Option<PathBuf>,
    },
    /// Generate pack-validated named id constants (TS modules + manifest) for
    /// the server from `data/names/910/*.json` joined with the runtime 948 pack.
    /// Fails (exit 2) listing every named id missing or structurally wrong in the
    /// pack; `--check` compares against disk and exits 3 on drift.
    ///
    /// Example:
    /// ```bash
    /// cd tools/rs3-cache-rs
    /// cargo run --release -- generate-ts-ids
    /// ```
    #[command(name = "generate-ts-ids")]
    GenerateTsIds {
        /// Runtime pack root (holds the `client.*.js5` files). READ-ONLY.
        #[arg(long, default_value = "../../server/data/pack-910-base-948-overlay")]
        pack_root: PathBuf,
        /// Curated name maps directory (default: `<data-dir>/names/910`).
        #[arg(long)]
        names_dir: Option<PathBuf>,
        /// Output directory (default: `../../server/src/generated/cache`).
        #[arg(long, default_value = "../../server/src/generated/cache")]
        out_dir: PathBuf,
        /// Compare-only: write nothing; exit 3 on any difference.
        #[arg(long)]
        check: bool,
    },
    /// Extract the canonical game-protocol schema from the client and diff the server.
    ///
    /// Parses the client's `ServerProt.java` / `ClientProt.java` / `LoginProt.java`
    /// (the source of truth) into schema JSON, cross-diffs the server's three TS
    /// mirrors (checks P1–P6), and writes a findings report plus a checked-in
    /// divergence baseline. Read-only over both source trees.
    ///
    /// Example:
    /// ```bash
    /// cd tools/rs3-cache-rs
    /// cargo run --release -- --data-dir data extract-protocol
    /// ```
    #[command(name = "extract-protocol")]
    ExtractProtocol {
        /// Root of the client checkout (holds `client/src/main/java/...`).
        #[arg(long, default_value = "../../client")]
        client_root: PathBuf,
        /// Root of the server checkout (holds `src/jagex/network/protocol/...`).
        #[arg(long, default_value = "../../server")]
        server_root: PathBuf,
        /// Output directory (default: `<data-dir>/protocol/910`).
        #[arg(long)]
        out_dir: Option<PathBuf>,
    },
    /// Generate the protocol parity-gate artifacts from the extracted schema.
    ///
    /// Emits `server/src/generated/protocol/protocol910.ts` and
    /// `client/client/src/test/resources/protocol-910.tsv` from the schema +
    /// divergence baseline. With `--check`, writes nothing and exits 3 if any
    /// generated artifact differs from disk.
    ///
    /// Example:
    /// ```bash
    /// cd tools/rs3-cache-rs
    /// cargo run --release -- --data-dir data generate-protocol --check
    /// ```
    #[command(name = "generate-protocol")]
    GenerateProtocol {
        /// Schema directory (default: `<data-dir>/protocol/910`).
        #[arg(long)]
        schema_dir: Option<PathBuf>,
        /// Root of the server checkout.
        #[arg(long, default_value = "../../server")]
        server_root: PathBuf,
        /// Root of the client checkout.
        #[arg(long, default_value = "../../client")]
        client_root: PathBuf,
        /// Compare-only mode: write nothing; exit 3 on any difference.
        #[arg(long)]
        check: bool,
    },
    /// Dump config text files (config/dump.{type}) for `CacheOverlay` compatibility.
    #[command(name = "dump-configs")]
    DumpConfigs {
        /// Output directory (writes config/dump.{obj,npc,loc,...})
        #[arg(long)]
        out_dir: PathBuf,
    },
    /// Prepare semantic tree for `CacheOverlay` (raw-flat + refs + manifest).
    #[command(name = "prepare-overlay")]
    PrepareOverlay {
        /// Semantic root (e.g. cache/rs3-cache/947-all)
        #[arg(long)]
        out_dir: PathBuf,
        /// Comma-separated archive IDs for raw-flat (default: all)
        #[arg(long)]
        archives: Option<String>,
    },
    /// Verify all mapsquare groups decode from the raw-flat map archive.
    #[command(name = "verify-map-archive")]
    VerifyMapArchive,
    /// Build the RS clip-flag collision grid for one map square (NXT model).
    #[command(name = "build-collision")]
    BuildCollision {
        /// Map-square X coordinate (region X, 0..127).
        #[arg(long = "map-x")]
        map_x: u32,
        /// Map-square Z coordinate (region Z, 0..255).
        #[arg(long = "map-z")]
        map_z: u32,
        /// Write per-level flag grids as JSON to this file (default: summary to stdout).
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Build canonical overlay plan JSON for Bun cacheoverlay wrapper.
    #[command(name = "overlay-plan")]
    OverlayPlan {
        /// Existing Bun cacheoverlay manifest JSON path
        #[arg(long)]
        manifest: PathBuf,
        /// Optional output file; prints plan JSON to stdout when omitted
        #[arg(long)]
        out_file: Option<PathBuf>,
        /// Optional directory for split proof audit artifacts
        #[arg(long)]
        audit_dir: Option<PathBuf>,
        /// Allow heuristic proof gaps without blocking plan
        #[arg(long)]
        allow_heuristic_sites: bool,
        /// Target base build for proof metadata
        #[arg(long, default_value_t = 910)]
        base_build: u32,
        /// Target donor build for proof metadata
        #[arg(long, default_value_t = 947)]
        donor_build: u32,
        /// Target base subbuild for proof metadata
        #[arg(long, default_value_t = 0)]
        base_subbuild: u32,
        /// Target donor subbuild for proof metadata
        #[arg(long, default_value_t = 1)]
        donor_subbuild: u32,
    },
}

/// CS2 build-time subcommands (`cs2 <sub>`).
#[derive(Subcommand, Debug)]
pub enum Cs2Command {
    /// Diff spliced donor CS2 listings against a target opcode book and flag (or
    /// `--fix`) the known port rewrites (`sub`→negate+`add`, `enum`→`_enum`,
    /// db-field `>>4`, `db_find` arity, signature-drift stubs).
    ///
    /// Example:
    /// ```bash
    /// cd tools/rs3-cache-rs
    /// cargo run --release -- cs2 lint-splice \
    ///   --scripts ../../server/cache-patches/relic-system-948/scripts --target-book 910
    /// ```
    #[command(name = "lint-splice")]
    LintSplice {
        /// Directory of `*.asm.ts` listings to lint.
        #[arg(long)]
        scripts: PathBuf,
        /// Target opcode book build (the base client's; 910).
        #[arg(long, default_value_t = crate::cs2::lint::TARGET_BUILD)]
        target_book: u32,
        /// Donor opcode book build the listings were lifted from (948).
        #[arg(long, default_value_t = crate::cs2::lint::DONOR_BUILD)]
        donor_book: u32,
        /// Apply the table-driven rewrites in place. WRITES the listing files.
        #[arg(long)]
        fix: bool,
        /// Emit the report as JSON instead of the human table.
        #[arg(long)]
        json: bool,
    },
    /// Semantic 948→910 CS2 port (the port layer, plan `semantic-port-layer.md`):
    /// decode → typed IR → represent → lower(named passes) → encode(validating).
    /// Today the `--closure-of-interface 1224` driver reproduces the committed
    /// ritual `.asm.ts` byte-for-byte. The global `--cache-dir`/`--build` point at
    /// the DONOR (948) flat cache (the source side).
    ///
    /// Example:
    /// ```bash
    /// cd tools/rs3-cache-rs
    /// cargo run --release -- --cache-dir ../../cache/unpacked/948 --data-dir data \
    ///   --build 948 --subbuild 1 cs2 port --from 948 --to 910 \
    ///   --closure-of-interface 1224 --out-dir /tmp/ritual-port
    /// ```
    Port {
        /// Donor build (948).
        #[arg(long, default_value_t = 948)]
        from: u32,
        /// Target build (910).
        #[arg(long, default_value_t = 910)]
        to: u32,
        /// Re-port the CS2 closure of this interface. Supported drivers: 1224
        /// (ritual selection), 691 (relic powers), 660 (material storage), 1092
        /// (lodestone).
        #[arg(long = "closure-of-interface")]
        closure_of_interface: u32,
        /// 910-base flat cache dir, for ports that AUGMENT a base script (660's
        /// 9239 grid-slot builder). Defaults to `../../cache/unpacked/910`.
        #[arg(long)]
        base_cache_dir: Option<PathBuf>,
        /// Output dir for the `.asm.ts` listings. When omitted, only the byte-exact
        /// diff vs the committed oracle is reported (no files written).
        #[arg(long)]
        out_dir: Option<PathBuf>,
        /// Compare the produced listings against the committed oracle and report
        /// the diff (default true; the regression gate).
        #[arg(long, default_value_t = true)]
        check_oracle: bool,
        /// Emit a JSON summary instead of the human report.
        #[arg(long)]
        json: bool,
    },
}

/// Config-format subcommands (`config <sub>`).
#[derive(Subcommand, Debug)]
pub enum ConfigCommand {
    /// Re-encode a donor config group from its wire format to the base client's.
    /// Today supports `--archive 2 --group 40 --from 948 --to 910` (the relic
    /// DBTABLETYPE opcode-1 schemas + the DbTableIndex BaseVarType serial form).
    ///
    /// Example:
    /// ```bash
    /// cd tools/rs3-cache-rs
    /// cargo run --release -- config transcode --archive 2 --group 40 --from 948 --to 910
    /// ```
    Transcode {
        /// Config archive id (2 for DBTABLETYPE).
        #[arg(long)]
        archive: u32,
        /// Group id (40 for DBTABLETYPE).
        #[arg(long)]
        group: u32,
        /// Donor build (948).
        #[arg(long)]
        from: u32,
        /// Target build (910).
        #[arg(long)]
        to: u32,
        /// Donor semantic config dir (`dbtables.json` / `dbrows.json`). READ-ONLY.
        #[arg(long, default_value = crate::config_transcode::DEFAULT_DONOR_SEMANTIC)]
        donor_semantic: PathBuf,
        /// Donor raw-flat root. READ-ONLY.
        #[arg(long, default_value = crate::config_transcode::DEFAULT_DONOR_RAW)]
        donor_raw: PathBuf,
        /// Base (910) raw-flat root. READ-ONLY.
        #[arg(long, default_value = crate::config_transcode::DEFAULT_BASE_RAW)]
        base_raw: PathBuf,
        /// Optional output dir for the re-encoded `.dat(+metadata)` files. Never
        /// point this at the committed relic overlay (the regression oracle).
        #[arg(long)]
        out_dir: Option<PathBuf>,
        /// Emit a JSON summary instead of the human report.
        #[arg(long)]
        json: bool,
    },
    /// Port a donor (948) config group to the 910 client through the typed config
    /// IR (plan §9 step 6): the relic DBTABLETYPE schemas + DbTableIndex re-encoded
    /// via `DbTable` / `DbTableIndex` IR records. Same scope + byte-stable
    /// (decompressed-body) contract as `config transcode`, routed through the IR.
    ///
    /// Example:
    /// ```bash
    /// cd tools/rs3-cache-rs
    /// cargo run --release -- config port --archive 2 --group 40 --from 948 --to 910
    /// ```
    Port {
        /// Config archive id (2 for DBTABLETYPE).
        #[arg(long)]
        archive: u32,
        /// Group id (40 for DBTABLETYPE).
        #[arg(long)]
        group: u32,
        /// Donor build (948).
        #[arg(long, default_value_t = 948)]
        from: u32,
        /// Target build (910).
        #[arg(long, default_value_t = 910)]
        to: u32,
        /// Donor semantic config dir (`dbtables.json` / `dbrows.json`). READ-ONLY.
        #[arg(long, default_value = crate::config_transcode::DEFAULT_DONOR_SEMANTIC)]
        donor_semantic: PathBuf,
        /// Donor raw-flat root. READ-ONLY.
        #[arg(long, default_value = crate::config_transcode::DEFAULT_DONOR_RAW)]
        donor_raw: PathBuf,
        /// Base (910) raw-flat root. READ-ONLY.
        #[arg(long, default_value = crate::config_transcode::DEFAULT_BASE_RAW)]
        base_raw: PathBuf,
        /// Optional output dir for the re-encoded `.dat(+metadata)` files. Never
        /// point this at the committed relic overlay (the regression oracle).
        #[arg(long)]
        out_dir: Option<PathBuf>,
        /// Emit a JSON summary instead of the human report.
        #[arg(long)]
        json: bool,
    },
}

/// Port-layer subcommands (`port <sub>`).
#[derive(Subcommand, Debug)]
pub enum PortCommand {
    /// The representability dry-run (plan §6/§10): enumerate every construct the
    /// 948→910 port of an interface's CS2 closure will hit, classified, with the
    /// named lowering that bridges each (or `Unhandled`). Subsumes the manual
    /// `--transitive` closure + collision + component-type + cc-model analysis.
    ///
    /// Example:
    /// ```bash
    /// cd tools/rs3-cache-rs
    /// cargo run --release -- --cache-dir ../../cache/unpacked/948 --data-dir data \
    ///   --build 948 --subbuild 1 port plan --interface 1224 --from 948 --to 910
    /// ```
    Plan {
        /// The interface whose CS2 closure to analyse (1224 = ritual selection).
        #[arg(long)]
        interface: u32,
        /// Donor build (948).
        #[arg(long, default_value_t = 948)]
        from: u32,
        /// Target build (910).
        #[arg(long, default_value_t = 910)]
        to: u32,
        /// Emit the findings as JSON instead of the human report.
        #[arg(long)]
        json: bool,
    },
}

/// Interface-group subcommands (`interface <sub>`).
#[derive(Subcommand, Debug)]
pub enum InterfaceCommand {
    /// Downcode a donor (948) interface group to the 910 client wire format. The
    /// 910 `Component.decode` only has bodies for the primitive component types
    /// {layer, rectangle, text, graphic, model, line}; a newer donor interface
    /// also uses composite widgets (button/check/…) whose bodies the 910 decoder
    /// skips, misaligning the stream into an `AIOOBE` at `Component.decode:973`.
    /// This rewrites each unsupported widget to a 910-decodable equivalent
    /// (preserving its ops/hooks/label), keeps every primitive component, and
    /// VALIDATES every output through a faithful Rust mirror of the 910 decoder.
    ///
    /// Example:
    /// ```bash
    /// cd tools/rs3-cache-rs
    /// cargo run --release -- interface transcode --group 1224 --from 948 --to 910 \
    ///   --raw-dat ../../server/cache-patches/ritual-pedestal-948/interfaces/1224.dat \
    ///   --out-dir /tmp/transcoded
    /// ```
    Transcode {
        /// Interface group id (used for the pack lookup, output filename, report).
        #[arg(long)]
        group: u32,
        /// Donor build (948).
        #[arg(long)]
        from: u32,
        /// Target build (910).
        #[arg(long)]
        to: u32,
        /// Build number the donor components decode at (the 948/947 layout).
        #[arg(long, default_value_t = BUILD)]
        decode_build: u32,
        /// Read the donor group from a raw group `.dat` (gzip JS5 container + 2-byte
        /// version trailer) instead of the runtime pack. READ-ONLY.
        #[arg(long)]
        raw_dat: Option<PathBuf>,
        /// Runtime pack root holding `client.interfaces.js5` (when `--raw-dat` is
        /// not given). READ-ONLY.
        #[arg(long, default_value = crate::interface::transcode::DEFAULT_PACK_ROOT_STR)]
        pack_root: PathBuf,
        /// Optional output dir; writes `interfaces/<group>-948.dat`. Never point
        /// this at a protected oracle dir.
        #[arg(long)]
        out_dir: Option<PathBuf>,
        /// Emit a JSON summary instead of the human report.
        #[arg(long)]
        json: bool,
    },
    /// Port a donor (948) interface group to the 910 client through the typed
    /// interface IR (plan §9 step 5): decode → represent → lower
    /// (`list_to_server_driven`) → encode, validating representability at encode
    /// time (the composite-widget downcode the old `interface transcode` did, now a
    /// named IR pass). Reproduces the committed `1224-910.dat` byte-for-byte.
    ///
    /// Example:
    /// ```bash
    /// cd tools/rs3-cache-rs
    /// cargo run --release -- interface port --group 1224 --from 948 --to 910 \
    ///   --raw-dat ../../server/cache-patches/ritual-pedestal-948/interfaces/1224.dat \
    ///   --out-dir /tmp/ported
    /// ```
    Port {
        /// Interface group id (used for the pack lookup, output filename, report).
        #[arg(long)]
        group: u32,
        /// Donor build (948).
        #[arg(long, default_value_t = 948)]
        from: u32,
        /// Target build (910).
        #[arg(long, default_value_t = 910)]
        to: u32,
        /// Build number the donor components decode at (the 948/947 layout).
        #[arg(long, default_value_t = BUILD)]
        decode_build: u32,
        /// Read the donor group from a raw group `.dat` instead of the runtime
        /// pack. READ-ONLY.
        #[arg(long)]
        raw_dat: Option<PathBuf>,
        /// Runtime pack root holding `client.interfaces.js5` (when `--raw-dat` is
        /// not given). READ-ONLY.
        #[arg(long, default_value = crate::interface::transcode::DEFAULT_PACK_ROOT_STR)]
        pack_root: PathBuf,
        /// Optional output dir; writes `interfaces/<group>-910.dat`. Never point
        /// this at a protected oracle dir.
        #[arg(long)]
        out_dir: Option<PathBuf>,
        /// Emit a JSON summary instead of the human report.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum VarDomainArg {
    All,
    Player,
    Npc,
    Client,
    World,
    Region,
    Object,
    Clan,
    ClanSetting,
    Controller,
    Global,
    PlayerGroup,
}

impl VarDomainArg {
    fn groups(self) -> &'static [(u32, VarDomain)] {
        const ALL: &[(u32, VarDomain)] = &[
            (CONFIG_GROUP_VAR_PLAYER, VarDomain::Player),
            (CONFIG_GROUP_VAR_NPC, VarDomain::Npc),
            (CONFIG_GROUP_VAR_CLIENT, VarDomain::Client),
            (CONFIG_GROUP_VAR_WORLD, VarDomain::World),
            (CONFIG_GROUP_VAR_REGION, VarDomain::Region),
            (CONFIG_GROUP_VAR_OBJECT, VarDomain::Object),
            (CONFIG_GROUP_VAR_CLAN, VarDomain::Clan),
            (CONFIG_GROUP_VAR_CLAN_SETTING, VarDomain::ClanSetting),
            (CONFIG_GROUP_VAR_CONTROLLER, VarDomain::Controller),
            (CONFIG_GROUP_VAR_GLOBAL, VarDomain::Global),
            (CONFIG_GROUP_VAR_PLAYER_GROUP, VarDomain::PlayerGroup),
        ];
        const PLAYER: &[(u32, VarDomain)] = &[(CONFIG_GROUP_VAR_PLAYER, VarDomain::Player)];
        const NPC: &[(u32, VarDomain)] = &[(CONFIG_GROUP_VAR_NPC, VarDomain::Npc)];
        const CLIENT: &[(u32, VarDomain)] = &[(CONFIG_GROUP_VAR_CLIENT, VarDomain::Client)];
        const WORLD: &[(u32, VarDomain)] = &[(CONFIG_GROUP_VAR_WORLD, VarDomain::World)];
        const REGION: &[(u32, VarDomain)] = &[(CONFIG_GROUP_VAR_REGION, VarDomain::Region)];
        const OBJECT: &[(u32, VarDomain)] = &[(CONFIG_GROUP_VAR_OBJECT, VarDomain::Object)];
        const CLAN: &[(u32, VarDomain)] = &[(CONFIG_GROUP_VAR_CLAN, VarDomain::Clan)];
        const CLAN_SETTING: &[(u32, VarDomain)] =
            &[(CONFIG_GROUP_VAR_CLAN_SETTING, VarDomain::ClanSetting)];
        const CONTROLLER: &[(u32, VarDomain)] =
            &[(CONFIG_GROUP_VAR_CONTROLLER, VarDomain::Controller)];
        const GLOBAL: &[(u32, VarDomain)] = &[(CONFIG_GROUP_VAR_GLOBAL, VarDomain::Global)];
        const PLAYER_GROUP: &[(u32, VarDomain)] =
            &[(CONFIG_GROUP_VAR_PLAYER_GROUP, VarDomain::PlayerGroup)];

        match self {
            Self::All => ALL,
            Self::Player => PLAYER,
            Self::Npc => NPC,
            Self::Client => CLIENT,
            Self::World => WORLD,
            Self::Region => REGION,
            Self::Object => OBJECT,
            Self::Clan => CLAN,
            Self::ClanSetting => CLAN_SETTING,
            Self::Controller => CONTROLLER,
            Self::Global => GLOBAL,
            Self::PlayerGroup => PLAYER_GROUP,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum TranspileOutputStyle {
    HighTs,
    Reversible,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum ConfigKindArg {
    Param,
    Enum,
    DbTable,
    DbRow,
    Loc,
    Npc,
    Obj,
    Seq,
    Spot,
    Struct,
    Inv,
    Cursor,
    Idk,
    Bas,
    Mel,
    Water,
    Achievement,
    Material,
    Quest,
    SeqGroup,
    Headbar,
    Hitmark,
    Light,
    SkyBox,
    WorldArea,
    Billboard,
    ParticleEmitter,
    ParticleEffector,
    Texture,
    Stylesheet,
    Controller,
    Category,
    Area,
    Hunt,
    MesAnim,
    ItemCode,
    GameLogEvent,
    BugTemplate,
    QuickChatCat,
    QuickChatPhrase,
    Underlay,
    Overlay,
    Msi,
}

impl ConfigKindArg {
    fn entity_type(self) -> EntityType {
        match self {
            Self::Param => EntityType::Param,
            Self::Enum => EntityType::Enum,
            Self::DbTable => EntityType::DbTable,
            Self::DbRow => EntityType::DbRow,
            Self::Loc => EntityType::Loc,
            Self::Npc => EntityType::Npc,
            Self::Obj => EntityType::Obj,
            Self::Seq => EntityType::Seq,
            Self::Spot => EntityType::Spot,
            Self::Struct => EntityType::Struct,
            Self::Inv => EntityType::Inv,
            Self::Cursor => EntityType::Cursor,
            Self::Idk => EntityType::Idk,
            Self::Bas => EntityType::Bas,
            Self::Mel => EntityType::Mel,
            Self::Water => EntityType::Water,
            Self::Achievement => EntityType::Achievement,
            Self::Material => EntityType::Material,
            Self::Quest => EntityType::Quest,
            Self::SeqGroup => EntityType::SeqGroup,
            Self::Headbar => EntityType::Headbar,
            Self::Hitmark => EntityType::Hitmark,
            Self::Light => EntityType::Light,
            Self::SkyBox => EntityType::SkyBox,
            Self::WorldArea => EntityType::WorldArea,
            Self::Billboard => EntityType::Billboard,
            Self::ParticleEmitter => EntityType::ParticleEmitter,
            Self::ParticleEffector => EntityType::ParticleEffector,
            Self::Texture => EntityType::Texture,
            Self::Stylesheet => EntityType::Stylesheet,
            Self::Controller => EntityType::ControllerConfig,
            Self::Category => EntityType::Category,
            Self::Area => EntityType::Area,
            Self::Hunt => EntityType::Hunt,
            Self::MesAnim => EntityType::MesAnim,
            Self::ItemCode => EntityType::ItemCode,
            Self::GameLogEvent => EntityType::GameLogEvent,
            Self::BugTemplate => EntityType::BugTemplate,
            Self::QuickChatCat => EntityType::QuickChatCat,
            Self::QuickChatPhrase => EntityType::QuickChatPhrase,
            Self::Underlay => EntityType::Underlay,
            Self::Overlay => EntityType::Overlay,
            Self::Msi => EntityType::Msi,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Param => "param",
            Self::Enum => "enum",
            Self::DbTable => "dbtable",
            Self::DbRow => "dbrow",
            Self::Loc => "loc",
            Self::Npc => "npc",
            Self::Obj => "obj",
            Self::Seq => "seq",
            Self::Spot => "spot",
            Self::Struct => "struct",
            Self::Inv => "inv",
            Self::Cursor => "cursor",
            Self::Idk => "idk",
            Self::Bas => "bas",
            Self::Mel => "mel",
            Self::Water => "water",
            Self::Achievement => "achievement",
            Self::Material => "material",
            Self::Quest => "quest",
            Self::SeqGroup => "seqgroup",
            Self::Headbar => "headbar",
            Self::Hitmark => "hitmark",
            Self::Light => "light",
            Self::SkyBox => "skybox",
            Self::WorldArea => "worldarea",
            Self::Billboard => "billboard",
            Self::ParticleEmitter => "particle_emitter",
            Self::ParticleEffector => "particle_effector",
            Self::Texture => "texture",
            Self::Stylesheet => "stylesheet",
            Self::Controller => "controller",
            Self::Category => "category",
            Self::Area => "area",
            Self::Hunt => "hunt",
            Self::MesAnim => "mesanim",
            Self::ItemCode => "itemcode",
            Self::GameLogEvent => "gamelogevent",
            Self::BugTemplate => "bugtemplate",
            Self::QuickChatCat => "quickchatcat",
            Self::QuickChatPhrase => "quickchatphrase",
            Self::Underlay => "underlay",
            Self::Overlay => "overlay",
            Self::Msi => "msi",
        }
    }
}

#[derive(Debug, Serialize)]
struct InterfacesSummary {
    archive: u32,
    groups: usize,
    files: usize,
    parsed_groups: usize,
}

#[derive(Debug, Serialize)]
struct VarSummary {
    groups: usize,
    entries: usize,
}

#[derive(Debug, Serialize)]
struct ConfigSummary {
    params: usize,
    enums: usize,
    dbtables: usize,
    dbrows: usize,
    idks: usize,
    locs: usize,
    npcs: usize,
    objs: usize,
    seqs: usize,
    spots: usize,
    bass: usize,
    quests: usize,
    mels: usize,
    waters: usize,
    achievements: usize,
    materials: usize,
    invs: usize,
    cursors: usize,
    seqgroups: usize,
    structs: usize,
    controllers: usize,
    categories: usize,
    areas: usize,
    hunts: usize,
    mesanims: usize,
    itemcodes: usize,
    gamelogevents: usize,
    bugtemplates: usize,
    varcstrs: usize,
    varnbits: usize,
    vars: usize,
    varsstrs: usize,
    underlays: usize,
    overlays: usize,
    msis: usize,
    skyboxes: usize,
    worldareas: usize,
    quickchatcats: usize,
    headbars: usize,
    hitmarks: usize,
    lights: usize,
    quickchatphrases: usize,
    billboards: usize,
    particleeffectors: usize,
    particleemitters: usize,
    textures: usize,
    stylesheets: usize,
}

#[derive(Debug, Serialize)]
struct Cs2Summary {
    scripts: usize,
    instructions: usize,
    unique_opcodes: usize,
}

#[derive(Debug, Serialize)]
struct ModelsSummary {
    groups_parsed: usize,
    parse_errors: usize,
}

#[derive(Debug, Serialize)]
struct AudioSummary {
    archives: BTreeMap<u32, usize>,
    kinds: BTreeMap<String, usize>,
    extracted_embedded_ogg: usize,
    manifest_path: Option<String>,
}

#[derive(Debug, Serialize)]
struct AudioManifestEntry {
    archive: u32,
    group: u32,
    file: u32,
    size: usize,
    kind: String,
    raw_extension: String,
    embedded_ogg_offset: Option<usize>,
    extracted_ogg: bool,
}

#[derive(Clone, Copy, Debug)]
struct RuntimeVersion {
    build: u32,
    subbuild: u32,
}

#[derive(Clone, Copy, Debug)]
struct UnpackRunOptions {
    sample_models: bool,
    skip_audio: bool,
    best_effort_maps: bool,
    max_audio_files: Option<usize>,
}

pub fn run(cli: Cli) -> Result<()> {
    let _ = crate::parallel::init_global_rayon()?;

    if let Command::OverlayPlan {
        manifest,
        out_file,
        audit_dir,
        allow_heuristic_sites,
        base_build,
        donor_build,
        base_subbuild,
        donor_subbuild,
    } = &cli.command
    {
        return crate::overlay_plan::run_overlay_plan_command(
            crate::overlay_plan::OverlayPlanCommandOptions {
                manifest,
                out_file: out_file.as_deref(),
                audit_dir: audit_dir.as_deref(),
                allow_heuristic_sites: *allow_heuristic_sites,
                data_dir: &cli.data_dir,
                base_build: *base_build,
                donor_build: *donor_build,
                base_subbuild: *base_subbuild,
                donor_subbuild: *donor_subbuild,
            },
        );
    }
    if let Command::ExtractCs2Registry {
        client_root,
        out_file,
        report_file,
    } = &cli.command
    {
        crate::cs2_registry::run(&crate::cs2_registry::Cs2RegistryOpts {
            client_root,
            data_dir: &cli.data_dir,
            out_file: out_file.as_deref(),
            report_file: report_file.as_deref(),
        })?;
        return Ok(());
    }
    if let Command::GenerateCs2Java {
        registry,
        client_root,
        out_dir,
        check,
    } = &cli.command
    {
        crate::cs2_javagen::run(&crate::cs2_javagen::Cs2JavaGenOpts {
            registry: registry.as_deref(),
            client_root,
            out_dir: out_dir.as_deref(),
            data_dir: &cli.data_dir,
            check: *check,
        })?;
        return Ok(());
    }
    if let Command::GenerateCs2Data {
        registry,
        out_dir,
        check,
    } = &cli.command
    {
        let drift = crate::cs2_datagen::run(&crate::cs2_datagen::Cs2DataGenOpts {
            registry: registry.as_deref(),
            out_dir: out_dir.as_deref(),
            data_dir: &cli.data_dir,
            check: *check,
        })?;
        if drift {
            std::process::exit(3);
        }
        return Ok(());
    }
    if let Command::Cs2Coverage {
        pack_root,
        pack_file,
        registry,
        out_file,
    } = &cli.command
    {
        let has_findings = crate::cs2_coverage::run(&crate::cs2_coverage::Cs2CoverageOpts {
            pack_root,
            pack_file: pack_file.as_deref(),
            registry: registry.as_deref(),
            out_file: out_file.as_deref(),
            data_dir: &cli.data_dir,
        })?;
        if has_findings {
            std::process::exit(4);
        }
        return Ok(());
    }
    if let Command::GenerateTsIds {
        pack_root,
        names_dir,
        out_dir,
        check,
    } = &cli.command
    {
        let default_names = cli.data_dir.join("names").join("910");
        let drift = crate::ts_idgen::run(&crate::ts_idgen::GenerateTsIdsOpts {
            pack_root,
            names_dir: names_dir.as_deref().unwrap_or(&default_names),
            out_dir,
            check: *check,
        })?;
        if drift {
            std::process::exit(3);
        }
        return Ok(());
    }
    if let Command::ExtractProtocol {
        client_root,
        server_root,
        out_dir,
    } = &cli.command
    {
        let default_out = cli.data_dir.join("protocol").join("910");
        crate::protocol_registry::run_extract(&crate::protocol_registry::ExtractProtocolOpts {
            client_root,
            server_root,
            out_dir: out_dir.as_deref().unwrap_or(&default_out),
        })?;
        return Ok(());
    }
    if let Command::GenerateProtocol {
        schema_dir,
        server_root,
        client_root,
        check,
    } = &cli.command
    {
        let default_schema = cli.data_dir.join("protocol").join("910");
        let drift = crate::protocol_registry::run_generate(&crate::protocol_registry::GenerateProtocolOpts {
            schema_dir: schema_dir.as_deref().unwrap_or(&default_schema),
            server_root,
            client_root,
            check: *check,
        })?;
        if drift {
            std::process::exit(3);
        }
        return Ok(());
    }
    if let Command::AssembleScriptBatch { manifest, out_dir } = &cli.command {
        return run_assemble_script_batch(
            &cli.data_dir,
            manifest,
            out_dir,
            RuntimeVersion {
                build: cli.build,
                subbuild: cli.subbuild,
            },
        );
    }
    if let Command::Font { command } = &cli.command {
        return Ok(crate::font::cli::run(command)?);
    }
    if let Command::ExplainInterface {
        id,
        json,
        pack_root,
        raw_dat,
        decode_build,
        transitive,
        scripts_cache,
        scripts_build,
        base_pack_root,
    } = &cli.command
    {
        let source = raw_dat.as_deref().map_or_else(
            || crate::explain::InterfaceSource::Pack(pack_root.as_path()),
            crate::explain::InterfaceSource::RawDat,
        );
        // `--transitive` walks the donor clientscript graph from a flat cache
        // (defaulting to the global `--cache-dir`/`--build`) and scores it against
        // the 910-base roster. The donor cache holds the un-down-coded bodies, so
        // it — not the runtime overlay pack — is the source of the splice burden.
        let scripts_cache_path = scripts_cache
            .as_deref()
            .or(cli.cache_dir.as_deref())
            .unwrap_or_else(|| Path::new("../../cache/unpacked/948"));
        let transitive_opts = transitive.then(|| crate::explain::TransitiveOptions {
            scripts_cache: scripts_cache_path,
            scripts_build: scripts_build.unwrap_or(cli.build),
            scripts_subbuild: cli.subbuild,
            data_dir: cli.data_dir.as_path(),
            base_pack_root: base_pack_root.as_path(),
        });
        return Ok(crate::explain::run(&crate::explain::ExplainInterfaceOptions {
            interface: *id,
            build: *decode_build,
            source,
            json: *json,
            transitive: transitive_opts,
        })?);
    }
    if let Command::Decode {
        archive,
        group,
        format,
        pack_root,
        json,
    } = &cli.command
    {
        let format = format.parse::<crate::decode::Format>()?;
        return Ok(crate::decode::run(&crate::decode::DecodeOptions {
            archive: *archive,
            group: *group,
            format,
            pack_root: pack_root.as_path(),
            json: *json,
        })?);
    }
    // `cs2 lint-splice` operates on text listings + the opcode-book registries;
    // it needs no flat cache, so intercept it before `open_cache` (like font /
    // decode / explain-interface). The legacy `cs2` dump (no subcommand) falls
    // through to the cache-backed match below.
    if let Command::Cs2 {
        command: Some(Cs2Command::LintSplice { .. }),
        ..
    } = &cli.command
    {
        let Command::Cs2 {
            command: Some(Cs2Command::LintSplice {
                scripts,
                target_book,
                donor_book,
                fix,
                json,
            }),
            ..
        } = &cli.command
        else {
            unreachable!("guarded by the outer match")
        };
        return Ok(crate::cs2::lint::run(&crate::cs2::lint::LintOptions {
            scripts_dir: scripts.as_path(),
            data_dir: cli.data_dir.as_path(),
            target_book: *target_book,
            donor_book: *donor_book,
            fix: *fix,
            json: *json,
        })?);
    }
    // `config transcode` reads donor semantic JSON + raw-flat caches directly; no
    // flat cache needed.
    if let Command::Config {
        command:
            ConfigCommand::Transcode {
                archive,
                group,
                from,
                to,
                donor_semantic,
                donor_raw,
                base_raw,
                out_dir,
                json,
            },
    } = &cli.command
    {
        return Ok(crate::config_transcode::run(
            &crate::config_transcode::TranscodeOptions {
                archive: *archive,
                group: *group,
                from: *from,
                to: *to,
                donor_semantic: donor_semantic.as_path(),
                donor_raw: donor_raw.as_path(),
                base_raw: base_raw.as_path(),
                out_dir: out_dir.as_deref(),
                json: *json,
            },
        )?);
    }
    // `config port` reads donor semantic JSON + raw-flat caches directly through the
    // typed config IR; like `config transcode` no flat cache is needed, so intercept
    // it before `open_cache`.
    if let Command::Config {
        command:
            ConfigCommand::Port {
                archive,
                group,
                from,
                to,
                donor_semantic,
                donor_raw,
                base_raw,
                out_dir,
                json,
            },
    } = &cli.command
    {
        return Ok(crate::port::config::run(&crate::port::config::ConfigPortOptions {
            archive: *archive,
            group: *group,
            from: *from,
            to: *to,
            donor_semantic: donor_semantic.as_path(),
            donor_raw: donor_raw.as_path(),
            base_raw: base_raw.as_path(),
            data_dir: cli.data_dir.as_path(),
            out_dir: out_dir.as_deref(),
            json: *json,
        })?);
    }
    // `interface transcode` reads the donor group from a raw `.dat` or the runtime
    // pack directly and validates through the in-process 910 mirror; no flat cache
    // needed, so intercept it before `open_cache` (like `config transcode`).
    if let Command::Interface {
        command:
            InterfaceCommand::Transcode {
                group,
                from,
                to,
                decode_build,
                raw_dat,
                pack_root,
                out_dir,
                json,
            },
    } = &cli.command
    {
        let source = raw_dat.as_deref().map_or_else(
            || crate::interface::transcode::GroupSource::Pack(pack_root.as_path()),
            crate::interface::transcode::GroupSource::RawDat,
        );
        return Ok(crate::interface::transcode::run(
            &crate::interface::transcode::InterfaceTranscodeOptions {
                group: *group,
                from: *from,
                to: *to,
                decode_build: *decode_build,
                source,
                out_dir: out_dir.as_deref(),
                json: *json,
            },
        )?);
    }
    // `interface port` runs the donor group through the typed interface IR (decode
    // → lower → encode); like `interface transcode` it reads a raw `.dat` or the
    // runtime pack and validates through the 910 mirror, so intercept it before the
    // cache is opened.
    if let Command::Interface {
        command:
            InterfaceCommand::Port {
                group,
                from,
                to,
                decode_build,
                raw_dat,
                pack_root,
                out_dir,
                json,
            },
    } = &cli.command
    {
        let source = raw_dat.as_deref().map_or_else(
            || crate::interface::transcode::GroupSource::Pack(pack_root.as_path()),
            crate::interface::transcode::GroupSource::RawDat,
        );
        return Ok(crate::port::interface::run(
            &crate::port::interface::InterfacePortOptions {
                group: *group,
                from: *from,
                to: *to,
                decode_build: *decode_build,
                source,
                data_dir: cli.data_dir.as_path(),
                out_dir: out_dir.as_deref(),
                json: *json,
            },
        )?);
    }

    let tar_path = cli.cache_tar.unwrap_or_else(default_tar_path);
    let cache = open_cache(cli.cache_dir.as_deref())?;
    let version = RuntimeVersion {
        build: cli.build,
        subbuild: cli.subbuild,
    };

    match cli.command {
        Command::Interfaces { out_dir } => {
            run_interfaces(&cache, &tar_path, out_dir.as_deref(), version.build)
        }
        Command::Varps { out_file, domain } => {
            run_varps(&cache, &tar_path, out_file.as_deref(), domain)
        }
        Command::Varbits { out_file } => run_varbits(&cache, &tar_path, out_file.as_deref()),
        Command::Configs { out_dir } => {
            run_configs(&cache, &tar_path, out_dir.as_deref(), version.build)
        }
        Command::Cs2 {
            command,
            out_file,
            out_dir,
        } => {
            // `lint-splice` is intercepted before the cache is opened; `port` needs
            // the donor cache (decoded here). No subcommand = the legacy dump.
            match command {
                Some(Cs2Command::Port {
                    from,
                    to,
                    closure_of_interface,
                    base_cache_dir,
                    out_dir: port_out_dir,
                    check_oracle,
                    json,
                }) => run_cs2_port(
                    &cache,
                    &cli.data_dir,
                    from,
                    to,
                    closure_of_interface,
                    base_cache_dir.as_deref(),
                    port_out_dir.as_deref(),
                    check_oracle,
                    json,
                ),
                Some(Cs2Command::LintSplice { .. }) => {
                    bail!("cs2 lint-splice should have been dispatched before cache open")
                }
                None => run_cs2(
                    &cache,
                    &tar_path,
                    &cli.data_dir,
                    out_file.as_deref(),
                    out_dir.as_deref(),
                    version,
                ),
            }
        }
        Command::Port { command } => run_port_command(&cache, &cli.data_dir, &command),
        // `config transcode` is intercepted before the cache is opened.
        Command::Config { .. } => {
            bail!("config subcommand should have been dispatched before cache open")
        }
        Command::Models {
            out_file,
            out_dir,
            sample_only,
        } => run_models(
            &cache,
            &tar_path,
            out_file.as_deref(),
            out_dir.as_deref(),
            sample_only,
            version.build,
        ),
        Command::Audio { out_dir, max_files } => {
            run_audio(&cache, &tar_path, out_dir.as_deref(), max_files)
        }
        Command::Unpack {
            out_dir,
            sample_models,
            skip_audio,
            best_effort_maps,
            max_audio_files,
        } => run_unpack(
            &cache,
            &tar_path,
            &cli.data_dir,
            &out_dir,
            UnpackRunOptions {
                sample_models,
                skip_audio,
                best_effort_maps,
                max_audio_files,
            },
            version,
        ),
        Command::DepTreeInterface {
            id,
            max_depth,
            out_file,
        } => run_dep_tree_interface(
            &cache,
            &tar_path,
            &cli.data_dir,
            id,
            max_depth,
            &out_file,
            version,
        ),
        Command::DepTreeScript {
            id,
            max_depth,
            out_file,
        } => run_dep_tree_script(
            &cache,
            &tar_path,
            &cli.data_dir,
            id,
            max_depth,
            &out_file,
            version,
        ),
        Command::DepTreeVarp {
            id,
            domain,
            max_depth,
            out_file,
        } => run_dep_tree_varp(
            &cache,
            &tar_path,
            &cli.data_dir,
            id,
            domain,
            max_depth,
            &out_file,
            version,
        ),
        Command::DepTreeVarbit {
            id,
            max_depth,
            out_file,
        } => run_dep_tree_varbit(
            &cache,
            &tar_path,
            &cli.data_dir,
            id,
            max_depth,
            &out_file,
            version,
        ),
        Command::ExplainLoc {
            id,
            max_candidates,
            json,
        } => Ok(crate::explain_loc::run(
            &cache,
            &tar_path,
            &cli.data_dir,
            &crate::explain_loc::ExplainLocOptions {
                loc: id,
                build: version.build,
                subbuild: version.subbuild,
                max_candidates,
                json,
            },
        )?),
        Command::DepTreeConfig {
            kind,
            id,
            max_depth,
            out_file,
        } => run_dep_tree_config(
            &cache,
            &tar_path,
            &cli.data_dir,
            kind,
            id,
            max_depth,
            &out_file,
            version,
        ),
        Command::TsExport { out_dir } => {
            run_ts_export(&cache, &tar_path, &cli.data_dir, &out_dir, version)
        }
        Command::TranspileScripts {
            out_dir,
            filter_script,
            output_style,
            max_scripts,
            all_scripts,
            max_script_bytes,
            max_script_instructions,
            max_generated_bytes,
        } => run_transpile_scripts(
            &cache,
            &tar_path,
            &cli.data_dir,
            &out_dir,
            filter_script.as_deref(),
            output_style,
            max_scripts,
            all_scripts,
            crate::transpile::TranspileLimits {
                max_script_bytes,
                max_instructions: max_script_instructions,
                max_generated_bytes,
            },
            version,
        ),
        Command::MigrateCheck {
            interface_group,
            out_file,
            audit_dir,
            source_cache_tar,
            source_build,
            source_subbuild,
            remap,
            remap_buffer,
            validate_target,
            allow_heuristic_sites,
        } => run_migrate_check(
            &cache,
            &tar_path,
            &cli.data_dir,
            interface_group,
            &out_file,
            audit_dir.as_deref(),
            version,
            source_cache_tar.as_deref(),
            source_build,
            source_subbuild,
            remap,
            remap_buffer,
            validate_target,
            allow_heuristic_sites,
        ),
        Command::MigrateScript {
            script_id,
            out_file,
            audit_dir,
            source_cache_tar,
            source_build,
            source_subbuild,
            remap,
            remap_buffer,
            validate_target,
            allow_heuristic_sites,
        } => run_migrate_script(
            &cache,
            &tar_path,
            &cli.data_dir,
            script_id,
            &out_file,
            audit_dir.as_deref(),
            version,
            source_cache_tar.as_deref(),
            source_build,
            source_subbuild,
            remap,
            remap_buffer,
            validate_target,
            allow_heuristic_sites,
        ),
        Command::ValidateScript {
            script_id,
            out_file,
            json,
        } => run_validate_script(
            &cache,
            &tar_path,
            &cli.data_dir,
            script_id,
            out_file.as_deref(),
            json,
            version,
        ),
        Command::AssembleScript {
            input,
            output,
            build,
            subbuild,
            strict_structured,
            no_verify,
            json,
        } => run_assemble_script(
            &cache,
            &tar_path,
            &cli.data_dir,
            &input,
            &output,
            build,
            subbuild,
            strict_structured,
            no_verify,
            json,
            version,
        ),
        Command::AssembleScriptBatch { .. } => unreachable!("handled before cache open"),
        Command::Font { .. } => unreachable!("handled before cache open"),
        Command::ExplainInterface { .. } => unreachable!("handled before cache open"),
        Command::Interface { .. } => unreachable!("handled before cache open"),
        Command::Decode { .. } => unreachable!("handled before cache open"),
        Command::ExtractCs2Registry { .. } => unreachable!("handled before cache open"),
        Command::GenerateCs2Java { .. } => unreachable!("handled before cache open"),
        Command::ExtractProtocol { .. } => unreachable!("handled before cache open"),
        Command::GenerateProtocol { .. } => unreachable!("handled before cache open"),
        Command::GenerateCs2Data { .. } => unreachable!("handled before cache open"),
        Command::Cs2Coverage { .. } => unreachable!("handled before cache open"),
        Command::GenerateTsIds { .. } => unreachable!("handled before cache open"),
        Command::DumpRawFlat { out_dir, archives } => {
            run_dump_raw_flat(&cache, &tar_path, &out_dir, archives.as_deref())
        }
        Command::DumpRefs { out_dir } => run_dump_refs(&cache, &tar_path, &out_dir, version.build),
        Command::DumpConfigs { out_dir } => {
            run_dump_configs(&cache, &tar_path, &out_dir, version.build)
        }
        Command::PrepareOverlay { out_dir, archives } => run_prepare_overlay(
            &cache,
            &tar_path,
            &cli.data_dir,
            &out_dir,
            version.build,
            version.subbuild,
            archives.as_deref(),
        ),
        Command::VerifyMapArchive => run_verify_map_archive(&cache, version.build),
        Command::BuildCollision { map_x, map_z, out } => {
            run_build_collision(&cache, version.build, map_x, map_z, out.as_deref())
        }
        Command::OverlayPlan { .. } => unreachable!("handled before cache open"),
    }
}

fn run_verify_map_archive(cache: &FlatCache, build: u32) -> Result<()> {
    let started = Instant::now();
    let index = cache.archive_index(ARCHIVE_MAPSQUARES)?;
    let count = index.group_id.len();
    index
        .group_id
        .par_iter()
        .try_for_each(|group| -> Result<()> {
            let files = cache.group_files_with_index(&index, ARCHIVE_MAPSQUARES, *group)?;
            let square_x = group & 0b111_1111;
            let square_z = group >> 7;
            decode_map_square(&files, build).with_context(|| {
                format!("decode mapsquare group {group} ({square_x}_{square_z})")
            })?;
            Ok(())
        })?;
    eprintln!(
        "verify-map-archive: decoded {} mapsquare group(s) in {}ms",
        count,
        started.elapsed().as_millis()
    );
    Ok(())
}

/// Load every loc config and reduce it to its collision-relevant [`LocClip`].
fn load_loc_clips(cache: &FlatCache) -> Result<HashMap<i32, crate::collision::LocClip>> {
    use crate::collision::LocClip;
    let mut clips = HashMap::new();
    if let Ok(loc_index) = cache.archive_index(ARCHIVE_LOC_CONFIG) {
        for group in &loc_index.group_id {
            let files = cache.group_files_with_index(&loc_index, ARCHIVE_LOC_CONFIG, *group)?;
            for (file, data) in files {
                let loc_id = (*group << 8) | file;
                let entry =
                    parse_loc(loc_id, &data).with_context(|| format!("parse_loc id {loc_id}"))?;
                clips.insert(loc_id as i32, LocClip::from_loc_ops(&entry.ops));
            }
        }
    } else if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_LOC_LEGACY)? {
        let config_index = cache.archive_index(ARCHIVE_CONFIG)?;
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_LOC_LEGACY, &payload)?;
        for (id, data) in files {
            let entry = parse_loc(id, &data).with_context(|| format!("parse_loc id {id}"))?;
            clips.insert(id as i32, LocClip::from_loc_ops(&entry.ops));
        }
    }
    Ok(clips)
}

fn run_build_collision(
    cache: &FlatCache,
    build: u32,
    map_x: u32,
    map_z: u32,
    out: Option<&Path>,
) -> Result<()> {
    let group = (map_z << 7) | map_x;
    let index = cache.archive_index(ARCHIVE_MAPSQUARES)?;
    let files = cache
        .group_files_with_index(&index, ARCHIVE_MAPSQUARES, group)
        .with_context(|| format!("load mapsquare group {group} ({map_x}_{map_z})"))?;
    let map = decode_map_square_best_effort(&files, build);
    let clips = load_loc_clips(cache)?;
    let grids = crate::collision::build_collision(&map, |id| {
        clips.get(&id).copied().unwrap_or_default()
    });

    #[derive(serde::Serialize)]
    struct LevelSummary {
        level: usize,
        blocked: usize,
    }
    let level_summaries: Vec<LevelSummary> = grids
        .iter()
        .enumerate()
        .map(|(level, g)| LevelSummary {
            level,
            blocked: g.nonzero_count(),
        })
        .collect();

    if let Some(path) = out {
        #[derive(serde::Serialize)]
        struct LevelDump {
            level: usize,
            blocked: usize,
            flags: Vec<Vec<i32>>,
        }
        #[derive(serde::Serialize)]
        struct FullDump {
            build: u32,
            #[serde(rename = "mapX")]
            map_x: u32,
            #[serde(rename = "mapZ")]
            map_z: u32,
            size: usize,
            levels: Vec<LevelDump>,
        }
        let dump = FullDump {
            build,
            map_x,
            map_z,
            size: crate::collision::SQUARE_SIZE,
            levels: grids
                .iter()
                .enumerate()
                .map(|(level, g)| LevelDump {
                    level,
                    blocked: g.nonzero_count(),
                    flags: g.to_rows(),
                })
                .collect(),
        };
        write_text(path, &serde_json::to_string(&dump)?)?;
        eprintln!("build-collision: wrote grids to {}", path.display());
    }

    #[derive(serde::Serialize)]
    struct Summary {
        build: u32,
        #[serde(rename = "mapX")]
        map_x: u32,
        #[serde(rename = "mapZ")]
        map_z: u32,
        size: usize,
        #[serde(rename = "locCount")]
        loc_count: usize,
        levels: Vec<LevelSummary>,
    }
    print_json(&Summary {
        build,
        map_x,
        map_z,
        size: crate::collision::SQUARE_SIZE,
        loc_count: map.locs.len(),
        levels: level_summaries,
    })
}

fn run_interfaces(
    cache: &FlatCache,
    tar_path: &Path,
    out_dir: Option<&Path>,
    build: u32,
) -> Result<()> {
    ensure_archive_complete(cache.root(), tar_path, ARCHIVE_INTERFACES)?;
    let cache = FlatCache::open(cache.root())?;
    let index = cache.archive_index(ARCHIVE_INTERFACES)?;
    let group_counts = index
        .group_id
        .par_iter()
        .map(|group| -> Result<usize> {
            let group_files = cache.group_files_with_index(&index, ARCHIVE_INTERFACES, *group)?;
            let file_count = group_files.len();
            let rendered = render_interface_group(*group, &group_files, build);
            if let Some(out) = out_dir {
                let is_scripted = group_files
                    .values()
                    .any(|bytes| bytes.first().copied() == Some(u8::MAX));
                let extension = if is_scripted { "if3" } else { "if" };
                let path = out.join(format!("interface_{group}.{extension}"));
                write_text(&path, &rendered.join("\n"))?;
            }
            Ok(file_count)
        })
        .collect::<Vec<_>>();

    let mut files = 0_usize;
    for count in group_counts {
        files += count?;
    }

    print_json(&InterfacesSummary {
        archive: ARCHIVE_INTERFACES,
        groups: index.group_count,
        files,
        parsed_groups: index.group_id.len(),
    })
}

fn run_varps(
    cache: &FlatCache,
    tar_path: &Path,
    out_file: Option<&Path>,
    domain: VarDomainArg,
) -> Result<()> {
    ensure_archive_complete(cache.root(), tar_path, ARCHIVE_CONFIG)?;
    let cache = FlatCache::open(cache.root())?;
    let index = cache.archive_index(ARCHIVE_CONFIG)?;

    let mut out = Vec::new();
    for (group_id, var_domain) in domain.groups() {
        let group_payload = cache
            .get(ARCHIVE_CONFIG, *group_id)?
            .with_context(|| format!("missing group {group_id} in archive 2"))?;
        let vars = crate::js5::unpack_group(&index, *group_id, &group_payload)?;
        for (id, bytes) in vars {
            out.push(parse_var(*var_domain, id, &bytes)?);
        }
    }

    if let Some(path) = out_file {
        write_json(path, &out)?;
    }
    print_json(&VarSummary {
        groups: domain.groups().len(),
        entries: out.len(),
    })
}

fn run_varbits(cache: &FlatCache, tar_path: &Path, out_file: Option<&Path>) -> Result<()> {
    ensure_archive_complete(cache.root(), tar_path, ARCHIVE_CONFIG)?;
    let cache = FlatCache::open(cache.root())?;
    let index = cache.archive_index(ARCHIVE_CONFIG)?;
    let varbit_group_payload = cache
        .get(ARCHIVE_CONFIG, CONFIG_GROUP_VAR_BIT)?
        .context("missing varbit group data 2/69.dat")?;
    let varbits = crate::js5::unpack_group(&index, CONFIG_GROUP_VAR_BIT, &varbit_group_payload)?;
    let mut out = Vec::with_capacity(varbits.len());
    for (id, bytes) in varbits {
        out.push(parse_varbit(id, &bytes)?);
    }
    if let Some(path) = out_file {
        write_json(path, &out)?;
    }
    print_json(&VarSummary {
        groups: 1,
        entries: out.len(),
    })
}

fn run_configs(
    cache: &FlatCache,
    tar_path: &Path,
    out_dir: Option<&Path>,
    build: u32,
) -> Result<()> {
    ensure_archive_complete(cache.root(), tar_path, ARCHIVE_CONFIG)?;
    ensure_archive_complete(cache.root(), tar_path, ARCHIVE_ENUM_CONFIG)?;
    let struct_archive_available =
        ensure_archive_complete(cache.root(), tar_path, ARCHIVE_STRUCT_CONFIG).is_ok();
    let quickchat_archive_available =
        ensure_archive_complete(cache.root(), tar_path, ARCHIVE_QUICKCHAT_CONFIG).is_ok();
    let loc_archive_available =
        ensure_archive_complete(cache.root(), tar_path, ARCHIVE_LOC_CONFIG).is_ok();
    let npc_archive_available =
        ensure_archive_complete(cache.root(), tar_path, ARCHIVE_NPC_CONFIG).is_ok();
    let obj_archive_available =
        ensure_archive_complete(cache.root(), tar_path, ARCHIVE_OBJ_CONFIG).is_ok();
    let seq_archive_available =
        ensure_archive_complete(cache.root(), tar_path, ARCHIVE_SEQ_CONFIG).is_ok();
    let spot_archive_available =
        ensure_archive_complete(cache.root(), tar_path, ARCHIVE_SPOT_CONFIG).is_ok();
    let particle_archive_available =
        ensure_archive_complete(cache.root(), tar_path, ARCHIVE_PARTICLES).is_ok();
    let billboard_archive_available =
        ensure_archive_complete(cache.root(), tar_path, ARCHIVE_BILLBOARDS).is_ok();
    let texture_archive_available =
        ensure_archive_complete(cache.root(), tar_path, ARCHIVE_TEXTURES).is_ok();
    let materials_archive_available =
        ensure_archive_complete(cache.root(), tar_path, ARCHIVE_MATERIALS).is_ok();
    let achievements_archive_available =
        ensure_archive_complete(cache.root(), tar_path, ARCHIVE_ACHIEVEMENTS).is_ok();
    let stylesheet_archive_available =
        ensure_archive_complete(cache.root(), tar_path, ARCHIVE_STYLESHEETS).is_ok();
    let cache = FlatCache::open(cache.root())?;

    let config_index = cache.archive_index(ARCHIVE_CONFIG)?;
    let enum_index = cache.archive_index(ARCHIVE_ENUM_CONFIG)?;

    let param_payload = cache
        .get(ARCHIVE_CONFIG, CONFIG_GROUP_PARAM)?
        .with_context(|| format!("missing group {CONFIG_GROUP_PARAM} in archive 2"))?;
    let param_files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_PARAM, &param_payload)?;
    let mut params = Vec::with_capacity(param_files.len());
    for (id, data) in param_files {
        params.push(parse_param(id, &data).with_context(|| format!("parse_param id {id}"))?);
    }

    let dbtable_payload = cache
        .get(ARCHIVE_CONFIG, CONFIG_GROUP_DBTABLE)?
        .with_context(|| format!("missing group {CONFIG_GROUP_DBTABLE} in archive 2"))?;
    let dbtable_files =
        crate::js5::unpack_group(&config_index, CONFIG_GROUP_DBTABLE, &dbtable_payload)?;
    let mut dbtables = Vec::with_capacity(dbtable_files.len());
    for (id, data) in dbtable_files {
        dbtables.push(parse_dbtable(id, &data).with_context(|| format!("parse_dbtable id {id}"))?);
    }

    let dbrow_payload = cache
        .get(ARCHIVE_CONFIG, CONFIG_GROUP_DBROW)?
        .with_context(|| format!("missing group {CONFIG_GROUP_DBROW} in archive 2"))?;
    let dbrow_files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_DBROW, &dbrow_payload)?;
    let mut dbrows = Vec::with_capacity(dbrow_files.len());
    for (id, data) in dbrow_files {
        dbrows.push(parse_dbrow(id, &data).with_context(|| format!("parse_dbrow id {id}"))?);
    }

    let mut idks = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_IDK)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_IDK, &payload)?;
        idks.reserve(files.len());
        for (id, data) in files {
            idks.push(parse_idk(id, &data).with_context(|| format!("parse_idk id {id}"))?);
        }
    }

    let mut locs = Vec::new();
    if loc_archive_available {
        let loc_index = cache.archive_index(ARCHIVE_LOC_CONFIG)?;
        for group in &loc_index.group_id {
            let files = cache.group_files_with_index(&loc_index, ARCHIVE_LOC_CONFIG, *group)?;
            for (file, data) in files {
                let loc_id = (group << 8) | file;
                locs.push(
                    parse_loc(loc_id, &data).with_context(|| format!("parse_loc id {loc_id}"))?,
                );
            }
        }
    } else if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_LOC_LEGACY)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_LOC_LEGACY, &payload)?;
        locs.reserve(files.len());
        for (id, data) in files {
            locs.push(parse_loc(id, &data).with_context(|| format!("parse_loc id {id}"))?);
        }
    }

    let mut npcs = Vec::new();
    if npc_archive_available {
        let npc_index = cache.archive_index(ARCHIVE_NPC_CONFIG)?;
        for group in &npc_index.group_id {
            let files = cache.group_files_with_index(&npc_index, ARCHIVE_NPC_CONFIG, *group)?;
            for (file, data) in files {
                let npc_id = (group << 7) | file;
                npcs.push(
                    parse_npc(npc_id, &data).with_context(|| format!("parse_npc id {npc_id}"))?,
                );
            }
        }
    } else if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_NPC_LEGACY)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_NPC_LEGACY, &payload)?;
        npcs.reserve(files.len());
        for (id, data) in files {
            npcs.push(parse_npc(id, &data).with_context(|| format!("parse_npc id {id}"))?);
        }
    }

    let mut objs = Vec::new();
    if obj_archive_available {
        let obj_index = cache.archive_index(ARCHIVE_OBJ_CONFIG)?;
        for group in &obj_index.group_id {
            let files = cache.group_files_with_index(&obj_index, ARCHIVE_OBJ_CONFIG, *group)?;
            for (file, data) in files {
                let obj_id = (group << 8) | file;
                objs.push(
                    parse_obj(obj_id, &data).with_context(|| format!("parse_obj id {obj_id}"))?,
                );
            }
        }
    } else if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_OBJ_LEGACY)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_OBJ_LEGACY, &payload)?;
        objs.reserve(files.len());
        for (id, data) in files {
            objs.push(parse_obj(id, &data).with_context(|| format!("parse_obj id {id}"))?);
        }
    }

    let mut seqs = Vec::new();
    if seq_archive_available {
        let seq_index = cache.archive_index(ARCHIVE_SEQ_CONFIG)?;
        for group in &seq_index.group_id {
            let files = cache.group_files_with_index(&seq_index, ARCHIVE_SEQ_CONFIG, *group)?;
            for (file, data) in files {
                let seq_id = (group << 7) | file;
                seqs.push(
                    parse_seq(seq_id, &data).with_context(|| format!("parse_seq id {seq_id}"))?,
                );
            }
        }
    } else if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_SEQ)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_SEQ, &payload)?;
        seqs.reserve(files.len());
        for (id, data) in files {
            seqs.push(parse_seq(id, &data).with_context(|| format!("parse_seq id {id}"))?);
        }
    }

    let mut spots = Vec::new();
    if spot_archive_available {
        let spot_index = cache.archive_index(ARCHIVE_SPOT_CONFIG)?;
        for group in &spot_index.group_id {
            let files = cache.group_files_with_index(&spot_index, ARCHIVE_SPOT_CONFIG, *group)?;
            for (file, data) in files {
                let spot_id = (group << 8) | file;
                spots.push(
                    parse_spot(spot_id, &data)
                        .with_context(|| format!("parse_spot id {spot_id}"))?,
                );
            }
        }
    } else if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_SPOT)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_SPOT, &payload)?;
        spots.reserve(files.len());
        for (id, data) in files {
            spots.push(parse_spot(id, &data).with_context(|| format!("parse_spot id {id}"))?);
        }
    }

    let mut bass = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_BAS)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_BAS, &payload)?;
        bass.reserve(files.len());
        for (id, data) in files {
            bass.push(parse_bas(id, &data, build).with_context(|| format!("parse_bas id {id}"))?);
        }
    }

    let mut quests = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_QUEST)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_QUEST, &payload)?;
        quests.reserve(files.len());
        for (id, data) in files {
            quests.push(parse_quest(id, &data)?);
        }
    }

    let mut mels = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_MEL)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_MEL, &payload)?;
        mels.reserve(files.len());
        for (id, data) in files {
            mels.push(parse_mel(id, &data)?);
        }
    }

    let mut waters = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_WATER)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_WATER, &payload)?;
        waters.reserve(files.len());
        for (id, data) in files {
            waters.push(parse_water(id, &data)?);
        }
    }

    let mut achievements = Vec::new();
    if achievements_archive_available {
        let achievements_index = cache.archive_index(ARCHIVE_ACHIEVEMENTS)?;
        for group in &achievements_index.group_id {
            let files =
                cache.group_files_with_index(&achievements_index, ARCHIVE_ACHIEVEMENTS, *group)?;
            for (file, data) in files {
                let achievement_id = (group << CONFIG_GROUP_ACHIEVEMENT_ARCHIVE57) | file;
                achievements.push(parse_achievement(achievement_id, &data)?);
            }
        }
    }

    let mut materials = Vec::new();
    if materials_archive_available {
        let materials_index = cache.archive_index(ARCHIVE_MATERIALS)?;
        if let Some(payload) = cache.get(ARCHIVE_MATERIALS, CONFIG_GROUP_MATERIAL_ARCHIVE26)? {
            let files = crate::js5::unpack_group(
                &materials_index,
                CONFIG_GROUP_MATERIAL_ARCHIVE26,
                &payload,
            )?;
            materials.reserve(files.len());
            for (id, data) in files {
                materials.push(parse_material(id, &data)?);
            }
        } else {
            for group in &materials_index.group_id {
                let files =
                    cache.group_files_with_index(&materials_index, ARCHIVE_MATERIALS, *group)?;
                for (file, data) in files {
                    materials.push(parse_material(group + file, &data)?);
                }
            }
        }
    }

    let mut invs = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_INV)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_INV, &payload)?;
        invs.reserve(files.len());
        for (id, data) in files {
            invs.push(parse_inv(id, &data)?);
        }
    }

    let mut cursors = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_CURSOR)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_CURSOR, &payload)?;
        cursors.reserve(files.len());
        for (id, data) in files {
            cursors.push(parse_cursor(id, &data)?);
        }
    }

    let mut seqgroups = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_SEQGROUP)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_SEQGROUP, &payload)?;
        seqgroups.reserve(files.len());
        for (id, data) in files {
            seqgroups.push(parse_seqgroup(id, &data)?);
        }
    }

    let mut categories = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_CATEGORY)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_CATEGORY, &payload)?;
        categories.reserve(files.len());
        for (id, data) in files {
            categories.push(parse_category(id, &data)?);
        }
    }

    let mut controllers = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_CONTROLLER)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_CONTROLLER, &payload)?;
        controllers.reserve(files.len());
        for (id, data) in files {
            controllers.push(parse_controller(id, &data)?);
        }
    }

    let mut areas = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_AREA)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_AREA, &payload)?;
        areas.reserve(files.len());
        for (id, data) in files {
            areas.push(parse_area(id, &data)?);
        }
    }

    let mut hunts = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_HUNT)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_HUNT, &payload)?;
        hunts.reserve(files.len());
        for (id, data) in files {
            hunts.push(parse_hunt(id, &data)?);
        }
    }

    let mut mesanims = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_MESANIM)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_MESANIM, &payload)?;
        mesanims.reserve(files.len());
        for (id, data) in files {
            mesanims.push(parse_mesanim(id, &data)?);
        }
    }

    let mut itemcodes = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_ITEMCODE)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_ITEMCODE, &payload)?;
        itemcodes.reserve(files.len());
        for (id, data) in files {
            itemcodes.push(parse_itemcode(id, &data)?);
        }
    }

    let mut gamelogevents = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_GAMELOGEVENT)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_GAMELOGEVENT, &payload)?;
        gamelogevents.reserve(files.len());
        for (id, data) in files {
            gamelogevents.push(parse_gamelogevent(id, &data)?);
        }
    }

    let mut bugtemplates = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_BUGTEMPLATE)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_BUGTEMPLATE, &payload)?;
        bugtemplates.reserve(files.len());
        for (id, data) in files {
            bugtemplates.push(parse_bugtemplate(id, &data)?);
        }
    }

    let mut var_client_strings = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_VAR_CLIENT_STRING)? {
        let files =
            crate::js5::unpack_group(&config_index, CONFIG_GROUP_VAR_CLIENT_STRING, &payload)?;
        var_client_strings.reserve(files.len());
        for (id, data) in files {
            var_client_strings.push(parse_var_client_string(id, &data)?);
        }
    }

    let mut varnbits = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_VAR_NPC_BIT)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_VAR_NPC_BIT, &payload)?;
        varnbits.reserve(files.len());
        for (id, data) in files {
            varnbits.push(parse_var_npc_bit(id, &data)?);
        }
    }

    let mut vars = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_VAR_SHARED)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_VAR_SHARED, &payload)?;
        vars.reserve(files.len());
        for (id, data) in files {
            vars.push(parse_var_shared(id, &data)?);
        }
    }

    let mut var_shared_strings = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_VAR_SHARED_STRING)? {
        let files =
            crate::js5::unpack_group(&config_index, CONFIG_GROUP_VAR_SHARED_STRING, &payload)?;
        var_shared_strings.reserve(files.len());
        for (id, data) in files {
            var_shared_strings.push(parse_var_shared_string(id, &data)?);
        }
    }

    let mut underlays = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_UNDERLAY)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_UNDERLAY, &payload)?;
        underlays.reserve(files.len());
        for (id, data) in files {
            underlays.push(parse_underlay(id, &data)?);
        }
    }

    let mut overlays = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_OVERLAY)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_OVERLAY, &payload)?;
        overlays.reserve(files.len());
        for (id, data) in files {
            overlays.push(parse_overlay(id, &data)?);
        }
    }

    let mut msis = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_MSI)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_MSI, &payload)?;
        msis.reserve(files.len());
        for (id, data) in files {
            msis.push(parse_msi(id, &data)?);
        }
    }

    let mut skyboxes = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_SKYBOX)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_SKYBOX, &payload)?;
        skyboxes.reserve(files.len());
        for (id, data) in files {
            skyboxes.push(parse_skybox(id, &data)?);
        }
    }

    let mut worldareas = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_WORLDAREA)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_WORLDAREA, &payload)?;
        worldareas.reserve(files.len());
        for (id, data) in files {
            worldareas.push(parse_worldarea(id, &data)?);
        }
    }

    let mut quickchat_categories = Vec::new();
    if quickchat_archive_available
        && let Some(payload) = cache.get(
            ARCHIVE_QUICKCHAT_CONFIG,
            CONFIG_GROUP_QUICKCHATCAT_ARCHIVE24,
        )?
    {
        let quickchat_index = cache.archive_index(ARCHIVE_QUICKCHAT_CONFIG)?;
        let files = crate::js5::unpack_group(
            &quickchat_index,
            CONFIG_GROUP_QUICKCHATCAT_ARCHIVE24,
            &payload,
        )?;
        quickchat_categories.reserve(files.len());
        for (id, data) in files {
            quickchat_categories.push(parse_quickchatcat(id, &data)?);
        }
    }

    let mut headbars = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_HEADBAR)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_HEADBAR, &payload)?;
        headbars.reserve(files.len());
        for (id, data) in files {
            headbars.push(parse_headbar(id, &data)?);
        }
    }

    let mut hitmarks = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_HITMARK)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_HITMARK, &payload)?;
        hitmarks.reserve(files.len());
        for (id, data) in files {
            hitmarks.push(parse_hitmark(id, &data)?);
        }
    }

    let mut lights = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_LIGHT)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_LIGHT, &payload)?;
        lights.reserve(files.len());
        for (id, data) in files {
            lights.push(parse_light(id, &data)?);
        }
    }

    let mut quickchat_phrases = Vec::new();
    if quickchat_archive_available
        && let Some(payload) = cache.get(
            ARCHIVE_QUICKCHAT_CONFIG,
            CONFIG_GROUP_QUICKCHATPHRASE_ARCHIVE24,
        )?
    {
        let quickchat_index = cache.archive_index(ARCHIVE_QUICKCHAT_CONFIG)?;
        let files = crate::js5::unpack_group(
            &quickchat_index,
            CONFIG_GROUP_QUICKCHATPHRASE_ARCHIVE24,
            &payload,
        )?;
        quickchat_phrases.reserve(files.len());
        for (id, data) in files {
            quickchat_phrases.push(parse_quickchatphrase(id, &data)?);
        }
    }

    let mut billboards = Vec::new();
    if billboard_archive_available
        && let Some(payload) = cache.get(ARCHIVE_BILLBOARDS, CONFIG_GROUP_BILLBOARD_ARCHIVE29)?
    {
        let billboard_index = cache.archive_index(ARCHIVE_BILLBOARDS)?;
        let files =
            crate::js5::unpack_group(&billboard_index, CONFIG_GROUP_BILLBOARD_ARCHIVE29, &payload)?;
        billboards.reserve(files.len());
        for (id, data) in files {
            billboards.push(parse_billboard(id, &data)?);
        }
    }

    let mut particleeffectors = Vec::new();
    let mut particleemitters = Vec::new();
    if particle_archive_available {
        let particle_index = cache.archive_index(ARCHIVE_PARTICLES)?;
        if let Some(payload) =
            cache.get(ARCHIVE_PARTICLES, CONFIG_GROUP_PARTICLE_EMITTER_ARCHIVE27)?
        {
            let files = crate::js5::unpack_group(
                &particle_index,
                CONFIG_GROUP_PARTICLE_EMITTER_ARCHIVE27,
                &payload,
            )?;
            particleemitters.reserve(files.len());
            for (id, data) in files {
                particleemitters.push(parse_particle_emitter(id, &data)?);
            }
        }
        if let Some(payload) =
            cache.get(ARCHIVE_PARTICLES, CONFIG_GROUP_PARTICLE_EFFECTOR_ARCHIVE27)?
        {
            let files = crate::js5::unpack_group(
                &particle_index,
                CONFIG_GROUP_PARTICLE_EFFECTOR_ARCHIVE27,
                &payload,
            )?;
            particleeffectors.reserve(files.len());
            for (id, data) in files {
                particleeffectors.push(parse_particle_effector(id, &data)?);
            }
        }
    }

    let mut textures = Vec::new();
    if texture_archive_available {
        let texture_index = cache.archive_index(ARCHIVE_TEXTURES)?;
        for group in &texture_index.group_id {
            let files = cache.group_files_with_index(&texture_index, ARCHIVE_TEXTURES, *group)?;
            for (file, data) in files {
                textures.push(parse_texture(group + file, &data)?);
            }
        }
    }

    let mut stylesheets = Vec::new();
    if stylesheet_archive_available {
        let stylesheet_index = cache.archive_index(ARCHIVE_STYLESHEETS)?;
        for group in &stylesheet_index.group_id {
            let files =
                cache.group_files_with_index(&stylesheet_index, ARCHIVE_STYLESHEETS, *group)?;
            for (file, data) in files {
                stylesheets.push(parse_stylesheet(group + file, &data)?);
            }
        }
    }

    let mut structs = Vec::new();
    if struct_archive_available {
        let struct_index = cache.archive_index(ARCHIVE_STRUCT_CONFIG)?;
        for group in &struct_index.group_id {
            let files =
                cache.group_files_with_index(&struct_index, ARCHIVE_STRUCT_CONFIG, *group)?;
            for (file, data) in files {
                structs.push(parse_struct((group << 5) | file, &data)?);
            }
        }
    } else if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_STRUCT)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_STRUCT, &payload)?;
        structs.reserve(files.len());
        for (id, data) in files {
            structs.push(parse_struct(id, &data)?);
        }
    }

    let mut enums = Vec::new();
    for group in &enum_index.group_id {
        let files = cache.group_files_with_index(&enum_index, ARCHIVE_ENUM_CONFIG, *group)?;
        for (file, data) in files {
            let enum_id = (group << 8) | file;
            enums.push(parse_enum(enum_id, &data)?);
        }
    }

    if let Some(dir) = out_dir {
        write_json(&dir.join("params.json"), &params)?;
        write_json(&dir.join("enums.json"), &enums)?;
        write_json(&dir.join("dbtables.json"), &dbtables)?;
        write_json(&dir.join("dbrows.json"), &dbrows)?;
        write_json(&dir.join("idks.json"), &idks)?;
        write_json(&dir.join("locs.json"), &locs)?;
        write_json(&dir.join("npcs.json"), &npcs)?;
        write_json(&dir.join("objs.json"), &objs)?;
        write_json(&dir.join("seqs.json"), &seqs)?;
        write_json(&dir.join("spots.json"), &spots)?;
        write_json(&dir.join("bass.json"), &bass)?;
        write_json(&dir.join("quests.json"), &quests)?;
        write_json(&dir.join("mels.json"), &mels)?;
        write_json(&dir.join("waters.json"), &waters)?;
        write_json(&dir.join("achievements.json"), &achievements)?;
        write_json(&dir.join("materials.json"), &materials)?;
        write_json(&dir.join("invs.json"), &invs)?;
        write_json(&dir.join("cursors.json"), &cursors)?;
        write_json(&dir.join("seqgroups.json"), &seqgroups)?;
        write_json(&dir.join("structs.json"), &structs)?;
        write_json(&dir.join("controllers.json"), &controllers)?;
        write_json(&dir.join("categories.json"), &categories)?;
        write_json(&dir.join("areas.json"), &areas)?;
        write_json(&dir.join("hunts.json"), &hunts)?;
        write_json(&dir.join("mesanims.json"), &mesanims)?;
        write_json(&dir.join("itemcodes.json"), &itemcodes)?;
        write_json(&dir.join("gamelogevents.json"), &gamelogevents)?;
        write_json(&dir.join("bugtemplates.json"), &bugtemplates)?;
        write_json(&dir.join("varcstrs.json"), &var_client_strings)?;
        write_json(&dir.join("varnbits.json"), &varnbits)?;
        write_json(&dir.join("vars.json"), &vars)?;
        write_json(&dir.join("varsstrs.json"), &var_shared_strings)?;
        write_json(&dir.join("underlays.json"), &underlays)?;
        write_json(&dir.join("overlays.json"), &overlays)?;
        write_json(&dir.join("msis.json"), &msis)?;
        write_json(&dir.join("skyboxes.json"), &skyboxes)?;
        write_json(&dir.join("worldareas.json"), &worldareas)?;
        write_json(&dir.join("quickchatcats.json"), &quickchat_categories)?;
        write_json(&dir.join("headbars.json"), &headbars)?;
        write_json(&dir.join("hitmarks.json"), &hitmarks)?;
        write_json(&dir.join("lights.json"), &lights)?;
        write_json(&dir.join("quickchatphrases.json"), &quickchat_phrases)?;
        write_json(&dir.join("billboards.json"), &billboards)?;
        write_json(&dir.join("particleeffectors.json"), &particleeffectors)?;
        write_json(&dir.join("particleemitters.json"), &particleemitters)?;
        write_json(&dir.join("textures.json"), &textures)?;
        write_json(&dir.join("stylesheets.json"), &stylesheets)?;
    }

    print_json(&ConfigSummary {
        params: params.len(),
        enums: enums.len(),
        dbtables: dbtables.len(),
        dbrows: dbrows.len(),
        idks: idks.len(),
        locs: locs.len(),
        npcs: npcs.len(),
        objs: objs.len(),
        seqs: seqs.len(),
        spots: spots.len(),
        bass: bass.len(),
        quests: quests.len(),
        mels: mels.len(),
        waters: waters.len(),
        achievements: achievements.len(),
        materials: materials.len(),
        invs: invs.len(),
        cursors: cursors.len(),
        seqgroups: seqgroups.len(),
        structs: structs.len(),
        controllers: controllers.len(),
        categories: categories.len(),
        areas: areas.len(),
        hunts: hunts.len(),
        mesanims: mesanims.len(),
        itemcodes: itemcodes.len(),
        gamelogevents: gamelogevents.len(),
        bugtemplates: bugtemplates.len(),
        varcstrs: var_client_strings.len(),
        varnbits: varnbits.len(),
        vars: vars.len(),
        varsstrs: var_shared_strings.len(),
        underlays: underlays.len(),
        overlays: overlays.len(),
        msis: msis.len(),
        skyboxes: skyboxes.len(),
        worldareas: worldareas.len(),
        quickchatcats: quickchat_categories.len(),
        headbars: headbars.len(),
        hitmarks: hitmarks.len(),
        lights: lights.len(),
        quickchatphrases: quickchat_phrases.len(),
        billboards: billboards.len(),
        particleeffectors: particleeffectors.len(),
        particleemitters: particleemitters.len(),
        textures: textures.len(),
        stylesheets: stylesheets.len(),
    })
}

/// `cs2 port` — the semantic 948→910 CS2 port (plan §9/§10). Decodes the donor
/// closure from `cache` (the 948 flat cache), runs it through the port layer, and
/// (optionally) writes the `.asm.ts` listings + checks them byte-for-byte against
/// the committed oracle. Today only `--closure-of-interface 1224` is wired (the
/// ritual driver, the byte-exact oracle).
// reason: CLI dispatch fn — each arg is a distinct decoded flag with no natural grouping.
#[allow(clippy::too_many_arguments)]
fn run_cs2_port(
    cache: &FlatCache,
    data_dir: &Path,
    from: u32,
    to: u32,
    closure_of_interface: u32,
    base_cache_dir: Option<&Path>,
    out_dir: Option<&Path>,
    check_oracle: bool,
    json: bool,
) -> Result<()> {
    use crate::port::book::BuildDescriptor;
    use crate::port::{lodestone, material_storage, relic, ritual};

    ensure!(
        from == 948 && to == 910,
        "cs2 port currently supports only --from 948 --to 910 (got {from} → {to})"
    );
    ensure!(
        matches!(closure_of_interface, 1224 | 691 | 660 | 1092),
        "cs2 port currently supports --closure-of-interface 1224 (ritual selection), 691 \
         (relic powers), 660 (material storage), or 1092 (lodestone); got {closure_of_interface}"
    );

    let index = cache.archive_index(ARCHIVE_CLIENTSCRIPTS)?;
    let book_948 = OpcodeBook::load(data_dir, 948, 1)?;
    let d948 = BuildDescriptor::load(data_dir, 948)?;
    let d910 = BuildDescriptor::load(data_dir, 910)?;
    let source = ritual::cache_source(cache, &index, &book_948);
    let ported = match closure_of_interface {
        1224 => ritual::port_ritual_scripts(&source, &d948, &d910)?,
        691 => relic::port_relic_scripts(&source, &d948, &d910)?,
        660 => {
            // The 9239 base augmentation needs a 910-base cache.
            let base_dir = base_cache_dir
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("../../cache/unpacked/910"));
            let base_cache = FlatCache::open(&base_dir)?;
            let base_index = base_cache.archive_index(ARCHIVE_CLIENTSCRIPTS)?;
            let book_910 = OpcodeBook::load(data_dir, 910, 0)?;
            let base_source =
                ritual::flat_cache_source(&base_cache, &base_index, &book_910, 910);
            material_storage::port_material_storage_scripts(&source, &base_source, &d948, &d910)?
        }
        1092 => {
            // Lodestone patches augment 910-base scripts only.
            let base_dir = base_cache_dir
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("../../cache/unpacked/910"));
            let base_cache = FlatCache::open(&base_dir)?;
            let base_index = base_cache.archive_index(ARCHIVE_CLIENTSCRIPTS)?;
            let book_910 = OpcodeBook::load(data_dir, 910, 0)?;
            let base_source =
                ritual::flat_cache_source(&base_cache, &base_index, &book_910, 910);
            lodestone::port_lodestone_scripts(&base_source, &d910)?
        }
        other => bail!("unsupported interface {other}"),
    };

    // Optionally write the listings.
    if let Some(dir) = out_dir {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("create out dir {}", dir.display()))?;
        for p in &ported {
            let path = dir.join(format!("script{}.asm.ts", p.out_id));
            std::fs::write(&path, &p.text)
                .with_context(|| format!("write {}", path.display()))?;
        }
    }

    // The byte-exact oracle: diff each produced listing against the committed one.
    let mut mismatches: Vec<i32> = Vec::new();
    let mut checked = 0_usize;
    if check_oracle {
        let oracle_family = match closure_of_interface {
            1224 => "ritual-pedestal-948",
            691 => "relic-system-948",
            660 => "material-storage-948",
            1092 => "lodestone-948",
            other => bail!("no committed oracle for interface {other}"),
        };
        let oracle_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../server/cache-patches")
            .join(oracle_family)
            .join("scripts");
        for p in &ported {
            let committed_path = oracle_dir.join(format!("script{}.asm.ts", p.out_id));
            match std::fs::read_to_string(&committed_path) {
                Ok(committed) => {
                    checked += 1;
                    if committed != p.text {
                        mismatches.push(p.out_id);
                    }
                }
                Err(_) => mismatches.push(p.out_id),
            }
        }
    }

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "event": "cs2_port",
                "from": from,
                "to": to,
                "interface": closure_of_interface,
                "listings": ported.len(),
                "out_dir": out_dir.map(|p| p.display().to_string()),
                "oracle_checked": checked,
                "oracle_mismatches": mismatches,
                "byte_exact": mismatches.is_empty(),
            }))?
        );
    } else {
        println!(
            "cs2 port — interface {closure_of_interface} ({from}→{to}): {} listing(s)",
            ported.len()
        );
        if let Some(dir) = out_dir {
            println!("  wrote to {}", dir.display());
        }
        if check_oracle {
            if mismatches.is_empty() {
                println!("  oracle: BYTE-EXACT ({checked} listing(s) match the committed artifacts)");
            } else {
                println!(
                    "  oracle: {} of {} listing(s) DIFFER: {:?}",
                    mismatches.len(),
                    checked,
                    mismatches
                );
            }
        }
    }

    if check_oracle && !mismatches.is_empty() {
        bail!(
            "cs2 port is not byte-exact against the committed oracle ({} mismatch(es))",
            mismatches.len()
        );
    }
    Ok(())
}

/// `port <sub>` dispatch (currently `plan`).
fn run_port_command(cache: &FlatCache, data_dir: &Path, command: &PortCommand) -> Result<()> {
    match *command {
        PortCommand::Plan {
            interface,
            from,
            to,
            json,
        } => {
            let donor_pack_root = PathBuf::from(crate::pack_root::DONOR_PACK_ROOT);
            let base_pack_root = crate::explain::default_base_pack_root();
            Ok(crate::port::plan::run(&crate::port::plan::PlanOptions {
                interface,
                from,
                to,
                donor_cache: cache,
                donor_pack_root: &donor_pack_root,
                base_pack_root: &base_pack_root,
                data_dir,
                json,
            })?)
        }
    }
}

fn run_cs2(
    cache: &FlatCache,
    tar_path: &Path,
    data_dir: &Path,
    out_file: Option<&Path>,
    out_dir: Option<&Path>,
    version: RuntimeVersion,
) -> Result<()> {
    ensure_archive_complete(cache.root(), tar_path, ARCHIVE_CLIENTSCRIPTS)?;
    let cache = FlatCache::open(cache.root())?;
    let opcode_book = OpcodeBook::load(data_dir, version.build, version.subbuild)?;
    let index = cache.archive_index(ARCHIVE_CLIENTSCRIPTS)?;
    let keep_decoded = out_file.is_some();
    let script_group_names = load_script_group_names(&index, data_dir)?;

    if let Some(path) = out_dir {
        fs::create_dir_all(path).with_context(|| format!("failed creating {}", path.display()))?;
    }

    let mut scripts = 0_usize;
    let mut instructions = 0_usize;
    let mut opcode_names = HashMap::<String, usize>::new();
    let mut decoded_all = Vec::new();

    struct GroupCs2Result {
        scripts: usize,
        instructions: usize,
        opcode_counts: HashMap<String, usize>,
        decoded: Vec<CompiledScript>,
    }

    let group_results = index
        .group_id
        .par_iter()
        .map(|group| -> Result<GroupCs2Result> {
            let files = cache.group_files_with_index(&index, ARCHIVE_CLIENTSCRIPTS, *group)?;
            let single_file_group = files.len() == 1;
            let mut scripts = 0_usize;
            let mut instructions = 0_usize;
            let mut opcode_counts = HashMap::<String, usize>::new();
            let mut decoded = Vec::new();

            for (file, bytes) in files {
                let script = match decode_script(&bytes, &opcode_book, version.build) {
                    Ok(s) => s,
                    Err(e) => {
                        if version.build < MIN_SCRIPT_BUILD {
                            eprintln!("warning: skipping script {file} in group {group}: {e}");
                            continue;
                        }
                        return Err(e.into());
                    }
                };
                scripts += 1;
                instructions += script.code.len();
                for instruction in &script.code {
                    *opcode_counts
                        .entry(instruction.command.clone())
                        .or_insert(0) += 1;
                }

                if let Some(dir) = out_dir {
                    let hint = script_group_names
                        .get(group)
                        .map(String::as_str)
                        .or(script.name.as_deref())
                        .unwrap_or("script");
                    let source_name = sanitize_file_component(hint);
                    let file_name = if single_file_group {
                        format!("{group}_{source_name}.cs2")
                    } else {
                        format!("{group}_{file}_{source_name}.cs2")
                    };
                    let path = dir.join(file_name);
                    write_text(&path, &format_script_source(*group, file, &script))?;
                }

                if keep_decoded {
                    decoded.push(script);
                }
            }

            Ok(GroupCs2Result {
                scripts,
                instructions,
                opcode_counts,
                decoded,
            })
        })
        .collect::<Vec<_>>();

    for result in group_results {
        let result = result?;
        scripts += result.scripts;
        instructions += result.instructions;
        for (opcode, count) in result.opcode_counts {
            *opcode_names.entry(opcode).or_insert(0) += count;
        }
        if keep_decoded {
            decoded_all.extend(result.decoded);
        }
    }

    if let Some(path) = out_file {
        write_json(path, &decoded_all)?;
    }
    print_json(&Cs2Summary {
        scripts,
        instructions,
        unique_opcodes: opcode_names.len(),
    })
}

fn run_models(
    cache: &FlatCache,
    tar_path: &Path,
    out_file: Option<&Path>,
    out_dir: Option<&Path>,
    sample_only: bool,
    build: u32,
) -> Result<()> {
    ensure_archive_complete(cache.root(), tar_path, ARCHIVE_MODELS_RT7)?;
    let cache = FlatCache::open(cache.root())?;
    let index = cache.archive_index(ARCHIVE_MODELS_RT7)?;
    let available_groups: HashSet<u32> = index.group_id.iter().copied().collect();

    let groups: Vec<u32> = if sample_only {
        let mut sample: Vec<u32> = std::env::var("MODEL_ONLY")
            .ok()
            .map(|v| v.split(',').filter_map(|s| s.trim().parse().ok()).collect())
            .unwrap_or_else(|| {
                let mut s = (0_u32..=100).collect::<Vec<_>>();
                s.extend([1_000, 5_000, 10_000, 50_000, 100_000]);
                if let Some(last) = index.group_id.last() {
                    s.push(*last);
                }
                s
            });
        sample.sort_unstable();
        sample.dedup();
        sample.retain(|group| available_groups.contains(group));
        sample
    } else {
        index.group_id.clone()
    };

    if let Some(path) = out_dir {
        fs::create_dir_all(path).with_context(|| format!("failed creating {}", path.display()))?;
    }

    struct ModelGroupResult {
        parsed_count: usize,
        parse_errors: usize,
        parsed_model: Option<(u32, Model)>,
    }

    let keep_models = out_file.is_some();
    let group_results = groups
        .par_iter()
        .map(|group| -> Result<ModelGroupResult> {
            let files = cache.group_files_with_index(&index, ARCHIVE_MODELS_RT7, *group)?;
            let Some(bytes) = files.get(&0) else {
                return Ok(ModelGroupResult {
                    parsed_count: 0,
                    parse_errors: 0,
                    parsed_model: None,
                });
            };
            match Model::decode(bytes, build) {
                Ok(model) => {
                    if let Some(dir) = out_dir {
                        let model_path = dir.join(format!("model_{group}.json"));
                        write_json(&model_path, &model)?;
                    }
                    Ok(ModelGroupResult {
                        parsed_count: 1,
                        parse_errors: 0,
                        parsed_model: keep_models.then_some((*group, model)),
                    })
                }
                Err(_) => Ok(ModelGroupResult {
                    parsed_count: 0,
                    parse_errors: 1,
                    parsed_model: None,
                }),
            }
        })
        .collect::<Vec<_>>();

    let mut parsed = Vec::new();
    let mut parsed_count = 0_usize;
    let mut parse_errors = 0_usize;
    for result in group_results {
        let result = result?;
        parsed_count += result.parsed_count;
        parse_errors += result.parse_errors;
        if let Some(model) = result.parsed_model {
            parsed.push(model);
        }
    }

    if let Some(path) = out_file {
        write_json(path, &parsed)?;
    }
    print_json(&ModelsSummary {
        groups_parsed: parsed_count,
        parse_errors,
    })
}

fn run_unpack(
    cache: &FlatCache,
    tar_path: &Path,
    data_dir: &Path,
    out_dir: &Path,
    options: UnpackRunOptions,
    version: RuntimeVersion,
) -> Result<()> {
    let interface_dir = out_dir.join("interface");
    let config_dir = out_dir.join("config");
    let script_dir = out_dir.join("script");
    let model_dir = out_dir.join("model");
    let audio_dir = out_dir.join("audio");

    run_interfaces(cache, tar_path, Some(&interface_dir), version.build)?;
    run_varps(
        cache,
        tar_path,
        Some(&config_dir.join("varps.json")),
        VarDomainArg::All,
    )?;
    run_varbits(cache, tar_path, Some(&config_dir.join("varbits.json")))?;
    run_configs(cache, tar_path, Some(config_dir.as_path()), version.build)?;
    run_cs2(
        cache,
        tar_path,
        data_dir,
        Some(&script_dir.join("scripts.json")),
        Some(&script_dir.join("decompiled")),
        version,
    )?;

    if options.sample_models {
        run_models(
            cache,
            tar_path,
            Some(&model_dir.join("models_sample.json")),
            Some(&model_dir.join("decoded")),
            true,
            version.build,
        )?;
    } else {
        run_models(
            cache,
            tar_path,
            Some(&model_dir.join("models.json")),
            Some(&model_dir.join("decoded")),
            false,
            version.build,
        )?;
    }

    if !options.skip_audio {
        run_audio(cache, tar_path, Some(&audio_dir), options.max_audio_files)?;
    }

    run_top_level_exports(
        cache,
        tar_path,
        data_dir,
        out_dir,
        version.build,
        options.best_effort_maps,
    )?;

    Ok(())
}

fn run_top_level_exports(
    cache: &FlatCache,
    tar_path: &Path,
    data_dir: &Path,
    out_dir: &Path,
    build: u32,
    best_effort_maps: bool,
) -> Result<()> {
    let hash_names = load_other_names_map(data_dir)?;

    export_archive_raw(
        cache,
        tar_path,
        ARCHIVE_BINARY,
        &out_dir.join("binary"),
        ".dat",
        &hash_names,
    )
    .context("export binary archive")?;
    export_archive_raw(
        cache,
        tar_path,
        ARCHIVE_TTF,
        &out_dir.join("ttf"),
        ".ttf",
        &hash_names,
    )
    .context("export ttf archive")?;
    export_archive_json(
        cache,
        tar_path,
        ARCHIVE_FONTMETRICS,
        &out_dir.join("fontmetrics"),
        ".json",
        &hash_names,
        |_, _, data| parse_fontmetrics(data),
    )
    .context("export fontmetrics archive")?;
    export_archive_json(
        cache,
        tar_path,
        ARCHIVE_VFX,
        &out_dir.join("vfx"),
        ".json",
        &hash_names,
        |_, _, data| Ok(decode_vfx(data)?),
    )
    .context("export vfx archive")?;
    export_archive_json(
        cache,
        tar_path,
        ARCHIVE_ANIMATOR,
        &out_dir.join("animator"),
        ".json",
        &hash_names,
        |_, _, data| Ok(decode_animator_controller(data)?),
    )
    .context("export animator archive")?;
    export_archive_json(
        cache,
        tar_path,
        ARCHIVE_CUTSCENE2D,
        &out_dir.join("cutscene2d"),
        ".json",
        &hash_names,
        |_, _, data| Ok(decode_cutscene2d(data)?),
    )
    .context("export cutscene2d archive")?;

    export_group_json(
        cache,
        tar_path,
        ARCHIVE_UI_ANIM,
        0,
        &out_dir.join("uianimcurve"),
        ".json",
        |_, _, data| parse_uianimcurve(data),
    )
    .context("export uianimcurve group")?;
    export_group_json(
        cache,
        tar_path,
        ARCHIVE_UI_ANIM,
        1,
        &out_dir.join("uianim"),
        ".json",
        |_, _, data| parse_uianim(data),
    )
    .context("export uianim group")?;

    export_mapsquares_json(
        cache,
        tar_path,
        &out_dir.join("maps"),
        build,
        best_effort_maps,
    )
    .context("export mapsquares")?;
    export_chunk_instances_json(
        cache,
        tar_path,
        &out_dir.join("chunk-instances"),
        best_effort_maps,
    )
    .context("export chunk instances")?;
    export_defaults_text(
        cache,
        tar_path,
        DEFAULTS_GROUP_GRAPHICS,
        &out_dir.join("config/graphics.defaults"),
        |id, data| parse_graphics_defaults(id, data, build),
    )
    .context("export graphics defaults")?;
    export_defaults_text(
        cache,
        tar_path,
        DEFAULTS_GROUP_AUDIO,
        &out_dir.join("config/audio.defaults"),
        |id, data| parse_audio_defaults(id, data, build),
    )
    .context("export audio defaults")?;
    export_defaults_text(
        cache,
        tar_path,
        DEFAULTS_GROUP_WEARPOS,
        &out_dir.join("config/wearpos.defaults"),
        parse_wearpos_defaults,
    )
    .context("export wearpos defaults")?;
    export_defaults_text(
        cache,
        tar_path,
        DEFAULTS_GROUP_WORLDMAP,
        &out_dir.join("config/worldmap.defaults"),
        parse_worldmap_defaults,
    )
    .context("export worldmap defaults")?;
    export_defaults_text(
        cache,
        tar_path,
        DEFAULTS_GROUP_TITLE,
        &out_dir.join("config/title.defaults"),
        parse_title_defaults,
    )
    .context("export title defaults")?;
    export_worldmap_dump(cache, tar_path, &out_dir.join("worldmap"))
        .context("export worldmap dump")?;
    export_worldarea_png(cache, tar_path, &out_dir.join("areas.png"))
        .context("export worldarea png")?;
    Ok(())
}

fn load_other_names_map(data_dir: &Path) -> Result<HashMap<i32, String>> {
    let other = data_dir.join("names/other.txt");
    if !other.is_file() {
        return Ok(HashMap::new());
    }
    load_hash_name_map(&other)
}

fn export_archive_raw(
    cache: &FlatCache,
    tar_path: &Path,
    archive: u32,
    out_dir: &Path,
    extension: &str,
    hash_names: &HashMap<i32, String>,
) -> Result<usize> {
    if ensure_archive_complete(cache.root(), tar_path, archive).is_err() {
        return Ok(0);
    }

    fs::create_dir_all(out_dir)
        .with_context(|| format!("failed creating {}", out_dir.display()))?;
    let index = cache.archive_index(archive)?;
    let mut count = 0_usize;

    for group in &index.group_id {
        let files = cache.group_files_with_index(&index, archive, *group)?;
        let group_name = resolve_group_name(&index, *group, hash_names);
        if files.len() == 1 && files.contains_key(&0) {
            let mut name = group_name
                .or_else(|| resolve_file_name(&index, *group, 0, hash_names))
                .unwrap_or_else(|| group.to_string());
            if !name.ends_with(extension) {
                name.push_str(extension);
            }
            write_binary(
                &out_dir.join(sanitize_path_component(&name)),
                files[&0].as_slice(),
            )?;
            count += 1;
            continue;
        }

        let group_dir = out_dir.join(
            group_name
                .as_deref()
                .map(sanitize_path_component)
                .unwrap_or_else(|| group.to_string()),
        );
        fs::create_dir_all(&group_dir)
            .with_context(|| format!("failed creating {}", group_dir.display()))?;

        for (file, data) in files {
            let mut name = resolve_file_name(&index, *group, file, hash_names)
                .unwrap_or_else(|| file.to_string());
            if !name.ends_with(extension) {
                name.push_str(extension);
            }
            write_binary(&group_dir.join(sanitize_path_component(&name)), &data)?;
            count += 1;
        }
    }

    Ok(count)
}

fn export_archive_json<T, F>(
    cache: &FlatCache,
    tar_path: &Path,
    archive: u32,
    out_dir: &Path,
    extension: &str,
    hash_names: &HashMap<i32, String>,
    parse: F,
) -> Result<usize>
where
    T: Serialize,
    F: Fn(u32, u32, &[u8]) -> Result<T>,
{
    if ensure_archive_complete(cache.root(), tar_path, archive).is_err() {
        return Ok(0);
    }

    fs::create_dir_all(out_dir)
        .with_context(|| format!("failed creating {}", out_dir.display()))?;
    let index = cache.archive_index(archive)?;
    let mut count = 0_usize;

    for group in &index.group_id {
        let files = cache.group_files_with_index(&index, archive, *group)?;
        let group_name = resolve_group_name(&index, *group, hash_names);
        if files.len() == 1 && files.contains_key(&0) {
            let mut name = group_name
                .or_else(|| resolve_file_name(&index, *group, 0, hash_names))
                .unwrap_or_else(|| group.to_string());
            if !name.ends_with(extension) {
                name.push_str(extension);
            }
            let parsed = parse(*group, 0, files[&0].as_slice())?;
            write_json(&out_dir.join(sanitize_path_component(&name)), &parsed)?;
            count += 1;
            continue;
        }

        let group_dir = out_dir.join(
            group_name
                .as_deref()
                .map(sanitize_path_component)
                .unwrap_or_else(|| group.to_string()),
        );
        fs::create_dir_all(&group_dir)
            .with_context(|| format!("failed creating {}", group_dir.display()))?;

        for (file, data) in files {
            let mut name = resolve_file_name(&index, *group, file, hash_names)
                .unwrap_or_else(|| file.to_string());
            if !name.ends_with(extension) {
                name.push_str(extension);
            }
            let parsed = parse(*group, file, &data)?;
            write_json(&group_dir.join(sanitize_path_component(&name)), &parsed)?;
            count += 1;
        }
    }

    Ok(count)
}

fn export_group_json<T, F>(
    cache: &FlatCache,
    tar_path: &Path,
    archive: u32,
    group: u32,
    out_dir: &Path,
    extension: &str,
    parse: F,
) -> Result<usize>
where
    T: Serialize,
    F: Fn(u32, u32, &[u8]) -> Result<T>,
{
    if ensure_archive_complete(cache.root(), tar_path, archive).is_err() {
        return Ok(0);
    }
    let Some(_payload) = cache.get(archive, group)? else {
        return Ok(0);
    };

    fs::create_dir_all(out_dir)
        .with_context(|| format!("failed creating {}", out_dir.display()))?;
    let index = cache.archive_index(archive)?;
    let files = cache.group_files_with_index(&index, archive, group)?;
    let mut count = 0_usize;
    for (file, data) in files {
        let parsed = parse(group, file, &data)?;
        let path = out_dir.join(format!("{file}{extension}"));
        write_json(&path, &parsed)?;
        count += 1;
    }
    Ok(count)
}

fn export_mapsquares_json(
    cache: &FlatCache,
    tar_path: &Path,
    out_dir: &Path,
    build: u32,
    best_effort: bool,
) -> Result<usize> {
    if ensure_archive_complete(cache.root(), tar_path, ARCHIVE_MAPSQUARES).is_err() {
        return Ok(0);
    }
    fs::create_dir_all(out_dir)
        .with_context(|| format!("failed creating {}", out_dir.display()))?;

    let index = cache.archive_index(ARCHIVE_MAPSQUARES)?;
    let mut count = 0_usize;
    for group in &index.group_id {
        let files = cache.group_files_with_index(&index, ARCHIVE_MAPSQUARES, *group)?;
        let square_x = group & 0b111_1111;
        let square_z = group >> 7;
        let decoded = if best_effort {
            decode_map_square_best_effort(&files, build)
        } else {
            decode_map_square(&files, build).with_context(|| {
                format!("decode mapsquare group {group} ({square_x}_{square_z})")
            })?
        };
        let path = out_dir.join(format!("{square_x}_{square_z}.json"));
        write_json(&path, &decoded)?;
        count += 1;
    }

    Ok(count)
}

fn export_chunk_instances_json(
    cache: &FlatCache,
    tar_path: &Path,
    out_dir: &Path,
    best_effort: bool,
) -> Result<usize> {
    if ensure_archive_complete(cache.root(), tar_path, ARCHIVE_CHUNK_INSTANCES).is_err() {
        return Ok(0);
    }
    fs::create_dir_all(out_dir)
        .with_context(|| format!("failed creating {}", out_dir.display()))?;

    let index = cache.archive_index(ARCHIVE_CHUNK_INSTANCES)?;
    let mut count = 0_usize;
    for group in &index.group_id {
        let files = cache.group_files_with_index(&index, ARCHIVE_CHUNK_INSTANCES, *group)?;
        let Some(data) = files.get(&0) else {
            continue;
        };
        let decoded = match decode_chunk_instance_stream(data) {
            Ok(decoded) => decoded,
            Err(err) if best_effort => {
                eprintln!("chunk instance decode warning group {group}: {err}");
                continue;
            }
            Err(err) => return Err(err).with_context(|| format!("decode chunk instance {group}")),
        };
        let path = out_dir.join(format!("{group}.json"));
        write_json(&path, &decoded)?;
        count += 1;
    }

    Ok(count)
}

fn export_defaults_text<F>(
    cache: &FlatCache,
    tar_path: &Path,
    group: u32,
    out_file: &Path,
    parse: F,
) -> Result<usize>
where
    F: Fn(u32, &[u8]) -> Result<Vec<String>>,
{
    if ensure_archive_complete(cache.root(), tar_path, ARCHIVE_DEFAULTS).is_err() {
        return Ok(0);
    }
    let index = cache.archive_index(ARCHIVE_DEFAULTS)?;
    if !index.group_id.contains(&group) {
        return Ok(0);
    }

    let files = cache.group_files_with_index(&index, ARCHIVE_DEFAULTS, group)?;
    if files.is_empty() {
        return Ok(0);
    }

    let mut file_ids = files.keys().copied().collect::<Vec<_>>();
    file_ids.sort_unstable();

    let mut lines = Vec::new();
    for file in &file_ids {
        let data = files
            .get(file)
            .with_context(|| format!("missing defaults file {file} in group {group}"))?;
        lines.extend(parse(*file, data)?);
        lines.push(String::new());
    }

    if let Some(parent) = out_file.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed creating {}", parent.display()))?;
    }
    write_text(out_file, &lines.join("\n"))?;
    Ok(file_ids.len())
}

fn resolve_group_name(
    index: &crate::js5::ArchiveIndex,
    group: u32,
    hash_names: &HashMap<i32, String>,
) -> Option<String> {
    let names = index.group_name_hash.as_ref()?;
    let group_idx = usize::try_from(group).ok()?;
    let hash = *names.get(group_idx)?;
    if hash == -1 {
        return None;
    }
    hash_names.get(&hash).cloned()
}

fn resolve_file_name(
    index: &crate::js5::ArchiveIndex,
    group: u32,
    file: u32,
    hash_names: &HashMap<i32, String>,
) -> Option<String> {
    let group_names = index.group_file_names.as_ref()?;
    let group_idx = usize::try_from(group).ok()?;
    let file_idx = usize::try_from(file).ok()?;
    let file_hashes = group_names.get(group_idx)?.as_ref()?;
    let hash = *file_hashes.get(file_idx)?;
    if hash == -1 {
        return None;
    }
    hash_names.get(&hash).cloned()
}

fn sanitize_path_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '[' | ']' | ',') {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        String::from("unnamed")
    } else {
        out
    }
}

fn export_worldmap_dump(cache: &FlatCache, tar_path: &Path, out_dir: &Path) -> Result<()> {
    if ensure_archive_complete(cache.root(), tar_path, ARCHIVE_WORLDMAP).is_err() {
        return Ok(());
    }
    fs::create_dir_all(out_dir)
        .with_context(|| format!("failed creating {}", out_dir.display()))?;
    let index = cache.archive_index(ARCHIVE_WORLDMAP)?;
    if let Some(main_group) = find_group_by_name(&index, "main")
        && let Some(details_file) = find_file_by_name(&index, main_group, "details.dat")
        && let Some(labels_file) = find_file_by_name(&index, main_group, "labels.dat")
    {
        let lines = export_worldmap_legacy(cache, &index, main_group, details_file, labels_file)?;
        write_text(&out_dir.join("dump.wma"), &lines.join("\n"))?;
        return Ok(());
    }

    let details_group = find_group_by_name(&index, "details").unwrap_or(0);
    let Some(payload) = cache.get(ARCHIVE_WORLDMAP, details_group)? else {
        return Ok(());
    };
    let details_files = crate::js5::unpack_group(&index, details_group, &payload)?;
    let mut lines = Vec::new();
    for (id, data) in details_files {
        let debug_name = unpack_worldmap_details(id, &data, &mut lines)?;
        unpack_worldmap_static_elements(cache, &index, &debug_name, &mut lines)?;
        unpack_worldmap_labels(cache, &index, &debug_name, &mut lines)?;
        lines.push(String::new());
    }

    write_text(&out_dir.join("dump.wma"), &lines.join("\n"))?;
    Ok(())
}

fn export_worldmap_legacy(
    cache: &FlatCache,
    index: &crate::js5::ArchiveIndex,
    group: u32,
    details_file: u32,
    labels_file: u32,
) -> Result<Vec<String>> {
    let Some(payload) = cache.get(ARCHIVE_WORLDMAP, group)? else {
        return Ok(Vec::new());
    };
    let files = crate::js5::unpack_group(index, group, &payload)?;
    let details = files.get(&details_file).with_context(|| {
        format!("legacy worldmap missing details file {details_file} in group {group}")
    })?;
    let labels = files.get(&labels_file).with_context(|| {
        format!("legacy worldmap missing labels file {labels_file} in group {group}")
    })?;

    let mut detail_packet = crate::packet::Packet::new(details);
    let mut lines = Vec::new();
    lines.push(String::from("[main]"));
    lines.push(format!(
        "origin={},{}",
        detail_packet.g2()?,
        detail_packet.g2()?
    ));
    lines.push(format!(
        "min={},{}",
        detail_packet.g2()?,
        detail_packet.g2()?
    ));
    lines.push(format!(
        "max={},{}",
        detail_packet.g2()?,
        detail_packet.g2()?
    ));
    lines.push(String::new());

    let mut label_packet = crate::packet::Packet::new(labels);
    let label_count = usize::from(label_packet.g2()?);
    for _ in 0..label_count {
        let text = label_packet.gjstr()?;
        let x = label_packet.g2()?;
        let y = label_packet.g2()?;
        let kind = label_packet.g1()?;
        lines.push(format!("label={x},{y},{text},{kind}"));
    }
    Ok(lines)
}

fn unpack_worldmap_details(id: u32, data: &[u8], lines: &mut Vec<String>) -> Result<String> {
    let mut packet = crate::packet::Packet::new(data);
    let debug_name = packet.gjstr()?;
    lines.push(format!("[{debug_name}]"));
    lines.push(format!("name={}", packet.gjstr()?));
    lines.push(format!("origin={}", format_coordgrid(packet.g4s()?)));
    lines.push(format!("background={}", format_colour(packet.g4s()?)));
    lines.push(format!("listed={}", yes_no(packet.g1()? == 1)));
    let default_zoom = packet.g1()?;
    lines.push(if default_zoom == u8::MAX {
        String::from("zoom=default")
    } else {
        format!("zoom={default_zoom}")
    });
    lines.push(format!("buildarea={}", packet.g1()?));
    let count = usize::from(packet.g1()?);
    for _ in 0..count {
        lines.push(format!(
            "subarea={},{},{},{},{},{},{},{},{}",
            packet.g1()?,
            packet.g2()?,
            packet.g2()?,
            packet.g2()?,
            packet.g2()?,
            packet.g2()?,
            packet.g2()?,
            packet.g2()?,
            packet.g2()?
        ));
    }
    if !packet.is_done() {
        bail!("worldmap details {id} did not consume full payload");
    }
    Ok(debug_name)
}

fn unpack_worldmap_static_elements(
    cache: &FlatCache,
    index: &crate::js5::ArchiveIndex,
    debug_name: &str,
    lines: &mut Vec<String>,
) -> Result<()> {
    let Some(group) = find_group_by_name(index, &format!("{debug_name}_staticelements")) else {
        return Ok(());
    };
    let Some(payload) = cache.get(ARCHIVE_WORLDMAP, group)? else {
        return Ok(());
    };
    let files = crate::js5::unpack_group(index, group, &payload)?;
    for (_, data) in files {
        let mut packet = crate::packet::Packet::new(&data);
        lines.push(format!(
            "element={},{},{}",
            format_coordgrid(packet.g4s()?),
            format_map_element(packet.g2()?),
            packet.g1()?
        ));
    }
    Ok(())
}

fn unpack_worldmap_labels(
    cache: &FlatCache,
    index: &crate::js5::ArchiveIndex,
    debug_name: &str,
    lines: &mut Vec<String>,
) -> Result<()> {
    let Some(group) = find_group_by_name(index, &format!("{debug_name}_labels")) else {
        return Ok(());
    };
    let Some(payload) = cache.get(ARCHIVE_WORLDMAP, group)? else {
        return Ok(());
    };
    let files = crate::js5::unpack_group(index, group, &payload)?;
    for (_, data) in files {
        let mut packet = crate::packet::Packet::new(&data);
        lines.push(format!(
            "label={},{},{}",
            format_coordgrid(packet.g4s()?),
            format_map_element(packet.g2()?),
            packet.g1()?
        ));
    }
    Ok(())
}

fn format_coordgrid(value: i32) -> String {
    if value == -1 {
        return String::from("null");
    }
    let as_u32 = value as u32;
    let level = as_u32 >> 28;
    let x = (as_u32 >> 14) & 0x3fff;
    let z = as_u32 & 0x3fff;
    format!("{level}_{}_{}_{}_{}", x / 64, z / 64, x % 64, z % 64)
}

fn format_colour(value: i32) -> String {
    let as_u32 = value as u32;
    if as_u32 > 0x00ff_ffff {
        format!("0x{as_u32:08x}")
    } else {
        format!("0x{as_u32:06x}")
    }
}

fn format_map_element(value: u16) -> String {
    format!("mapelement_{value}")
}

fn find_group_by_name(index: &crate::js5::ArchiveIndex, name: &str) -> Option<u32> {
    let hash = java_string_hash(name);
    let hashes = index.group_name_hash.as_ref()?;
    index.group_id.iter().copied().find(|group| {
        usize::try_from(*group)
            .ok()
            .and_then(|idx| hashes.get(idx))
            .is_some_and(|value| *value == hash)
    })
}

fn find_file_by_name(index: &crate::js5::ArchiveIndex, group: u32, name: &str) -> Option<u32> {
    let hash = java_string_hash(name);
    let group_idx = usize::try_from(group).ok()?;
    let names = index.group_file_names.as_ref()?.get(group_idx)?.as_ref()?;
    names.iter().enumerate().find_map(|(file, entry_hash)| {
        if *entry_hash == hash {
            u32::try_from(file).ok()
        } else {
            None
        }
    })
}

fn export_worldarea_png(cache: &FlatCache, tar_path: &Path, out_file: &Path) -> Result<()> {
    if ensure_archive_complete(cache.root(), tar_path, ARCHIVE_WORLDMAP).is_err() {
        return Ok(());
    }
    let index = cache.archive_index(ARCHIVE_WORLDMAP)?;
    let Some(payload) = cache.get(ARCHIVE_WORLDMAP, 3)? else {
        return Ok(());
    };
    let files = crate::js5::unpack_group(&index, 3, &payload)?;

    let width = 128_usize * 8;
    let height = 256_usize * 8;
    let mut image = vec![0_u8; width * height * 3];

    for (file, data) in files {
        let square_x = usize::try_from(file & 0x7f).context("square_x overflow")?;
        let square_z = usize::try_from(file >> 7).context("square_z overflow")?;
        let colors = decode_worldmap_color(&data)?;
        for zone_x in 0..8_usize {
            for zone_z in 0..8_usize {
                let x = 8 * square_x + zone_x;
                let z = 8 * square_z + zone_z;
                if x >= width || z >= height {
                    continue;
                }
                let color = colors[8 * zone_x + zone_z];
                let offset = ((height - 1 - z) * width + x) * 3;
                image[offset] = u8::try_from((color >> 16) & 0xff).context("red overflow")?;
                image[offset + 1] = u8::try_from((color >> 8) & 0xff).context("green overflow")?;
                image[offset + 2] = u8::try_from(color & 0xff).context("blue overflow")?;
            }
        }
    }

    let width_u32 = u32::try_from(width).context("worldarea png width overflow")?;
    let height_u32 = u32::try_from(height).context("worldarea png height overflow")?;
    let Some(buffer) = ImageBuffer::<Rgb<u8>, Vec<u8>>::from_raw(width_u32, height_u32, image)
    else {
        bail!("failed to build worldarea image buffer");
    };
    if let Some(parent) = out_file.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed creating {}", parent.display()))?;
    }
    buffer
        .save(out_file)
        .with_context(|| format!("failed writing {}", out_file.display()))?;
    Ok(())
}

fn decode_worldmap_color(data: &[u8]) -> Result<[u32; 64]> {
    let mut result = [0_u32; 64];
    let mut packet = crate::packet::Packet::new(data);
    let mut index = 0_usize;
    let mut target = 0_usize;

    while target < 64 {
        let value = packet.g3()?;
        if packet.is_done() {
            target = 64;
        } else {
            target = target
                .checked_add(usize::from(packet.g1()?))
                .context("worldmap color run overflow")?;
        }
        while index < target && index < 64 {
            result[index] = value;
            index += 1;
        }
    }

    Ok(result)
}

fn parse_defaults_eof(kind: &str, id: u32, packet: &crate::packet::Packet<'_>) -> Result<()> {
    if packet.is_done() {
        return Ok(());
    }
    bail!("{kind}_{id} end of file not reached")
}

fn parse_audio_defaults(id: u32, data: &[u8], build: u32) -> Result<Vec<String>> {
    let mut packet = crate::packet::Packet::new(data);
    let mut lines = vec![format!("[audiodefaults_{id}]")];
    loop {
        match packet.g1()? {
            0 => {
                parse_defaults_eof("audiodefaults", id, &packet)?;
                return Ok(lines);
            }
            1 => {
                let song = if build >= 912 {
                    packet.g4s()?
                } else {
                    i32::from(packet.g2()?)
                };
                lines.push(format!("titlescreensong={song}"));
            }
            opcode => bail!("audiodefaults_{id} unknown opcode {opcode}"),
        }
    }
}

fn format_wearpos(slot: u8) -> Result<&'static str> {
    let value = match slot {
        0 => "hat",
        1 => "back",
        2 => "front",
        3 => "righthand",
        4 => "torso",
        5 => "lefthand",
        6 => "arms",
        7 => "legs",
        8 => "head",
        9 => "hands",
        10 => "feet",
        11 => "jaw",
        12 => "ring",
        13 => "quiver",
        14 => "aura",
        15 => "wearpos_15",
        16 => "wearpos_16",
        17 => "pocket",
        18 => "wings",
        value => bail!("wearpos {value}"),
    };
    Ok(value)
}

fn parse_wearpos_defaults(id: u32, data: &[u8]) -> Result<Vec<String>> {
    let mut packet = crate::packet::Packet::new(data);
    let mut lines = vec![format!("[wearposdefaults_{id}]")];
    loop {
        match packet.g1()? {
            0 => {
                parse_defaults_eof("wearposdefaults", id, &packet)?;
                return Ok(lines);
            }
            1 => {
                let count = usize::from(packet.g1()?);
                let mut values = Vec::with_capacity(count);
                for _ in 0..count {
                    values.push(packet.g1()?.to_string());
                }
                lines.push(format!("unknown1={}", values.join(",")));
            }
            3 => lines.push(format!("lefthand={}", format_wearpos(packet.g1()?)?)),
            4 => lines.push(format!("righthand={}", format_wearpos(packet.g1()?)?)),
            5 => {
                let count = usize::from(packet.g1()?);
                let mut values = Vec::with_capacity(count);
                for _ in 0..count {
                    values.push(format_wearpos(packet.g1()?)?.to_string());
                }
                lines.push(format!("lefthandextra={}", values.join(",")));
            }
            6 => {
                let count = usize::from(packet.g1()?);
                let mut values = Vec::with_capacity(count);
                for _ in 0..count {
                    values.push(format_wearpos(packet.g1()?)?.to_string());
                }
                lines.push(format!("righthandextra={}", values.join(",")));
            }
            opcode => bail!("wearposdefaults_{id} unknown opcode {opcode}"),
        }
    }
}

fn parse_worldmap_defaults(id: u32, data: &[u8]) -> Result<Vec<String>> {
    let mut packet = crate::packet::Packet::new(data);
    let mut lines = vec![format!("[worldmapdefaults_{id}]")];
    loop {
        match packet.g1()? {
            0 => {
                parse_defaults_eof("worldmapdefaults", id, &packet)?;
                return Ok(lines);
            }
            1 => lines.push(format!("unknown1={}", packet.g4s()?)),
            2 => lines.push(format!("membersfillcolour=0x{:x}", packet.g4s()? as u32)),
            3 => lines.push(format!("membersbordercolour=0x{:x}", packet.g4s()? as u32)),
            4 => lines.push(format!("membersborderthickness={}", packet.g1()?)),
            5 => lines.push(format!("memberschamferwidth={}", packet.g1()?)),
            6 => lines.push(format!("mainarea={}", packet.g4s()?)),
            7 => lines.push(format!("textshadowcolour=0x{:x}", packet.g4s()? as u32)),
            100 => lines.push(format!("font0zoom0={}", packet.g2()?)),
            101 => lines.push(format!("font1zoom0={}", packet.g2()?)),
            102 => lines.push(format!("font2zoom0={}", packet.g2()?)),
            108 => lines.push(format!("font0zoom1={}", packet.g2()?)),
            109 => lines.push(format!("font1zoom1={}", packet.g2()?)),
            110 => lines.push(format!("font2zoom1={}", packet.g2()?)),
            116 => lines.push(format!("font0zoom2={}", packet.g2()?)),
            117 => lines.push(format!("font1zoom2={}", packet.g2()?)),
            118 => lines.push(format!("font2zoom2={}", packet.g2()?)),
            124 => lines.push(format!("font0zoom3={}", packet.g2()?)),
            125 => lines.push(format!("font1zoom3={}", packet.g2()?)),
            126 => lines.push(format!("font2zoom3={}", packet.g2()?)),
            132 => lines.push(format!("font0zoom4={}", packet.g2()?)),
            133 => lines.push(format!("font1zoom4={}", packet.g2()?)),
            134 => lines.push(format!("font2zoom4={}", packet.g2()?)),
            opcode => bail!("worldmapdefaults_{id} unknown opcode {opcode}"),
        }
    }
}

fn parse_title_defaults(id: u32, data: &[u8]) -> Result<Vec<String>> {
    let mut packet = crate::packet::Packet::new(data);
    let mut lines = vec![format!("[titledefaults_{id}]")];
    loop {
        match packet.g1()? {
            0 => {
                parse_defaults_eof("titledefaults", id, &packet)?;
                return Ok(lines);
            }
            1 => lines.push(format!(
                "title={},{}",
                packet.gsmart2or4null()?,
                packet.gsmart2or4null()?
            )),
            2 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    lines.push(format!("unknown2={},{}", packet.g1()?, packet.g1()?));
                }
            }
            opcode => bail!("titledefaults_{id} unknown opcode {opcode}"),
        }
    }
}

fn parse_graphics_defaults(id: u32, data: &[u8], build: u32) -> Result<Vec<String>> {
    let mut packet = crate::packet::Packet::new(data);
    let mut lines = vec![format!("[graphicsdefaults_{id}]")];
    let mut hitmark_count = 4_u8;

    loop {
        match packet.g1()? {
            0 => {
                parse_defaults_eof("graphicsdefaults", id, &packet)?;
                return Ok(lines);
            }
            1 => {
                for i in 0..hitmark_count {
                    lines.push(format!("hitmark{i}pos={},{}", packet.g2s()?, packet.g2s()?));
                }
            }
            2 => {
                let model = if build < 681 {
                    packet.g2null()?
                } else {
                    packet.gsmart2or4null()?
                };
                lines.push(format!("performancemetricsmodel={model}"));
            }
            3 => {
                hitmark_count = packet.g1()?;
                lines.push(format!("hitmarkcount={hitmark_count}"));
            }
            4 => lines.push(String::from("unknown4=no")),
            5 => lines.push(format!("titleinterface={}", packet.g3()?)),
            6 => lines.push(format!("lobbyinterface={}", packet.g3()?)),
            7 => {
                for i in 0..10_u8 {
                    for j in 0..4_u8 {
                        lines.push(format!("playerrecol{i}s{j}={}", packet.g2null()?));
                        let count = usize::from(packet.g2()?);
                        let mut values = Vec::with_capacity(count);
                        for _ in 0..count {
                            values.push(packet.g2null()?.to_string());
                        }
                        lines.push(format!("playerrecol{i}d{j}={}", values.join(",")));
                    }
                }
            }
            8 => lines.push(String::from("npcchatline=no")),
            9 => lines.push(format!("npcchatlineduration={}", packet.g1()?)),
            10 => lines.push(String::from("playerchatline=no")),
            11 => lines.push(format!("playerchatlineduration={}", packet.g1()?)),
            12 => lines.push(format!("initialsize={},{}", packet.g2()?, packet.g2()?)),
            13 => lines.push(format!("headbarcount={}", packet.g1()?)),
            14 => lines.push(format!("headbarupdatecount={}", packet.g1()?)),
            15 => lines.push(format!("entityoverlayoffset={}", packet.g1()?)),
            16 => lines.push(String::from("somethingcamera=yes")),
            17 => lines.push(format!("objnumcolour=0x{:x}", packet.g4s()? as u32)),
            18 => lines.push(format!("objnumcolourk=0x{:x}", packet.g4s()? as u32)),
            19 => lines.push(format!("objnumcolourm=0x{:x}", packet.g4s()? as u32)),
            20 => lines.push(format!(
                "spotshadowtexture={},{}",
                packet.g2()?,
                packet.g1()?
            )),
            21 => lines.push(format!("minimapscale={}", packet.g1()?)),
            22 => {
                let p11full = packet.gsmart2or4null()?;
                let p12full = packet.gsmart2or4null()?;
                let b12full = packet.gsmart2or4null()?;
                let hintheadicon = packet.gsmart2or4null()?;
                let hintmapmarker = packet.gsmart2or4null()?;
                let mapflag = packet.gsmart2or4null()?;
                let mapflag_origin = (packet.g1s()?, packet.g1s()?);
                let cross = packet.gsmart2or4null()?;
                let mapdot = packet.gsmart2or4null()?;
                let nameicon = packet.gsmart2or4null()?;
                let floorshadow = packet.gsmart2or4null()?;
                let compass = packet.gsmart2or4null()?;
                let otherlevel = packet.gsmart2or4null()?;
                let mapedge = packet.gsmart2or4null()?;
                lines.push(format!(
                    "sprites={p11full},{p12full},{b12full},{hintheadicon},{hintmapmarker},{mapflag},{},{},{cross},{mapdot},{nameicon},{floorshadow},{compass},{otherlevel},{mapedge}",
                    mapflag_origin.0, mapflag_origin.1
                ));
            }
            23 => {
                for i in 0..10_u8 {
                    for j in 0..4_u8 {
                        lines.push(format!("playerretex{i}s{j}={}", packet.g2null()?));
                        let count = usize::from(packet.g2()?);
                        let mut values = Vec::with_capacity(count);
                        for _ in 0..count {
                            values.push(packet.g2null()?.to_string());
                        }
                        lines.push(format!("playerretex{i}d{j}={}", values.join(",")));
                    }
                }
            }
            24 => lines.push(format!("unknown24={}", packet.g4s()?)),
            25 => lines.push(format!(
                "unknown25={},{},{},{},{},{}",
                packet.gsmart2or4null()?,
                packet.gsmart2or4null()?,
                packet.gsmart2or4null()?,
                packet.gsmart2or4null()?,
                packet.gsmart2or4null()?,
                packet.gsmart2or4null()?
            )),
            26 => lines.push(format!("objnumcolourb=0x{:x}", packet.g4s()? as u32)),
            27 => lines.push(format!("objnumcolourt=0x{:x}", packet.g4s()? as u32)),
            28 => lines.push(format!("objnumcolourq=0x{:x}", packet.g4s()? as u32)),
            29 => lines.push(format!("unknown29={},{}", packet.g4s()?, packet.g4s()?)),
            opcode => bail!("graphicsdefaults_{id} unknown opcode {opcode}"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct UiAnimCurveEntry {
    keyframes: Vec<[f32; 4]>,
}

fn parse_uianimcurve(data: &[u8]) -> Result<UiAnimCurveEntry> {
    let mut packet = crate::packet::Packet::new(data);
    let count = usize::from(packet.g1()?);
    let mut keyframes = Vec::with_capacity(count);
    for _ in 0..count {
        keyframes.push([
            read_f32_be(&mut packet)?,
            read_f32_be(&mut packet)?,
            read_f32_be(&mut packet)?,
            read_f32_be(&mut packet)?,
        ]);
    }
    if !packet.is_done() {
        bail!("uianimcurve did not consume full payload");
    }
    Ok(UiAnimCurveEntry { keyframes })
}

#[derive(Clone, Debug, Serialize)]
struct UiAnimEntry {
    mode: u8,
    curve: Option<i32>,
    easing_type: Option<i32>,
    easing_unknown: bool,
    target: u8,
    target_mode: u8,
    values: Vec<Vec<i32>>,
}

fn parse_uianim(data: &[u8]) -> Result<UiAnimEntry> {
    let mut packet = crate::packet::Packet::new(data);
    let mode = packet.g1()?;
    let (curve, easing_type, easing_unknown) = match mode {
        1 => (Some(packet.g4s()?), None, false),
        2 => (None, Some(packet.g4s()?), packet.g1()? == 1),
        value => bail!("unknown uianim mode {value}"),
    };

    let target = packet.g1()?;
    let target_mode = packet.g1()?;
    let count = usize::from(packet.g2()?);
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        if target == 0 || target == 3 {
            values.push(vec![packet.g4s()?, packet.g4s()?]);
        } else if target == 6 {
            values.push(vec![packet.g4s()?, packet.g4s()?, packet.g4s()?]);
        } else {
            values.push(vec![packet.g4s()?]);
        }
    }
    if !packet.is_done() {
        bail!("uianim did not consume full payload");
    }

    Ok(UiAnimEntry {
        mode,
        curve,
        easing_type,
        easing_unknown,
        target,
        target_mode,
        values,
    })
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum FontSourceType {
    SpriteBitmap,
    SpriteFontsheet,
    Vector,
}

#[derive(Clone, Debug, Serialize)]
struct FontGlyphInfo {
    width: u8,
    height: u8,
    bearing_y: u8,
}

#[derive(Clone, Debug, Serialize)]
struct FontSheetPosition {
    x: u16,
    y: u16,
}

#[derive(Clone, Debug, Serialize)]
struct FontKerningData {
    left_kern: Vec<Vec<i8>>,
    right_kern: Vec<Vec<i8>>,
}

#[derive(Clone, Debug, Serialize)]
struct FontMetricsEntry {
    source_type: FontSourceType,
    source_pack_id: Option<i32>,
    pixel_size: Option<u8>,
    glyph_info: Vec<FontGlyphInfo>,
    font_sheet_width: Option<u16>,
    font_sheet_height: Option<u16>,
    font_sheet_position: Vec<FontSheetPosition>,
    base_line: Option<u8>,
    upper_case_ascent: Option<u8>,
    byte3049: Option<u8>,
    max_ascent: Option<u8>,
    max_descent: Option<u8>,
    scale: Option<u8>,
    kerning_data: Option<FontKerningData>,
}

fn parse_fontmetrics(data: &[u8]) -> Result<FontMetricsEntry> {
    let mut packet = crate::packet::Packet::new(data);
    let source_type = match packet.g1()? {
        0 => FontSourceType::SpriteBitmap,
        1 => FontSourceType::SpriteFontsheet,
        2 => FontSourceType::Vector,
        value => bail!("invalid font source type id {value}"),
    };

    match source_type {
        FontSourceType::Vector => {
            let entry = FontMetricsEntry {
                source_type,
                source_pack_id: Some(packet.g4s()?),
                pixel_size: Some(packet.g1()?),
                glyph_info: Vec::new(),
                font_sheet_width: None,
                font_sheet_height: None,
                font_sheet_position: Vec::new(),
                base_line: None,
                upper_case_ascent: None,
                byte3049: None,
                max_ascent: None,
                max_descent: None,
                scale: None,
                kerning_data: None,
            };
            if !packet.is_done() {
                bail!("fontmetrics vector did not consume full payload");
            }
            Ok(entry)
        }
        FontSourceType::SpriteBitmap | FontSourceType::SpriteFontsheet => {
            let complex_kerning = packet.g1()? == 1;
            let source_pack_id = match source_type {
                FontSourceType::SpriteFontsheet => Some(packet.g4s()?),
                FontSourceType::SpriteBitmap | FontSourceType::Vector => None,
            };

            let mut glyph_info = vec![
                FontGlyphInfo {
                    width: 0,
                    height: 0,
                    bearing_y: 0,
                };
                256
            ];
            for glyph in &mut glyph_info {
                glyph.width = packet.g1()?;
            }
            for glyph in &mut glyph_info {
                glyph.height = packet.g1()?;
            }
            for glyph in &mut glyph_info {
                glyph.bearing_y = packet.g1()?;
            }

            let font_sheet_width = packet.g2()?;
            let font_sheet_height = packet.g2()?;
            let mut positions = vec![FontSheetPosition { x: 0, y: 0 }; 256];
            for item in &mut positions {
                item.x = packet.g2()?;
            }
            for item in &mut positions {
                item.y = packet.g2()?;
            }

            let kerning_data = if complex_kerning {
                Some(parse_font_kerning(&mut packet)?)
            } else {
                None
            };
            let base_line = if complex_kerning {
                Some(0)
            } else {
                Some(packet.g1()?)
            };

            let entry = FontMetricsEntry {
                source_type,
                source_pack_id,
                pixel_size: None,
                glyph_info,
                font_sheet_width: Some(font_sheet_width),
                font_sheet_height: Some(font_sheet_height),
                font_sheet_position: positions,
                base_line,
                upper_case_ascent: Some(packet.g1()?),
                byte3049: Some(packet.g1()?),
                max_ascent: Some(packet.g1()?),
                max_descent: Some(packet.g1()?),
                scale: Some(packet.g1()?),
                kerning_data,
            };

            if !packet.is_done() {
                bail!("fontmetrics sprite did not consume full payload");
            }
            Ok(entry)
        }
    }
}

fn parse_font_kerning(packet: &mut crate::packet::Packet<'_>) -> Result<FontKerningData> {
    let mut right_kern = Vec::with_capacity(256);
    for _ in 0..256_usize {
        let mut kerns = Vec::with_capacity(256);
        let mut kern = 0_i32;
        for _ in 0..256_usize {
            kern += i32::from(packet.g1s()?);
            kerns.push(kern as i8);
        }
        right_kern.push(kerns);
    }

    let mut left_kern = Vec::with_capacity(256);
    for _ in 0..256_usize {
        let mut kerns = Vec::with_capacity(256);
        let mut kern = 0_i32;
        for _ in 0..256_usize {
            kern += i32::from(packet.g1s()?);
            kerns.push(kern as i8);
        }
        left_kern.push(kerns);
    }

    Ok(FontKerningData {
        left_kern,
        right_kern,
    })
}

fn read_f32_be(packet: &mut crate::packet::Packet<'_>) -> Result<f32> {
    Ok(f32::from_bits(packet.g4s()? as u32))
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn run_audio(
    cache: &FlatCache,
    tar_path: &Path,
    out_dir: Option<&Path>,
    max_files: Option<usize>,
) -> Result<()> {
    let mut available = Vec::new();
    for archive in AUDIO_ARCHIVES {
        if ensure_archive_complete(cache.root(), tar_path, archive).is_ok() {
            available.push(archive);
        }
    }
    let cache = FlatCache::open(cache.root())?;
    let mut archive_counts = BTreeMap::new();
    let mut kind_counts = BTreeMap::new();
    let mut extracted_embedded_ogg = 0_usize;
    let mut manifest = Vec::new();

    let mut processed = 0_usize;
    let process_limit = max_files.unwrap_or(usize::MAX);
    let mut limit_hit = false;

    for archive in available {
        let index = cache.archive_index(archive)?;
        let mut file_count = 0_usize;
        for group in &index.group_id {
            let files = cache.group_files_with_index(&index, archive, *group)?;
            for (file, data) in &files {
                if processed >= process_limit {
                    limit_hit = true;
                    break;
                }
                let inspection = inspect_audio_file(data);
                *kind_counts
                    .entry(inspection.kind.as_str().to_string())
                    .or_insert(0) += 1;
                let mut extracted_ogg = false;

                if let Some(out) = out_dir {
                    let raw_path =
                        out.join(format!("{archive}_{group}_{file}.{}", inspection.extension));
                    write_binary(&raw_path, data)?;

                    if inspection.kind == AudioKind::Jaga
                        && let Some(ogg) = inspection.embedded_ogg_slice(data)
                    {
                        let ogg_path = out.join(format!("{archive}_{group}_{file}.ogg"));
                        write_binary(&ogg_path, ogg)?;
                        extracted_ogg = true;
                        extracted_embedded_ogg += 1;
                    }
                }

                manifest.push(AudioManifestEntry {
                    archive,
                    group: *group,
                    file: *file,
                    size: data.len(),
                    kind: inspection.kind.as_str().to_string(),
                    raw_extension: inspection.extension.to_string(),
                    embedded_ogg_offset: inspection.embedded_ogg_offset,
                    extracted_ogg,
                });
                file_count += 1;
                processed += 1;
            }
            if limit_hit {
                break;
            }
        }
        archive_counts.insert(archive, file_count);
        if limit_hit {
            break;
        }
    }

    let manifest_path = if let Some(out) = out_dir {
        let manifest_path = out.join("audio_manifest.json");
        write_json(&manifest_path, &manifest)?;
        Some(manifest_path.display().to_string())
    } else {
        None
    };

    print_json(&AudioSummary {
        archives: archive_counts,
        kinds: kind_counts,
        extracted_embedded_ogg,
        manifest_path,
    })
}

fn format_script_source(group: u32, file: u32, script: &CompiledScript) -> String {
    let mut out = String::new();
    let script_name = script.name.as_deref().unwrap_or("null");
    let _ = writeln!(out, "// group={group} file={file}");
    let _ = writeln!(out, "// name={script_name}");
    let _ = writeln!(
        out,
        "// locals int={} object={} long={}",
        script.local_count_int, script.local_count_object, script.local_count_long
    );
    let _ = writeln!(
        out,
        "// args int={} object={} long={}",
        script.argument_count_int, script.argument_count_object, script.argument_count_long
    );
    for (index, instruction) in script.code.iter().enumerate() {
        let _ = writeln!(out, "{index:05}: {}", format_instruction(instruction));
    }
    out
}

fn format_instruction(instruction: &Instruction) -> String {
    format!(
        "{} {}",
        instruction.command,
        format_operand(&instruction.operand)
    )
    .trim_end()
    .to_string()
}

fn format_operand(operand: &Operand) -> String {
    match operand {
        Operand::Int(value) => value.to_string(),
        Operand::Long(value) => value.to_string(),
        Operand::Str(value) => format!("\"{}\"", escape_string(value)),
        Operand::Local(value) => format!("local_{value}"),
        Operand::VarRef(value) => {
            let mut tag = format!("{}:{}", value.domain.as_label(), value.id);
            if value.transmog {
                tag.push_str(":transmog");
            }
            tag
        }
        Operand::VarBitRef(value) => {
            let mut tag = format!("varbit:{}", value.id);
            if value.transmog {
                tag.push_str(":transmog");
            }
            tag
        }
        Operand::Branch(value) => format!("->{value}"),
        Operand::Switch(cases) => {
            let mut text = String::new();
            text.push('{');
            for (index, case) in cases.iter().enumerate() {
                if index != 0 {
                    text.push_str(", ");
                }
                let _ = write!(text, "{}->{}", case.value, case.target);
            }
            text.push('}');
            text
        }
        Operand::Script(value) => format!("script_{value}"),
        Operand::Array(value) => format!("array_{value}"),
        Operand::Count(value) => format!("count_{value}"),
        Operand::Byte(value) => value.to_string(),
    }
}

fn escape_string(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
        .replace('"', "\\\"")
}

fn sanitize_file_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "script".to_string()
    } else {
        out
    }
}

fn load_script_group_names(
    index: &crate::js5::ArchiveIndex,
    data_dir: &Path,
) -> Result<HashMap<u32, String>> {
    let Some(group_hashes) = &index.group_name_hash else {
        return Ok(HashMap::new());
    };

    let names_path = data_dir.join("names/scripts.txt");
    if !names_path.is_file() {
        return Ok(HashMap::new());
    }

    let hash_names = load_hash_name_map(&names_path)?;
    let mut by_group = HashMap::new();
    for group in &index.group_id {
        let idx = usize::try_from(*group).context("script group index overflow")?;
        let hash = *group_hashes
            .get(idx)
            .with_context(|| format!("missing group hash slot for {group}"))?;
        if hash == -1 {
            continue;
        }
        if let Some(name) = hash_names.get(&hash) {
            by_group.insert(*group, extract_name_suffix(name));
        }
    }
    Ok(by_group)
}

fn load_script_group_names_from_cache(
    cache: &FlatCache,
    data_dir: &Path,
) -> Result<HashMap<u32, String>> {
    let index = cache.archive_index(ARCHIVE_CLIENTSCRIPTS)?;
    load_script_group_names(&index, data_dir)
}

fn export_script_signatures_from_cache(
    cache: &FlatCache,
    tar_path: &Path,
    out_dir: &Path,
    opcode_book: &OpcodeBook,
    build: u32,
    group_names: &HashMap<u32, String>,
) -> Result<()> {
    let mut lines = vec![
        "// Auto-generated CS2 script signatures".to_string(),
        "// Source: RS3 cache clientscript archive".to_string(),
        String::new(),
    ];
    let mut entries: Vec<(String, String)> = Vec::new();

    if crate::fixture::ensure_archive_complete(cache.root(), tar_path, ARCHIVE_CLIENTSCRIPTS)
        .is_err()
    {
        return write_lines(&out_dir.join("scripts.d.ts"), &lines);
    }

    let cache2 = FlatCache::open(cache.root())?;
    let index = cache2.archive_index(ARCHIVE_CLIENTSCRIPTS)?;
    for group in &index.group_id {
        let files = cache2.group_files_with_index(&index, ARCHIVE_CLIENTSCRIPTS, *group)?;
        for (_file, data) in files {
            let Ok(script) = decode_script(&data, opcode_book, build) else {
                continue;
            };
            let display_name = script
                .name
                .as_deref()
                .map(crate::transpile::extract_script_name_suffix)
                .filter(|name| !name.is_empty())
                .or_else(|| group_names.get(group).cloned());
            // Name by group (`script<group>`), matching the catalog and the
            // transpiled output files — not the packed id.
            let function_name = crate::transpile::script_function_name(
                crate::transpile::ScriptId(*group as i32),
                display_name.as_deref(),
            );
            let mut arg_types: Vec<&str> = Vec::new();
            arg_types.extend(std::iter::repeat_n(
                "number",
                script.argument_count_int as usize,
            ));
            arg_types.extend(std::iter::repeat_n(
                "string",
                script.argument_count_object as usize,
            ));
            arg_types.extend(std::iter::repeat_n(
                "bigint",
                script.argument_count_long as usize,
            ));
            let args = (0..arg_types.len())
                .map(|index| format!("arg{index}: {}", arg_types[index]))
                .collect::<Vec<_>>()
                .join(", ");
            entries.push((
                function_name.clone(),
                format!("export function {function_name}({args}): unknown;"),
            ));
        }
    }

    entries.sort_by(|a, b| a.0.cmp(&b.0));
    lines.extend(entries.into_iter().map(|(_, line)| line));
    write_lines(&out_dir.join("scripts.d.ts"), &lines)
}

fn load_hash_name_map(path: &Path) -> Result<HashMap<i32, String>> {
    let content =
        fs::read_to_string(path).with_context(|| format!("failed reading {}", path.display()))?;
    let mut map = HashMap::new();
    for line in content.lines() {
        let name = line.trim();
        if name.is_empty() {
            continue;
        }
        expand_name_pattern(name, &mut map);
    }
    Ok(map)
}

fn expand_name_pattern(name: &str, out: &mut HashMap<i32, String>) {
    if let Some(index) = name.find('#') {
        let prefix = &name[..index];
        let suffix = &name[index + 1..];
        for value in 0..500 {
            let expanded = format!("{prefix}{value}{suffix}");
            expand_name_pattern(&expanded, out);
        }
    } else {
        out.insert(java_string_hash(name), name.to_string());
    }
}

fn java_string_hash(value: &str) -> i32 {
    let mut hash = 0_i32;
    for c in value.chars() {
        hash = hash.wrapping_mul(31).wrapping_add(c as i32);
    }
    hash
}

fn extract_name_suffix(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.starts_with('[') && trimmed.ends_with(']') {
        let inner = &trimmed[1..trimmed.len() - 1];
        if let Some((_, suffix)) = inner.split_once(',') {
            return suffix.to_string();
        }
    }
    trimmed.to_string()
}

fn write_binary(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed creating {}", parent.display()))?;
    }
    fs::write(path, data).with_context(|| format!("failed writing {}", path.display()))
}

fn write_text(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed creating {}", parent.display()))?;
    }
    fs::write(path, text).with_context(|| format!("failed writing {}", path.display()))
}

fn write_lines(path: &Path, lines: &[String]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed creating {}", parent.display()))?;
    }

    let file =
        fs::File::create(path).with_context(|| format!("failed writing {}", path.display()))?;
    let mut writer = BufWriter::new(file);
    for (index, line) in lines.iter().enumerate() {
        if index != 0 {
            writer.write_all(b"\n")?;
        }
        writer.write_all(line.as_bytes())?;
    }
    writer
        .flush()
        .with_context(|| format!("failed writing {}", path.display()))
}

struct TextFileWriter {
    path: PathBuf,
    writer: BufWriter<fs::File>,
    wrote_line: bool,
}

impl TextFileWriter {
    fn create(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed creating {}", parent.display()))?;
        }

        let file =
            fs::File::create(path).with_context(|| format!("failed writing {}", path.display()))?;
        Ok(Self {
            path: path.to_path_buf(),
            writer: BufWriter::new(file),
            wrote_line: false,
        })
    }

    fn line(&mut self, line: impl AsRef<str>) -> Result<()> {
        if self.wrote_line {
            self.writer.write_all(b"\n")?;
        }
        self.writer.write_all(line.as_ref().as_bytes())?;
        self.wrote_line = true;
        Ok(())
    }

    fn finish(mut self) -> Result<()> {
        self.writer
            .flush()
            .with_context(|| format!("failed writing {}", self.path.display()))
    }
}

fn write_json<T: Serialize>(path: &Path, data: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed creating {}", parent.display()))?;
    }
    let json = serde_json::to_vec_pretty(data).context("failed to encode json")?;
    fs::write(path, json).with_context(|| format!("failed writing {}", path.display()))
}

fn print_json<T: Serialize>(value: &T) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(value).context("failed to encode summary json")?
    );
    Ok(())
}

fn run_dep_tree_interface(
    cache: &FlatCache,
    tar_path: &Path,
    data_dir: &Path,
    id: u32,
    max_depth: u32,
    out_file: &Path,
    version: RuntimeVersion,
) -> Result<()> {
    let ctx = ResolverContext::load(cache, tar_path, data_dir, version.build, version.subbuild)?;
    let root = EntityRef::new(EntityType::Interface, id);
    let tree = build_tree(&ctx, &root, max_depth);
    write_json(out_file, &tree)?;
    eprintln!(
        "dependency tree written to {} (nodes={}, cycles={}, depth_hits={})",
        out_file.display(),
        tree.total_nodes,
        tree.cycles_detected,
        tree.max_depth_hits
    );
    Ok(())
}

fn run_dep_tree_script(
    cache: &FlatCache,
    tar_path: &Path,
    data_dir: &Path,
    id: u32,
    max_depth: u32,
    out_file: &Path,
    version: RuntimeVersion,
) -> Result<()> {
    let ctx = ResolverContext::load(cache, tar_path, data_dir, version.build, version.subbuild)?;
    let root = EntityRef::new(EntityType::Script, id);
    let tree = build_tree(&ctx, &root, max_depth);
    write_json(out_file, &tree)?;
    eprintln!(
        "dependency tree written to {} (nodes={}, cycles={}, depth_hits={})",
        out_file.display(),
        tree.total_nodes,
        tree.cycles_detected,
        tree.max_depth_hits
    );
    Ok(())
}

// Resolves dependency tree for a varp entry across 9 parameter sources.
#[allow(clippy::too_many_arguments)]
fn run_dep_tree_varp(
    cache: &FlatCache,
    tar_path: &Path,
    data_dir: &Path,
    id: u32,
    domain: VarDomainArg,
    max_depth: u32,
    out_file: &Path,
    version: RuntimeVersion,
) -> Result<()> {
    let ctx = ResolverContext::load(cache, tar_path, data_dir, version.build, version.subbuild)?;
    let entity_type = match domain {
        VarDomainArg::Player => EntityType::VarPlayer,
        VarDomainArg::Npc => EntityType::VarNpc,
        VarDomainArg::Client => EntityType::VarClient,
        VarDomainArg::World => EntityType::VarWorld,
        VarDomainArg::Region => EntityType::VarRegion,
        VarDomainArg::Object => EntityType::VarObject,
        VarDomainArg::Clan => EntityType::VarClan,
        VarDomainArg::ClanSetting => EntityType::VarClanSetting,
        VarDomainArg::Controller => EntityType::VarController,
        VarDomainArg::Global => EntityType::VarGlobal,
        VarDomainArg::PlayerGroup => EntityType::VarPlayerGroup,
        VarDomainArg::All => bail!("dep-tree-varp requires a specific domain, not 'all'"),
    };
    let root = EntityRef::new(entity_type, id);
    let tree = build_tree(&ctx, &root, max_depth);
    write_json(out_file, &tree)?;
    eprintln!(
        "dependency tree written to {} (nodes={}, cycles={}, depth_hits={})",
        out_file.display(),
        tree.total_nodes,
        tree.cycles_detected,
        tree.max_depth_hits
    );
    Ok(())
}

fn run_dep_tree_varbit(
    cache: &FlatCache,
    tar_path: &Path,
    data_dir: &Path,
    id: u32,
    max_depth: u32,
    out_file: &Path,
    version: RuntimeVersion,
) -> Result<()> {
    let ctx = ResolverContext::load(cache, tar_path, data_dir, version.build, version.subbuild)?;
    let root = EntityRef::new(EntityType::VarBit, id);
    let tree = build_tree(&ctx, &root, max_depth);
    write_json(out_file, &tree)?;
    eprintln!(
        "dependency tree written to {} (nodes={}, cycles={}, depth_hits={})",
        out_file.display(),
        tree.total_nodes,
        tree.cycles_detected,
        tree.max_depth_hits
    );
    Ok(())
}

// Resolves dependency tree for a config entry across 8 parameter sources.
#[allow(clippy::too_many_arguments)]
fn run_dep_tree_config(
    cache: &FlatCache,
    tar_path: &Path,
    data_dir: &Path,
    kind: ConfigKindArg,
    id: u32,
    max_depth: u32,
    out_file: &Path,
    version: RuntimeVersion,
) -> Result<()> {
    let ctx = ResolverContext::load(cache, tar_path, data_dir, version.build, version.subbuild)?;
    let entity_type = kind.entity_type();
    let root = EntityRef::new(entity_type, id).labeled(kind.label());
    let tree = build_tree(&ctx, &root, max_depth);
    write_json(out_file, &tree)?;
    eprintln!(
        "dependency tree written to {} (nodes={}, cycles={}, depth_hits={})",
        out_file.display(),
        tree.total_nodes,
        tree.cycles_detected,
        tree.max_depth_hits
    );
    Ok(())
}

fn run_validate_script(
    cache: &FlatCache,
    tar_path: &Path,
    data_dir: &Path,
    script_id: u32,
    out_file: Option<&Path>,
    emit_json: bool,
    version: RuntimeVersion,
) -> Result<()> {
    let ctx = ResolverContext::load(cache, tar_path, data_dir, version.build, version.subbuild)?;
    let validator = crate::validate::Cs2Validator::new(&ctx);
    let report = validator.validate(script_id);

    if emit_json {
        println!("{}", serde_json::to_string(&report)?);
        return Ok(());
    }

    if let Some(path) = out_file {
        let json = serde_json::to_string_pretty(&report)?;
        std::fs::write(path, &json)?;
        eprintln!("validation report written to {}", path.display());
    } else {
        let name = report.script_name.as_deref().unwrap_or("(unnamed)");
        eprintln!(
            "script_{id} \"{name}\" ({count} instructions, build {build})",
            id = report.script_id,
            count = report.instruction_count,
            build = report.build
        );
        if report.errors.is_empty() {
            eprintln!("  [PASS] 0 errors");
        } else {
            for err in &report.errors {
                match err {
                    crate::validate::ValidationError::UnknownOpcode { index, opcode } => {
                        eprintln!("  FAIL [{index}] unknown opcode {opcode}");
                    }
                    crate::validate::ValidationError::InvalidBranchTarget {
                        index,
                        target,
                        total_instructions,
                    } => {
                        eprintln!(
                            "  FAIL [{index}] branch target {target} out of range (0..{total_instructions})"
                        );
                    }
                    crate::validate::ValidationError::VarpNotFound { index, domain, id } => {
                        eprintln!("  FAIL [{index}] varp {domain}:{id} not found");
                    }
                    crate::validate::ValidationError::VarbitNotFound { index, id } => {
                        eprintln!("  FAIL [{index}] varbit {id} not found");
                    }
                    crate::validate::ValidationError::ScriptNotFound { index, called_id } => {
                        eprintln!("  FAIL [{index}] called script {called_id} not found");
                    }
                    crate::validate::ValidationError::StackUnderflow {
                        index,
                        stack,
                        needed,
                        available,
                    } => {
                        eprintln!(
                            "  FAIL [{index}] {stack} stack underflow: needs {needed}, has {available}"
                        );
                    }
                    crate::validate::ValidationError::UnbalancedReturn {
                        index,
                        int_stack,
                        obj_stack,
                        long_stack,
                    } => {
                        eprintln!(
                            "  FAIL [{index}] return with values on stacks: int={int_stack}, obj={obj_stack}, long={long_stack}"
                        );
                    }
                    crate::validate::ValidationError::MissingReturn => {
                        eprintln!("  FAIL missing return statement");
                    }
                }
            }
            eprintln!("  {} error(s)", report.errors.len());
        }
        for warn in &report.warnings {
            eprintln!("  WARN {warn}");
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_assemble_script(
    cache: &FlatCache,
    tar_path: &Path,
    data_dir: &Path,
    input: &Path,
    output: &Path,
    build: Option<u32>,
    subbuild: Option<u32>,
    strict_structured: bool,
    no_verify: bool,
    emit_json: bool,
    version: RuntimeVersion,
) -> Result<()> {
    let started = Instant::now();
    let source =
        std::fs::read_to_string(input).with_context(|| format!("reading {}", input.display()))?;
    let reversible = if is_reversible_source(&source) {
        Some(
            parse_reversible_source(&source)
                .with_context(|| format!("parsing reversible TS from {}", input.display()))?,
        )
    } else {
        None
    };

    if let Some(parsed) = &reversible {
        if parsed.metadata.format_version != REVERSIBLE_FORMAT_VERSION {
            bail!(
                "unsupported reversible TS format version {} in {}",
                parsed.metadata.format_version,
                input.display()
            );
        }
        if let Some(requested_build) = build
            && requested_build != parsed.metadata.build
        {
            bail!(
                "assemble build {} mismatches source metadata build {}",
                requested_build,
                parsed.metadata.build
            );
        }
        if let Some(requested_subbuild) = subbuild
            && requested_subbuild != parsed.metadata.subbuild
        {
            bail!(
                "assemble subbuild {} mismatches source metadata subbuild {}",
                requested_subbuild,
                parsed.metadata.subbuild
            );
        }
    }

    let effective_build = reversible.as_ref().map_or_else(
        || build.unwrap_or(version.build),
        |parsed| parsed.metadata.build,
    );
    let effective_subbuild = reversible.as_ref().map_or_else(
        || subbuild.unwrap_or(version.subbuild),
        |parsed| parsed.metadata.subbuild,
    );
    let ctx = ResolverContext::load_transpile(
        cache,
        tar_path,
        data_dir,
        effective_build,
        effective_subbuild,
    )?;
    let opcode_book = &ctx.opcode_book;

    let (script, assemble_mode) = if let Some(parsed) = &reversible {
        let structured = parse_structured_typescript(&parsed.structured_source)
            .with_context(|| format!("parsing structured TS from {}", input.display()))?;
        let current_digest = structured_digest(&structured);
        if !strict_structured && current_digest == parsed.metadata.structured_digest {
            (
                parse_cs2_asm(&parsed.asm_trailer).with_context(|| {
                    format!("parsing embedded ASM trailer from {}", input.display())
                })?,
                "embedded-asm",
            )
        } else {
            if !parsed.metadata.editable_structured {
                let blockers = if parsed.metadata.blocking_diagnostics.is_empty() {
                    "unknown blocker".to_string()
                } else {
                    parsed.metadata.blocking_diagnostics.join(", ")
                };
                bail!(
                    "structured edits blocked for {}: {}. edit embedded ASM trailer or remove blocker",
                    parsed.metadata.export_name,
                    blockers
                );
            }
            let reverse_ctx = build_reverse_compile_context(&ctx, cache, data_dir)?;
            let compiled = lower_structured_script(&structured, &parsed.metadata, &reverse_ctx)
                .with_context(|| format!("lowering structured TS from {}", input.display()))?;
            (compiled, "structured")
        }
    } else {
        (
            parse_cs2_asm(&source)
                .with_context(|| format!("parsing ASM pragmas from {}", input.display()))?,
            "pragma-asm",
        )
    };
    let binary =
        encode_script(&script, opcode_book, effective_build).context("encoding CS2 binary")?;

    if !no_verify {
        verify_assembled_script(cache, data_dir, &ctx, &script, &binary, output)?;
    }

    std::fs::write(output, &binary).with_context(|| format!("writing {}", output.display()))?;

    if emit_json {
        let event = serde_json::json!({
            "event": "assemble_script",
            "outcome": "ok",
            "build": effective_build,
            "subbuild": effective_subbuild,
            "mode": assemble_mode,
            "instruction_count": script.code.len(),
            "bytes": binary.len(),
            "verified": !no_verify,
            "output": output.display().to_string(),
            "duration_ms": started.elapsed().as_millis() as u64,
        });
        println!("{}", serde_json::to_string(&event)?);
    } else {
        eprintln!(
            "Assembled {} instructions → {} ({} bytes, build {}, mode {})",
            script.code.len(),
            output.display(),
            binary.len(),
            effective_build,
            assemble_mode,
        );
    }
    Ok(())
}

#[derive(Deserialize)]
struct BatchAssembleManifest {
    scripts: Vec<BatchAssembleScript>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BatchAssembleScript {
    script_id: u32,
    input: PathBuf,
    build: Option<u32>,
    subbuild: Option<u32>,
}

fn run_assemble_script_batch(
    data_dir: &Path,
    manifest: &Path,
    out_dir: &Path,
    version: RuntimeVersion,
) -> Result<()> {
    let started = Instant::now();
    let batch: BatchAssembleManifest = serde_json::from_slice(
        &fs::read(manifest).with_context(|| format!("reading {}", manifest.display()))?,
    )?;
    fs::create_dir_all(out_dir).with_context(|| format!("creating {}", out_dir.display()))?;
    let build = batch
        .scripts
        .first()
        .and_then(|script| script.build)
        .unwrap_or(version.build);
    let subbuild = batch
        .scripts
        .first()
        .and_then(|script| script.subbuild)
        .unwrap_or(version.subbuild);
    let opcode_book = OpcodeBook::load(data_dir, build, subbuild)?;
    for script in &batch.scripts {
        let effective_build = script.build.unwrap_or(build);
        let effective_subbuild = script.subbuild.unwrap_or(subbuild);
        ensure!(
            effective_build == build && effective_subbuild == subbuild,
            "assemble-script-batch requires one build/subbuild per invocation"
        );
        let source = fs::read_to_string(&script.input)
            .with_context(|| format!("reading {}", script.input.display()))?;
        ensure!(
            !is_reversible_source(&source),
            "assemble-script-batch only supports pragma ASM inputs: {}",
            script.input.display()
        );
        let compiled = parse_cs2_asm(&source)
            .with_context(|| format!("parsing ASM pragmas from {}", script.input.display()))?;
        let binary = encode_script(&compiled, &opcode_book, build)
            .with_context(|| format!("encoding script {}", script.script_id))?;
        fs::write(out_dir.join(format!("script-{}.cs2", script.script_id)), binary)
            .with_context(|| format!("writing script {}", script.script_id))?;
    }
    eprintln!(
        "assemble-script-batch: assembled {} script(s) in {}ms",
        batch.scripts.len(),
        started.elapsed().as_millis()
    );
    Ok(())
}

/// Verify a freshly assembled script before writing it: (1) the emitted bytes
/// must decode back to an identical script (encoder/operand fidelity), and (2)
/// structural + stack-effect validation against the target build's catalog must
/// pass (no stack underflow, dangling branch targets, or unknown references).
/// This stops `assemble-script` from silently producing CS2 the client cannot
/// run. Bypass with `--no-verify`.
fn verify_assembled_script(
    cache: &FlatCache,
    data_dir: &Path,
    ctx: &ResolverContext,
    script: &CompiledScript,
    binary: &[u8],
    output: &Path,
) -> Result<()> {
    let decoded = decode_script(binary, &ctx.opcode_book, ctx.build)
        .with_context(|| format!("verifying {}: re-decoding emitted CS2", output.display()))?;
    // Compare command + operand + header, ignoring the numeric `opcode` field:
    // lowering/assembly leaves it as a `0` placeholder while decode fills in the
    // real id, but the byte fidelity we care about lives in command names and
    // operands. A self-consistent encoder bug (e.g. a zeroed placeholder operand)
    // still surfaces here, which a bytes-only `encode(decode(b)) == b` check misses.
    let normalize = |source: &CompiledScript| -> Result<serde_json::Value> {
        let mut clone = source.clone();
        for instruction in &mut clone.code {
            instruction.opcode = 0;
        }
        Ok(serde_json::to_value(&clone)?)
    };
    if normalize(&decoded)? != normalize(script)? {
        bail!(
            "post-compile verification failed for {}: re-decoded CS2 does not match the compiled \
             script (encoder fidelity bug); pass --no-verify to override",
            output.display()
        );
    }

    let reverse_ctx = build_reverse_compile_context(ctx, cache, data_dir)?;
    let script_id = script
        .name
        .as_deref()
        .and_then(|name| reverse_ctx.script_catalog.resolve_export_name(name))
        .map_or(0, |meta| meta.packed_id.0.unsigned_abs());
    let validator = crate::validate::Cs2Validator::new(ctx);
    let report = validator.validate_compiled(
        script_id,
        script,
        &reverse_ctx.script_catalog,
        &reverse_ctx.script_signatures,
        script.name.clone(),
    );
    if !report.errors.is_empty() {
        let detail = report
            .errors
            .iter()
            .map(|error| format!("{error:?}"))
            .collect::<Vec<_>>()
            .join("; ");
        bail!(
            "post-compile validation failed for {} ({} error(s)): {detail}; pass --no-verify to \
             override",
            output.display(),
            report.errors.len(),
        );
    }
    Ok(())
}

fn build_reverse_compile_context(
    ctx: &ResolverContext,
    cache: &FlatCache,
    data_dir: &Path,
) -> Result<ReverseCompileContext> {
    let script_group_names = load_script_group_names_from_cache(cache, data_dir)?;
    let mut builder = ScriptCatalogBuilder::new(&script_group_names, &ctx.opcode_book, ctx.build);
    for (&packed_id_raw, data) in &ctx.scripts {
        builder.add_script(packed_id_raw, data);
    }
    Ok(build_reverse_compile_context_from_catalog(
        ctx,
        builder.build(),
    ))
}

fn build_reverse_compile_context_from_catalog(
    ctx: &ResolverContext,
    script_catalog: ScriptCatalog,
) -> ReverseCompileContext {
    let mut var_refs_by_name = HashMap::new();
    for (domain, vars) in &ctx.varps_by_domain {
        for (&id, entry) in vars {
            var_refs_by_name.insert(
                entry.var_name.clone(),
                VarRef {
                    domain: *domain,
                    id: id as u16,
                    transmog: false,
                },
            );
            var_refs_by_name.insert(
                format!("{}_transmog", entry.var_name),
                VarRef {
                    domain: *domain,
                    id: id as u16,
                    transmog: true,
                },
            );
        }
    }

    let mut varbit_refs_by_name = HashMap::new();
    for (&id, entry) in &ctx.varbits {
        let id = id as u16;
        varbit_refs_by_name.insert(
            entry.varbit_name.clone(),
            VarBitRef {
                id,
                transmog: false,
            },
        );
        varbit_refs_by_name.insert(
            format!("{}_transmog", entry.varbit_name),
            VarBitRef { id, transmog: true },
        );
        varbit_refs_by_name.insert(
            format!("varbit_{id}"),
            VarBitRef {
                id,
                transmog: false,
            },
        );
        varbit_refs_by_name.insert(
            format!("varbit_{id}_transmog"),
            VarBitRef { id, transmog: true },
        );
    }

    let string_param_ids = ctx
        .params
        .values()
        .filter(|entry| {
            matches!(entry.type_char, Some(b's' | b'S'))
                || matches!(entry.default, Some(crate::config::ScalarValue::Str(_)))
        })
        .map(|entry| entry.id as i32)
        .collect::<HashSet<_>>();

    let mut enum_values_by_name = HashMap::new();
    for entry in ctx.enums.values() {
        let object_name = format!("Enum_{}", entry.id);
        let mut used_properties = HashSet::new();
        for pair in &entry.values {
            let property_name =
                enum_pair_property_name(&pair.value, pair.key, &mut used_properties);
            enum_values_by_name.insert(format!("{object_name}.{property_name}"), pair.key);
        }
    }

    let mut component_ids_by_name = HashMap::new();
    for (&interface_id, components) in &ctx.parsed_components {
        for (&component_id, deps) in components {
            let property_name = deps
                .name
                .as_deref()
                .map(sanitize_ts_prop)
                .filter(|prop| !prop.is_empty() && prop != "unnamed")
                .unwrap_or_else(|| {
                    sanitize_ts_prop(&crate::interface::component_fallback_name(
                        interface_id,
                        component_id,
                    ))
                });
            component_ids_by_name.insert(
                format!("ComponentId.{property_name}"),
                crate::interface::component_uid(interface_id, component_id) as i32,
            );
        }
    }

    ReverseCompileContext {
        build: ctx.build,
        script_signatures: script_catalog.signature_map(),
        script_catalog,
        var_refs_by_name,
        varbit_refs_by_name,
        string_param_ids,
        enum_values_by_name,
        component_ids_by_name,
        opcode_commands: ctx.opcode_book.commands().map(str::to_string).collect(),
    }
}

/// Append `diagnostic` to `diagnostics` only if not already present, keeping the
/// blocker list free of duplicates across re-checks.
fn push_unique_diagnostic(diagnostics: &mut Vec<String>, diagnostic: String) {
    if !diagnostics.iter().any(|existing| existing == &diagnostic) {
        diagnostics.push(diagnostic);
    }
}

fn finalize_reversible_transpile_output(
    source: String,
    reverse_ctx: &ReverseCompileContext,
    opcode_book: &OpcodeBook,
    diagnostics: &mut crate::transpile::Diagnostics,
    editable_structured: &mut bool,
    blocking_diagnostics: &mut Vec<String>,
) -> Result<String> {
    if !is_reversible_source(&source) {
        return Ok(source);
    }

    let mut parsed = parse_reversible_source(&source)?;
    // G3: annotate local declarations with inferred semantic types (opt-in). Applied
    // before the fidelity gate so the gate validates the annotated form — annotations
    // are byte-irrelevant (the reverse compiler recovers a local's slot/domain from its
    // name), so this never changes the recompile result.
    if std::env::var_os("RS3_INFER_LOCAL_TYPES").is_some() {
        annotate_parsed_local_types(&mut parsed, reverse_ctx);
    }
    let mut metadata = parsed.metadata.clone();
    if metadata.editable_structured
        && let Err(block) = recompile_fidelity_check(&parsed, &metadata, reverse_ctx, opcode_book)
    {
        metadata.editable_structured = false;
        push_unique_diagnostic(
            &mut metadata.blocking_diagnostics,
            block.blocker.to_string(),
        );
        // A low-cardinality sub-bucket so the coverage histogram ranks *why* a
        // blocker fired (recompile_mismatch_cause:push_constant_string->... or
        // reverse_unsupported_cause:ui_method) instead of collapsing them into
        // one opaque tag.
        if let Some(cause) = block.cause {
            push_unique_diagnostic(
                &mut metadata.blocking_diagnostics,
                format!("{}_cause:{cause}", block.blocker),
            );
        }
        diagnostics.warning(block.message);
    }

    // G1.4: the RuneScript-surface byte gate (opt-in, informational). Runs whenever the structured
    // TS surface is itself byte-exact (`editable_structured`); it gates whichever build is being
    // transpiled (driven by `--build`). It records a `runescript_gate` diagnostic on failure but
    // NEVER flips `editable_structured` — the TS editing surface is unaffected. This is the
    // authoritative proof that the RuneScript round-trip (`render_runescript` → `parse_runescript`
    // → encode) reproduces the original bytes.
    if std::env::var_os("RS3_RUNESCRIPT_GATE").is_some()
        && metadata.editable_structured
        && let Err(block) =
            recompile_fidelity_check_runescript(&parsed, &metadata, reverse_ctx, opcode_book)
    {
        push_unique_diagnostic(&mut metadata.blocking_diagnostics, "runescript_gate".to_string());
        if let Some(cause) = block.cause {
            push_unique_diagnostic(
                &mut metadata.blocking_diagnostics,
                format!("runescript_gate_cause:{cause}"),
            );
        }
        // Diagnostic visibility (opt-in): record the sanitized failure message so the opaque
        // `other`/`ui_method`/`array` buckets can be sub-classified by the actual `bail!` reason.
        push_unique_diagnostic(
            &mut metadata.blocking_diagnostics,
            format!("runescript_gate_msg:{}", gate_message_head(&block.message)),
        );
    }

    *editable_structured = metadata.editable_structured;
    blocking_diagnostics.clone_from(&metadata.blocking_diagnostics);
    Ok(render_reversible_source(
        &parsed.structured_source,
        &metadata,
        &parsed.asm_trailer,
    )?)
}

/// Rewrite local-declaration annotations in a parsed reversible source with G3's
/// inferred semantic types. Gosub callee arities come from the reverse context's
/// cross-script signatures, so gosub-calling scripts model too. Byte-irrelevant
/// (the semantic annotation maps to the same base as today's), so the fidelity gate
/// is unaffected — proven by the corpus run staying `blocked:0`.
fn annotate_parsed_local_types(
    parsed: &mut crate::transpile::ParsedReversibleSource,
    reverse_ctx: &ReverseCompileContext,
) {
    use crate::transpile::ScriptId;
    use crate::transpile::type_constraints::{
        CalleeSig, SignatureTable, annotate_local_declarations, infer_program,
    };
    let Ok(script) = crate::script::parse_cs2_asm(&parsed.asm_trailer) else {
        return;
    };
    let sigs = SignatureTable::embedded(parsed.metadata.build);
    // `script_signatures` is keyed by the *packed* script id (group << 16), while a
    // `gosub_with_params` operand is the bare group id — try the packed form first,
    // then the raw id in case a caller already holds a packed reference.
    let callee = |id: i32| {
        let sig = reverse_ctx
            .script_signatures
            .get(&ScriptId(id << 16))
            .or_else(|| reverse_ctx.script_signatures.get(&ScriptId(id)))?;
        Some(CalleeSig {
            arg_int: sig.arg_count_int,
            arg_obj: sig.arg_count_obj,
            arg_long: sig.arg_count_long,
            ret_int: sig.return_count_int,
            ret_obj: sig.return_count_obj,
            ret_long: sig.return_count_long,
        })
    };
    let inferred = infer_program(&[(0, &script)], sigs, &callee);
    if let Some(locals) = inferred.get(&0) {
        parsed.structured_source = annotate_local_declarations(&parsed.structured_source, locals);
    }
}

struct FinalizedTranspileOutput {
    source: String,
    fallback_reason: Option<String>,
    primary_blocking_diagnostics: Vec<String>,
    primary_gate_messages: Vec<String>,
}

#[expect(
    clippy::too_many_arguments,
    reason = "fallback finalization threads existing command context and mutable diagnostics"
)]
fn finalize_with_linear_fallback(
    source: String,
    transpiler: &Transpiler,
    script: &CompiledScript,
    script_id: crate::transpile::ScriptId,
    script_catalog: &ScriptCatalog,
    reverse_ctx: &ReverseCompileContext,
    opcode_book: &OpcodeBook,
    diagnostics: &mut crate::transpile::Diagnostics,
    editable_structured: &mut bool,
    blocking_diagnostics: &mut Vec<String>,
) -> Result<FinalizedTranspileOutput> {
    let primary_control_fallback = source_control_flow_fallback_reason(&source);
    let primary_diagnostic_start = diagnostics.diagnostics.len();
    let finalized = finalize_reversible_transpile_output(
        source,
        reverse_ctx,
        opcode_book,
        diagnostics,
        editable_structured,
        blocking_diagnostics,
    )?;
    let primary_blocking_diagnostics = if *editable_structured {
        Vec::new()
    } else {
        blocking_diagnostics.clone()
    };
    let primary_gate_messages = diagnostics.diagnostics[primary_diagnostic_start..]
        .iter()
        .map(|diagnostic| diagnostic.message.clone())
        .collect::<Vec<_>>();
    let should_try_linear_fallback = blocking_diagnostics.iter().any(|blocker| {
        matches!(
            blocker.as_str(),
            "recompile_mismatch" | "reverse_unsupported"
        )
    });
    if *editable_structured || !should_try_linear_fallback {
        return Ok(FinalizedTranspileOutput {
            source: finalized,
            fallback_reason: primary_control_fallback,
            primary_blocking_diagnostics,
            primary_gate_messages,
        });
    }

    let conservative = transpiler.transpile_structured_conservative(script, script_id)?;
    let crate::transpile::TranspiledScript {
        source: conservative_source,
        diagnostics: mut conservative_diagnostics,
        editable_structured: mut conservative_editable,
        blocking_diagnostics: mut conservative_blocking,
        ..
    } = conservative;
    add_ambiguous_export_warning(&mut conservative_diagnostics, script_catalog, script_id);
    let finalized_conservative = finalize_reversible_transpile_output(
        conservative_source,
        reverse_ctx,
        opcode_book,
        &mut conservative_diagnostics,
        &mut conservative_editable,
        &mut conservative_blocking,
    )?;
    if conservative_editable {
        let fallback_reason = source_control_flow_fallback_reason(&finalized_conservative);
        *diagnostics = conservative_diagnostics;
        *editable_structured = conservative_editable;
        *blocking_diagnostics = conservative_blocking;
        return Ok(FinalizedTranspileOutput {
            source: finalized_conservative,
            fallback_reason,
            primary_blocking_diagnostics,
            primary_gate_messages,
        });
    }

    let linear = transpiler.transpile_linear(script, script_id)?;
    let crate::transpile::TranspiledScript {
        source: linear_source,
        diagnostics: mut linear_diagnostics,
        editable_structured: mut linear_editable,
        blocking_diagnostics: mut linear_blocking,
        ..
    } = linear;
    add_ambiguous_export_warning(&mut linear_diagnostics, script_catalog, script_id);
    let finalized_linear = finalize_reversible_transpile_output(
        linear_source,
        reverse_ctx,
        opcode_book,
        &mut linear_diagnostics,
        &mut linear_editable,
        &mut linear_blocking,
    )?;
    if linear_editable {
        *diagnostics = linear_diagnostics;
        *editable_structured = linear_editable;
        *blocking_diagnostics = linear_blocking;
        Ok(FinalizedTranspileOutput {
            source: finalized_linear,
            fallback_reason: Some("gate_mismatch".to_string()),
            primary_blocking_diagnostics,
            primary_gate_messages,
        })
    } else {
        Ok(FinalizedTranspileOutput {
            source: finalized,
            fallback_reason: primary_control_fallback,
            primary_blocking_diagnostics,
            primary_gate_messages,
        })
    }
}

fn source_control_flow_fallback_reason(source: &str) -> Option<String> {
    if source
        .lines()
        .any(|line| line.contains("stackpush_then(") && line.contains("goto("))
    {
        Some("stack_goto".to_string())
    } else if source.contains("goto(") || source.contains("label(") {
        Some("residual_goto".to_string())
    } else {
        None
    }
}

fn output_style_fallback_reason(
    output_style: TranspileOutputStyle,
    fallback_reason: Option<String>,
) -> Option<String> {
    match output_style {
        TranspileOutputStyle::HighTs => fallback_reason,
        TranspileOutputStyle::Reversible => Some("forced_reversible".to_string()),
    }
}

fn add_ambiguous_export_warning(
    diagnostics: &mut crate::transpile::Diagnostics,
    script_catalog: &ScriptCatalog,
    script_id: crate::transpile::ScriptId,
) {
    if let Some(metadata) = script_catalog.get(script_id) {
        let base_name = script_base_export_name(metadata);
        if metadata.export_name != base_name {
            diagnostics.warning(format!(
                "ambiguous export name '{}' resolved to '{}'",
                base_name, metadata.export_name
            ));
        }
    }
}

fn transpile_script_with_style(
    transpiler: &Transpiler,
    script: &CompiledScript,
    script_id: crate::transpile::ScriptId,
    output_style: TranspileOutputStyle,
) -> Result<crate::transpile::TranspiledScript> {
    match output_style {
        TranspileOutputStyle::HighTs => Ok(transpiler.transpile(script, script_id)?),
        TranspileOutputStyle::Reversible => Ok(transpiler.transpile_linear(script, script_id)?),
    }
}

/// A script is only truly `editable_structured` if its structured TS recompiles
/// to the **same bytes** as the original. The original is the embedded ASM
/// trailer (canonical); the candidate is the structured body lowered + encoded.
/// Comparing them gates out scripts that lower cleanly but to different bytes —
/// the false-editables that would silently corrupt the script if a user edited
/// the structured form. Returns `Err((blocker, message))` to mark non-editable.
/// Why a structured recompile was rejected by the fidelity gate. `blocker` is
/// the stable coverage tag; `cause` is a low-cardinality sub-bucket that turns
/// the opaque blocker into a ranked histogram (`<blocker>_cause:*`); `message`
/// is the human-readable detail.
struct RecompileBlock {
    blocker: &'static str,
    cause: Option<String>,
    message: String,
}

/// Bucket a `reverse_unsupported` failure into a low-cardinality cause so the
/// coverage histogram ranks *which* lowering gap blocked the script (parallel
/// to `recompile_mismatch_cause:*`). Keys are static substrings of the bail
/// sites, never interpolated names/ids.
fn classify_reverse_unsupported(message: &str) -> &'static str {
    // The non-lowering phases of recompile_fidelity_check each get their own
    // bucket; everything else is a structured-lowering bail, keyed by the bail
    // text regardless of anyhow context position.
    if message.starts_with("embedded ASM parse") {
        return "asm_parse";
    }
    if message.starts_with("encoding original") {
        return "encode_original";
    }
    if message.starts_with("structured parse") {
        return "structured_parse";
    }
    if message.starts_with("encoding structured") {
        return "encode_structured";
    }
    let patterns: &[(&str, &str)] = &[
        ("goto", "goto"),
        ("comment-only", "comment_control_flow"),
        ("UI hook", "ui_hook"),
        ("UI method", "ui_method"),
        ("UI.", "ui_arity"),
        ("callback watcher", "callback_watcher"),
        ("callback target", "callback_target"),
        ("callback literal", "callback_literal"),
        ("component constant", "unknown_component"),
        ("property access", "property_access"),
        ("negation", "negation"),
        ("logical not", "logical_not"),
        ("string arrays", "string_array"),
        ("array", "array"),
        ("void", "void_local"),
        ("identifier expression", "unknown_identifier"),
        ("assignment target", "assignment_target"),
        ("stack pseudo", "stack_pseudo"),
        ("branch label", "missing_branch_label"),
        ("switch label", "missing_switch_label"),
        ("numeric suffix", "numeric_suffix"),
        ("outside loop", "break_continue_outside_loop"),
    ];
    for (needle, bucket) in patterns {
        if message.contains(needle) {
            return bucket;
        }
    }
    "other"
}

impl RecompileBlock {
    fn reverse_unsupported(message: String) -> Self {
        let cause = classify_reverse_unsupported(&message).to_string();
        Self {
            blocker: "reverse_unsupported",
            cause: Some(cause),
            message,
        }
    }
}

fn recompile_fidelity_check(
    parsed: &crate::transpile::ParsedReversibleSource,
    metadata: &crate::transpile::ReversibleMetadata,
    reverse_ctx: &ReverseCompileContext,
    opcode_book: &OpcodeBook,
) -> std::result::Result<(), RecompileBlock> {
    let build = metadata.build;
    let original = parse_cs2_asm(&parsed.asm_trailer).map_err(|e| {
        RecompileBlock::reverse_unsupported(format!("embedded ASM parse failed: {e}"))
    })?;
    let expected = encode_script(&original, opcode_book, build).map_err(|e| {
        RecompileBlock::reverse_unsupported(format!("encoding original failed: {e}"))
    })?;

    let structured = parse_structured_typescript(&parsed.structured_source).map_err(|e| {
        RecompileBlock::reverse_unsupported(format!("structured parse failed: {e}"))
    })?;
    let compiled = lower_structured_script(&structured, metadata, reverse_ctx).map_err(|e| {
        RecompileBlock::reverse_unsupported(format!("structured lowering failed: {e}"))
    })?;
    let actual = encode_script(&compiled, opcode_book, build).map_err(|e| {
        RecompileBlock::reverse_unsupported(format!("encoding structured failed: {e}"))
    })?;

    if actual != expected {
        let (cause, message) = recompile_divergence(&original, &compiled);
        return Err(RecompileBlock {
            blocker: "recompile_mismatch",
            cause: Some(cause),
            message,
        });
    }
    Ok(())
}

/// Reduce a gate failure message to a low-cardinality head for histogramming (diagnostic-only):
/// strip the `… failed: ` wrapper and collapse digit runs to `#`, so the opaque buckets group by
/// their actual `bail!` reason instead of per-script operand noise.
fn gate_message_head(message: &str) -> String {
    const PREFIXES: [&str; 6] = [
        "runescript lowering failed: ",
        "runescript parse failed: ",
        "structured parse failed: ",
        "embedded ASM parse failed: ",
        "encoding original failed: ",
        "encoding runescript failed: ",
    ];
    let mut body = message;
    for prefix in PREFIXES {
        if let Some(rest) = body.strip_prefix(prefix) {
            body = rest;
            break;
        }
    }
    let mut out = String::new();
    let mut last_digit = false;
    for c in body.chars() {
        if c.is_ascii_digit() {
            if !last_digit {
                out.push('#');
                last_digit = true;
            }
        } else {
            last_digit = false;
            out.push(c);
        }
        if out.len() >= 200 {
            break;
        }
    }
    out
}

/// The byte gate over the **RuneScript** surface (G1.4): render the structured form to RuneScript,
/// parse it back, lower the result, and compare bytes against the original — proving the RuneScript
/// editing surface round-trips byte-exactly. Reuses the same `reverse_ctx`/`opcode_book` and the same
/// `expected` bytes as the TS gate; the only inserted steps are `render_runescript` + `parse_runescript`.
/// Build-948-only (the gate context's command registry is 948).
fn recompile_fidelity_check_runescript(
    parsed: &crate::transpile::ParsedReversibleSource,
    metadata: &crate::transpile::ReversibleMetadata,
    reverse_ctx: &ReverseCompileContext,
    opcode_book: &OpcodeBook,
) -> std::result::Result<(), RecompileBlock> {
    let build = metadata.build;
    let original = parse_cs2_asm(&parsed.asm_trailer).map_err(|e| {
        RecompileBlock::reverse_unsupported(format!("embedded ASM parse failed: {e}"))
    })?;
    let expected = encode_script(&original, opcode_book, build).map_err(|e| {
        RecompileBlock::reverse_unsupported(format!("encoding original failed: {e}"))
    })?;

    let structured = parse_structured_typescript(&parsed.structured_source).map_err(|e| {
        RecompileBlock::reverse_unsupported(format!("structured parse failed: {e}"))
    })?;
    let ctx = runescript_gate_context(reverse_ctx);
    let rendered = crate::transpile::render_runescript(&structured, ctx);
    let reparsed = crate::transpile::parse_runescript(&rendered, ctx).map_err(|e| {
        RecompileBlock::reverse_unsupported(format!("runescript parse failed: {e}"))
    })?;
    let compiled = lower_structured_script(&reparsed, metadata, reverse_ctx).map_err(|e| {
        RecompileBlock::reverse_unsupported(format!("runescript lowering failed: {e}"))
    })?;
    let actual = encode_script(&compiled, opcode_book, build).map_err(|e| {
        RecompileBlock::reverse_unsupported(format!("encoding runescript failed: {e}"))
    })?;

    // Deep-dive diagnostic: dump the original vs round-tripped instruction streams for a named script
    // (RS3_RS_DUMP=<substring of the structured source>) so ALL divergences are visible, not just the
    // first one `recompile_divergence` reports.
    if let Ok(target) = std::env::var("RS3_RS_DUMP")
        && !target.is_empty()
        && parsed.structured_source.contains(&target)
    {
        let dump = |s: &crate::script::CompiledScript| -> String {
            s.code
                .iter()
                .enumerate()
                .map(|(i, ins)| format!("{i}: {} {:?}", ins.command, ins.operand))
                .collect::<Vec<_>>()
                .join("\n")
        };
        std::fs::write("/tmp/rs-orig.asm", dump(&original)).ok();
        std::fs::write("/tmp/rs-roundtrip.asm", dump(&compiled)).ok();
        std::fs::write("/tmp/rs-rendered.rs", &rendered).ok();
    }

    if actual != expected {
        let (cause, message) = recompile_divergence(&original, &compiled);
        return Err(RecompileBlock {
            blocker: "runescript_mismatch",
            cause: Some(cause),
            message,
        });
    }
    Ok(())
}

/// The shared `RuneScriptContext` for the byte gate, built once from the script catalog. The gosub
/// name-set is **not** cosmetic for byte fidelity: a gosub whose script name collides with a command
/// name (`date_runeday`, `error`, …) or contains underscores must render with `~` so it parses back
/// as a gosub rather than re-lowering as that command. The catalog is the same for every script in a
/// build run, so the first call populates the set.
fn runescript_gate_context(
    reverse_ctx: &ReverseCompileContext,
) -> &'static crate::transpile::RuneScriptContext {
    use std::sync::OnceLock;
    static CTX: OnceLock<crate::transpile::RuneScriptContext> = OnceLock::new();
    CTX.get_or_init(|| {
        let scripts = reverse_ctx
            .script_catalog
            .export_name_map()
            .into_values()
            .collect();
        crate::transpile::RuneScriptContext::new(scripts)
    })
}

/// Describe the first instruction-level divergence between the original script
/// and the structured recompile, to make `recompile_mismatch` actionable.
/// Returns `(cause, message)`: `cause` is a low-cardinality bucket key (command
/// names only, never operand values) for the coverage histogram; `message` is
/// the human-readable detail for the diagnostics log.
fn recompile_divergence(
    original: &crate::script::CompiledScript,
    compiled: &crate::script::CompiledScript,
) -> (String, String) {
    for (i, (a, b)) in original.code.iter().zip(compiled.code.iter()).enumerate() {
        let command_differs = a.command != b.command;
        let operand_differs = format!("{:?}", a.operand) != format!("{:?}", b.operand);
        if command_differs || operand_differs {
            let cause = if command_differs {
                format!("{}->{}", a.command, b.command)
            } else {
                format!("{}:operand", a.command)
            };
            let message = format!(
                "recompile diverges at [{i}]: original `{} {:?}` vs structured `{} {:?}`",
                a.command, a.operand, b.command, b.operand
            );
            return (cause, message);
        }
    }
    if original.code.len() != compiled.code.len() {
        let cause = if original.code.len() < compiled.code.len() {
            "length:structured_longer"
        } else {
            "length:structured_shorter"
        };
        let message = format!(
            "recompile length mismatch: original {} instructions vs structured {}",
            original.code.len(),
            compiled.code.len()
        );
        return (cause.to_string(), message);
    }
    (
        "header_or_locals".to_string(),
        "recompile differs in header/locals/args only".to_string(),
    )
}

fn run_dump_raw_flat(
    cache: &FlatCache,
    tar_path: &Path,
    out_dir: &Path,
    archives: Option<&str>,
) -> Result<()> {
    let archive_filter: Option<Vec<u32>> = archives.map(|s| {
        s.split(',')
            .filter_map(|p| p.trim().parse::<u32>().ok())
            .collect()
    });

    // Ensure all archives are extracted from tar if needed
    if tar_path.is_file() {
        let archives_to_ensure = if let Some(filter) = archive_filter.as_deref() {
            filter.to_vec()
        } else {
            crate::dump::discover_archives(cache)?
        };
        for id in archives_to_ensure {
            crate::fixture::ensure_archive_complete(cache.root(), tar_path, id)?;
        }
    }

    let stats = crate::dump::dump_raw_flat(cache, out_dir, archive_filter.as_deref())?;

    eprintln!(
        "Dumped {} archives, {} groups, {} bytes in {} ms",
        stats.archives, stats.groups_copied, stats.total_bytes, stats.elapsed_ms
    );
    Ok(())
}

fn run_dump_refs(cache: &FlatCache, tar_path: &Path, out_dir: &Path, build: u32) -> Result<()> {
    for archive in [
        ARCHIVE_CONFIG,
        ARCHIVE_ENUM_CONFIG,
        ARCHIVE_OBJ_CONFIG,
        ARCHIVE_NPC_CONFIG,
        ARCHIVE_LOC_CONFIG,
        ARCHIVE_SEQ_CONFIG,
        ARCHIVE_SPOT_CONFIG,
        ARCHIVE_STRUCT_CONFIG,
    ] {
        ensure_archive_complete(cache.root(), tar_path, archive)?;
    }
    let cache = FlatCache::open(cache.root())?;
    let graph = crate::config_refs::build_config_ref_graph(&cache, build)?;
    crate::config_refs::write_refs_json(&graph, out_dir)?;
    Ok(())
}

fn run_dump_deps(
    cache: &FlatCache,
    tar_path: &Path,
    data_dir: &Path,
    out_dir: &Path,
    build: u32,
    subbuild: u32,
) -> Result<()> {
    let ctx = ResolverContext::load_lazy(cache, tar_path, data_dir, build, subbuild)?;
    let _ = crate::overlay_deps::write_dependency_files(&ctx, out_dir)?;
    Ok(())
}

fn run_dump_configs(cache: &FlatCache, tar_path: &Path, out_dir: &Path, build: u32) -> Result<()> {
    for archive in [
        ARCHIVE_CONFIG,
        ARCHIVE_ENUM_CONFIG,
        ARCHIVE_OBJ_CONFIG,
        ARCHIVE_NPC_CONFIG,
        ARCHIVE_LOC_CONFIG,
        ARCHIVE_SEQ_CONFIG,
        ARCHIVE_SPOT_CONFIG,
        ARCHIVE_STRUCT_CONFIG,
    ] {
        ensure_archive_complete(cache.root(), tar_path, archive)?;
    }
    let cache2 = FlatCache::open(cache.root())?;
    crate::config_dump::dump_config_texts(&cache2, out_dir, build)?;
    Ok(())
}

fn run_prepare_overlay(
    cache: &FlatCache,
    tar_path: &Path,
    data_dir: &Path,
    semantic_root: &Path,
    build: u32,
    subbuild: u32,
    archives: Option<&str>,
) -> Result<()> {
    let mut commands_run = Vec::new();
    let raw_flat_started = Instant::now();

    let raw_flat_dir = semantic_root.join("raw-flat");
    run_dump_raw_flat(cache, tar_path, &raw_flat_dir, archives)?;
    let raw_flat_elapsed = raw_flat_started.elapsed();
    commands_run.push("dump-raw-flat".to_string());

    let refs_dir = semantic_root.join("refs");
    let deps_dir = semantic_root.join("deps");
    for archive in [
        ARCHIVE_CONFIG,
        ARCHIVE_ENUM_CONFIG,
        ARCHIVE_OBJ_CONFIG,
        ARCHIVE_NPC_CONFIG,
        ARCHIVE_LOC_CONFIG,
        ARCHIVE_SEQ_CONFIG,
        ARCHIVE_SPOT_CONFIG,
        ARCHIVE_STRUCT_CONFIG,
        ARCHIVE_INTERFACES,
        ARCHIVE_CLIENTSCRIPTS,
    ] {
        ensure_archive_complete(cache.root(), tar_path, archive)?;
    }
    let cache_root = cache.root().to_path_buf();
    let (refs_result, deps_result) = rayon::join(
        || -> Result<std::time::Duration> {
            let started = Instant::now();
            let refs_cache = FlatCache::open(&cache_root)?;
            run_dump_refs(&refs_cache, tar_path, &refs_dir, build)?;
            Ok(started.elapsed())
        },
        || -> Result<std::time::Duration> {
            let started = Instant::now();
            let deps_cache = FlatCache::open(&cache_root)?;
            run_dump_deps(&deps_cache, tar_path, data_dir, &deps_dir, build, subbuild)?;
            Ok(started.elapsed())
        },
    );
    let refs_elapsed = refs_result?;
    let deps_elapsed = deps_result?;
    commands_run.push("dump-refs".to_string());
    commands_run.push("dump-deps".to_string());

    let cache_fingerprint = crate::overlay_manifest::cache_fingerprint(cache);
    let artifacts = vec![
        crate::overlay_manifest::artifact_record("raw-flat", semantic_root)?,
        crate::overlay_manifest::artifact_record("refs", semantic_root)?,
        crate::overlay_manifest::artifact_record("deps", semantic_root)?,
    ];

    let manifest = crate::overlay_manifest::Rs3CacheManifest {
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
        build,
        subbuild,
        cache_dir: cache
            .root()
            .canonicalize()
            .unwrap_or_else(|_| cache.root().to_path_buf())
            .to_string_lossy()
            .into_owned(),
        cache_fingerprint,
        semantic_root: semantic_root
            .canonicalize()
            .unwrap_or_else(|_| semantic_root.to_path_buf())
            .to_string_lossy()
            .into_owned(),
        artifacts,
        commands_run,
        finished_at: crate::overlay_manifest::now_rfc3339(),
        skip_config_dumps: Some(true),
    };

    let manifest_path = crate::overlay_manifest::write_manifest(&manifest, semantic_root)?;
    eprintln!(
        "Prepared overlay semantic tree at {} (manifest: {})",
        semantic_root.display(),
        manifest_path.display()
    );
    eprintln!(
        "prepare-overlay timing: raw-flat={}ms refs={}ms deps={}ms",
        raw_flat_elapsed.as_millis(),
        refs_elapsed.as_millis(),
        deps_elapsed.as_millis(),
    );
    let _ = print_json(&manifest);
    Ok(())
}

// Loads both source and target caches for migration impact analysis.
fn load_source_resolver_context(
    target_cache: &FlatCache,
    target_tar_path: &Path,
    data_dir: &Path,
    source_cache_tar: Option<&Path>,
    source_build: u32,
    source_subbuild: u32,
) -> Result<(ResolverContext, Option<tempfile::TempDir>)> {
    let Some(source_tar) = source_cache_tar else {
        return Ok((
            ResolverContext::load_lazy(
                target_cache,
                target_tar_path,
                data_dir,
                source_build,
                source_subbuild,
            )?,
            None,
        ));
    };

    let temp_dir = tempfile::Builder::new()
        .prefix("rs3-cache-rs-source-")
        .tempdir()
        .context("creating isolated source cache")?;
    let source_cache_root = temp_dir.path().join("cache");
    fs::create_dir_all(&source_cache_root)
        .with_context(|| format!("creating {}", source_cache_root.display()))?;
    let source_cache = FlatCache::open(&source_cache_root)?;
    let source_ctx = ResolverContext::load_lazy(
        &source_cache,
        source_tar,
        data_dir,
        source_build,
        source_subbuild,
    )?;
    Ok((source_ctx, Some(temp_dir)))
}

// Loads both source and target caches for migration impact analysis.
#[allow(clippy::too_many_arguments)]
fn run_migrate_check(
    cache: &FlatCache,
    tar_path: &Path,
    data_dir: &Path,
    interface_group: u32,
    out_file: &Path,
    audit_dir: Option<&Path>,
    target_version: RuntimeVersion,
    source_cache_tar: Option<&Path>,
    source_build: u32,
    source_subbuild: u32,
    enable_remap: bool,
    remap_buffer: u32,
    validate_target: bool,
    allow_heuristic_sites: bool,
) -> Result<()> {
    let target_ctx = ResolverContext::load_lazy(
        cache,
        tar_path,
        data_dir,
        target_version.build,
        target_version.subbuild,
    )?;

    let (source_ctx, _source_cache_temp) = load_source_resolver_context(
        cache,
        tar_path,
        data_dir,
        source_cache_tar,
        source_build,
        source_subbuild,
    )?;

    let analyzer = crate::migrate::MigrationAnalyzer::new(source_ctx, target_ctx);
    let mut report = if enable_remap {
        analyzer.remap_interface(interface_group, remap_buffer)
    } else {
        analyzer.analyze_interface(interface_group)
    };
    if validate_target {
        report.target_validation = Some(analyzer.validate_interface_target(
            interface_group,
            &report.entities,
            report.remap.as_ref(),
            allow_heuristic_sites,
        ));
    }

    let json = serde_json::to_string_pretty(&report)?;
    std::fs::write(out_file, &json)?;

    eprintln!(
        "migration report: {} entities ({} safe, {} missing, {} id_conflict, {} changed, {} script_changed) written to {}",
        report.total_entities,
        report.summary.safe,
        report.summary.missing,
        report.summary.id_conflict,
        report.summary.changed,
        report.summary.script_changed,
        out_file.display()
    );
    if let Some(target_validation) = &report.target_validation {
        eprintln!(
            "target validation: {} components ({} blocked), {} scripts checked, {} encoded, {} valid, {} with errors, {} blocked, {} heuristic sites, {} unsupported sites",
            target_validation.summary.components_checked,
            target_validation.summary.components_blocked,
            target_validation.summary.scripts_checked,
            target_validation.summary.scripts_encoded,
            target_validation.summary.scripts_valid,
            target_validation.summary.scripts_with_errors,
            target_validation.summary.scripts_blocked,
            target_validation.summary.heuristic_sites,
            target_validation.summary.unsupported_sites,
        );
    }
    if let Some(audit_dir) = audit_dir {
        write_conflict_audit(&report, audit_dir)?;
    }
    Ok(())
}

// Loads both source and target caches for single-script migration analysis.
#[allow(clippy::too_many_arguments)]
fn run_migrate_script(
    cache: &FlatCache,
    tar_path: &Path,
    data_dir: &Path,
    script_id: u32,
    out_file: &Path,
    audit_dir: Option<&Path>,
    target_version: RuntimeVersion,
    source_cache_tar: Option<&Path>,
    source_build: u32,
    source_subbuild: u32,
    enable_remap: bool,
    remap_buffer: u32,
    validate_target: bool,
    allow_heuristic_sites: bool,
) -> Result<()> {
    let target_ctx = ResolverContext::load_lazy(
        cache,
        tar_path,
        data_dir,
        target_version.build,
        target_version.subbuild,
    )?;

    let (source_ctx, _source_cache_temp) = load_source_resolver_context(
        cache,
        tar_path,
        data_dir,
        source_cache_tar,
        source_build,
        source_subbuild,
    )?;

    let analyzer = crate::migrate::MigrationAnalyzer::new(source_ctx, target_ctx);
    let mut report = if enable_remap {
        analyzer.remap_script(script_id, remap_buffer)
    } else {
        analyzer.analyze_script(script_id)
    };
    if validate_target {
        report.target_validation = Some(analyzer.validate_script_target(
            &report.entities,
            report.remap.as_ref(),
            allow_heuristic_sites,
        ));
    }

    let json = serde_json::to_string_pretty(&report)?;
    std::fs::write(out_file, &json)?;

    eprintln!(
        "script migration report: {} entities ({} safe, {} missing, {} id_conflict, {} changed, {} script_changed) written to {}",
        report.total_entities,
        report.summary.safe,
        report.summary.missing,
        report.summary.id_conflict,
        report.summary.changed,
        report.summary.script_changed,
        out_file.display()
    );
    if let Some(target_validation) = &report.target_validation {
        eprintln!(
            "target validation: {} scripts checked, {} encoded, {} valid, {} with errors, {} blocked, {} heuristic sites, {} unsupported sites",
            target_validation.summary.scripts_checked,
            target_validation.summary.scripts_encoded,
            target_validation.summary.scripts_valid,
            target_validation.summary.scripts_with_errors,
            target_validation.summary.scripts_blocked,
            target_validation.summary.heuristic_sites,
            target_validation.summary.unsupported_sites,
        );
    }
    if let Some(audit_dir) = audit_dir {
        write_script_audit(&report, audit_dir)?;
    }
    Ok(())
}

fn write_conflict_audit(report: &crate::migrate::ConflictReport, audit_dir: &Path) -> Result<()> {
    let Some(target_validation) = &report.target_validation else {
        return Ok(());
    };
    write_target_validation_audit(
        target_validation,
        report.reference_updates.as_deref().unwrap_or(&[]),
        audit_dir,
        &serde_json::json!({
            "source_build": report.source_build,
            "target_build": report.target_build,
            "interface_group": report.interface_group,
            "total_entities": report.total_entities,
            "summary": report.summary,
        }),
    )
}

fn write_script_audit(report: &crate::migrate::ScriptReport, audit_dir: &Path) -> Result<()> {
    let Some(target_validation) = &report.target_validation else {
        return Ok(());
    };
    write_target_validation_audit(
        target_validation,
        report.reference_updates.as_deref().unwrap_or(&[]),
        audit_dir,
        &serde_json::json!({
            "source_build": report.source_build,
            "target_build": report.target_build,
            "script_id": report.script_id,
            "total_entities": report.total_entities,
            "summary": report.summary,
        }),
    )
}

fn write_target_validation_audit(
    target_validation: &crate::migrate::TargetValidationReport,
    reference_updates: &[crate::migrate::ReferenceUpdate],
    audit_dir: &Path,
    migration_summary: &serde_json::Value,
) -> Result<()> {
    fs::create_dir_all(audit_dir).with_context(|| format!("creating {}", audit_dir.display()))?;

    let summary_path = audit_dir.join("summary.json");
    fs::write(
        &summary_path,
        serde_json::to_string_pretty(&serde_json::json!({
            "migration": migration_summary,
            "target_validation": target_validation.summary,
            "remap_applied": target_validation.remap_applied,
            "target_build": target_validation.target_build,
        }))?,
    )
    .with_context(|| format!("writing {}", summary_path.display()))?;

    let failed_components = target_validation
        .components
        .iter()
        .filter(|component| {
            !component.blocking_issues.is_empty()
                || !component.heuristic_sites.is_empty()
                || !component.unsupported_sites.is_empty()
        })
        .collect::<Vec<_>>();
    write_jsonl_file(
        &audit_dir.join("components_failed.jsonl"),
        &failed_components,
    )?;

    let failed_scripts = target_validation
        .scripts
        .iter()
        .filter(|script| {
            script.failure.is_some()
                || !script.validation_errors.is_empty()
                || !script.blockers.is_empty()
                || !script.unsupported_sites.is_empty()
        })
        .collect::<Vec<_>>();
    write_jsonl_file(&audit_dir.join("scripts_failed.jsonl"), &failed_scripts)?;

    let heuristic_sites = target_validation
        .components
        .iter()
        .flat_map(|component| {
            component.heuristic_sites.iter().map(move |site| {
                serde_json::json!({
                    "owner_kind": "component",
                    "component_id": component.component_id,
                    "component_name": component.name,
                    "site": site,
                })
            })
        })
        .chain(target_validation.scripts.iter().flat_map(|script| {
            script.heuristic_sites.iter().map(move |site| {
                serde_json::json!({
                    "owner_kind": "script",
                    "source_script_id": script.source_script_id,
                    "target_script_id": script.target_script_id,
                    "script_name": script.script_name,
                    "site": site,
                })
            })
        }))
        .collect::<Vec<_>>();
    write_jsonl_file(&audit_dir.join("heuristic_sites.jsonl"), &heuristic_sites)?;

    let unsupported_sites = target_validation
        .components
        .iter()
        .flat_map(|component| {
            component.unsupported_sites.iter().map(move |site| {
                serde_json::json!({
                    "owner_kind": "component",
                    "component_id": component.component_id,
                    "component_name": component.name,
                    "site": site,
                })
            })
        })
        .chain(target_validation.scripts.iter().flat_map(|script| {
            script.unsupported_sites.iter().map(move |site| {
                serde_json::json!({
                    "owner_kind": "script",
                    "source_script_id": script.source_script_id,
                    "target_script_id": script.target_script_id,
                    "script_name": script.script_name,
                    "site": site,
                })
            })
        }))
        .collect::<Vec<_>>();
    write_jsonl_file(
        &audit_dir.join("unsupported_sites.jsonl"),
        &unsupported_sites,
    )?;

    write_jsonl_file(&audit_dir.join("rewrites_applied.jsonl"), reference_updates)?;
    Ok(())
}

fn write_jsonl_file<T: Serialize>(path: &Path, rows: &[T]) -> Result<()> {
    let file = fs::File::create(path).with_context(|| format!("creating {}", path.display()))?;
    let mut writer = BufWriter::new(file);
    for row in rows {
        serde_json::to_writer(&mut writer, row)
            .with_context(|| format!("writing {}", path.display()))?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

fn run_ts_export(
    cache: &FlatCache,
    tar_path: &Path,
    data_dir: &Path,
    out_dir: &Path,
    version: RuntimeVersion,
) -> Result<()> {
    let ctx = ResolverContext::load_ts_export(
        cache,
        tar_path,
        data_dir,
        version.build,
        version.subbuild,
    )?;
    let opcode_book = ctx.opcode_book.clone();
    let script_group_names = load_script_group_names_from_cache(cache, data_dir)?;
    fs::create_dir_all(out_dir)?;

    export_var_types(&ctx, out_dir)?;
    export_varbit_types(&ctx, out_dir)?;
    export_enum_types(&ctx, out_dir)?;
    export_struct_types(&ctx, out_dir)?;
    export_param_types(&ctx, out_dir)?;
    export_interface_ids(&ctx, out_dir)?;
    export_inv_types(&ctx, out_dir)?;
    export_obj_types(&ctx, out_dir)?;
    export_npc_types(&ctx, out_dir)?;
    export_loc_types(&ctx, out_dir)?;
    export_seq_types(&ctx, out_dir)?;
    export_spot_types(&ctx, out_dir)?;
    export_named_config_ids(&ctx, out_dir)?;
    export_db_types(&ctx, out_dir)?;
    export_script_signatures_from_cache(
        cache,
        tar_path,
        out_dir,
        &opcode_book,
        version.build,
        &script_group_names,
    )?;
    export_index(out_dir)?;

    eprintln!("typescript definitions exported to {}", out_dir.display());
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "CLI dispatcher passes parsed command fields"
)]
fn run_transpile_scripts(
    cache: &FlatCache,
    tar_path: &Path,
    data_dir: &Path,
    out_dir: &Path,
    filter_script: Option<&str>,
    output_style: TranspileOutputStyle,
    max_scripts: usize,
    all_scripts: bool,
    limits: crate::transpile::TranspileLimits,
    version: RuntimeVersion,
) -> Result<()> {
    if let Some(filter) = filter_script
        && !all_scripts
    {
        return run_filtered_transpile_scripts(
            cache,
            tar_path,
            data_dir,
            out_dir,
            filter,
            output_style,
            max_scripts,
            limits,
            version,
        );
    }

    let types_exist = out_dir.join("index.ts").exists();
    let mut ctx = if types_exist {
        ResolverContext::load_transpile(cache, tar_path, data_dir, version.build, version.subbuild)?
    } else {
        ResolverContext::load_lazy(cache, tar_path, data_dir, version.build, version.subbuild)?
    };
    let opcode_book = ctx.opcode_book.clone();
    let script_group_names = load_script_group_names_from_cache(cache, data_dir)?;
    let mut script_catalog_builder = crate::transpile::ScriptCatalogBuilder::new(
        &script_group_names,
        &opcode_book,
        version.build,
    )
    .without_return_types();
    for (&packed_id_raw, data) in &ctx.scripts {
        script_catalog_builder.add_script(packed_id_raw, data);
    }
    let script_catalog = script_catalog_builder.build();
    let mut transpiler = Transpiler::new()
        .with_version(version.build, version.subbuild)
        .with_enums(&ctx.enums)
        .with_enums_map(&ctx.enums)
        .with_vars(&ctx.varps_by_domain)
        .with_varbits(&ctx.varbits)
        .with_params(&ctx.params)
        .with_limits(limits)
        .with_script_catalog(script_catalog.clone())
        .with_components(&ctx.parsed_components)
        .with_script_signatures(&ctx.scripts, &opcode_book, version.build);

    let mut reverse_ctx = build_reverse_compile_context_from_catalog(&ctx, script_catalog.clone());
    reverse_ctx.script_signatures.extend(
        transpiler
            .script_signatures()
            .iter()
            .map(|(script_id, signature)| (*script_id, signature.clone())),
    );

    fs::create_dir_all(out_dir)?;

    // Generate type definitions so script imports resolve.
    // Skip if index.ts already exists (user may have run ts-export).
    if !out_dir.join("index.ts").exists() {
        export_var_types(&ctx, out_dir)?;
        export_varbit_types(&ctx, out_dir)?;
        export_enum_types(&ctx, out_dir)?;
        export_param_types(&ctx, out_dir)?;
        export_interface_ids(&ctx, out_dir)?;
        export_inv_types(&ctx, out_dir)?;
        export_obj_types(&ctx, out_dir)?;
        export_npc_types(&ctx, out_dir)?;
        export_loc_types(&ctx, out_dir)?;
        export_seq_types(&ctx, out_dir)?;
        export_spot_types(&ctx, out_dir)?;
        export_named_config_ids(&ctx, out_dir)?;
        export_db_types(&ctx, out_dir)?;
        export_script_signatures(out_dir, &script_catalog)?;
        export_index(out_dir)?;
    }

    trim_transpile_runtime_context(&mut ctx);

    let script_limit = if all_scripts { usize::MAX } else { max_scripts };
    let trace_transpile = std::env::var_os("RS3_TRANSPILE_TRACE").is_some();

    let mut signature_cache = transpiler.script_signatures().clone();
    let mut script_count = 0;
    let mut errors = 0;
    let mut barrel_exports: Vec<String> = Vec::new();
    let mut script_diagnostics = Vec::new();

    for (&script_id_raw, data) in &ctx.scripts {
        let script_id = crate::transpile::ScriptId(script_id_raw as i32);
        if trace_transpile {
            let script_name = transpiler
                .script_name_for(script_id)
                .unwrap_or_else(|| format!("script{script_id}"));
            eprintln!("trace: transpile script_{script_id_raw} {script_name}");
        }

        if let Some(filter) = filter_script {
            let name = transpiler.script_name_for(script_id);
            if name.map(|n| !n.contains(filter)).unwrap_or(true) {
                continue;
            }
        }

        let script = match decode_script(data, &opcode_book, version.build) {
            Ok(script) => script,
            Err(err) => {
                eprintln!("failed to decode script_{script_id}: {err}");
                errors += 1;
                continue;
            }
        };
        for referenced_script in collect_referenced_scripts(&script) {
            let Some(metadata) = script_catalog.resolve_call_target(referenced_script.0) else {
                continue;
            };
            let Some(target_data) = ctx.scripts.get(&(metadata.packed_id.0 as u32)) else {
                continue;
            };
            ensure_transpile_script_signature_from_bytes(
                &mut signature_cache,
                &mut transpiler,
                &script_catalog,
                metadata.packed_id,
                target_data,
                &opcode_book,
                version.build,
            );
            // Mirror the inferred return type into the lowering context so the
            // recompile-fidelity gate classifies this call (void vs value) the
            // same way the renderer did.
            if let Some(signature) = signature_cache.get(&metadata.packed_id) {
                reverse_ctx
                    .script_signatures
                    .insert(metadata.packed_id, signature.clone());
            }
        }
        ensure_transpile_script_signature(
            &mut signature_cache,
            &mut transpiler,
            &script_catalog,
            script_id,
            &script,
            version.build,
        );

        match transpile_script_with_style(&transpiler, &script, script_id, output_style) {
            Ok(ts) => {
                let crate::transpile::TranspiledScript {
                    source,
                    diagnostics,
                    editable_structured,
                    blocking_diagnostics,
                    control_flow_fallback_reason,
                    ..
                } = ts;
                let script_name = transpiler
                    .script_name_for(script_id)
                    .unwrap_or_else(|| format!("script{script_id}"));
                let function_name = script_name.clone();
                let filename = format!("{}.ts", sanitize_file_component(&script_name));
                let out_path = out_dir.join(&filename);
                barrel_exports.push(format!(
                    "export {{ {function_name} }} from './{filename_no_ext}';",
                    filename_no_ext = filename.trim_end_matches(".ts")
                ));
                let mut diagnostics = diagnostics;
                let mut editable_structured = editable_structured;
                let mut blocking_diagnostics = blocking_diagnostics;
                add_ambiguous_export_warning(&mut diagnostics, &script_catalog, script_id);
                let finalized = finalize_with_linear_fallback(
                    source,
                    &transpiler,
                    &script,
                    script_id,
                    &script_catalog,
                    &reverse_ctx,
                    &opcode_book,
                    &mut diagnostics,
                    &mut editable_structured,
                    &mut blocking_diagnostics,
                )?;
                let high_ts_style = HighTsScriptStyle::from_source(&finalized.source);
                let high_ts_fallback_reason = output_style_fallback_reason(
                    output_style,
                    finalized.fallback_reason.or(control_flow_fallback_reason),
                );
                let high_ts_gate_diagnostics = match output_style {
                    TranspileOutputStyle::HighTs => finalized.primary_blocking_diagnostics,
                    TranspileOutputStyle::Reversible => Vec::new(),
                };
                let high_ts_gate_messages = match output_style {
                    TranspileOutputStyle::HighTs => finalized.primary_gate_messages,
                    TranspileOutputStyle::Reversible => Vec::new(),
                };
                fs::write(&out_path, &finalized.source)?;
                if let Some(metadata) = script_catalog.get(script_id) {
                    script_diagnostics.push(ScriptDiagnosticsEntry {
                        packed_id: metadata.packed_id.0,
                        group_id: metadata.group_id.0,
                        export_name: metadata.export_name.clone(),
                        module_name: metadata.module_name.clone(),
                        editable_structured,
                        blocking_diagnostics,
                        high_ts_style,
                        high_ts_fallback_reason,
                        high_ts_gate_diagnostics,
                        high_ts_gate_messages,
                        diagnostics: diagnostics.diagnostics,
                    });
                }
                script_count += 1;
                if script_count >= script_limit {
                    break;
                }
            }
            Err(e) => {
                eprintln!("failed to transpile script_{script_id}: {e}");
                errors += 1;
            }
        }
    }

    // Write scripts barrel file so you can import { script_N } from './scripts'
    if !barrel_exports.is_empty() {
        barrel_exports.sort();
        let mut lines = vec![
            "// Auto-generated scripts barrel".to_string(),
            "// Source: RS3 cache transpile-scripts".to_string(),
            String::new(),
        ];
        lines.extend(barrel_exports);
        write_text(&out_dir.join("scripts.ts"), &lines.join("\n"))?;
    }

    write_transpile_diagnostics_report(out_dir, version.build, Vec::new(), script_diagnostics)?;

    eprintln!(
        "transpiled {script_count} scripts ({errors} errors) to {}",
        out_dir.display()
    );
    Ok(())
}

struct LoadedScript {
    packed_id: u32,
    script: CompiledScript,
    data: Vec<u8>,
}

#[expect(
    clippy::too_many_arguments,
    reason = "filtered transpile path needs CLI/runtime inputs and cache helpers"
)]
fn run_filtered_transpile_scripts(
    cache: &FlatCache,
    tar_path: &Path,
    data_dir: &Path,
    out_dir: &Path,
    filter_script: &str,
    output_style: TranspileOutputStyle,
    max_scripts: usize,
    limits: crate::transpile::TranspileLimits,
    version: RuntimeVersion,
) -> Result<()> {
    let types_exist = out_dir.join("index.ts").exists();
    let ctx = if types_exist {
        ResolverContext::load_transpile(cache, tar_path, data_dir, version.build, version.subbuild)?
    } else {
        ResolverContext::load_ts_export(cache, tar_path, data_dir, version.build, version.subbuild)?
    };
    let opcode_book = ctx.opcode_book.clone();
    let script_group_names = load_script_group_names_from_cache(cache, data_dir)?;
    let script_archive = FlatCache::open(cache.root())?;
    let script_index = script_archive.archive_index(ARCHIVE_CLIENTSCRIPTS)?;
    let selected_scripts = load_matching_scripts_from_cache(
        &script_archive,
        &script_index,
        &opcode_book,
        version.build,
        &script_group_names,
        filter_script,
        max_scripts,
    )?;

    fs::create_dir_all(out_dir)?;

    // Build the catalog over the selected scripts plus their call targets first,
    // with return-type inference. The filtered run only emits this small set, so
    // the catalog is sufficient for scripts.d.ts and lets it carry accurate
    // signatures (group-based names + real return types) instead of the bulk
    // `script<packed>(): unknown` declarations the cache scan produced.
    let mut script_data = BTreeMap::new();
    let mut script_catalog_builder = crate::transpile::ScriptCatalogBuilder::new(
        &script_group_names,
        &opcode_book,
        version.build,
    );
    for loaded in &selected_scripts {
        script_data.insert(loaded.packed_id, loaded.data.clone());
        script_catalog_builder.add_script(loaded.packed_id, &loaded.data);
    }
    for loaded in &selected_scripts {
        for referenced_script in collect_referenced_scripts(&loaded.script) {
            let raw_id = referenced_script.0;
            let Some((packed_id, data)) =
                load_script_call_target_from_cache(&script_archive, &script_index, raw_id)?
            else {
                continue;
            };
            if script_data.contains_key(&packed_id) {
                continue;
            }
            script_catalog_builder.add_script(packed_id, &data);
            script_data.insert(packed_id, data);
        }
    }
    let script_catalog = script_catalog_builder.build();
    let reverse_ctx = build_reverse_compile_context_from_catalog(&ctx, script_catalog.clone());

    if !types_exist {
        export_var_types(&ctx, out_dir)?;
        export_varbit_types(&ctx, out_dir)?;
        export_enum_types(&ctx, out_dir)?;
        export_param_types(&ctx, out_dir)?;
        export_interface_ids(&ctx, out_dir)?;
        export_inv_types(&ctx, out_dir)?;
        export_obj_types(&ctx, out_dir)?;
        export_npc_types(&ctx, out_dir)?;
        export_loc_types(&ctx, out_dir)?;
        export_seq_types(&ctx, out_dir)?;
        export_spot_types(&ctx, out_dir)?;
        export_named_config_ids(&ctx, out_dir)?;
        export_db_types(&ctx, out_dir)?;
        export_script_signatures(out_dir, &script_catalog)?;
        export_index(out_dir)?;
    }

    let mut transpiler = Transpiler::new()
        .with_version(version.build, version.subbuild)
        .with_enums(&ctx.enums)
        .with_enums_map(&ctx.enums)
        .with_vars(&ctx.varps_by_domain)
        .with_varbits(&ctx.varbits)
        .with_params(&ctx.params)
        .with_limits(limits)
        .with_script_catalog(script_catalog.clone())
        .with_components(&ctx.parsed_components);

    let mut signature_cache = HashMap::new();
    let mut script_count = 0;
    let mut errors = 0;
    let mut barrel_exports: Vec<String> = Vec::new();
    let mut script_diagnostics = Vec::new();

    for loaded in &selected_scripts {
        let script_id = crate::transpile::ScriptId(loaded.packed_id as i32);
        for referenced_script in collect_referenced_scripts(&loaded.script) {
            let Some(metadata) = script_catalog.resolve_call_target(referenced_script.0) else {
                continue;
            };
            let Some(target_data) = script_data.get(&(metadata.packed_id.0 as u32)) else {
                continue;
            };
            ensure_transpile_script_signature_from_bytes(
                &mut signature_cache,
                &mut transpiler,
                &script_catalog,
                metadata.packed_id,
                target_data,
                &opcode_book,
                version.build,
            );
        }
        ensure_transpile_script_signature(
            &mut signature_cache,
            &mut transpiler,
            &script_catalog,
            script_id,
            &loaded.script,
            version.build,
        );

        match transpile_script_with_style(&transpiler, &loaded.script, script_id, output_style) {
            Ok(ts) => {
                let crate::transpile::TranspiledScript {
                    source,
                    diagnostics,
                    editable_structured,
                    blocking_diagnostics,
                    control_flow_fallback_reason,
                    ..
                } = ts;
                let script_name = transpiler
                    .script_name_for(script_id)
                    .unwrap_or_else(|| format!("script{script_id}"));
                let function_name = script_name.clone();
                let filename = format!("{}.ts", sanitize_file_component(&script_name));
                let out_path = out_dir.join(&filename);
                barrel_exports.push(format!(
                    "export {{ {function_name} }} from './{filename_no_ext}';",
                    filename_no_ext = filename.trim_end_matches(".ts")
                ));
                let mut diagnostics = diagnostics;
                let mut editable_structured = editable_structured;
                let mut blocking_diagnostics = blocking_diagnostics;
                add_ambiguous_export_warning(&mut diagnostics, &script_catalog, script_id);
                let finalized = finalize_with_linear_fallback(
                    source,
                    &transpiler,
                    &loaded.script,
                    script_id,
                    &script_catalog,
                    &reverse_ctx,
                    &opcode_book,
                    &mut diagnostics,
                    &mut editable_structured,
                    &mut blocking_diagnostics,
                )?;
                let high_ts_style = HighTsScriptStyle::from_source(&finalized.source);
                let high_ts_fallback_reason = output_style_fallback_reason(
                    output_style,
                    finalized.fallback_reason.or(control_flow_fallback_reason),
                );
                let high_ts_gate_diagnostics = match output_style {
                    TranspileOutputStyle::HighTs => finalized.primary_blocking_diagnostics,
                    TranspileOutputStyle::Reversible => Vec::new(),
                };
                let high_ts_gate_messages = match output_style {
                    TranspileOutputStyle::HighTs => finalized.primary_gate_messages,
                    TranspileOutputStyle::Reversible => Vec::new(),
                };
                fs::write(&out_path, &finalized.source)?;
                if let Some(metadata) = script_catalog.get(script_id) {
                    script_diagnostics.push(ScriptDiagnosticsEntry {
                        packed_id: metadata.packed_id.0,
                        group_id: metadata.group_id.0,
                        export_name: metadata.export_name.clone(),
                        module_name: metadata.module_name.clone(),
                        editable_structured,
                        blocking_diagnostics,
                        high_ts_style,
                        high_ts_fallback_reason,
                        high_ts_gate_diagnostics,
                        high_ts_gate_messages,
                        diagnostics: diagnostics.diagnostics,
                    });
                }
                script_count += 1;
            }
            Err(err) => {
                eprintln!("failed to transpile script_{script_id}: {err}");
                errors += 1;
            }
        }
    }

    if !barrel_exports.is_empty() {
        barrel_exports.sort();
        let mut lines = vec![
            "// Auto-generated scripts barrel".to_string(),
            "// Source: RS3 cache transpile-scripts".to_string(),
            String::new(),
        ];
        lines.extend(barrel_exports);
        write_text(&out_dir.join("scripts.ts"), &lines.join("\n"))?;
    }

    write_transpile_diagnostics_report(out_dir, version.build, Vec::new(), script_diagnostics)?;

    eprintln!(
        "transpiled {script_count} scripts ({errors} errors) to {}",
        out_dir.display()
    );
    Ok(())
}

fn load_matching_scripts_from_cache<S: std::hash::BuildHasher + Clone>(
    cache: &FlatCache,
    index: &crate::js5::ArchiveIndex,
    opcode_book: &OpcodeBook,
    build: u32,
    group_names: &HashMap<u32, String, S>,
    filter_script: &str,
    max_scripts: usize,
) -> Result<Vec<LoadedScript>> {
    let mut selected = Vec::new();
    let mut seen_groups = HashSet::new();
    let mut preferred_groups: Vec<u32> = group_names
        .iter()
        .filter(|(_, name)| name.contains(filter_script))
        .map(|(&group, _)| group)
        .collect();
    preferred_groups.sort_unstable();
    preferred_groups.dedup();

    load_matching_scripts_from_groups(
        cache,
        index,
        opcode_book,
        build,
        group_names,
        filter_script,
        max_scripts,
        &preferred_groups,
        &mut seen_groups,
        &mut selected,
    )?;
    if selected.len() < max_scripts {
        load_matching_scripts_from_groups(
            cache,
            index,
            opcode_book,
            build,
            group_names,
            filter_script,
            max_scripts,
            &index.group_id,
            &mut seen_groups,
            &mut selected,
        )?;
    }
    Ok(selected)
}

#[expect(
    clippy::too_many_arguments,
    reason = "group scan needs filter and output accumulators"
)]
fn load_matching_scripts_from_groups<S: std::hash::BuildHasher + Clone>(
    cache: &FlatCache,
    index: &crate::js5::ArchiveIndex,
    opcode_book: &OpcodeBook,
    build: u32,
    group_names: &HashMap<u32, String, S>,
    filter_script: &str,
    max_scripts: usize,
    groups: &[u32],
    seen_groups: &mut HashSet<u32>,
    selected: &mut Vec<LoadedScript>,
) -> Result<()> {
    for &group in groups {
        if selected.len() >= max_scripts || !seen_groups.insert(group) {
            continue;
        }
        let files = cache.group_files_with_index(index, ARCHIVE_CLIENTSCRIPTS, group)?;
        for (file, data) in files {
            let packed_id = (group << 16) | file;
            let Ok(script) = decode_script(&data, opcode_book, build) else {
                continue;
            };
            let display_name = script
                .name
                .as_deref()
                .map(crate::transpile::extract_script_name_suffix)
                .filter(|name| !name.is_empty())
                .or_else(|| group_names.get(&group).cloned());
            // Match against the GROUP-based synthetic name (`script<group>`), the
            // same name the catalog and output files use. Naming by the packed id
            // here was the bug: group 621 became "script40697856" (unmatched)
            // while group 9476's packed id 621215744 spuriously contained the
            // "script621" filter substring.
            let function_name = crate::transpile::script_function_name(
                crate::transpile::ScriptId(group as i32),
                display_name.as_deref(),
            );
            if function_name.contains(filter_script)
                || display_name
                    .as_deref()
                    .is_some_and(|name| name.contains(filter_script))
            {
                selected.push(LoadedScript {
                    packed_id,
                    script,
                    data,
                });
                if selected.len() >= max_scripts {
                    return Ok(());
                }
            }
        }
    }
    Ok(())
}

fn load_script_call_target_from_cache(
    cache: &FlatCache,
    index: &crate::js5::ArchiveIndex,
    raw_id: i32,
) -> Result<Option<(u32, Vec<u8>)>> {
    let Ok(raw_id_u32) = u32::try_from(raw_id) else {
        return Ok(None);
    };

    if index.group_id.contains(&raw_id_u32) {
        let files = cache.group_files_with_index(index, ARCHIVE_CLIENTSCRIPTS, raw_id_u32)?;
        if let Some((file, data)) = files.into_iter().min_by_key(|(file, _)| *file) {
            return Ok(Some(((raw_id_u32 << 16) | file, data)));
        }
    }

    let group = raw_id_u32 >> 16;
    let file = raw_id_u32 & 0xffff;
    if !index.group_id.contains(&group) {
        return Ok(None);
    }
    let files = cache.group_files_with_index(index, ARCHIVE_CLIENTSCRIPTS, group)?;
    Ok(files.get(&file).cloned().map(|data| (raw_id_u32, data)))
}

fn collect_referenced_scripts(script: &CompiledScript) -> Vec<crate::transpile::ScriptId> {
    script
        .code
        .iter()
        .filter_map(|instruction| match instruction.operand {
            Operand::Script(id) => Some(crate::transpile::ScriptId(id)),
            _ => None,
        })
        .collect()
}

fn ensure_transpile_script_signature(
    signature_cache: &mut HashMap<crate::transpile::ScriptId, crate::transpile::ScriptSignature>,
    transpiler: &mut Transpiler,
    script_catalog: &crate::transpile::ScriptCatalog,
    script_id: crate::transpile::ScriptId,
    script: &CompiledScript,
    build: u32,
) {
    if signature_cache.contains_key(&script_id) {
        return;
    }
    if std::env::var_os("RS3_TRANSPILE_TRACE").is_some() {
        let script_name = script_catalog.export_name(script_id).unwrap_or("script");
        eprintln!(
            "trace: signature script_{script_id} {script_name} instructions={} args={}/{}/{}",
            script.code.len(),
            script.argument_count_int,
            script.argument_count_object,
            script.argument_count_long,
        );
        if std::env::var_os("RS3_TRANSPILE_TRACE_OPS").is_some() {
            for (index, instruction) in script.code.iter().enumerate() {
                eprintln!(
                    "trace: op[{index}] {} {:?}",
                    instruction.command, instruction.operand
                );
            }
        }
    }

    let signature =
        infer_transpile_script_signature(signature_cache, script_catalog, script_id, script, build);
    signature_cache.insert(script_id, signature.clone());
    transpiler.set_script_signature(script_id, signature);
}

fn ensure_transpile_script_signature_from_bytes(
    signature_cache: &mut HashMap<crate::transpile::ScriptId, crate::transpile::ScriptSignature>,
    transpiler: &mut Transpiler,
    script_catalog: &crate::transpile::ScriptCatalog,
    script_id: crate::transpile::ScriptId,
    data: &[u8],
    opcode_book: &OpcodeBook,
    version: u32,
) {
    if signature_cache.contains_key(&script_id) {
        return;
    }

    let Ok(script) = decode_script(data, opcode_book, version) else {
        return;
    };
    ensure_transpile_script_signature(
        signature_cache,
        transpiler,
        script_catalog,
        script_id,
        &script,
        version,
    );
}

fn infer_transpile_script_signature(
    signature_cache: &HashMap<crate::transpile::ScriptId, crate::transpile::ScriptSignature>,
    script_catalog: &crate::transpile::ScriptCatalog,
    script_id: crate::transpile::ScriptId,
    script: &CompiledScript,
    build: u32,
) -> crate::transpile::ScriptSignature {
    let empty_components: HashMap<u32, String> = HashMap::new();
    let empty_enums: HashMap<i32, String> = HashMap::new();
    let inferred = crate::transpile::infer_return_signature_for_script(
        script,
        script_id,
        build,
        &empty_components,
        &empty_enums,
        script_catalog,
        signature_cache,
    );
    crate::transpile::ScriptSignature {
        arg_count_int: script.argument_count_int,
        arg_count_obj: script.argument_count_object,
        arg_count_long: script.argument_count_long,
        return_count_int: inferred.return_counts.int,
        return_count_obj: inferred.return_counts.obj,
        return_count_long: inferred.return_counts.long,
        return_type: inferred.return_type,
    }
}

fn trim_transpile_runtime_context(ctx: &mut ResolverContext) {
    ctx.interfaces.clear();
    ctx.decoded_scripts.clear();
    ctx.structs.clear();
    ctx.npcs.clear();
    ctx.objs.clear();
    ctx.locs.clear();
    ctx.seqs.clear();
    ctx.spots.clear();
    ctx.invs.clear();
    ctx.dbtables.clear();
    ctx.dbrows.clear();
}

#[derive(Serialize)]
struct TranspileDiagnosticsReport {
    build: u32,
    coverage: CoverageSummary,
    high_ts_coverage: HighTsCoverageSummary,
    catalog: Vec<crate::transpile::Diagnostic>,
    scripts: Vec<ScriptDiagnosticsEntry>,
}

/// Aggregate structured-decompilation coverage: how many scripts produced
/// editable structured TypeScript vs. fell back to the lossless ASM trailer,
/// plus a histogram of the blockers that forced the fallback. This is the
/// headline completeness metric (see docs/cs2-completeness-plan.md).
#[derive(Serialize)]
struct CoverageSummary {
    total: usize,
    editable: usize,
    blocked: usize,
    editable_pct: f64,
    blockers: BTreeMap<String, usize>,
}

impl CoverageSummary {
    fn from_scripts(scripts: &[ScriptDiagnosticsEntry]) -> Self {
        let total = scripts.len();
        let editable = scripts.iter().filter(|s| s.editable_structured).count();
        let mut blockers = BTreeMap::<String, usize>::new();
        for script in scripts {
            for blocker in &script.blocking_diagnostics {
                *blockers.entry(blocker.clone()).or_default() += 1;
            }
        }
        let editable_pct = if total == 0 {
            0.0
        } else {
            (editable as f64) * 100.0 / (total as f64)
        };
        Self {
            total,
            editable,
            blocked: total - editable,
            editable_pct,
            blockers,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "diagnostic report mirrors independent marker flags"
)]
struct HighTsScriptStyle {
    #[serde(rename = "controlFlowMarkers")]
    control_flow_markers: bool,
    #[serde(rename = "controlFlowMarkerOccurrences")]
    control_flow_marker_occurrences: usize,
    #[serde(rename = "gotoCalls")]
    goto_calls: usize,
    #[serde(rename = "labelCalls")]
    label_calls: usize,
    #[serde(rename = "stackPseudos")]
    stack_pseudos: bool,
    #[serde(rename = "stackPseudoOccurrences")]
    stack_pseudo_occurrences: usize,
    #[serde(rename = "withMode")]
    with_mode: bool,
    #[serde(rename = "withModeOccurrences")]
    with_mode_occurrences: usize,
    #[serde(rename = "unknownCommand")]
    unknown_command: bool,
    #[serde(rename = "unknownCommandOccurrences")]
    unknown_command_occurrences: usize,
    #[serde(rename = "noVisibleLowLevelMarkers")]
    no_visible_low_level_markers: bool,
}

impl HighTsScriptStyle {
    fn from_source(source: &str) -> Self {
        let goto_calls = source.matches("goto(").count();
        let label_calls = source.matches("label(").count();
        let control_flow_marker_occurrences = goto_calls + label_calls;
        let control_flow_markers = control_flow_marker_occurrences > 0;
        let stack_pseudo_occurrences = source.matches("stackpush_then(").count()
            + source.matches("stackassign_").count()
            + source.matches("pop(").count()
            + source.matches("push(").count()
            + source
                .matches("push_array_int_leave_index_on_stack")
                .count();
        let stack_pseudos = stack_pseudo_occurrences > 0;
        let with_mode_occurrences = source.matches("WithMode").count();
        let with_mode = with_mode_occurrences > 0;
        let unknown_command_occurrences = source.matches("unknowncommand").count();
        let unknown_command = unknown_command_occurrences > 0;
        let no_visible_low_level_markers =
            !(control_flow_markers || stack_pseudos || with_mode || unknown_command);
        Self {
            control_flow_markers,
            control_flow_marker_occurrences,
            goto_calls,
            label_calls,
            stack_pseudos,
            stack_pseudo_occurrences,
            with_mode,
            with_mode_occurrences,
            unknown_command,
            unknown_command_occurrences,
            no_visible_low_level_markers,
        }
    }
}

#[derive(Serialize)]
struct HighTsCoverageSummary {
    total: usize,
    #[serde(rename = "controlFlowMarkers")]
    control_flow_markers: usize,
    #[serde(rename = "controlFlowMarkerOccurrences")]
    control_flow_marker_occurrences: usize,
    #[serde(rename = "gotoCalls")]
    goto_calls: usize,
    #[serde(rename = "labelCalls")]
    label_calls: usize,
    #[serde(rename = "stackPseudos")]
    stack_pseudos: usize,
    #[serde(rename = "stackPseudoOccurrences")]
    stack_pseudo_occurrences: usize,
    #[serde(rename = "withMode")]
    with_mode: usize,
    #[serde(rename = "withModeOccurrences")]
    with_mode_occurrences: usize,
    #[serde(rename = "unknownCommand")]
    unknown_command: usize,
    #[serde(rename = "unknownCommandOccurrences")]
    unknown_command_occurrences: usize,
    #[serde(rename = "noVisibleLowLevelMarkers")]
    no_visible_low_level_markers: usize,
    #[serde(rename = "noVisibleLowLevelMarkersPct")]
    no_visible_low_level_markers_pct: f64,
    #[serde(rename = "fallbackReasons")]
    fallback_reasons: BTreeMap<String, usize>,
    #[serde(rename = "fallbackGateBlockers")]
    fallback_gate_blockers: BTreeMap<String, usize>,
}

impl HighTsCoverageSummary {
    fn from_scripts(scripts: &[ScriptDiagnosticsEntry]) -> Self {
        let total = scripts.len();
        let mut fallback_reasons = BTreeMap::<String, usize>::new();
        let mut fallback_gate_blockers = BTreeMap::<String, usize>::new();
        for script in scripts {
            if let Some(reason) = &script.high_ts_fallback_reason {
                *fallback_reasons.entry(reason.clone()).or_default() += 1;
            }
            for blocker in &script.high_ts_gate_diagnostics {
                *fallback_gate_blockers.entry(blocker.clone()).or_default() += 1;
            }
        }
        let no_visible_low_level_markers = scripts
            .iter()
            .filter(|script| script.high_ts_style.no_visible_low_level_markers)
            .count();
        let no_visible_low_level_markers_pct = if total == 0 {
            0.0
        } else {
            (no_visible_low_level_markers as f64) * 100.0 / (total as f64)
        };
        Self {
            total,
            control_flow_markers: scripts
                .iter()
                .filter(|script| script.high_ts_style.control_flow_markers)
                .count(),
            control_flow_marker_occurrences: scripts
                .iter()
                .map(|script| script.high_ts_style.control_flow_marker_occurrences)
                .sum(),
            goto_calls: scripts
                .iter()
                .map(|script| script.high_ts_style.goto_calls)
                .sum(),
            label_calls: scripts
                .iter()
                .map(|script| script.high_ts_style.label_calls)
                .sum(),
            stack_pseudos: scripts
                .iter()
                .filter(|script| script.high_ts_style.stack_pseudos)
                .count(),
            stack_pseudo_occurrences: scripts
                .iter()
                .map(|script| script.high_ts_style.stack_pseudo_occurrences)
                .sum(),
            with_mode: scripts
                .iter()
                .filter(|script| script.high_ts_style.with_mode)
                .count(),
            with_mode_occurrences: scripts
                .iter()
                .map(|script| script.high_ts_style.with_mode_occurrences)
                .sum(),
            unknown_command: scripts
                .iter()
                .filter(|script| script.high_ts_style.unknown_command)
                .count(),
            unknown_command_occurrences: scripts
                .iter()
                .map(|script| script.high_ts_style.unknown_command_occurrences)
                .sum(),
            no_visible_low_level_markers,
            no_visible_low_level_markers_pct,
            fallback_reasons,
            fallback_gate_blockers,
        }
    }
}

#[derive(Serialize)]
struct ScriptDiagnosticsEntry {
    packed_id: i32,
    group_id: i32,
    export_name: String,
    module_name: String,
    #[serde(rename = "editableStructured")]
    editable_structured: bool,
    #[serde(rename = "blockingDiagnostics")]
    blocking_diagnostics: Vec<String>,
    #[serde(rename = "highTsStyle")]
    high_ts_style: HighTsScriptStyle,
    #[serde(rename = "highTsFallbackReason")]
    high_ts_fallback_reason: Option<String>,
    #[serde(rename = "highTsGateDiagnostics")]
    high_ts_gate_diagnostics: Vec<String>,
    #[serde(rename = "highTsGateMessages")]
    high_ts_gate_messages: Vec<String>,
    diagnostics: Vec<crate::transpile::Diagnostic>,
}

fn write_transpile_diagnostics_report(
    out_dir: &Path,
    build: u32,
    catalog: Vec<crate::transpile::Diagnostic>,
    mut scripts: Vec<ScriptDiagnosticsEntry>,
) -> Result<()> {
    scripts.sort_by(|a, b| a.packed_id.cmp(&b.packed_id));
    let coverage = CoverageSummary::from_scripts(&scripts);
    let high_ts_coverage = HighTsCoverageSummary::from_scripts(&scripts);
    // Canonical coverage event (queryable; the headline completeness metric).
    eprintln!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "event": "transpile_coverage",
            "build": build,
            "total": coverage.total,
            "editable": coverage.editable,
            "blocked": coverage.blocked,
            "editable_pct": coverage.editable_pct,
            "blockers": coverage.blockers,
        }))?
    );
    eprintln!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "event": "high_ts_coverage",
            "build": build,
            "total": high_ts_coverage.total,
            "control_flow_markers": high_ts_coverage.control_flow_markers,
            "control_flow_marker_occurrences": high_ts_coverage.control_flow_marker_occurrences,
            "goto_calls": high_ts_coverage.goto_calls,
            "label_calls": high_ts_coverage.label_calls,
            "stack_pseudos": high_ts_coverage.stack_pseudos,
            "stack_pseudo_occurrences": high_ts_coverage.stack_pseudo_occurrences,
            "with_mode": high_ts_coverage.with_mode,
            "with_mode_occurrences": high_ts_coverage.with_mode_occurrences,
            "unknown_command": high_ts_coverage.unknown_command,
            "unknown_command_occurrences": high_ts_coverage.unknown_command_occurrences,
            "no_visible_low_level_markers": high_ts_coverage.no_visible_low_level_markers,
            "no_visible_low_level_markers_pct": high_ts_coverage.no_visible_low_level_markers_pct,
            "fallback_reasons": high_ts_coverage.fallback_reasons,
            "fallback_gate_blockers": high_ts_coverage.fallback_gate_blockers,
        }))?
    );
    let report = TranspileDiagnosticsReport {
        build,
        coverage,
        high_ts_coverage,
        catalog,
        scripts,
    };
    let json = serde_json::to_string_pretty(&report)?;
    write_text(&out_dir.join("transpile-diagnostics.json"), &json)
}

fn script_base_export_name(metadata: &crate::transpile::ScriptMetadata) -> String {
    let base_name = crate::transpile::sanitize_export_name(&metadata.short_name);
    if base_name.is_empty() || base_name == "script" {
        format!("script{}", metadata.group_id.0)
    } else {
        base_name
    }
}

fn export_var_types(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    let mut entries: Vec<_> = ctx
        .varps_by_domain
        .iter()
        .flat_map(|(domain, vars)| {
            vars.values().map(|entry| VarTypeEntry {
                id: entry.id,
                domain: *domain,
                var_name: entry.var_name.clone(),
                type_id: entry.type_id,
                lifetime: entry.lifetime,
                transmit_level: entry.transmit_level,
                client_code: entry.client_code,
                domain_default: entry.domain_default,
                wiki_sync: entry.wiki_sync,
            })
        })
        .collect();

    entries.sort_by_key(|e| (e.domain as u8, e.id));

    let mut lines = vec![
        "// Auto-generated Var definitions".to_string(),
        "// Source: RS3 cache var config".to_string(),
        String::new(),
        "export type VarDomain = 'player' | 'npc' | 'client' | 'world' | 'region' | 'object' | 'clan' | 'clan_setting' | 'controller' | 'player_group' | 'global';".to_string(),
        "export type VarType = 'int' | 'long' | 'string' | 'unknown';".to_string(),
        "export type VarLifetime = 'temp' | 'perm' | 'serverperm' | 'unknown';".to_string(),
        "export type VarTransmitLevel = 'never' | 'on_set_different' | 'on_set_always' | 'unknown';".to_string(),
        String::new(),
        "export interface VarEntry {".to_string(),
        "    id: number;".to_string(),
        "    domain: VarDomain;".to_string(),
        "    name: string;".to_string(),
        "    type: VarType;".to_string(),
        "    lifetime: VarLifetime;".to_string(),
        "    transmitLevel: VarTransmitLevel;".to_string(),
        "    clientCode: number | null;".to_string(),
        "    domainDefault: boolean;".to_string(),
        "    wikiSync: boolean;".to_string(),
        "}".to_string(),
        String::new(),
        // Use composite key: domain_id * 1000000 + var_id
        "export const VARS: ReadonlyMap<number, VarEntry> = new Map([".to_string(),
    ];
    for entry in &entries {
        let type_str = match entry.type_id {
            Some(0) => "'int'",
            Some(1) => "'long'",
            Some(2) => "'string'",
            _ => "'unknown'",
        };
        let lifetime = entry.lifetime.unwrap_or("unknown");
        let transmit = entry.transmit_level.unwrap_or("unknown");
        let client_code = match entry.client_code {
            Some(c) => c.to_string(),
            None => "null".to_string(),
        };
        let domain_label = entry.domain.as_label();
        let composite_key = (u64::from(entry.domain) * 1_000_000) + u64::from(entry.id);
        lines.push(format!(
            "    [{}, {{ id: {}, domain: '{}', name: '{}', type: {}, lifetime: '{}', transmitLevel: '{}', clientCode: {}, domainDefault: {}, wikiSync: {} }}],",
            composite_key,
            entry.id,
            domain_label,
            escape_ts_string(&entry.var_name),
            type_str,
            lifetime,
            transmit,
            client_code,
            entry.domain_default,
            entry.wiki_sync
        ));
    }
    lines.push("]);".to_string());
    lines.push(String::new());
    lines.push(format!("export const VAR_COUNT = {};", entries.len()));

    write_lines(&out_dir.join("vars.ts"), &lines)
}

struct VarTypeEntry {
    id: u32,
    domain: crate::vars::VarDomain,
    var_name: String,
    type_id: Option<u8>,
    lifetime: Option<&'static str>,
    transmit_level: Option<&'static str>,
    client_code: Option<u16>,
    domain_default: bool,
    wiki_sync: bool,
}

fn export_varbit_types(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    let mut lines = vec![
        "// Auto-generated VarBit definitions".to_string(),
        "// Source: RS3 cache varbit config".to_string(),
        String::new(),
        "export interface VarBitEntry {".to_string(),
        "    id: number;".to_string(),
        "    name: string;".to_string(),
        "    domain: string | null;".to_string(),
        "    baseVar: number | null;".to_string(),
        "    startBit: number | null;".to_string(),
        "    endBit: number | null;".to_string(),
        "    wikiSync: boolean;".to_string(),
        "}".to_string(),
        String::new(),
    ];

    lines.push("export const VARBITS: ReadonlyMap<number, VarBitEntry> = new Map([".to_string());
    for entry in ctx.varbits.values() {
        let domain_str = match entry.domain {
            Some(d) => format!("'{}'", d.as_label()),
            None => "null".to_string(),
        };
        let base_var = entry
            .base_var
            .map(|v| v.to_string())
            .unwrap_or_else(|| "null".to_string());
        let start_bit = entry
            .start_bit
            .map(|v| v.to_string())
            .unwrap_or_else(|| "null".to_string());
        let end_bit = entry
            .end_bit
            .map(|v| v.to_string())
            .unwrap_or_else(|| "null".to_string());
        lines.push(format!(
            "    [{}, {{ id: {}, name: '{}', domain: {}, baseVar: {}, startBit: {}, endBit: {}, wikiSync: {} }}],",
            entry.id,
            entry.id,
            escape_ts_string(&entry.varbit_name),
            domain_str,
            base_var,
            start_bit,
            end_bit,
            entry.wiki_sync
        ));
    }
    lines.push("]);".to_string());
    lines.push(String::new());
    lines.push(format!(
        "export const VARBIT_COUNT = {};",
        ctx.varbits.len()
    ));

    write_lines(&out_dir.join("varbits.ts"), &lines)
}

fn export_enum_types(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    let mut writer = TextFileWriter::create(&out_dir.join("enums.ts"))?;
    writer.line("// Auto-generated Enum definitions")?;
    writer.line("// Source: RS3 cache enum config")?;
    writer.line("")?;

    // ── Per-enum const objects with named constants ──
    let mut reverse_lookup: Vec<(i32, String)> = Vec::new();

    for entry in ctx.enums.values() {
        if entry.values.is_empty() {
            continue;
        }
        let obj_name = format!("Enum_{id}", id = entry.id);
        let mut props: Vec<String> = Vec::new();
        let mut used_properties = HashSet::new();

        for pair in &entry.values {
            let unique_prop = enum_pair_property_name(&pair.value, pair.key, &mut used_properties);
            props.push(format!("    {unique_prop}: {key},", key = pair.key));
            reverse_lookup.push((pair.key, format!("{obj_name}.{unique_prop}")));
        }

        writer.line(format!("export const {obj_name} = {{"))?;
        for prop in props {
            writer.line(prop)?;
        }
        writer.line("} as const;")?;
        writer.line("")?;
    }

    // ── Reverse lookup: enum value → qualified name ──
    reverse_lookup.sort_by_key(|(k, _)| *k);
    reverse_lookup.dedup_by_key(|(k, _)| *k);
    if !reverse_lookup.is_empty() {
        writer.line("// Reverse lookup: maps enum key values to qualified names.")?;
        writer.line("export const ENUM_VALUE_TO_NAME: ReadonlyMap<number, string> = new Map([")?;
        for (key, name) in &reverse_lookup {
            writer.line(format!("    [{key}, '{name}'],"))?;
        }
        writer.line("]);")?;
        writer.line("")?;
    }

    // ── Existing types and runtime map ──
    writer.line("export interface EnumPair {")?;
    writer.line("    key: number;")?;
    writer.line("    value: number | string;")?;
    writer.line("    dense: boolean;")?;
    writer.line("}")?;
    writer.line("")?;
    writer.line("export interface EnumEntry {")?;
    writer.line("    id: number;")?;
    writer.line("    inputType: string;")?;
    writer.line("    outputType: string;")?;
    writer.line("    default: number | string | null;")?;
    writer.line("    values: EnumPair[];")?;
    writer.line("}")?;
    writer.line("")?;

    writer.line("export const ENUMS: ReadonlyMap<number, EnumEntry> = new Map([")?;
    for entry in ctx.enums.values() {
        let input_type = match entry.input_type_char {
            Some(b'i') => "'int'",
            Some(b's') => "'string'",
            _ => "'unknown'",
        };
        let output_type = match entry.output_type_char {
            Some(b'i') => "'int'",
            Some(b's') => "'string'",
            _ => "'unknown'",
        };
        let default = match &entry.default {
            Some(crate::config::ScalarValue::Int(i)) => i.to_string(),
            Some(crate::config::ScalarValue::Long(l)) => l.to_string(),
            Some(crate::config::ScalarValue::Str(s)) => format!("'{}'", escape_ts_string(s)),
            None => "null".to_string(),
        };
        let values_json: String = entry
            .values
            .iter()
            .map(|pair| {
                let val_str = match &pair.value {
                    crate::config::ScalarValue::Int(i) => i.to_string(),
                    crate::config::ScalarValue::Long(l) => l.to_string(),
                    crate::config::ScalarValue::Str(s) => format!("'{}'", escape_ts_string(s)),
                };
                format!(
                    "{{ key: {}, value: {}, dense: {} }}",
                    pair.key, val_str, pair.dense
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        writer.line(format!(
            "    [{}, {{ id: {}, inputType: {}, outputType: {}, default: {}, values: [{}] }}],",
            entry.id, entry.id, input_type, output_type, default, values_json
        ))?;
    }
    writer.line("]);")?;
    writer.line("")?;
    writer.line(format!("export const ENUM_COUNT = {};", ctx.enums.len()))?;
    writer.finish()
}

/// Convert a lowercase or mixed-case string value (e.g. "`skill_type`",
/// "my value") to `SCREAMING_SNAKE_CASE` for use as a
/// TypeScript const property name.
fn str_to_screaming_snake(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_uppercase());
        } else if c == ' ' || c == '-' || c == '/' || c == '.' {
            out.push('_');
        }
    }
    // Trim leading/trailing underscores
    let trimmed = out.trim_matches('_');
    // Can't start with a digit
    if trimmed.starts_with(|c: char| c.is_ascii_digit()) {
        format!("_{trimmed}")
    } else if trimmed.is_empty() {
        String::new()
    } else {
        trimmed.to_string()
    }
}

fn export_struct_types(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    let mut lines = vec![
        "// Auto-generated Struct definitions".to_string(),
        "// Source: RS3 cache struct config".to_string(),
        String::new(),
        "export interface StructParamEntry {".to_string(),
        "    id: number;".to_string(),
        "    value: number | string;".to_string(),
        "}".to_string(),
        String::new(),
        "export interface StructEntry {".to_string(),
        "    id: number;".to_string(),
        "    params: StructParamEntry[];".to_string(),
        "}".to_string(),
        String::new(),
    ];

    lines.push("export const STRUCTS: ReadonlyMap<number, StructEntry> = new Map([".to_string());
    for entry in ctx.structs.values() {
        let params_json = entry
            .params
            .iter()
            .map(|p| {
                format!(
                    "{{ id: {}, value: {} }}",
                    p.param_id,
                    format_scalar_value(&p.value)
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(format!(
            "    [{}, {{ id: {}, params: [{}] }}],",
            entry.id, entry.id, params_json
        ));
    }
    lines.push("]);".to_string());
    lines.push(String::new());
    lines.push(format!(
        "export const STRUCT_COUNT = {};",
        ctx.structs.len()
    ));

    write_lines(&out_dir.join("structs.ts"), &lines)
}

fn format_scalar_value(value: &crate::config::ScalarValue) -> String {
    match value {
        crate::config::ScalarValue::Int(i) => i.to_string(),
        crate::config::ScalarValue::Long(l) => l.to_string(),
        crate::config::ScalarValue::Str(s) => format!("'{}'", escape_ts_string(s)),
    }
}

fn export_param_types(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    let mut lines = vec![
        "// Auto-generated Param definitions".to_string(),
        "// Source: RS3 cache param config".to_string(),
        String::new(),
        "export interface ParamEntry {".to_string(),
        "    id: number;".to_string(),
        "    typeChar: string | null;".to_string(),
        "    typeId: number | null;".to_string(),
        "    defaultInt: number | null;".to_string(),
        "    defaultString: string | null;".to_string(),
        "    autoDisable: boolean;".to_string(),
        "}".to_string(),
        String::new(),
        "export type ParamValue = number | string;".to_string(),
        String::new(),
    ];

    lines.push("export const PARAMS: ReadonlyMap<number, ParamEntry> = new Map([".to_string());
    for entry in ctx.params.values() {
        let type_char = entry
            .type_char
            .map(|c| format!("'{}'", c as char))
            .unwrap_or_else(|| "null".to_string());
        let type_id = entry
            .type_id
            .map(|t| t.to_string())
            .unwrap_or_else(|| "null".to_string());
        let (default_int, default_string) = match &entry.default {
            Some(crate::config::ScalarValue::Int(i)) => (i.to_string(), "null".to_string()),
            Some(crate::config::ScalarValue::Long(l)) => (l.to_string(), "null".to_string()),
            Some(crate::config::ScalarValue::Str(s)) => {
                ("null".to_string(), format!("'{}'", escape_ts_string(s)))
            }
            None => ("null".to_string(), "null".to_string()),
        };
        lines.push(format!(
            "    [{}, {{ id: {}, typeChar: {}, typeId: {}, defaultInt: {}, defaultString: {}, autoDisable: {} }}],",
            entry.id, entry.id, type_char, type_id, default_int, default_string, entry.autodisable
        ));
    }
    lines.push("]);".to_string());
    lines.push(String::new());
    lines.push(format!("export const PARAM_COUNT = {};", ctx.params.len()));

    write_lines(&out_dir.join("params.ts"), &lines)
}

fn export_inv_types(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    let mut lines = vec![
        "// Auto-generated Inventory definitions".to_string(),
        "// Source: RS3 cache inv config".to_string(),
        String::new(),
    ];

    lines.push("export interface InvStockEntry {".to_string());
    lines.push("    objId: number;".to_string());
    lines.push("    count: number;".to_string());
    lines.push("}".to_string());
    lines.push(String::new());
    lines.push("export interface InvEntry {".to_string());
    lines.push("    id: number;".to_string());
    lines.push("    size: number | null;".to_string());
    lines.push("    stocks: InvStockEntry[];".to_string());
    lines.push("}".to_string());
    lines.push(String::new());

    if !ctx.invs.is_empty() {
        lines.push("export const INVS: ReadonlyMap<number, InvEntry> = new Map([".to_string());
        for entry in ctx.invs.values() {
            let size = entry
                .size
                .map(|s| s.to_string())
                .unwrap_or_else(|| "null".to_string());
            let stocks_json: String = entry
                .stocks
                .iter()
                .map(|s| format!("{{ objId: {}, count: {} }}", s.obj_id, s.count))
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(format!(
                "    [{id}, {{ id: {id}, size: {size}, stocks: [{stocks_json}] }}],",
                id = entry.id
            ));
        }
        lines.push("]);".to_string());
        lines.push(String::new());
    }
    lines.push(format!("export const INV_COUNT = {};", ctx.invs.len()));

    write_lines(&out_dir.join("invs.ts"), &lines)
}

fn export_obj_types(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    let mut writer = TextFileWriter::create(&out_dir.join("objs.ts"))?;
    writer.line("// Auto-generated Item (Obj) definitions")?;
    writer.line("// Source: RS3 cache obj config")?;
    writer.line("")?;
    writer.line("export interface ObjEntry {")?;
    writer.line("    id: number;")?;
    writer.line("    name: string | null;")?;
    writer.line("    ops: string[];")?;
    writer.line("}")?;
    writer.line("")?;

    if !ctx.objs.is_empty() {
        writer.line("export const OBJS: ReadonlyMap<number, ObjEntry> = new Map([")?;
        for entry in ctx.objs.values() {
            let name = extract_oplist_name(&entry.ops);
            let ops_json: String = entry
                .ops
                .iter()
                .map(|o| format!("'{}'", escape_ts_string(o)))
                .collect::<Vec<_>>()
                .join(", ");
            writer.line(format!(
                "    [{id}, {{ id: {id}, name: {name}, ops: [{ops_json}] }}],",
                id = entry.id
            ))?;
        }
        writer.line("]);")?;
        writer.line("")?;
    }
    writer.line(format!("export const OBJ_COUNT = {};", ctx.objs.len()))?;
    writer.finish()
}

fn export_npc_types(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    let mut writer = TextFileWriter::create(&out_dir.join("npcs.ts"))?;
    writer.line("// Auto-generated NPC definitions")?;
    writer.line("// Source: RS3 cache npc config")?;
    writer.line("")?;
    writer.line("export interface NpcEntry {")?;
    writer.line("    id: number;")?;
    writer.line("    name: string | null;")?;
    writer.line("    ops: string[];")?;
    writer.line("}")?;
    writer.line("")?;

    if !ctx.npcs.is_empty() {
        writer.line("export const NPCS: ReadonlyMap<number, NpcEntry> = new Map([")?;
        for entry in ctx.npcs.values() {
            let name = extract_oplist_name(&entry.ops);
            let ops_json: String = entry
                .ops
                .iter()
                .map(|o| format!("'{}'", escape_ts_string(o)))
                .collect::<Vec<_>>()
                .join(", ");
            writer.line(format!(
                "    [{id}, {{ id: {id}, name: {name}, ops: [{ops_json}] }}],",
                id = entry.id
            ))?;
        }
        writer.line("]);")?;
        writer.line("")?;
    }
    writer.line(format!("export const NPC_COUNT = {};", ctx.npcs.len()))?;
    writer.finish()
}

fn export_loc_types(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    let mut writer = TextFileWriter::create(&out_dir.join("locs.ts"))?;
    writer.line("// Auto-generated Loc (Object) definitions")?;
    writer.line("// Source: RS3 cache loc config")?;
    writer.line("")?;
    writer.line("export interface LocEntry {")?;
    writer.line("    id: number;")?;
    writer.line("    name: string | null;")?;
    writer.line("    ops: string[];")?;
    writer.line("}")?;
    writer.line("")?;

    if !ctx.locs.is_empty() {
        writer.line("export const LOCS: ReadonlyMap<number, LocEntry> = new Map([")?;
        for entry in ctx.locs.values() {
            let name = extract_oplist_name(&entry.ops);
            let ops_json: String = entry
                .ops
                .iter()
                .map(|o| format!("'{}'", escape_ts_string(o)))
                .collect::<Vec<_>>()
                .join(", ");
            writer.line(format!(
                "    [{id}, {{ id: {id}, name: {name}, ops: [{ops_json}] }}],",
                id = entry.id
            ))?;
        }
        writer.line("]);")?;
        writer.line("")?;
    }
    writer.line(format!("export const LOC_COUNT = {};", ctx.locs.len()))?;
    writer.finish()
}

fn export_seq_types(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    let mut writer = TextFileWriter::create(&out_dir.join("seqs.ts"))?;
    writer.line("// Auto-generated Sequence (Animation) definitions")?;
    writer.line("// Source: RS3 cache seq config")?;
    writer.line("")?;
    writer.line("export interface SeqFrame {")?;
    writer.line("    animId: number;")?;
    writer.line("    frameId: number;")?;
    writer.line("    delay: number;")?;
    writer.line("}")?;
    writer.line("")?;
    writer.line("export interface SeqEntry {")?;
    writer.line("    id: number;")?;
    writer.line("    frames: SeqFrame[];")?;
    writer.line("    stretches: boolean;")?;
    writer.line("    priority: number | null;")?;
    writer.line("    leftHand: number | null;")?;
    writer.line("    rightHand: number | null;")?;
    writer.line("    loopCount: number | null;")?;
    writer.line("    params: StructParamEntry[];")?;
    writer.line("}")?;
    writer.line("")?;

    if !ctx.seqs.is_empty() {
        writer.line("export const SEQS: ReadonlyMap<number, SeqEntry> = new Map([")?;
        for entry in ctx.seqs.values() {
            let frames_json: String = entry
                .frames
                .iter()
                .map(|f| {
                    format!(
                        "{{ animId: {}, frameId: {}, delay: {} }}",
                        f.anim_id, f.frame_id, f.delay
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            let params_json: String = entry
                .params
                .iter()
                .map(|p| {
                    format!(
                        "{{ id: {}, value: {} }}",
                        p.param_id,
                        format_scalar_value(&p.value)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            writer.line(format!(
                "    [{id}, {{ id: {id}, frames: [{frames_json}], stretches: {stretches}, priority: {priority}, leftHand: {lefthand}, rightHand: {righthand}, loopCount: {loopcount}, params: [{params_json}] }}],",
                id = entry.id,
                stretches = entry.stretches,
                priority = entry.priority.map(|p| p.to_string()).unwrap_or_else(|| "null".to_string()),
                lefthand = entry.lefthand_raw.map(|l| l.to_string()).unwrap_or_else(|| "null".to_string()),
                righthand = entry.righthand_raw.map(|r| r.to_string()).unwrap_or_else(|| "null".to_string()),
                loopcount = entry.loopcount.map(|l| l.to_string()).unwrap_or_else(|| "null".to_string()),
            ))?;
        }
        writer.line("]);")?;
        writer.line("")?;
    }
    writer.line(format!("export const SEQ_COUNT = {};", ctx.seqs.len()))?;
    writer.finish()
}

fn export_spot_types(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    let mut entries: Vec<_> = ctx.spots.values().collect();
    entries.sort_by_key(|e| e.id);

    let mut lines = vec![
        "// Auto-generated Spotanim (Graphic) definitions".to_string(),
        "// Source: RS3 cache spot config".to_string(),
        String::new(),
        "export interface SpotEntry {".to_string(),
        "    id: number;".to_string(),
        "    ops: string[];".to_string(),
        "}".to_string(),
        String::new(),
    ];

    if !entries.is_empty() {
        lines.push("export const SPOTS: ReadonlyMap<number, SpotEntry> = new Map([".to_string());
        for entry in &entries {
            let ops_json: String = entry
                .ops
                .iter()
                .map(|o| format!("'{}'", escape_ts_string(&format!("{o:?}"))))
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(format!(
                "    [{id}, {{ id: {id}, ops: [{ops_json}] }}],",
                id = entry.id
            ));
        }
        lines.push("]);".to_string());
        lines.push(String::new());
    }
    lines.push(format!("export const SPOT_COUNT = {};", entries.len()));

    write_lines(&out_dir.join("spots.ts"), &lines)
}

/// Extract a name from op list entries like "name=Attack" or "name=Man".
fn extract_oplist_name(ops: &[String]) -> String {
    for op in ops {
        if let Some(name) = op.strip_prefix("name=") {
            return format!("'{}'", escape_ts_string(name));
        }
    }
    "null".to_string()
}

fn extract_oplist_name_raw(ops: &[String]) -> Option<String> {
    for op in ops {
        if let Some(name) = op.strip_prefix("name=") {
            return Some(name.to_string());
        }
    }
    None
}

fn export_named_config_ids(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    export_named_oplist_ids("Obj", "objs", &ctx.objs, out_dir)?;
    export_named_oplist_ids("Npc", "npcs", &ctx.npcs, out_dir)?;
    export_named_oplist_ids("Loc", "locs", &ctx.locs, out_dir)?;
    Ok(())
}

fn export_named_oplist_ids(
    prefix: &str,
    source_file: &str,
    entries: &BTreeMap<u32, crate::config::OpListEntry>,
    out_dir: &Path,
) -> Result<()> {
    let const_name = format!("Named{prefix}Ids");
    let mut lines = vec![
        format!("// Auto-generated named {prefix} ID constants"),
        format!("// Source: RS3 cache {source_file} config (named entries only)"),
        String::new(),
    ];

    let mut named: Vec<(String, u32)> = Vec::new();
    for entry in entries.values() {
        if let Some(raw_name) = extract_oplist_name_raw(&entry.ops) {
            let prop = str_to_screaming_snake(&raw_name);
            if !prop.is_empty() {
                named.push((prop, entry.id));
            }
        }
    }
    named.sort_by(|a, b| a.0.cmp(&b.0));
    named.dedup_by(|a, b| a.0 == b.0);

    if named.is_empty() {
        lines.push(format!("export const {const_name} = {{}} as const;"));
    } else {
        lines.push(format!("export const {const_name} = {{"));
        for (prop, id) in &named {
            lines.push(format!("    {prop}: {id},"));
        }
        lines.push("} as const;".to_string());
    }
    lines.push(String::new());
    lines.push(format!(
        "export const NAMED_{}_COUNT = {};",
        prefix.to_uppercase(),
        named.len()
    ));

    write_lines(&out_dir.join(format!("named_{source_file}.ts")), &lines)
}

fn export_script_signatures(
    out_dir: &Path,
    script_catalog: &crate::transpile::ScriptCatalog,
) -> Result<()> {
    let mut lines = vec![
        "// Auto-generated CS2 script signatures".to_string(),
        "// Source: RS3 cache clientscript archive".to_string(),
        String::new(),
    ];

    let mut entries: Vec<(String, String)> = Vec::new();
    for script in script_catalog.iter() {
        let function_name = script.export_name.clone();
        let mut arg_types: Vec<&str> = Vec::new();
        arg_types.extend(std::iter::repeat_n(
            "number",
            script.signature.arg_count_int as usize,
        ));
        arg_types.extend(std::iter::repeat_n(
            "string",
            script.signature.arg_count_obj as usize,
        ));
        arg_types.extend(std::iter::repeat_n(
            "bigint",
            script.signature.arg_count_long as usize,
        ));
        let args = (0..arg_types.len())
            .map(|i| format!("arg{i}: {}", arg_types[i]))
            .collect::<Vec<_>>()
            .join(", ");

        entries.push((
            function_name.clone(),
            format!(
                "export function {function_name}({args}): {};",
                script.signature.return_type
            ),
        ));
    }

    entries.sort_by(|a, b| a.0.cmp(&b.0));
    lines.extend(entries.into_iter().map(|(_, line)| line));

    write_lines(&out_dir.join("scripts.d.ts"), &lines)
}

fn export_db_types(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    let mut writer = TextFileWriter::create(&out_dir.join("dbtables.ts"))?;
    writer.line("// Auto-generated Database definitions")?;
    writer.line("// Source: RS3 cache DB tables and rows (archive 2, groups 40/41)")?;
    writer.line("")?;
    writer.line("export interface DbTableColumn {")?;
    writer.line("    column: number;")?;
    writer.line("    tupleTypes: number[];")?;
    writer.line("    defaults: (number | string)[][];")?;
    writer.line("}")?;
    writer.line("")?;
    writer.line("export interface DbTableEntry {")?;
    writer.line("    id: number;")?;
    writer.line("    columns: DbTableColumn[];")?;
    writer.line("}")?;
    writer.line("")?;
    writer.line("export interface DbRowColumn {")?;
    writer.line("    column: number;")?;
    writer.line("    tupleTypes: number[];")?;
    writer.line("    rows: (number | string)[][];")?;
    writer.line("}")?;
    writer.line("")?;
    writer.line("export interface DbRowEntry {")?;
    writer.line("    id: number;")?;
    writer.line("    table: number | null;")?;
    writer.line("    columns: DbRowColumn[];")?;
    writer.line("}")?;
    writer.line("")?;

    if !ctx.dbtables.is_empty() {
        writer.line("export const DB_TABLES: ReadonlyMap<number, DbTableEntry> = new Map([")?;
        for entry in ctx.dbtables.values() {
            writer.line(format!("    [{id}, {{ id: {id}, columns: [", id = entry.id))?;
            for column in &entry.columns {
                let types = column
                    .tuple_types
                    .iter()
                    .map(u16::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                let defaults = column
                    .defaults
                    .iter()
                    .map(|row| {
                        let values = row
                            .iter()
                            .map(format_scalar_value)
                            .collect::<Vec<_>>()
                            .join(", ");
                        format!("[{values}]")
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                writer.line(format!(
                    "        {{ column: {}, tupleTypes: [{}], defaults: [{}] }},",
                    column.column, types, defaults
                ))?;
            }
            writer.line("    ] }],")?;
        }
        writer.line("]);")?;
        writer.line("")?;
    }
    writer.line(format!(
        "export const DB_TABLE_COUNT = {};",
        ctx.dbtables.len()
    ))?;
    writer.line("")?;
    writer.line("// Reverse-engineered table column meanings:")?;
    writer.line("// Table 163 (5,237 rows, 32 cols) — Items")?;
    writer.line("//   col  0: itemId (int)")?;
    writer.line("//   col  1: parentId (int) — parent item or category")?;
    writer.line("//   col  2: name (string)")?;
    writer.line("//   col  3: description (string)")?;
    writer.line("//   col  4: paramId (int) — linked param config entry")?;
    writer.line("//   col  5: typeId (int) — item type/category")?;
    writer.line("//   col  6: value (int, default 99) — shop price")?;
    writer.line("//   col  7: flags (int, default 268435454)")?;
    writer.line("//   col  8: stackable (int, default 1)")?;
    writer.line("//   col 11: membersOnly (boolean, default false)")?;
    writer.line("//   col 13: categoryId (int)")?;
    writer.line("//   col 23: modelId (int)")?;
    writer.line("//   col 24: modelId2 (int)")?;
    writer.line("//   col 26: color (int) — RGBA tint")?;
    writer.line("//   col 30: equipmentOverrides (int[6]) — only for special items")?;
    writer.line("//         index 0-5: stab/slash/crush/magic/range/strength bonus")?;
    writer.line("//   col 31: soundId (int)")?;
    writer.line("//")?;
    writer.line("// Table 29 (105 rows, 46 cols) — NPC stats")?;
    writer.line("//   cols 1-3: model IDs")?;
    writer.line("//   col  5: name (string)")?;
    writer.line("//   col  7: size (int)")?;
    writer.line("//   col  9: combatLevel (int)")?;
    writer.line("//   col 10: hitpoints (int)")?;
    writer.line("//   col 14: attack (int)")?;
    writer.line("//   col 17: defence (int)")?;
    writer.line("//   col 18: accuracy (int)")?;
    writer.line("//")?;
    writer.line("// Note: Most equipment/weapon stats are computed client-side")?;
    writer.line("// from item tier + category, not stored per-item in this table.")?;
    writer.line("// Only override stats (halos, special items) use col 30.")?;
    writer.line("")?;

    if !ctx.dbrows.is_empty() {
        let mut by_table: BTreeMap<u32, Vec<&crate::config::DbRowEntry>> = BTreeMap::new();
        for row in ctx.dbrows.values() {
            if let Some(table) = row.table {
                by_table.entry(table).or_default().push(row);
            }
        }
        writer.line("// DB rows grouped by table ID. Key = tableId, value = rows.")?;
        writer.line("export const DB_ROWS: ReadonlyMap<number, DbRowEntry[]> = new Map([")?;
        for (table_id, rows) in &by_table {
            writer.line(format!("    [{table_id}, ["))?;
            for row in rows {
                writer.line(format!(
                    "        {{ id: {}, table: {}, columns: [",
                    row.id, table_id
                ))?;
                for column in &row.columns {
                    let types = column
                        .tuple_types
                        .iter()
                        .map(u16::to_string)
                        .collect::<Vec<_>>()
                        .join(", ");
                    let row_data = column
                        .rows
                        .iter()
                        .map(|tuple| {
                            let values = tuple
                                .iter()
                                .map(format_scalar_value)
                                .collect::<Vec<_>>()
                                .join(", ");
                            format!("[{values}]")
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    writer.line(format!(
                        "            {{ column: {}, tupleTypes: [{}], rows: [{}] }},",
                        column.column, types, row_data
                    ))?;
                }
                writer.line("        ] },")?;
            }
            writer.line("    ]],")?;
        }
        writer.line("]);")?;
        writer.line("")?;
    }
    writer.line(format!("export const DB_ROW_COUNT = {};", ctx.dbrows.len()))?;
    writer.line("")?;
    writer.line("export interface ItemEntry {")?;
    writer.line("    id: number;")?;
    writer.line("    name: string | null;")?;
    writer.line("    description: string | null;")?;
    writer.line("    /** Shop / GE price. */")?;
    writer.line("    value: number;")?;
    writer.line("    /** Non-zero means stackable. */")?;
    writer.line("    stackable: boolean;")?;
    writer.line("    membersOnly: boolean;")?;
    writer.line("    categoryId: number | null;")?;
    writer.line("    parentId: number | null;")?;
    writer.line("    modelId: number | null;")?;
    writer.line("    /** RGBA tint (e.g. 16832257). */")?;
    writer.line("    color: number | null;")?;
    writer.line("    paramId: number | null;")?;
    writer.line("    soundId: number | null;")?;
    writer.line("    /** Key→value pairs for linked param configs. */")?;
    writer.line("    params: Array<{ key: number; value: number | string }>;")?;
    writer.line("    /** Equipment stat overrides (only 2 items). */")?;
    writer.line("    equipmentOverrides: number[] | null;")?;
    writer.line("}")?;
    writer.line("")?;

    let items: Vec<_> = ctx
        .dbrows
        .values()
        .filter(|row| row.table == Some(163))
        .collect();
    if !items.is_empty() {
        writer.line("export const ITEMS: ReadonlyMap<number, ItemEntry> = new Map([")?;
        for row in &items {
            let id = row_column_int(row, 0).unwrap_or(0);
            let name = row_column_str(row, 2);
            let desc = row_column_str(row, 3);
            let value = row_column_int(row, 6).unwrap_or(99);
            let stackable = row_column_int(row, 8).unwrap_or(1) != 0;
            let members = row_column_bool(row, 11);
            let category = row_column_int_or_null(row, 13);
            let parent = row_column_int_or_null(row, 1);
            let model = row_column_int_or_null(row, 23);
            let color = row_column_int_or_null(row, 26);
            let param = row_column_int_or_null(row, 4);
            let sound = row_column_int_or_null(row, 31);
            let eq_overrides = row_column_int_array(row, 30);
            let name_str = name
                .map(|value| format!("'{value}'"))
                .unwrap_or_else(|| "null".to_string());
            let desc_str = desc
                .map(|value| format!("'{value}'"))
                .unwrap_or_else(|| "null".to_string());
            let eq_str = eq_overrides
                .map(|values| {
                    format!(
                        "[{}]",
                        values
                            .iter()
                            .map(i32::to_string)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })
                .unwrap_or_else(|| "null".to_string());
            writer.line(format!(
                "    [{id}, {{ id: {id}, name: {name_str}, description: {desc_str}, value: {value}, stackable: {stackable}, membersOnly: {members}, categoryId: {category}, parentId: {parent}, modelId: {model}, color: {color}, paramId: {param}, soundId: {sound}, params: [], equipmentOverrides: {eq_str} }}],"
            ))?;
        }
        writer.line("]);")?;
        writer.line("")?;
    }
    writer.line(format!("export const ITEM_COUNT = {};", items.len()))?;
    writer.line("")?;
    writer.line("export interface NpcStatEntry {")?;
    writer.line("    id: number;")?;
    writer.line("    name: string | null;")?;
    writer.line("    combatLevel: number;")?;
    writer.line("    hitpoints: number;")?;
    writer.line("    attack: number;")?;
    writer.line("    defence: number;")?;
    writer.line("    accuracy: number;")?;
    writer.line("    size: number;")?;
    writer.line("    respawnMs: number | null;")?;
    writer.line("    modelIds: number[];")?;
    writer.line("}")?;
    writer.line("")?;

    let npc_stats: Vec<_> = ctx
        .dbrows
        .values()
        .filter(|row| row.table == Some(29))
        .collect();
    if !npc_stats.is_empty() {
        writer.line("export const NPC_STATS: ReadonlyMap<number, NpcStatEntry> = new Map([")?;
        for row in &npc_stats {
            let id = row_column_int(row, 0).unwrap_or(0);
            let name = row_column_str(row, 5);
            let combat = row_column_int(row, 9).unwrap_or(0);
            let hp = row_column_int(row, 10).unwrap_or(0);
            let atk = row_column_int(row, 14).unwrap_or(0);
            let def = row_column_int(row, 17).unwrap_or(0);
            let acc = row_column_int(row, 18).unwrap_or(0);
            let size = row_column_int(row, 7).unwrap_or(1);
            let respawn = row_column_int_or_null(row, 13);
            let models = [1, 2, 3]
                .iter()
                .filter_map(|&column| row_column_int(row, column))
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            let name_str = name
                .map(|value| format!("'{value}'"))
                .unwrap_or_else(|| "null".to_string());
            writer.line(format!(
                "    [{id}, {{ id: {id}, name: {name_str}, combatLevel: {combat}, hitpoints: {hp}, attack: {atk}, defence: {def}, accuracy: {acc}, size: {size}, respawnMs: {respawn}, modelIds: [{models}] }}],"
            ))?;
        }
        writer.line("]);")?;
        writer.line("")?;
    }
    writer.line(format!(
        "export const NPC_STAT_COUNT = {};",
        npc_stats.len()
    ))?;
    writer.line("")?;
    writer.line("export interface ClueLocationEntry {")?;
    writer.line("    id: number;")?;
    writer.line("    /** Difficulty tier (1-5). */")?;
    writer.line("    tier: number;")?;
    writer.line("    description: string | null;")?;
    writer.line("}")?;
    writer.line("")?;

    let clue_rows: Vec<_> = ctx
        .dbrows
        .values()
        .filter(|row| row.table == Some(7))
        .collect();
    if !clue_rows.is_empty() {
        writer.line(
            "export const CLUE_LOCATIONS: ReadonlyMap<number, ClueLocationEntry> = new Map([",
        )?;
        for row in &clue_rows {
            let id = row_column_int(row, 0).unwrap_or(0);
            let tier = row_column_int(row, 1).unwrap_or(1);
            let desc = row_column_str(row, 2);
            let desc_str = desc
                .map(|value| format!("'{value}'"))
                .unwrap_or_else(|| "null".to_string());
            writer.line(format!(
                "    [{id}, {{ id: {id}, tier: {tier}, description: {desc_str} }}],"
            ))?;
        }
        writer.line("]);")?;
        writer.line("")?;
    }
    writer.line(format!(
        "export const CLUE_LOCATION_COUNT = {};",
        clue_rows.len()
    ))?;
    writer.line("")?;
    writer.line("export interface ItemCategoryEntry {")?;
    writer.line("    id: number;")?;
    writer.line("    name: string | null;")?;
    writer.line("    modelId: number | null;")?;
    writer.line("    iconId: number | null;")?;
    writer.line("}")?;
    writer.line("")?;

    let categories: Vec<_> = ctx
        .dbrows
        .values()
        .filter(|row| row.table == Some(4))
        .collect();
    if !categories.is_empty() {
        writer.line(
            "export const ITEM_CATEGORIES: ReadonlyMap<number, ItemCategoryEntry> = new Map([",
        )?;
        for row in &categories {
            let id = row_column_int(row, 0).unwrap_or(0);
            let name = row_column_str(row, 1);
            let model = row_column_int_or_null(row, 4);
            let icon = row_column_int_or_null(row, 5);
            let name_str = name
                .map(|value| format!("'{value}'"))
                .unwrap_or_else(|| "null".to_string());
            writer.line(format!(
                "    [{id}, {{ id: {id}, name: {name_str}, modelId: {model}, iconId: {icon} }}],"
            ))?;
        }
        writer.line("]);")?;
        writer.line("")?;
    }
    writer.line(format!(
        "export const ITEM_CATEGORY_COUNT = {};",
        categories.len()
    ))?;
    writer.line("")?;
    writer.line("export interface ItemSetEntry {")?;
    writer.line("    id: number;")?;
    writer.line("    name: string | null;")?;
    writer.line("    description: string | null;")?;
    writer.line("    representativeItemId: number | null;")?;
    writer.line("}")?;
    writer.line("")?;

    let sets: Vec<_> = ctx
        .dbrows
        .values()
        .filter(|row| row.table == Some(5))
        .collect();
    if !sets.is_empty() {
        writer.line("export const ITEM_SETS: ReadonlyMap<number, ItemSetEntry> = new Map([")?;
        for row in &sets {
            let id = row_column_int(row, 0).unwrap_or(0);
            let name = row_column_str(row, 1);
            let desc = row_column_str(row, 2);
            let rep_item = row_column_int_or_null(row, 5);
            let name_str = name
                .map(|value| format!("'{value}'"))
                .unwrap_or_else(|| "null".to_string());
            let desc_str = desc
                .map(|value| format!("'{value}'"))
                .unwrap_or_else(|| "null".to_string());
            writer.line(format!(
                "    [{id}, {{ id: {id}, name: {name_str}, description: {desc_str}, representativeItemId: {rep_item} }}],"
            ))?;
        }
        writer.line("]);")?;
        writer.line("")?;
    }
    writer.line(format!("export const ITEM_SET_COUNT = {};", sets.len()))?;
    writer.line("")?;
    writer.line("// Named column indices for table 163 (items).")?;
    writer.line("// Example: row.columns[ItemColumn.NAME]")?;
    writer.line("export const ItemColumn = {")?;
    writer.line("    ID: 0,")?;
    writer.line("    PARENT_ID: 1,")?;
    writer.line("    NAME: 2,")?;
    writer.line("    DESCRIPTION: 3,")?;
    writer.line("    PARAM_ID: 4,")?;
    writer.line("    TYPE_ID: 5,")?;
    writer.line("    VALUE: 6,")?;
    writer.line("    FLAGS: 7,")?;
    writer.line("    STACKABLE: 8,")?;
    writer.line("    MEMBERS_ONLY: 11,")?;
    writer.line("    CATEGORY_ID: 13,")?;
    writer.line("    MODEL_ID: 23,")?;
    writer.line("    MODEL_ID2: 24,")?;
    writer.line("    COLOR: 26,")?;
    writer.line("    EQUIPMENT_OVERRIDES: 30,")?;
    writer.line("    SOUND_ID: 31,")?;
    writer.line("} as const;")?;
    writer.line("export type ItemColumn = (typeof ItemColumn)[keyof typeof ItemColumn];")?;
    writer.line("")?;
    writer.line("// Named column indices for table 29 (NPC stats).")?;
    writer.line("export const NpcColumn = {")?;
    writer.line("    ID: 0,")?;
    writer.line("    MODEL_ID1: 1,")?;
    writer.line("    MODEL_ID2: 2,")?;
    writer.line("    MODEL_ID3: 3,")?;
    writer.line("    NAME: 5,")?;
    writer.line("    SIZE: 7,")?;
    writer.line("    COMBAT_LEVEL: 9,")?;
    writer.line("    HITPOINTS: 10,")?;
    writer.line("    RESPAWN_MS: 13,")?;
    writer.line("    ATTACK: 14,")?;
    writer.line("    DEFENCE: 17,")?;
    writer.line("    ACCURACY: 18,")?;
    writer.line("} as const;")?;
    writer.line("export type NpcColumn = (typeof NpcColumn)[keyof typeof NpcColumn];")?;
    writer.finish()
}

/// Extract the first int value from a specific column in a DB row.
fn row_column_int(row: &crate::config::DbRowEntry, col: u8) -> Option<i32> {
    row.columns
        .iter()
        .find(|c| c.column == col)
        .and_then(|c| c.rows.first())
        .and_then(|r| r.first())
        .and_then(|v| match v {
            crate::config::ScalarValue::Int(i) => Some(*i),
            _ => None,
        })
}

/// Extract the first string value from a specific column in a DB row.
fn row_column_str(row: &crate::config::DbRowEntry, col: u8) -> Option<String> {
    row.columns
        .iter()
        .find(|c| c.column == col)
        .and_then(|c| c.rows.first())
        .and_then(|r| r.first())
        .and_then(|v| match v {
            crate::config::ScalarValue::Str(s) => Some(escape_ts_string(s)),
            _ => None,
        })
}

/// Extract a boolean from a specific column (0=false, non-zero=true).
fn row_column_bool(row: &crate::config::DbRowEntry, col: u8) -> bool {
    row_column_int(row, col).unwrap_or(0) != 0
}

/// Extract an int as a TS null-or-number string.
fn row_column_int_or_null(row: &crate::config::DbRowEntry, col: u8) -> String {
    row_column_int(row, col)
        .map(|v| v.to_string())
        .unwrap_or_else(|| "null".to_string())
}

/// Extract all ints from a tuple column as a Vec (equipment overrides etc.).
fn row_column_int_array(row: &crate::config::DbRowEntry, col: u8) -> Option<Vec<i32>> {
    row.columns
        .iter()
        .find(|c| c.column == col)
        .and_then(|c| c.rows.first())
        .map(|r| {
            r.iter()
                .filter_map(|v| match v {
                    crate::config::ScalarValue::Int(i) => Some(*i),
                    _ => None,
                })
                .collect()
        })
}

fn export_interface_ids(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    struct ComponentExportEntry<'a> {
        uid: u32,
        interface_id: u32,
        component_id: u32,
        deps: &'a crate::interface::ComponentDeps,
    }

    let mut all_components: Vec<ComponentExportEntry<'_>> = Vec::new();
    for (&interface_id, comps) in &ctx.parsed_components {
        for (&component_id, deps) in comps {
            all_components.push(ComponentExportEntry {
                uid: crate::interface::component_uid(interface_id, component_id),
                interface_id,
                component_id,
                deps,
            });
        }
    }
    all_components.sort_by_key(|entry| entry.uid);

    let mut writer = TextFileWriter::create(&out_dir.join("interfaces.ts"))?;
    writer.line("// Auto-generated Interface Component definitions")?;
    writer.line("// Source: RS3 cache interfaces archive (parsed component deps)")?;
    writer.line("")?;

    // ── Interface ID constants ──
    if !ctx.parsed_components.is_empty() {
        writer.line("// Root interface group IDs.")?;
        writer.line("export const InterfaceId = {")?;
        for &interface_id in ctx.parsed_components.keys() {
            writer.line(format!("    interface_{interface_id}: {interface_id},"))?;
        }
        writer.line("} as const;")?;
        writer.line("")?;
        writer.line("export type InterfaceId = (typeof InterfaceId)[keyof typeof InterfaceId];")?;
        writer.line("")?;
    }

    // ── Named component ID constants (full UID keys) ──
    let mut named_entries: Vec<(String, u32, u32, u32, &str)> = Vec::new();
    for entry in &all_components {
        let label = entry
            .deps
            .name
            .as_deref()
            .map(sanitize_ts_prop)
            .filter(|prop| !prop.is_empty() && prop != "unnamed")
            .unwrap_or_else(|| {
                sanitize_ts_prop(&crate::interface::component_fallback_name(
                    entry.interface_id,
                    entry.component_id,
                ))
            });
        named_entries.push((
            label,
            entry.uid,
            entry.interface_id,
            entry.component_id,
            &entry.deps.component_type,
        ));
    }
    named_entries.sort_by(|a, b| a.0.cmp(&b.0));
    named_entries.dedup_by(|a, b| a.0 == b.0);
    named_entries.sort_by_key(|e| e.1);

    if !named_entries.is_empty() {
        writer.line("// Named component UIDs used by CS2 cc_* / if_* opcodes.")?;
        writer.line("export const ComponentId = {")?;
        for (prop, uid, interface_id, component_id, comp_type) in &named_entries {
            writer.line(format!(
                "    /** {comp_type} interface={interface_id} com={component_id} uid={uid} */"
            ))?;
            writer.line(format!("    {prop}: {uid},"))?;
        }
        writer.line("} as const;")?;
        writer.line("")?;
        writer.line("export type ComponentId = (typeof ComponentId)[keyof typeof ComponentId];")?;
        writer.line("")?;
    }

    // ── ComponentInfo interface and data ──
    writer.line("export interface ComponentInfo {")?;
    writer.line("    id: number;")?;
    writer.line("    interfaceId: number;")?;
    writer.line("    componentId: number;")?;
    writer.line("    type: string;")?;
    writer.line("    name: string | null;")?;
    writer.line("    children: number[];")?;
    writer.line("    scripts: number[];")?;
    writer.line("    varps: Array<{domain: string; id: number}>;")?;
    writer.line("    varbits: number[];")?;
    writer.line("    enums: number[];")?;
    writer.line("    params: number[];")?;
    writer.line("    invs: number[];")?;
    writer.line("    models: number[];")?;
    writer.line("    seqs: number[];")?;
    writer.line("}")?;
    writer.line("")?;

    writer.line("export const ALL_COMPONENTS: ReadonlyMap<number, ComponentInfo> = new Map([")?;
    for entry in &all_components {
        let deps = entry.deps;
        let varp_items: Vec<String> = deps
            .varps
            .iter()
            .map(|v| {
                let (domain, id) = match v {
                    crate::interface::VarTransmitRef::Player(id) => ("player", *id),
                    crate::interface::VarTransmitRef::Npc(id) => ("npc", *id),
                    crate::interface::VarTransmitRef::Client(id) => ("client", *id),
                    crate::interface::VarTransmitRef::World(id) => ("world", *id),
                    crate::interface::VarTransmitRef::Region(id) => ("region", *id),
                    crate::interface::VarTransmitRef::Object(id) => ("object", *id),
                    crate::interface::VarTransmitRef::Clan(id) => ("clan", *id),
                    crate::interface::VarTransmitRef::ClanSetting(id) => ("clan_setting", *id),
                    crate::interface::VarTransmitRef::Controller(id) => ("controller", *id),
                    crate::interface::VarTransmitRef::Global(id) => ("global", *id),
                    crate::interface::VarTransmitRef::PlayerGroup(id) => ("player_group", *id),
                    crate::interface::VarTransmitRef::VarClientString(id) => ("client", *id),
                };
                format!("{{domain:'{domain}',id:{id}}}")
            })
            .collect();
        let scripts_json = set_to_json(&deps.scripts);
        let varbits_json = set_to_json(&deps.varbits);
        let enums_json = set_to_json(&deps.enums);
        let params_json = set_to_json(&deps.params);
        let invs_json = set_to_json(&deps.invs);
        let models_json = set_to_json(&deps.models);
        let seqs_json = set_to_json(&deps.seqs);
        let children_json: String = deps
            .children
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let name_str = match &deps.name {
            Some(n) => format!("'{}'", escape_ts_string(n)),
            None => "null".to_string(),
        };
        writer.line(format!(
            "    [{uid}, {{ id:{uid}, interfaceId:{interface_id}, componentId:{component_id}, type:'{type}', name:{name}, children:[{children}], scripts:[{scripts}], varps:[{varps}], varbits:[{varbits}], enums:[{enums}], params:[{params}], invs:[{invs}], models:[{models}], seqs:[{seqs}] }}],",
            uid = entry.uid,
            interface_id = entry.interface_id,
            component_id = entry.component_id,
            type = deps.component_type,
            name = name_str,
            children = children_json,
            scripts = scripts_json,
            varps = varp_items.join(", "),
            varbits = varbits_json,
            enums = enums_json,
            params = params_json,
            invs = invs_json,
            models = models_json,
            seqs = seqs_json,
        ))?;
    }
    writer.line("]);")?;
    writer.line("")?;
    writer.line(format!(
        "export const COMPONENT_COUNT = {};",
        all_components.len()
    ))?;
    writer.finish()
}

fn set_to_json(set: &std::collections::HashSet<u32>) -> String {
    let mut items: Vec<u32> = set.iter().copied().collect();
    items.sort_unstable();
    items
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

fn export_index(out_dir: &Path) -> Result<()> {
    write_text(&out_dir.join("index.ts"), &index_exports_source())
}

fn index_exports_source() -> String {
    let lines = vec![
        "// Auto-generated index file".to_string(),
        "// Source: RS3 cache ts-export".to_string(),
        String::new(),
        "export {".to_string(),
        "    VARS,".to_string(),
        "    VAR_COUNT,".to_string(),
        "    type VarEntry,".to_string(),
        "    type VarDomain,".to_string(),
        "    type VarType,".to_string(),
        "    type VarLifetime,".to_string(),
        "    type VarTransmitLevel,".to_string(),
        "} from './vars';".to_string(),
        String::new(),
        "export {".to_string(),
        "    VARBITS,".to_string(),
        "    VARBIT_COUNT,".to_string(),
        "    type VarBitEntry,".to_string(),
        "} from './varbits';".to_string(),
        String::new(),
        "export {".to_string(),
        "    ENUMS,".to_string(),
        "    ENUM_COUNT,".to_string(),
        "    ENUM_VALUE_TO_NAME,".to_string(),
        "    type EnumEntry,".to_string(),
        "    type EnumPair,".to_string(),
        "} from './enums';".to_string(),
        String::new(),
        "export {".to_string(),
        "    STRUCTS,".to_string(),
        "    STRUCT_COUNT,".to_string(),
        "    type StructEntry,".to_string(),
        "    type StructParamEntry,".to_string(),
        "} from './structs';".to_string(),
        String::new(),
        "export {".to_string(),
        "    PARAMS,".to_string(),
        "    PARAM_COUNT,".to_string(),
        "    type ParamEntry,".to_string(),
        "    type ParamValue,".to_string(),
        "} from './params';".to_string(),
        String::new(),
        "export {".to_string(),
        "    InterfaceId,".to_string(),
        "    ComponentId,".to_string(),
        "    ALL_COMPONENTS,".to_string(),
        "    COMPONENT_COUNT,".to_string(),
        "    type ComponentInfo,".to_string(),
        "    type InterfaceId as InterfaceIdType,".to_string(),
        "} from './interfaces';".to_string(),
        "export { type InvEntry, INVS, INV_COUNT } from './invs';".to_string(),
        "export { type ObjEntry, OBJS, OBJ_COUNT } from './objs';".to_string(),
        "export { type NpcEntry, NPCS, NPC_COUNT } from './npcs';".to_string(),
        "export { type LocEntry, LOCS, LOC_COUNT } from './locs';".to_string(),
        "export { type SeqEntry, SEQS, SEQ_COUNT } from './seqs';".to_string(),
        "export { type SpotEntry, SPOTS, SPOT_COUNT } from './spots';".to_string(),
        "export { NamedObjIds, NAMED_OBJ_COUNT } from './named_objs';".to_string(),
        "export { NamedNpcIds, NAMED_NPC_COUNT } from './named_npcs';".to_string(),
        "export { NamedLocIds, NAMED_LOC_COUNT } from './named_locs';".to_string(),
        "export { type ItemEntry, ITEMS, ITEM_COUNT,".to_string(),
        "    type ItemCategoryEntry, ITEM_CATEGORIES, ITEM_CATEGORY_COUNT,".to_string(),
        "    type ItemSetEntry, ITEM_SETS, ITEM_SET_COUNT,".to_string(),
        "    type NpcStatEntry, NPC_STATS, NPC_STAT_COUNT,".to_string(),
        "    type ClueLocationEntry, CLUE_LOCATIONS, CLUE_LOCATION_COUNT,".to_string(),
        "    ItemColumn, type ItemColumn, NpcColumn, type NpcColumn,".to_string(),
        "} from './dbtables';".to_string(),
        "export { DB_TABLES, DB_TABLE_COUNT, DB_ROWS, DB_ROW_COUNT,".to_string(),
        "    type DbTableEntry, type DbRowEntry, type DbTableColumn, type DbRowColumn } from './dbtables';".to_string(),
    ];

    lines.join("\n")
}

fn escape_ts_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Convert a RS3 interface component name (`snake_case` or kebab-case)
/// to a valid TypeScript object property name (also `snake_case`, but
/// with hyphens and spaces replaced by underscores).
fn sanitize_ts_prop(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else if c == '-' || c == ' ' || c == '/' {
            out.push('_');
        }
        // drop other chars
    }
    // Property can't start with a digit
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    // Can't be empty
    if out.is_empty() {
        out.push_str("unnamed");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_manifest_entry_serializes() {
        let entry = AudioManifestEntry {
            archive: 14,
            group: 1,
            file: 0,
            size: 123,
            kind: "jaga".to_string(),
            raw_extension: "jaga".to_string(),
            embedded_ogg_offset: Some(32),
            extracted_ogg: true,
        };
        let json = serde_json::to_string(&entry).expect("serialize manifest entry");
        assert!(json.contains("\"archive\":14"));
        assert!(json.contains("\"kind\":\"jaga\""));
    }

    #[test]
    fn sanitize_file_component_rewrites_unsupported_chars() {
        assert_eq!("hello_world", sanitize_file_component("hello/world"));
        assert_eq!("script", sanitize_file_component(""));
    }

    #[test]
    fn extract_name_suffix_parses_tag_syntax() {
        assert_eq!(
            "100guide_flour_drawitems",
            extract_name_suffix("[clientscript,100guide_flour_drawitems]")
        );
        assert_eq!("plain_name", extract_name_suffix("plain_name"));
    }

    #[test]
    fn java_string_hash_matches_known_value() {
        assert_eq!(2_111_159_123, java_string_hash("[clientscript,script0]"));
    }

    #[test]
    fn worldmap_format_helpers_match_expected_shape() {
        assert_eq!("null", format_coordgrid(-1));
        assert_eq!("0_0_0_0_0", format_coordgrid(0));
        assert_eq!("0_50_248_42_54", format_coordgrid(53_132_854));
        assert_eq!("0x00ab12", format_colour(43_794));
        assert_eq!("0xff00ab12", format_colour(-16_733_422));
        assert_eq!("mapelement_42", format_map_element(42));
    }

    #[test]
    fn index_exports_source_has_balanced_unescaped_braces() {
        let source = index_exports_source();

        assert!(!source.contains("{{"));
        assert!(!source.contains("}}"));
        assert_export_braces_are_balanced(&source);
    }

    #[test]
    fn high_ts_style_classifies_visible_low_level_markers() {
        let source = r#"
export function script0(): void {
    label(10);
    stackpush_then(1, goto(20));
    UI.setTextWithMode("x", 1);
    unknowncommand58(0);
}
"#;

        let style = HighTsScriptStyle::from_source(source);

        assert!(style.control_flow_markers);
        assert_eq!(1, style.goto_calls);
        assert_eq!(1, style.label_calls);
        assert_eq!(2, style.control_flow_marker_occurrences);
        assert!(style.stack_pseudos);
        assert_eq!(1, style.stack_pseudo_occurrences);
        assert!(style.with_mode);
        assert_eq!(1, style.with_mode_occurrences);
        assert!(style.unknown_command);
        assert_eq!(1, style.unknown_command_occurrences);
        assert!(!style.no_visible_low_level_markers);
    }

    #[test]
    fn high_ts_coverage_counts_markers_and_fallback_reasons() {
        let scripts = vec![
            ScriptDiagnosticsEntry {
                packed_id: 1,
                group_id: 1,
                export_name: "script1".to_string(),
                module_name: "script1".to_string(),
                editable_structured: true,
                blocking_diagnostics: Vec::new(),
                high_ts_style: HighTsScriptStyle::from_source("goto(1);"),
                high_ts_fallback_reason: Some("gate_mismatch".to_string()),
                high_ts_gate_diagnostics: vec![
                    "recompile_mismatch".to_string(),
                    "recompile_mismatch_cause:branch:operand".to_string(),
                ],
                high_ts_gate_messages: vec![
                    "recompile diverges at [0]: original `branch Branch(1)` vs structured `branch Branch(2)`"
                        .to_string(),
                ],
                diagnostics: Vec::new(),
            },
            ScriptDiagnosticsEntry {
                packed_id: 2,
                group_id: 2,
                export_name: "script2".to_string(),
                module_name: "script2".to_string(),
                editable_structured: true,
                blocking_diagnostics: Vec::new(),
                high_ts_style: HighTsScriptStyle::from_source("return;"),
                high_ts_fallback_reason: None,
                high_ts_gate_diagnostics: Vec::new(),
                high_ts_gate_messages: Vec::new(),
                diagnostics: Vec::new(),
            },
        ];

        let coverage = HighTsCoverageSummary::from_scripts(&scripts);

        assert_eq!(2, coverage.total);
        assert_eq!(1, coverage.control_flow_markers);
        assert_eq!(1, coverage.control_flow_marker_occurrences);
        assert_eq!(1, coverage.goto_calls);
        assert_eq!(0, coverage.label_calls);
        assert_eq!(1, coverage.no_visible_low_level_markers);
        assert_eq!(Some(&1), coverage.fallback_reasons.get("gate_mismatch"));
        assert_eq!(
            Some(&1),
            coverage.fallback_gate_blockers.get("recompile_mismatch")
        );
    }

    #[test]
    fn source_control_flow_fallback_reason_classifies_stack_and_residual_gotos() {
        assert_eq!(
            Some("stack_goto".to_string()),
            source_control_flow_fallback_reason("stackpush_then(1, goto(2));")
        );
        assert_eq!(
            Some("residual_goto".to_string()),
            source_control_flow_fallback_reason("stackpush_then(1, push(x));\ngoto(2);")
        );
        assert_eq!(None, source_control_flow_fallback_reason("return;"));
    }

    fn assert_export_braces_are_balanced(source: &str) {
        let mut depth = 0_i32;
        for line in source.lines() {
            for c in line.chars() {
                match c {
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    _ => {}
                }
                assert!(depth >= 0, "extra closing brace in {line}");
            }
        }
        assert_eq!(0, depth, "unclosed export brace");
    }

    #[test]
    fn format_script_source_renders_headers_and_code() {
        let script = CompiledScript {
            name: Some("my/script".to_string()),
            local_count_int: 1,
            local_count_object: 2,
            local_count_long: 3,
            argument_count_int: 4,
            argument_count_object: 5,
            argument_count_long: 6,
            code: vec![Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(42),
            }],
        };

        let source = format_script_source(10, 0, &script);
        assert!(source.contains("// group=10 file=0"));
        assert!(source.contains("// name=my/script"));
        assert!(source.contains("00000: push_constant_int 42"));
    }
}
