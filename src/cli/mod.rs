use crate::constants::{
    BUILD, CONFIG_GROUP_VAR_CLAN, CONFIG_GROUP_VAR_CLAN_SETTING, CONFIG_GROUP_VAR_CLIENT,
    CONFIG_GROUP_VAR_CONTROLLER, CONFIG_GROUP_VAR_GLOBAL, CONFIG_GROUP_VAR_NPC,
    CONFIG_GROUP_VAR_OBJECT, CONFIG_GROUP_VAR_PLAYER, CONFIG_GROUP_VAR_PLAYER_GROUP,
    CONFIG_GROUP_VAR_REGION, CONFIG_GROUP_VAR_WORLD, SUBBUILD,
};
use crate::dep_tree::EntityType;
use crate::fixture::{default_tar_path, open_cache};
use crate::vars::VarDomain;
use anyhow::{Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use std::path::{Path, PathBuf};

pub mod context;
pub(crate) mod shared;

use context::{CommandContext, RuntimeVersion};

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
    /// Survey the protocol payloads: classify every packet + emit the schema tranche.
    ///
    /// Parses every hand-written `ServerProt.<NAME>.encode` body in the server's
    /// `ServerProt.ts` and the matching `Client.java` decode branch, classifies
    /// each end simple/complex per the DSL v1/v2 rules, and writes
    /// `payload-classification.json` (all packets) + `payloads.json` (the
    /// simple+v2 tranche). Read-only over both source trees; errors (non-zero
    /// exit) before writing if a required tranche member is missing.
    ///
    /// Example:
    /// ```bash
    /// cd tools/rs3-cache-rs
    /// cargo run --release -- --data-dir data survey-payloads
    /// ```
    #[command(name = "survey-payloads")]
    SurveyPayloads {
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
    /// Extract the CS2 opcode stack-effect table from the client's clientscript
    /// handler sources.
    ///
    /// Globs every `*.java` in the clientscript package (`ScriptRunner.java` plus
    /// the `*Ops.java` handler classes), parses each `NAME(ClientScriptState
    /// arg0)` handler's net `isp`/`osp`/`lsp` pops/pushes, keeps the ones whose
    /// name is a known opcode with a non-zero effect, and writes the sorted table
    /// to `data/stack-effects.txt`. Build independent; read-only over the client
    /// tree.
    ///
    /// Example:
    /// ```bash
    /// cd tools/rs3-cache-rs
    /// cargo run --release -- extract-stack-effects
    /// ```
    #[command(name = "extract-stack-effects")]
    ExtractStackEffects {
        /// Clientscript package dir globbed for `*.java` handler sources.
        #[arg(long, default_value = crate::stack_effects::DEFAULT_CLIENTSCRIPT_DIR)]
        clientscript_dir: PathBuf,
        /// Opcode-name books (repeatable); a handler is kept only when its name
        /// appears in one. Defaults to `opcodes-947.txt` + `opcodes-910.txt`.
        #[arg(long)]
        opcodes: Vec<PathBuf>,
        /// Output path (default: `<data-dir>/stack-effects.txt`).
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Derive a NEW build's CS2 opcode book from an OLD build's by cross-cache
    /// script alignment.
    ///
    /// Opcodes are rescrambled every build, but unchanged scripts keep identical
    /// instruction structure. For every clientscripts group present in both
    /// caches with the same instruction count, decode the OLD script with the OLD
    /// book, walk the NEW script's bytecode in lockstep (operand widths reused
    /// from `script.rs`), and vote `old command → new opcode`. The votes give the
    /// new book (old-book order, then extras sorted). Reuses `js5::decompress`.
    ///
    /// Example:
    /// ```bash
    /// cd tools/rs3-cache-rs
    /// cargo run --release -- derive-opcode-book \
    ///   --old-cache ../../cache/unpacked/947 --old-book data/opcodes-947.txt \
    ///   --new-cache ../../cache/unpacked/948 --out data/opcodes-948.txt
    /// ```
    #[command(name = "derive-opcode-book")]
    DeriveOpcodeBook {
        /// OLD build flat cache dir (its `<archive>/*.dat` groups).
        #[arg(long)]
        old_cache: PathBuf,
        /// NEW build flat cache dir.
        #[arg(long)]
        new_cache: PathBuf,
        /// OLD build's opcode book (`name,id` per line).
        #[arg(long)]
        old_book: PathBuf,
        /// Output path for the derived NEW book.
        #[arg(long)]
        out: PathBuf,
        /// Cache archive holding the clientscripts groups (default `12`).
        #[arg(long, default_value_t = 12)]
        archive: u32,
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
    pub fn groups(self) -> &'static [(u32, VarDomain)] {
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
    pub fn entity_type(self) -> EntityType {
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

    pub fn label(self) -> &'static str {
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
        let drift = crate::protocol_registry::run_generate(
            &crate::protocol_registry::GenerateProtocolOpts {
                schema_dir: schema_dir.as_deref().unwrap_or(&default_schema),
                server_root,
                client_root,
                check: *check,
            },
        )?;
        if drift {
            std::process::exit(3);
        }
        return Ok(());
    }
    if let Command::SurveyPayloads {
        client_root,
        server_root,
        out_dir,
    } = &cli.command
    {
        let default_out = cli.data_dir.join("protocol").join("910");
        crate::protocol_registry::run_survey(&crate::protocol_registry::SurveyPayloadsOpts {
            client_root,
            server_root,
            out_dir: out_dir.as_deref().unwrap_or(&default_out),
        })?;
        return Ok(());
    }
    // `extract-stack-effects` parses the client clientscript Java sources + the
    // opcode books; it needs no flat cache, so intercept it before `open_cache`
    // (like `extract-protocol` / `survey-payloads`).
    if let Command::ExtractStackEffects {
        clientscript_dir,
        opcodes,
        out,
    } = &cli.command
    {
        let default_opcodes: Vec<PathBuf> = crate::stack_effects::DEFAULT_OPCODE_FILES
            .iter()
            .map(|rel| cli.data_dir.join(rel.trim_start_matches("data/")))
            .collect();
        let opcode_files = if opcodes.is_empty() {
            default_opcodes.as_slice()
        } else {
            opcodes.as_slice()
        };
        let default_out = cli.data_dir.join("stack-effects.txt");
        crate::stack_effects::run(&crate::stack_effects::ExtractStackEffectsOpts {
            clientscript_dir,
            opcode_files,
            out: out.as_deref().unwrap_or(&default_out),
        })?;
        return Ok(());
    }
    // `derive-opcode-book` reads two flat caches' archive-12 `.dat` groups + the
    // old book directly (via `js5::decompress` + the `script.rs` width model); it
    // does not open the global cache, so intercept it before `open_cache`.
    if let Command::DeriveOpcodeBook {
        old_cache,
        new_cache,
        old_book,
        out,
        archive,
    } = &cli.command
    {
        crate::opcode_book::run(&crate::opcode_book::DeriveOpcodeBookOpts {
            old_cache,
            new_cache,
            old_book,
            out,
            archive: *archive,
        })?;
        return Ok(());
    }
    if let Command::AssembleScriptBatch { manifest, out_dir } = &cli.command {
        return crate::commands::assemble::run_batch(
            &cli.data_dir,
            RuntimeVersion {
                build: cli.build,
                subbuild: cli.subbuild,
            },
            crate::commands::assemble::AssembleBatchOpts {
                manifest: manifest.clone(),
                out_dir: out_dir.clone(),
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
        return Ok(crate::explain::run(
            &crate::explain::ExplainInterfaceOptions {
                interface: *id,
                build: *decode_build,
                source,
                json: *json,
                transitive: transitive_opts,
            },
        )?);
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
            command:
                Some(Cs2Command::LintSplice {
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
        return Ok(crate::port::config::run(
            &crate::port::config::ConfigPortOptions {
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
            },
        )?);
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
    let opened_cache = open_cache(cli.cache_dir.as_deref())?;
    let version = RuntimeVersion {
        build: cli.build,
        subbuild: cli.subbuild,
    };
    // The shared context owns the cache; commands that still take piecemeal args
    // borrow it back via `ctx.cache()` / `ctx.tar_path()` until they are migrated.
    // `cli.data_dir` remains available as an owned value for those handlers.
    let ctx = CommandContext::new(opened_cache, tar_path, version, cli.data_dir.clone());

    match cli.command {
        Command::Interfaces { out_dir } => crate::commands::config_extract::run_interfaces(
            &ctx,
            crate::commands::config_extract::InterfacesOpts { out_dir },
        ),
        Command::Varps { out_file, domain } => crate::commands::config_extract::run_varps(
            &ctx,
            crate::commands::config_extract::VarpsOpts { out_file, domain },
        ),
        Command::Varbits { out_file } => crate::commands::config_extract::run_varbits(
            &ctx,
            crate::commands::config_extract::VarbitsOpts { out_file },
        ),
        Command::Configs { out_dir } => crate::commands::config_extract::run_configs(
            &ctx,
            crate::commands::config_extract::ConfigsOpts { out_dir },
        ),
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
                }) => crate::commands::cs2::run_port(
                    &ctx,
                    crate::commands::cs2::Cs2PortOpts {
                        from,
                        to,
                        closure_of_interface,
                        base_cache_dir,
                        out_dir: port_out_dir,
                        check_oracle,
                        json,
                    },
                ),
                Some(Cs2Command::LintSplice { .. }) => {
                    bail!("cs2 lint-splice should have been dispatched before cache open")
                }
                None => crate::commands::cs2::run_dump(
                    &ctx,
                    crate::commands::cs2::Cs2DumpOpts { out_file, out_dir },
                ),
            }
        }
        Command::Port { command } => crate::commands::cs2::run_port_command(&ctx, &command),
        // `config transcode` is intercepted before the cache is opened.
        Command::Config { .. } => {
            bail!("config subcommand should have been dispatched before cache open")
        }
        Command::Models {
            out_file,
            out_dir,
            sample_only,
        } => crate::commands::models::run(
            &ctx,
            crate::commands::models::ModelsOpts {
                out_file,
                out_dir,
                sample_only,
            },
        ),
        Command::Audio { out_dir, max_files } => crate::commands::audio::run(
            &ctx,
            crate::commands::audio::AudioOpts { out_dir, max_files },
        ),
        Command::Unpack {
            out_dir,
            sample_models,
            skip_audio,
            best_effort_maps,
            max_audio_files,
        } => crate::commands::unpack::run(
            &ctx,
            crate::commands::unpack::UnpackOpts {
                out_dir,
                sample_models,
                skip_audio,
                best_effort_maps,
                max_audio_files,
            },
        ),
        Command::DepTreeInterface {
            id,
            max_depth,
            out_file,
        } => crate::commands::dep_tree::run_interface(
            &ctx,
            crate::commands::dep_tree::DepTreeOpts {
                id,
                max_depth,
                out_file,
            },
        ),
        Command::DepTreeScript {
            id,
            max_depth,
            out_file,
        } => crate::commands::dep_tree::run_script(
            &ctx,
            crate::commands::dep_tree::DepTreeOpts {
                id,
                max_depth,
                out_file,
            },
        ),
        Command::DepTreeVarp {
            id,
            domain,
            max_depth,
            out_file,
        } => crate::commands::dep_tree::run_varp(
            &ctx,
            crate::commands::dep_tree::DepTreeVarpOpts {
                id,
                domain,
                max_depth,
                out_file,
            },
        ),
        Command::DepTreeVarbit {
            id,
            max_depth,
            out_file,
        } => crate::commands::dep_tree::run_varbit(
            &ctx,
            crate::commands::dep_tree::DepTreeOpts {
                id,
                max_depth,
                out_file,
            },
        ),
        Command::ExplainLoc {
            id,
            max_candidates,
            json,
        } => Ok(crate::explain_loc::run(
            ctx.cache(),
            ctx.tar_path(),
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
        } => crate::commands::dep_tree::run_config(
            &ctx,
            crate::commands::dep_tree::DepTreeConfigOpts {
                kind,
                id,
                max_depth,
                out_file,
            },
        ),
        Command::TsExport { out_dir } => crate::commands::ts_export::run(
            &ctx,
            crate::commands::ts_export::TsExportOpts { out_dir },
        ),
        Command::TranspileScripts {
            out_dir,
            filter_script,
            output_style,
            max_scripts,
            all_scripts,
            max_script_bytes,
            max_script_instructions,
            max_generated_bytes,
        } => crate::commands::transpile::run(
            &ctx,
            crate::commands::transpile::TranspileScriptsOpts {
                out_dir,
                filter_script,
                output_style,
                max_scripts,
                all_scripts,
                limits: crate::transpile::TranspileLimits {
                    max_script_bytes,
                    max_instructions: max_script_instructions,
                    max_generated_bytes,
                },
            },
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
        } => crate::commands::migrate::run_check(
            &ctx,
            crate::commands::migrate::MigrateCheckOpts {
                interface_group,
                out_file,
                audit_dir,
                source: crate::commands::migrate::MigrateSource {
                    cache_tar: source_cache_tar,
                    build: source_build,
                    subbuild: source_subbuild,
                },
                remap: crate::commands::migrate::RemapOpts {
                    enabled: remap,
                    buffer: remap_buffer,
                },
                validate_target,
                allow_heuristic_sites,
            },
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
        } => crate::commands::migrate::run_script(
            &ctx,
            crate::commands::migrate::MigrateScriptOpts {
                script_id,
                out_file,
                audit_dir,
                source: crate::commands::migrate::MigrateSource {
                    cache_tar: source_cache_tar,
                    build: source_build,
                    subbuild: source_subbuild,
                },
                remap: crate::commands::migrate::RemapOpts {
                    enabled: remap,
                    buffer: remap_buffer,
                },
                validate_target,
                allow_heuristic_sites,
            },
        ),
        Command::ValidateScript {
            script_id,
            out_file,
            json,
        } => crate::commands::validate::run(
            &ctx,
            crate::commands::validate::ValidateScriptOpts {
                script_id,
                out_file,
                emit_json: json,
            },
        ),
        Command::AssembleScript {
            input,
            output,
            build,
            subbuild,
            strict_structured,
            no_verify,
            json,
        } => crate::commands::assemble::run_script(
            &ctx,
            crate::commands::assemble::AssembleScriptOpts {
                input,
                output,
                build,
                subbuild,
                strict_structured,
                no_verify,
                emit_json: json,
            },
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
        Command::SurveyPayloads { .. } => unreachable!("handled before cache open"),
        Command::ExtractStackEffects { .. } => unreachable!("handled before cache open"),
        Command::DeriveOpcodeBook { .. } => unreachable!("handled before cache open"),
        Command::GenerateCs2Data { .. } => unreachable!("handled before cache open"),
        Command::Cs2Coverage { .. } => unreachable!("handled before cache open"),
        Command::GenerateTsIds { .. } => unreachable!("handled before cache open"),
        Command::DumpRawFlat { out_dir, archives } => crate::commands::dump::run_raw_flat(
            &ctx,
            crate::commands::dump::DumpRawFlatOpts { out_dir, archives },
        ),
        Command::DumpRefs { out_dir } => {
            crate::commands::dump::run_refs(&ctx, crate::commands::dump::DumpRefsOpts { out_dir })
        }
        Command::DumpConfigs { out_dir } => crate::commands::dump::run_configs(
            &ctx,
            crate::commands::dump::DumpConfigsOpts { out_dir },
        ),
        Command::PrepareOverlay { out_dir, archives } => {
            crate::commands::dump::run_prepare_overlay(
                &ctx,
                crate::commands::dump::PrepareOverlayOpts { out_dir, archives },
            )
        }
        Command::VerifyMapArchive => crate::commands::verify_map::run(&ctx),
        Command::BuildCollision { map_x, map_z, out } => crate::commands::build_collision::run(
            &ctx,
            crate::commands::build_collision::BuildCollisionOpts { map_x, map_z, out },
        ),
        Command::OverlayPlan { .. } => unreachable!("handled before cache open"),
    }
}
