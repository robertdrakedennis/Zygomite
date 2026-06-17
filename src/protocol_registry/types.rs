//! Protocol schema data model: the `Prot` table selector and the serde types
//! for Java/TS packets, schema documents, cross-diff findings, and the
//! divergence baseline. Pure data — split out of `protocol_registry` verbatim.

use serde::Serialize;

/// The three protocol tables, in their stable emission order.
pub const PROTS: [Prot; 3] = [Prot::Server, Prot::Client, Prot::Login];

/// One of the three protocol tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prot {
    /// `ServerProt` — packets the server sends to the client.
    Server,
    /// `ClientProt` — packets the client sends to the server.
    Client,
    /// `LoginProt` — login/handshake packets.
    Login,
}

impl Prot {
    /// Lower-case wire tag used in the TSV, baseline, and report (`server` / `client` / `login`).
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::Server => "server",
            Self::Client => "client",
            Self::Login => "login",
        }
    }

    /// The Java class name backing this prot.
    #[must_use]
    pub const fn java_class(self) -> &'static str {
        match self {
            Self::Server => "ServerProt",
            Self::Client => "ClientProt",
            Self::Login => "LoginProt",
        }
    }

    /// The Java source file name (under the protocol package).
    #[must_use]
    pub const fn java_file(self) -> &'static str {
        match self {
            Self::Server => "ServerProt.java",
            Self::Client => "ClientProt.java",
            Self::Login => "LoginProt.java",
        }
    }

    /// The server TS source file name (under the protocol package).
    #[must_use]
    pub const fn ts_file(self) -> &'static str {
        match self {
            Self::Server => "ServerProt.ts",
            Self::Client => "ClientProt.ts",
            Self::Login => "LoginProt.ts",
        }
    }
}

/// One packet extracted from a client Java table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JavaPacket {
    /// Packet name (the static field identifier).
    pub name: String,
    /// Wire opcode.
    pub opcode: i32,
    /// Declared size (positive fixed, `-1` 1-byte prefix, `-2` 2-byte prefix).
    pub size: i32,
    /// `@ObfuscatedName` value attached to the declaration, when present.
    pub obf: Option<String>,
}

/// Result of parsing a client Java protocol table.
#[derive(Debug, Default)]
pub struct JavaParse {
    /// Packets in declaration order.
    pub packets: Vec<JavaPacket>,
    /// `true` when a `size` instance field is declared in the class.
    pub has_size_field: bool,
    /// `true` when the constructor body assigns `this.size`.
    pub ctor_assigns_size: bool,
    /// The raw constructor signature/body line(s) joined, for diagnostics.
    pub ctor_evidence: Option<String>,
}

/// One packet extracted from a server TS table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TsPacket {
    /// Packet name (the static field identifier).
    pub name: String,
    /// Wire opcode.
    pub opcode: i32,
    /// Declared size.
    pub size: i32,
}

// ---------------------------------------------------------------------------
// Schema documents
// ---------------------------------------------------------------------------

/// One packet row in a schema document.
#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct SchemaPacket {
    /// Packet name.
    pub name: String,
    /// Wire opcode.
    pub opcode: i32,
    /// Declared size.
    pub size: i32,
    /// `@ObfuscatedName` value, when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub obf: Option<String>,
}

/// A schema document for one prot.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct Schema {
    /// Schema version tag.
    pub schema: String,
    /// Resolved client Java path the schema was extracted from.
    pub source: String,
    /// Packets, sorted by opcode.
    pub packets: Vec<SchemaPacket>,
}

// ---------------------------------------------------------------------------
// Report + divergence baseline documents
// ---------------------------------------------------------------------------

/// One cross-diff finding.
#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    /// Check id (`P1`..`P6`).
    pub check: String,
    /// Severity (`error` / `warning` / `info`).
    pub severity: String,
    /// Affected prot tag.
    pub prot: String,
    /// `add` / `mismatch` / `dup` — the kind of divergence, for stable sorting.
    pub kind: String,
    /// Affected opcode, when applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opcode: Option<i32>,
    /// Human-readable finding message.
    pub message: String,
}

/// One entry in the divergence baseline.
#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct Divergence {
    /// Affected prot tag.
    pub prot: String,
    /// Affected opcode.
    pub opcode: i32,
    /// Check id that produced this divergence.
    pub check: String,
}

/// The divergence baseline document.
#[derive(Debug, Serialize, serde::Deserialize)]
pub struct DivergenceBaseline {
    /// Schema version tag.
    pub schema: String,
    /// Divergences, sorted by `(prot, opcode, check)`.
    pub divergences: Vec<Divergence>,
}
