use super::{
    DivergenceBaseline, ExtractProtocolOpts, GenerateProtocolOpts, PayloadField, PayloadPacket,
    PayloadParam, Payloads, Prot, Schema, SchemaPacket, array_default_len, codec_fixed_width,
    emit_expr, encoder_fn_name, extract, generate, is_v1_codec, mirror_reads, parse_expr,
    parse_java, parse_ts, render_client_tsv, render_encoders_ts, render_server_ts,
    render_size_expr, validate_payload,
};
use crate::error::Result;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use tempfile::tempdir;

/// `(client_root, server_root)` produced by [`write_tree`].
type RootPair = (std::path::PathBuf, std::path::PathBuf);

const SERVER_JAVA: &str = r#"
package com.jagex.game.network.protocol;

@ObfuscatedName("nz")
public class ServerProt {

	@ObfuscatedName("nz.e")
	public static final ServerProt TELEMETRY_GRID_ADD_GROUP = new ServerProt(0, 5);

	@ObfuscatedName("nz.n")
	public static final ServerProt ENVIRONMENT_OVERRIDE = new ServerProt(1, -1);

	@ObfuscatedName("nz.gg")
	public final int id;

	@ObfuscatedName("nz.gr")
	public final int size;

	public ServerProt(int id, int size) {
		this.id = id;
		this.size = size;
	}
}
"#;

const LOGIN_JAVA: &str = r#"
package com.jagex.game.network.protocol;

@ObfuscatedName("nu")
public class LoginProt {

	@ObfuscatedName("nu.e")
	public static final LoginProt INIT_GAME_CONNECTION = new LoginProt(14, 0);

	@ObfuscatedName("nu.n")
	public static final LoginProt INIT_JS5REMOTE_CONNECTION = new LoginProt(15, -1);

	@ObfuscatedName("nu.r")
	public final int id;

	public LoginProt(int id, int size) {
		this.id = id;
	}
}
"#;

#[test]
fn java_parser_records_packets_and_obf() -> Result<()> {
    let parse = parse_java(SERVER_JAVA, "ServerProt", Path::new("ServerProt.java"))?;
    assert_eq!(parse.packets.len(), 2);
    assert_eq!(parse.packets[0].name, "TELEMETRY_GRID_ADD_GROUP");
    assert_eq!(parse.packets[0].opcode, 0);
    assert_eq!(parse.packets[0].size, 5);
    assert_eq!(parse.packets[0].obf.as_deref(), Some("nz.e"));
    assert_eq!(parse.packets[1].size, -1);
    assert!(parse.has_size_field);
    assert!(parse.ctor_assigns_size);
    Ok(())
}

#[test]
fn java_parser_detects_login_size_vacuity() -> Result<()> {
    let parse = parse_java(LOGIN_JAVA, "LoginProt", Path::new("LoginProt.java"))?;
    // No `public final int size;` field and the constructor never assigns
    // `this.size` — the size parameter is dead.
    assert!(!parse.has_size_field);
    assert!(!parse.ctor_assigns_size);
    assert!(
        parse
            .ctor_evidence
            .as_deref()
            .expect("ctor evidence")
            .contains("this.id")
    );
    // Sizes still come from declaration literals.
    assert_eq!(parse.packets[0].size, 0);
    assert_eq!(parse.packets[1].size, -1);
    Ok(())
}

#[test]
fn ts_parser_handles_new_and_register_forms() -> Result<()> {
    let server_ts = r"
export default class ServerProt {
    static readonly TELEMETRY_GRID_ADD_GROUP = new ServerProt(0, 5, 'TELEMETRY_GRID_ADD_GROUP');
    static readonly field3697 = new ServerProt(1, 9);
}
";
    let packets = parse_ts(server_ts, "ServerProt", Path::new("ServerProt.ts"))?;
    assert_eq!(packets.len(), 2);
    assert_eq!(packets[0].name, "TELEMETRY_GRID_ADD_GROUP");
    assert_eq!(packets[1].name, "field3697");
    assert_eq!(packets[1].size, 9);

    let login_ts = r"
export default class LoginProt {
    static readonly BY_ID: LoginProt[] = new Array(32);
    static readonly INIT_GAME_CONNECTION = LoginProt.register(14, 0, 'INIT_GAME_CONNECTION');
}
";
    let lp = parse_ts(login_ts, "LoginProt", Path::new("LoginProt.ts"))?;
    // The BY_ID array static is skipped; only the register() decl is kept.
    assert_eq!(lp.len(), 1);
    assert_eq!(lp[0].name, "INIT_GAME_CONNECTION");
    assert_eq!(lp[0].opcode, 14);
    Ok(())
}

/// Build a minimal client+server tree and run the full extraction.
fn write_tree(
    dir: &Path,
    server_java: &str,
    client_java: &str,
    login_java: &str,
    server_ts: &str,
    client_ts: &str,
    login_ts: &str,
) -> Result<RootPair> {
    let client = dir.join("client-root");
    let server = dir.join("server-root");
    let jdir = client.join("client/src/main/java/com/jagex/game/network/protocol");
    let tdir = server.join("src/jagex/network/protocol");
    fs::create_dir_all(&jdir)?;
    fs::create_dir_all(&tdir)?;
    fs::write(jdir.join("ServerProt.java"), server_java)?;
    fs::write(jdir.join("ClientProt.java"), client_java)?;
    fs::write(jdir.join("LoginProt.java"), login_java)?;
    fs::write(tdir.join("ServerProt.ts"), server_ts)?;
    fs::write(tdir.join("ClientProt.ts"), client_ts)?;
    fs::write(tdir.join("LoginProt.ts"), login_ts)?;
    Ok((client, server))
}

#[test]
fn cross_diff_fires_each_check() -> Result<()> {
    let dir = tempdir()?;

    // Client ClientProt: opcode 0 = A(size 3), opcode 1 = B(size 4). (no opcode 2)
    let client_java = r"
public class ClientProt {
	public static final ClientProt A = new ClientProt(0, 3);
	public static final ClientProt B = new ClientProt(1, 4);
	public final int id;
	public final int size;
	public ClientProt(int id, int size) {
		this.id = id;
		this.size = size;
	}
}
";
    // Server ClientProt.ts: opcode 0 renamed (P1), opcode 1 size differs (P2),
    // opcode 2 server-only (P4). B is also a P-something on the client side?
    // opcode 5 client-only handled by ServerProt below; here B(1) has wrong size.
    let client_ts = r"
export default class ClientProt {
    static readonly RENAMED = new ClientProt(0, 3, 'RENAMED');
    static readonly B = new ClientProt(1, 99, 'B');
    static readonly SERVER_ONLY = new ClientProt(2, 1, 'SERVER_ONLY');
}
";

    // ServerProt: opcode 0 only in client → P3 (server missing it).
    let server_java = r"
public class ServerProt {
	public static final ServerProt S0 = new ServerProt(0, 5);
	public static final ServerProt S1 = new ServerProt(1, 6);
	public final int id;
	public final int size;
	public ServerProt(int id, int size) {
		this.id = id;
		this.size = size;
	}
}
";
    let server_ts = r"
export default class ServerProt {
    static readonly S1 = new ServerProt(1, 6, 'S1');
}
";

    // LoginProt with vacuous size → P5. Duplicate opcode within Java → P6.
    let login_java = r"
public class LoginProt {
	public static final LoginProt L0 = new LoginProt(14, 0);
	public static final LoginProt L0DUP = new LoginProt(14, 1);
	public final int id;
	public LoginProt(int id, int size) {
		this.id = id;
	}
}
";
    let login_ts = r"
export default class LoginProt {
    static readonly L0 = LoginProt.register(14, 0, 'L0');
    static readonly L0DUP = LoginProt.register(14, 1, 'L0DUP');
}
";

    let (client, server) = write_tree(
        dir.path(),
        server_java,
        client_java,
        login_java,
        server_ts,
        client_ts,
        login_ts,
    )?;
    let out_dir = dir.path().join("protocol/910");
    let output = extract(&ExtractProtocolOpts {
        client_root: &client,
        server_root: &server,
        out_dir: &out_dir,
    })?;

    let checks: BTreeSet<String> = output.findings.iter().map(|f| f.check.clone()).collect();
    for expected in ["P1", "P2", "P3", "P4", "P5", "P6"] {
        assert!(checks.contains(expected), "missing {expected}: {checks:?}");
    }

    // Baseline carries P1/P2/P3/P4 (not P5/P6).
    let baseline: DivergenceBaseline = serde_json::from_str(&output.baseline)?;
    let baseline_checks: BTreeSet<String> = baseline
        .divergences
        .iter()
        .map(|d| d.check.clone())
        .collect();
    assert!(baseline_checks.contains("P1"));
    assert!(baseline_checks.contains("P2"));
    assert!(baseline_checks.contains("P3"));
    assert!(baseline_checks.contains("P4"));
    assert!(!baseline_checks.contains("P5"));
    assert!(!baseline_checks.contains("P6"));

    // Schema packet counts (client 2, server 2, login 2).
    assert_eq!(output.counts["client"], 2);
    assert_eq!(output.counts["server"], 2);
    assert_eq!(output.counts["login"], 2);
    Ok(())
}

#[test]
fn emission_formats_and_round_trips() {
    let server = Schema {
        schema: "protocol-910/v1".to_owned(),
        source: "x".to_owned(),
        packets: vec![super::SchemaPacket {
            name: "TELEMETRY_GRID_ADD_GROUP".to_owned(),
            opcode: 0,
            size: 5,
            obf: Some("nz.e".to_owned()),
        }],
    };
    let empty = Schema {
        schema: "protocol-910/v1".to_owned(),
        source: "x".to_owned(),
        packets: vec![],
    };
    let baseline = DivergenceBaseline {
        schema: "protocol-divergences/v1".to_owned(),
        divergences: vec![super::Divergence {
            prot: "client".to_owned(),
            opcode: 55,
            check: "P4".to_owned(),
        }],
    };

    let ts = render_server_ts(&server, &empty, &empty, &baseline);
    assert!(ts.contains("export const SERVER_PROT_910 = ["));
    assert!(ts.contains("{ name: 'TELEMETRY_GRID_ADD_GROUP', opcode: 0, size: 5 },"));
    assert!(ts.contains("{ prot: 'client', opcode: 55, check: 'P4' },"));
    assert!(ts.ends_with("] as const;\n"));

    let tsv = render_client_tsv(&server, &empty, &empty, &baseline);
    assert!(tsv.contains("divergence\tclient\t55\tP4\n"));
    assert!(tsv.contains("server\tTELEMETRY_GRID_ADD_GROUP\t0\t5\n"));
    assert!(tsv.ends_with('\n'));
}

#[test]
fn generate_check_round_trips_against_written_files() -> Result<()> {
    let dir = tempdir()?;
    // Minimal schema dir.
    let schema_dir = dir.path().join("protocol/910");
    fs::create_dir_all(&schema_dir)?;
    let schema = r#"{ "schema": "protocol-910/v1", "source": "x", "packets": [ { "name": "A", "opcode": 0, "size": 5 } ] }"#;
    fs::write(schema_dir.join("server_prot.json"), schema)?;
    fs::write(schema_dir.join("client_prot.json"), schema)?;
    fs::write(schema_dir.join("login_prot.json"), schema)?;
    fs::write(
        schema_dir.join("known-divergences.json"),
        r#"{ "schema": "protocol-divergences/v1", "divergences": [] }"#,
    )?;

    let server = dir.path().join("server-root");
    let client = dir.path().join("client-root");

    // Write, then --check must report no drift; double-write byte-identical.
    let drift = super::run_generate(&GenerateProtocolOpts {
        schema_dir: &schema_dir,
        server_root: &server,
        client_root: &client,
        check: false,
    })?;
    assert!(!drift);
    let g1 = generate(&GenerateProtocolOpts {
        schema_dir: &schema_dir,
        server_root: &server,
        client_root: &client,
        check: false,
    })?;
    let drift = super::run_generate(&GenerateProtocolOpts {
        schema_dir: &schema_dir,
        server_root: &server,
        client_root: &client,
        check: true,
    })?;
    assert!(!drift, "freshly written artifacts must not drift");
    // Byte-stable across runs.
    let g2 = generate(&GenerateProtocolOpts {
        schema_dir: &schema_dir,
        server_root: &server,
        client_root: &client,
        check: false,
    })?;
    assert_eq!(g1.server_ts.1, g2.server_ts.1);
    assert_eq!(g1.client_tsv.1, g2.client_tsv.1);
    Ok(())
}

#[test]
fn extract_is_byte_stable() -> Result<()> {
    let dir = tempdir()?;
    let (client, server) = write_tree(
        dir.path(),
        SERVER_JAVA,
        r"
public class ClientProt {
	public static final ClientProt A = new ClientProt(0, 3);
	public final int id;
	public final int size;
	public ClientProt(int id, int size) { this.id = id; this.size = size; }
}
",
        LOGIN_JAVA,
        r"
export default class ServerProt {
    static readonly TELEMETRY_GRID_ADD_GROUP = new ServerProt(0, 5, 'TELEMETRY_GRID_ADD_GROUP');
    static readonly ENVIRONMENT_OVERRIDE = new ServerProt(1, -1, 'ENVIRONMENT_OVERRIDE');
}
",
        r"
export default class ClientProt {
    static readonly A = new ClientProt(0, 3, 'A');
}
",
        r"
export default class LoginProt {
    static readonly INIT_GAME_CONNECTION = LoginProt.register(14, 0, 'INIT_GAME_CONNECTION');
    static readonly INIT_JS5REMOTE_CONNECTION = LoginProt.register(15, -1, 'INIT_JS5REMOTE_CONNECTION');
}
",
    )?;
    let opts = ExtractProtocolOpts {
        client_root: &client,
        server_root: &server,
        out_dir: &dir.path().join("protocol/910"),
    };
    let a = extract(&opts)?;
    let b = extract(&opts)?;
    assert_eq!(a.report, b.report);
    assert_eq!(a.baseline, b.baseline);
    assert_eq!(a.schemas, b.schemas);
    // Server prot extracted both packets in opcode order.
    assert!(a.schemas["server"].contains("TELEMETRY_GRID_ADD_GROUP"));
    assert_eq!(Prot::Server.tag(), "server");
    Ok(())
}

// -----------------------------------------------------------------------
// Stage 7 — payload encoder generation unit tests
// -----------------------------------------------------------------------

fn field(codec: &str, arg: &str) -> PayloadField {
    PayloadField {
        codec: codec.to_owned(),
        arg: arg.to_owned(),
    }
}

fn param(name: &str, ty: &str, default: Option<&str>) -> PayloadParam {
    PayloadParam {
        name: name.to_owned(),
        ty: ty.to_owned(),
        default: default.map(str::to_owned),
    }
}

/// Build a `PayloadPacket` with no computed alloc (fixed/v1 default).
fn pk(
    params: Vec<PayloadParam>,
    fields: Vec<PayloadField>,
    client_reads: &[&str],
) -> PayloadPacket {
    PayloadPacket {
        params,
        fields,
        client_reads: client_reads.iter().map(|s| (*s).to_owned()).collect(),
        alloc: None,
    }
}

/// Expect `validate_payload` to succeed and return its variable-size flag.
fn expect_variable(name: &str, packet: &PayloadPacket, size: i32) -> bool {
    match validate_payload(name, packet, size) {
        Ok(v) => v,
        Err(e) => panic!("validate_payload({name}) unexpectedly failed: {e}"),
    }
}

/// Expect `validate_payload` to fail and return the error message.
fn expect_error(name: &str, packet: &PayloadPacket, size: i32) -> String {
    match validate_payload(name, packet, size) {
        Ok(v) => panic!("validate_payload({name}) unexpectedly succeeded ({v})"),
        Err(e) => e.to_string(),
    }
}

#[test]
fn codec_width_table_matches_packet_ts() {
    // Fixed widths per Packet.ts write methods.
    assert_eq!(codec_fixed_width("p1"), Some(1));
    assert_eq!(codec_fixed_width("p1_alt3"), Some(1));
    assert_eq!(codec_fixed_width("pbool"), Some(1));
    assert_eq!(codec_fixed_width("p2_alt2"), Some(2));
    assert_eq!(codec_fixed_width("p3"), Some(3));
    assert_eq!(codec_fixed_width("p4_alt1"), Some(4));
    assert_eq!(codec_fixed_width("p5"), Some(5));
    assert_eq!(codec_fixed_width("p6"), Some(6));
    assert_eq!(codec_fixed_width("p8"), Some(8));
    // Variable / unknown codecs have no fixed width.
    assert_eq!(codec_fixed_width("pjstr"), None);
    assert_eq!(codec_fixed_width("pSmart1or2"), None);
    assert_eq!(codec_fixed_width("pdata"), None);
    // v1 admission.
    assert!(is_v1_codec("p2_alt2"));
    assert!(is_v1_codec("pjstr"));
    assert!(is_v1_codec("pSmart1or2"));
    assert!(!is_v1_codec("pdata"));
    assert!(!is_v1_codec("pSmart2or4"));
}

#[test]
fn mirror_table_pairs_widths_and_alts() {
    assert!(mirror_reads("p1").contains(&"g1b"));
    assert!(mirror_reads("p2_alt2").contains(&"g2_alt2"));
    assert!(mirror_reads("p2_alt1").contains(&"g2_alt1"));
    assert!(mirror_reads("p4").contains(&"g4s"));
    assert!(mirror_reads("p4_alt1").contains(&"g4_alt1"));
    assert!(mirror_reads("pjstr").contains(&"gjstr"));
    assert!(mirror_reads("pSmart1or2").contains(&"gSmart1or2"));
    // A width-mismatched read is never accepted.
    assert!(!mirror_reads("p1").contains(&"g2"));
    assert!(!mirror_reads("p2_alt2").contains(&"g2_alt1"));
    // Unknown codec yields an empty allow-set.
    assert!(mirror_reads("pdata").is_empty());
}

#[test]
fn encoder_fn_names_are_camel_cased() {
    assert_eq!(encoder_fn_name("VARP_SMALL"), "encodeVarpSmall");
    assert_eq!(
        encoder_fn_name("CLIENT_SETVARC_SMALL"),
        "encodeClientSetvarcSmall"
    );
    assert_eq!(encoder_fn_name("IF_MOVESUB"), "encodeIfMovesub");
    assert_eq!(
        encoder_fn_name("SPOTANIM_SPECIFIC"),
        "encodeSpotanimSpecific"
    );
}

#[test]
fn fixed_size_sum_must_equal_schema_size() {
    // VARP_SMALL: p1 (1) + p2_alt2 (2) = 3 == declared 3 → ok.
    let ok = pk(
        vec![param("id", "int", None), param("value", "int", None)],
        vec![field("p1", "value"), field("p2_alt2", "id")],
        &["g1b", "g2_alt2"],
    );
    assert!(!expect_variable("VARP_SMALL", &ok, 3));

    // Wrong declared size → hard error (sum 3 != 4).
    let err = expect_error("VARP_SMALL", &ok, 4);
    assert!(
        err.contains("fixed-size sum 3 != schema size 4"),
        "got: {err}"
    );
}

#[test]
fn variable_size_expression_emission() {
    // CLIENT_SETVARCSTR_SMALL: p2 (2) + pjstr → size -1 variable.
    let strpk = pk(
        vec![param("id", "int", None), param("value", "string", None)],
        vec![field("p2", "id"), field("pjstr", "value")],
        &["g2", "gjstr"],
    );
    assert!(expect_variable("CLIENT_SETVARCSTR_SMALL", &strpk, -1));
    assert_eq!(render_size_expr(&strpk).unwrap(), "2 + value.length + 1");

    // pSmart1or2 contributes a ternary term.
    let smart = pk(
        vec![param("type", "int", None)],
        vec![field("pSmart1or2", "type")],
        &["gSmart1or2"],
    );
    assert_eq!(render_size_expr(&smart).unwrap(), "(type < 128 ? 1 : 2)");

    // A fixed packet renders the literal byte count.
    let fixed = pk(
        vec![param("energy", "int", None)],
        vec![field("p1", "energy")],
        &["g1"],
    );
    assert_eq!(render_size_expr(&fixed).unwrap(), "1");

    // A variable schema size with no variable codec / no alloc is rejected.
    let bad = expect_error("X", &fixed, -1);
    assert!(
        bad.contains("neither a variable-width codec nor a computed `alloc`"),
        "got: {bad}"
    );
    // A fixed schema size with a variable codec is rejected.
    let bad2 = expect_error("Y", &strpk, 4);
    assert!(
        bad2.contains("variable-width codec is present"),
        "got: {bad2}"
    );
}

#[test]
fn mirror_symmetry_and_unknown_codec_rejected() {
    // Mirror mismatch: p1 paired with g2 (width mismatch) → error.
    let mismatch = pk(
        vec![param("v", "int", None)],
        vec![field("p1", "v")],
        &["g2"],
    );
    let err = expect_error("BAD", &mismatch, 1);
    assert!(err.contains("does not mirror"), "got: {err}");

    // Read/write length mismatch.
    let lenbad = pk(
        vec![param("a", "int", None), param("b", "int", None)],
        vec![field("p1", "a"), field("p1", "b")],
        &["g1"],
    );
    let err = expect_error("LEN", &lenbad, 2);
    assert!(err.contains("length mismatch"), "got: {err}");

    // Unknown (non-v1) codec.
    let unknown = pk(
        vec![param("v", "int", None)],
        vec![field("pdata", "v")],
        &["gdata"],
    );
    let err = expect_error("UNK", &unknown, 1);
    assert!(err.contains("not a DSL-v1 codec"), "got: {err}");
}

#[test]
fn render_encoders_ts_emits_validated_functions() -> Result<()> {
    let mut packets: BTreeMap<String, PayloadPacket> = BTreeMap::new();
    packets.insert(
        "VARP_SMALL".to_owned(),
        pk(
            vec![param("id", "int", None), param("value", "int", None)],
            vec![field("p1", "value"), field("p2_alt2", "id")],
            &["g1b", "g2_alt2"],
        ),
    );
    packets.insert(
        "SPOTANIM_SPECIFIC".to_owned(),
        pk(
            vec![
                param("targetHash", "int", None),
                param("height", "int", Some("0")),
            ],
            vec![field("p4_alt1", "targetHash"), field("p2_alt2", "height")],
            &["g4_alt1", "g2_alt2"],
        ),
    );
    let payloads = Payloads {
        schema: "protocol-payloads/v1".to_owned(),
        packets,
    };
    let server = Schema {
        schema: "protocol-910/v1".to_owned(),
        source: "x".to_owned(),
        packets: vec![
            SchemaPacket {
                name: "VARP_SMALL".to_owned(),
                opcode: 157,
                size: 3,
                obf: None,
            },
            SchemaPacket {
                name: "SPOTANIM_SPECIFIC".to_owned(),
                opcode: 99,
                size: 6,
                obf: None,
            },
        ],
    };
    let ts = render_encoders_ts(&payloads, &server)?;
    assert!(ts.starts_with("// GENERATED"));
    assert!(ts.contains("import Packet from '#jagex/bytepacking/Packet.js';"));
    assert!(ts.contains("export function encodeVarpSmall(id: number, value: number): Packet {"));
    assert!(ts.contains("    const buf: Packet = new Packet(new Uint8Array(3));"));
    assert!(ts.contains("    buf.p2_alt2(id);"));
    // Default preserved in the generated signature.
    assert!(ts.contains("height: number = 0"));

    // A bad schema size makes generation fail (sum check fires).
    let mut bad_server = server;
    bad_server.packets[0].size = 9;
    match render_encoders_ts(&payloads, &bad_server) {
        Ok(_) => panic!("render_encoders_ts should fail on a bad fixed-size schema"),
        Err(e) => assert!(e.to_string().contains("fixed-size sum"), "got: {e}"),
    }
    Ok(())
}

// -----------------------------------------------------------------------
// Stage 8 — DSL v2 expression + array-param + computed-alloc unit tests
// -----------------------------------------------------------------------

/// Parse then re-emit; assert the canonical output matches `want`.
fn roundtrip(input: &str, want: &str) {
    let e = parse_expr(input).unwrap_or_else(|err| panic!("parse `{input}`: {err}"));
    assert_eq!(emit_expr(&e), want, "round-trip of `{input}`");
}

#[test]
fn expr_parse_emit_round_trips() {
    // Plain v1 degenerate cases.
    roundtrip("value", "value");
    roundtrip("0xff", "0xff");
    roundtrip("42", "42");
    // Shift-or cuid (the IF_SET* family).
    roundtrip(
        "(interfaceId << 16) | component",
        "(interfaceId << 16) | component",
    );
    roundtrip(
        "(topLevelInterfaceId << 16) | component",
        "(topLevelInterfaceId << 16) | component",
    );
    // Mask.
    roundtrip("snapshotId & 0xff", "snapshotId & 0xff");
    roundtrip("fontId & 0xffffffff", "fontId & 0xffffffff");
    // Array access gains the `!` non-null assertion; `key[3]!` input also OK.
    roundtrip("key[3]", "key[3]!");
    roundtrip("key[3]!", "key[3]!");
    roundtrip("key[0]", "key[0]!");
}

#[test]
fn expr_rejects_out_of_grammar() {
    // Ternary, equality, multiplication, nullish — all rejected.
    assert!(parse_expr("hidden ? 1 : 0").is_err());
    assert!(parse_expr("objId === -1 ? 65535 : objId").is_err());
    assert!(parse_expr("sourceX * 2").is_err());
    assert!(parse_expr("text ?? 0").is_err());
    // Unbalanced parens / brackets.
    assert!(parse_expr("(a | b").is_err());
    assert!(parse_expr("key[1").is_err());
    // Non-decimal array index.
    assert!(parse_expr("key[0x1]").is_err());
}

#[test]
fn expr_validation_checks_idents_and_bounds() {
    use super::param_models;
    let params = vec![
        param("interfaceId", "int", None),
        param("component", "int", None),
        param("key", "int[]", Some("[0, 0, 0, 0]")),
    ];
    let pkt = pk(
        params.clone(),
        vec![
            field("p4_alt1", "(interfaceId << 16) | component"),
            field("p4", "key[3]"),
        ],
        &["g4_alt1", "g4s"],
    );
    // size: 4 + 4 = 8 fixed; passes full validation.
    assert!(!expect_variable("V2OK", &pkt, 8));

    // Undeclared identifier.
    let bad_ident = pk(
        params.clone(),
        vec![field("p4", "bogus | component")],
        &["g4s"],
    );
    let err = expect_error("V2BAD", &bad_ident, 4);
    assert!(
        err.contains("`bogus` is not a declared param"),
        "got: {err}"
    );

    // Index out of bounds (default has 4 elements, index 4 is OOB).
    let oob = pk(params.clone(), vec![field("p4", "key[4]")], &["g4s"]);
    let err = expect_error("V2OOB", &oob, 4);
    assert!(err.contains("index out of bounds"), "got: {err}");

    // Indexing a scalar param.
    let scalar_index = pk(
        params.clone(),
        vec![field("p4", "interfaceId[0]")],
        &["g4s"],
    );
    let err = expect_error("V2SCALAR", &scalar_index, 4);
    assert!(err.contains("indexes a non-array param"), "got: {err}");

    // Bare array param without an index.
    let bare_array = pk(params, vec![field("p4", "key")], &["g4s"]);
    let err = expect_error("V2BARE", &bare_array, 4);
    assert!(err.contains("used without an index"), "got: {err}");

    // Array param missing its default → param-model build fails.
    let model_params = [param("a", "int", None), param("k", "int[]", Some("[0, 0]"))];
    let (scalars, arrays) = param_models("OK", &model_params).unwrap();
    assert!(scalars.contains("a"));
    assert_eq!(arrays.get("k"), Some(&2));
    let bad_params = [param("k", "int[]", None)];
    assert!(param_models("BAD", &bad_params).is_err());
}

#[test]
fn array_default_len_counts_elements() {
    assert_eq!(array_default_len("[0, 0, 0, 0]").unwrap(), 4);
    assert_eq!(array_default_len("[1,2,3]").unwrap(), 3);
    assert_eq!(array_default_len("[ ]").unwrap(), 0);
    assert!(array_default_len("0, 0").is_err());
}

#[test]
fn if_opensub_shaped_end_to_end_render() -> Result<()> {
    // IF_OPENSUB: array param + shift-or cuid, all fixed (size 23).
    let mut packets: BTreeMap<String, PayloadPacket> = BTreeMap::new();
    packets.insert(
        "IF_OPENSUB".to_owned(),
        PayloadPacket {
            params: vec![
                param("topLevelInterfaceId", "int", None),
                param("component", "int", None),
                param("subInterfaceId", "int", None),
                param("type", "int", None),
                param("key", "int[]", Some("[0, 0, 0, 0]")),
            ],
            fields: vec![
                field("p4_alt2", "key[2]"),
                field("p4_alt1", "(topLevelInterfaceId << 16) | component"),
                field("p1_alt2", "type"),
                field("p4", "key[3]"),
                field("p2", "subInterfaceId"),
                field("p4_alt2", "key[1]"),
                field("p4_alt2", "key[0]"),
            ],
            client_reads: [
                "g4_alt2", "g4_alt1", "g1_alt2", "g4s", "g2", "g4_alt2", "g4_alt2",
            ]
            .iter()
            .map(|s| (*s).to_owned())
            .collect(),
            alloc: None,
        },
    );
    let payloads = Payloads {
        schema: "protocol-payloads/v2".to_owned(),
        packets,
    };
    let server = Schema {
        schema: "protocol-910/v1".to_owned(),
        source: "x".to_owned(),
        packets: vec![SchemaPacket {
            name: "IF_OPENSUB".to_owned(),
            opcode: 100,
            size: 23,
            obf: None,
        }],
    };
    let ts = render_encoders_ts(&payloads, &server)?;
    assert!(ts.contains(
            "export function encodeIfOpensub(topLevelInterfaceId: number, component: number, subInterfaceId: number, type: number, key: number[] = [0, 0, 0, 0]): Packet {"
        ), "signature:\n{ts}");
    assert!(ts.contains("    const buf: Packet = new Packet(new Uint8Array(23));"));
    // Array access carries `!`, shift-or is parenthesized as parsed.
    assert!(ts.contains("    buf.p4_alt2(key[2]!);"), "body:\n{ts}");
    assert!(ts.contains("    buf.p4_alt1((topLevelInterfaceId << 16) | component);"));
    assert!(ts.contains("    buf.p4(key[3]!);"));
    Ok(())
}

#[test]
fn computed_alloc_validates_and_renders() -> Result<()> {
    // A contrived variable packet whose size is a computed in-grammar expr.
    let mut packets: BTreeMap<String, PayloadPacket> = BTreeMap::new();
    packets.insert(
        "VARALLOC".to_owned(),
        PayloadPacket {
            params: vec![param("a", "int", None), param("b", "int", None)],
            fields: vec![field("p2", "a")],
            client_reads: vec!["g2".to_owned()],
            alloc: Some("2 + b".to_owned()),
        },
    );
    let payloads = Payloads {
        schema: "protocol-payloads/v2".to_owned(),
        packets,
    };
    let server = Schema {
        schema: "protocol-910/v1".to_owned(),
        source: "x".to_owned(),
        packets: vec![SchemaPacket {
            name: "VARALLOC".to_owned(),
            opcode: 1,
            size: -1,
            obf: None,
        }],
    };
    let ts = render_encoders_ts(&payloads, &server)?;
    assert!(
        ts.contains("    const buf: Packet = new Packet(new Uint8Array(2 + b));"),
        "body:\n{ts}"
    );

    // A computed alloc on a fixed-size packet is rejected.
    let fixed_with_alloc = pk(
        vec![param("a", "int", None)],
        vec![field("p2", "a")],
        &["g2"],
    );
    let mut bad = fixed_with_alloc;
    bad.alloc = Some("2".to_owned());
    let err = expect_error("FIXEDALLOC", &bad, 2);
    assert!(err.contains("fixed but a computed `alloc`"), "got: {err}");
    Ok(())
}
