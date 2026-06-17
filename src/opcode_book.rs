//! `derive-opcode-book` — bootstrap a NEW build's CS2 opcode book from an OLD
//! build's by cross-cache script alignment (the port of the retired
//! `scripts/derive-opcode-book.py`).
//!
//! CS2 opcodes are fully rescrambled every RS3 build, but unchanged scripts keep
//! identical instruction structure. For every clientscripts group (archive 12)
//! present in both caches with the same instruction count, decode the OLD script
//! with the OLD book, then walk the NEW script's bytecode in lockstep (operand
//! widths are determined by command semantics, identical for the same
//! instruction), reading the NEW opcode at each position. Votes per `old command
//! → new opcode` across ~30k scripts give the new book; a script is discarded
//! unless the lockstep walk lands exactly on the header boundary on both sides.
//!
//! Reuses the crate rather than re-deriving cache internals:
//! - [`crate::js5::decompress`] for the JS5 group container (replaces the
//!   Python's hand-rolled bz2/gzip/lzma `decompress`).
//! - [`crate::script::script_header_geometry`] + [`crate::script::skip_operand_bytes`]
//!   for the script header parse and per-command operand widths (replaces the
//!   Python's hardcoded `FOUR_BYTE`/`EIGHT_BYTE`/`VAR_CMDS`/… sets + `parse_header`),
//!   so a future bytecode-model change updates the real decoder and this walk
//!   together.
//!
//! Output: `<name>,<opcode>` lines, OLD-book order first, then any extra derived
//! commands sorted. `tests/opcode_book_oracle.rs` is the regression gate
//! (948-from-947 → `data/opcodes-948.txt`, byte-exact).

use crate::error::{Context, Result};
use crate::packet::Packet;
use crate::script::{script_header_geometry, skip_operand_bytes};
use crate::{cache_bail as bail, js5};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Both 947 and 948 are >= 800, and only the era thresholds in the shared script
/// decoder matter, so a single decode version drives both sides of the walk.
const WALK_VERSION: u32 = 948;

/// Options for [`run`] / [`derive`].
pub struct DeriveOpcodeBookOpts<'a> {
    /// OLD build flat cache dir (holds `<archive>/*.dat`).
    pub old_cache: &'a Path,
    /// NEW build flat cache dir.
    pub new_cache: &'a Path,
    /// OLD build's opcode book (`name,id` per line).
    pub old_book: &'a Path,
    /// Output path for the derived NEW book.
    pub out: &'a Path,
    /// Cache archive holding the clientscripts groups (12).
    pub archive: u32,
}

/// Derive the NEW book and write it to `opts.out`. Returns the exact bytes
/// written so callers can report/compare without re-reading.
pub fn run(opts: &DeriveOpcodeBookOpts<'_>) -> Result<String> {
    let outcome = derive(opts)?;
    fs::write(opts.out, &outcome.book)
        .with_context(|| format!("failed writing derived book {}", opts.out.display()))?;
    eprintln!(
        "groups: common={} used={} skipped(len-change)={} failed={}",
        outcome.common_groups, outcome.used, outcome.skipped, outcome.failed
    );
    eprintln!(
        "commands: book={} derived={} missing/unvoted={} conflicts={}",
        outcome.book_commands, outcome.derived, outcome.missing, outcome.conflicts
    );
    eprintln!("wrote {} ({} entries)", opts.out.display(), outcome.entries);
    Ok(outcome.book)
}

/// The derived book text plus the run statistics (mirrors the Python's stderr
/// progress lines, surfaced as fields for tests/callers).
pub struct DeriveOutcome {
    /// The full `name,id\n` book text.
    pub book: String,
    /// Number of groups present in BOTH caches' archive.
    pub common_groups: usize,
    /// Groups whose lockstep walk succeeded on both sides.
    pub used: usize,
    /// Groups skipped because the OLD/NEW instruction counts differed.
    pub skipped: usize,
    /// Groups discarded because a walk failed (unknown opcode / boundary miss).
    pub failed: usize,
    /// Commands in the OLD book.
    pub book_commands: usize,
    /// OLD-book commands that received a resolved mapping.
    pub derived: usize,
    /// OLD-book commands left without a mapping.
    pub missing: usize,
    /// Commands rejected by the confidence/bijectivity guard.
    pub conflicts: usize,
    /// Total entries written.
    pub entries: usize,
}

/// A vote tally for one OLD command: opcode → count, preserving the opcode
/// insertion order so `most_common`-style resolution breaks ties by first-seen
/// (matching Python's stable `Counter.most_common`).
#[derive(Default)]
struct OrderedCounter {
    /// `(opcode, count)` in first-seen order.
    entries: Vec<(u16, u32)>,
    /// opcode → index into `entries`.
    index: HashMap<u16, usize>,
}

impl OrderedCounter {
    fn add(&mut self, opcode: u16) {
        if let Some(&i) = self.index.get(&opcode) {
            self.entries[i].1 += 1;
        } else {
            self.index.insert(opcode, self.entries.len());
            self.entries.push((opcode, 1));
        }
    }

    /// Total votes (== the top entry's count is read separately).
    fn top_count(&self) -> u32 {
        self.entries.iter().map(|&(_, n)| n).max().unwrap_or(0)
    }

    /// The top two `(opcode, count)` by count descending, ties broken by
    /// first-seen order — a stable sort over the insertion-ordered `entries`.
    fn top_two(&self) -> ((u16, u32), Option<(u16, u32)>) {
        let mut ranked: Vec<(u16, u32)> = self.entries.clone();
        // Stable sort by count descending preserves insertion order on ties,
        // exactly like `Counter.most_common`.
        ranked.sort_by(|a, b| b.1.cmp(&a.1));
        let first = ranked.first().copied().unwrap_or((0, 0));
        let second = ranked.get(1).copied();
        (first, second)
    }
}

/// Run the derivation without writing the output file.
pub fn derive(opts: &DeriveOpcodeBookOpts<'_>) -> Result<DeriveOutcome> {
    let (order, old_by_op) = load_old_book(opts.old_book)?;

    let archive = opts.archive.to_string();
    let old_dir = opts.old_cache.join(&archive);
    let new_dir = opts.new_cache.join(&archive);
    let groups = common_groups(&old_dir, &new_dir)?;

    // `votes` keyed by command, plus a first-seen command order so the
    // confidence-ranked resolution is deterministic and matches the Python's
    // insertion-ordered `defaultdict` iteration.
    let mut votes: HashMap<String, OrderedCounter> = HashMap::new();
    let mut command_order: Vec<String> = Vec::new();
    let mut used = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;

    for group in &groups {
        match tally_group(&old_dir, &new_dir, group, &old_by_op) {
            Ok(GroupResult::Skipped) => skipped += 1,
            Ok(GroupResult::Voted(pairs)) => {
                for (cmd, new_op) in pairs {
                    let counter = votes.entry(cmd.clone()).or_insert_with(|| {
                        command_order.push(cmd.clone());
                        OrderedCounter::default()
                    });
                    counter.add(new_op);
                }
                used += 1;
            }
            Err(_) => failed += 1,
        }
    }

    let resolution = resolve(&votes, &command_order);

    // Emit OLD-book order first, then any extra derived commands sorted.
    let mut book = String::new();
    let mut entries = 0usize;
    for name in &order {
        if let Some(&op) = resolution.mapping.get(name) {
            book.push_str(name);
            book.push(',');
            book.push_str(&op.to_string());
            book.push('\n');
            entries += 1;
        }
    }
    let order_set: std::collections::HashSet<&str> = order.iter().map(String::as_str).collect();
    let mut extras: Vec<(&String, u16)> = resolution
        .mapping
        .iter()
        .filter(|(name, _)| !order_set.contains(name.as_str()))
        .map(|(name, &op)| (name, op))
        .collect();
    extras.sort_by(|a, b| a.0.cmp(b.0));
    for (name, op) in extras {
        book.push_str(name);
        book.push(',');
        book.push_str(&op.to_string());
        book.push('\n');
        entries += 1;
    }

    let derived = order
        .iter()
        .filter(|n| resolution.mapping.contains_key(*n))
        .count();
    let missing = order.len() - derived;

    Ok(DeriveOutcome {
        book,
        common_groups: groups.len(),
        used,
        skipped,
        failed,
        book_commands: order.len(),
        derived,
        missing,
        conflicts: resolution.conflicts,
        entries,
    })
}

/// Parse the OLD book into `(book_order, opcode → name)`. Lines without a comma
/// are skipped; the id is the text after the LAST comma (`rsplit(',', 1)`).
fn load_old_book(path: &Path) -> Result<(Vec<String>, HashMap<u16, String>)> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed reading old book {}", path.display()))?;
    let mut order = Vec::new();
    let mut old_by_op = HashMap::new();
    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() || !line.contains(',') {
            continue;
        }
        let (name, op_text) = line
            .rsplit_once(',')
            .context("old book row missing comma after check")?;
        let op = op_text
            .parse::<u16>()
            .with_context(|| format!("invalid opcode id in old book row: {line}"))?;
        order.push(name.to_string());
        // Later rows win for a duplicate opcode, mirroring the Python dict
        // comprehension `{v: k for k, v in name_to_old.items()}` over an
        // insertion-ordered dict (last assignment wins).
        old_by_op.insert(op, name.to_string());
    }
    Ok((order, old_by_op))
}

/// Group stems present in BOTH archive dirs, sorted by numeric stem (non-numeric
/// stems sort last, at `1 << 30`), matching the Python's `sorted(... key=...)`.
fn common_groups(old_dir: &Path, new_dir: &Path) -> Result<Vec<String>> {
    let old = dat_stems(old_dir)?;
    let new = dat_stems(new_dir)?;
    let mut common: Vec<String> = old.into_iter().filter(|s| new.contains(s)).collect();
    common.sort_by_key(|s| (sort_key(s), s.clone()));
    Ok(common)
}

/// Sort key for a group stem: its integer value, or `1 << 30` when non-numeric
/// (so they trail), exactly like the Python lambda.
fn sort_key(stem: &str) -> i64 {
    stem.parse::<i64>().unwrap_or(1 << 30)
}

/// Collect the `*.dat` file stems (filename without extension) in `dir`.
fn dat_stems(dir: &Path) -> Result<std::collections::HashSet<String>> {
    let entries = fs::read_dir(dir)
        .with_context(|| format!("failed reading archive dir {}", dir.display()))?;
    let mut stems = std::collections::HashSet::new();
    for entry in entries {
        let path = entry
            .with_context(|| format!("failed reading dir entry in {}", dir.display()))?
            .path();
        if path.extension().is_some_and(|ext| ext == "dat")
            && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
        {
            stems.insert(stem.to_string());
        }
    }
    Ok(stems)
}

/// One group's tally outcome.
enum GroupResult {
    /// The OLD/NEW instruction counts differed; the group is unusable.
    Skipped,
    /// `(old_command, new_opcode)` votes from the lockstep walk.
    Voted(Vec<(String, u16)>),
}

/// Decompress both sides, length-gate, lockstep-walk, and return the votes.
fn tally_group(
    old_dir: &Path,
    new_dir: &Path,
    group: &str,
    old_by_op: &HashMap<u16, String>,
) -> Result<GroupResult> {
    let old_raw = fs::read(old_dir.join(format!("{group}.dat")))?;
    let new_raw = fs::read(new_dir.join(format!("{group}.dat")))?;
    let old_data = js5::decompress(&old_raw)?;
    let new_data = js5::decompress(&new_raw)?;

    let (_, _, old_code_len) = script_header_geometry(&old_data, WALK_VERSION)?;
    let (_, _, new_code_len) = script_header_geometry(&new_data, WALK_VERSION)?;
    if old_code_len != new_code_len {
        return Ok(GroupResult::Skipped);
    }

    // OLD walk: commands come from the book (opcode → name).
    let old_walk = walk_with_book(&old_data, old_by_op)?;
    // NEW walk: impose the OLD command sequence per index, reading the NEW opcode.
    let commands: Vec<&str> = old_walk.iter().map(|(cmd, _)| cmd.as_str()).collect();
    let new_walk = walk_with_commands(&new_data, &commands)?;

    let votes = old_walk
        .iter()
        .zip(&new_walk)
        .map(|((cmd, _), &new_op)| (cmd.clone(), new_op))
        .collect();
    Ok(GroupResult::Voted(votes))
}

/// Walk a script's code region resolving each opcode through the book
/// (`opcode → name`). Returns `(command, opcode)` per instruction. Errors when an
/// opcode is unknown or the walk does not land exactly on the header boundary.
fn walk_with_book(data: &[u8], names_by_op: &HashMap<u16, String>) -> Result<Vec<(String, u16)>> {
    let (start, header_pos, code_len) = script_header_geometry(data, WALK_VERSION)?;
    let mut packet = Packet::with_pos(data, start)?;
    let mut out = Vec::with_capacity(code_len);
    while packet.pos() < header_pos {
        let opcode = packet.g2()?;
        let command = names_by_op
            .get(&opcode)
            .with_context(|| format!("unknown opcode {opcode}"))?
            .clone();
        skip_operand_bytes(&command, &mut packet, WALK_VERSION)?;
        out.push((command, opcode));
    }
    if packet.pos() != header_pos || out.len() != code_len {
        bail!("walk did not land on header boundary");
    }
    Ok(out)
}

/// Walk a script's code region imposing `commands[i]` at instruction `i`, reading
/// the (unknown, this-build) opcode at each position. Returns the opcode per
/// instruction. Errors when the imposed sequence is shorter than the stream or
/// the walk does not land exactly on the header boundary.
fn walk_with_commands(data: &[u8], commands: &[&str]) -> Result<Vec<u16>> {
    let (start, header_pos, code_len) = script_header_geometry(data, WALK_VERSION)?;
    let mut packet = Packet::with_pos(data, start)?;
    let mut out = Vec::with_capacity(code_len);
    let mut i = 0usize;
    while packet.pos() < header_pos {
        let command = commands
            .get(i)
            .context("more instructions than counterpart")?;
        let opcode = packet.g2()?;
        skip_operand_bytes(command, &mut packet, WALK_VERSION)?;
        out.push(opcode);
        i += 1;
    }
    if packet.pos() != header_pos || out.len() != code_len {
        bail!("walk did not land on header boundary");
    }
    Ok(out)
}

/// Result of resolving votes into a bijective `command → opcode` mapping.
struct Resolution {
    mapping: HashMap<String, u16>,
    conflicts: usize,
}

/// Resolve votes into a bijective mapping. Process commands in confidence order
/// (highest top-vote first; ties broken by first-seen command order via the
/// stable sort over `command_order`). Accept a winner only when it is
/// uncontested enough (`n1 >= max(4*n2, n2+2)` when a runner-up exists) and its
/// opcode is not already claimed.
fn resolve(votes: &HashMap<String, OrderedCounter>, command_order: &[String]) -> Resolution {
    // Rank commands by descending top-vote, stable over first-seen order.
    let mut ranked: Vec<&String> = command_order.iter().collect();
    ranked.sort_by(|a, b| {
        let na = votes.get(*a).map_or(0, OrderedCounter::top_count);
        let nb = votes.get(*b).map_or(0, OrderedCounter::top_count);
        nb.cmp(&na)
    });

    let mut mapping: HashMap<String, u16> = HashMap::new();
    let mut claimed: std::collections::HashSet<u16> = std::collections::HashSet::new();
    let mut conflicts = 0usize;

    for cmd in ranked {
        let Some(counter) = votes.get(cmd) else {
            continue;
        };
        let ((op1, n1), runner_up) = counter.top_two();
        let n2 = runner_up.map_or(0, |(_, n)| n);
        // A contested top vote needs a clear margin over the runner-up.
        if n2 != 0 && n1 < (4 * n2).max(n2 + 2) {
            conflicts += 1;
            continue;
        }
        if claimed.contains(&op1) {
            conflicts += 1;
            continue;
        }
        claimed.insert(op1);
        mapping.insert(cmd.clone(), op1);
    }

    Resolution { mapping, conflicts }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_counter_breaks_ties_by_first_seen() {
        let mut c = OrderedCounter::default();
        c.add(7);
        c.add(3);
        c.add(7);
        c.add(3); // 7 and 3 both at 2; 7 seen first.
        let ((op1, n1), second) = c.top_two();
        assert_eq!((op1, n1), (7, 2));
        assert_eq!(second, Some((3, 2)));
    }

    #[test]
    fn sort_key_trails_non_numeric() {
        assert_eq!(sort_key("42"), 42);
        assert_eq!(sort_key("foo"), 1 << 30);
    }

    #[test]
    fn resolve_rejects_contested_and_claimed() {
        // `a`: 10 vs 1 → clear winner op 100.
        // `b`: 5 vs 4 → contested (5 < max(16,6)) → conflict.
        // `c`: also wants op 100 → claimed → conflict.
        let mut votes: HashMap<String, OrderedCounter> = HashMap::new();
        let mut a = OrderedCounter::default();
        for _ in 0..10 {
            a.add(100);
        }
        a.add(101);
        votes.insert("a".to_string(), a);
        let mut b = OrderedCounter::default();
        for _ in 0..5 {
            b.add(200);
        }
        for _ in 0..4 {
            b.add(201);
        }
        votes.insert("b".to_string(), b);
        let mut c = OrderedCounter::default();
        for _ in 0..3 {
            c.add(100);
        }
        votes.insert("c".to_string(), c);

        let order = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let res = resolve(&votes, &order);
        assert_eq!(res.mapping.get("a"), Some(&100));
        assert_eq!(res.mapping.get("b"), None);
        assert_eq!(res.mapping.get("c"), None);
        assert_eq!(res.conflicts, 2);
    }
}
