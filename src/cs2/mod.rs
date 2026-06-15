//! CS2 build-time tooling that operates on the reversible `// @cs2` assembler
//! listings (the opcode-book-agnostic pragma form `assemble-script` consumes).
//!
//! [`lint`] diffs a spliced donor script against a target opcode book and flags
//! (or, with `--fix`, applies) the known port rewrites — the same set the relic
//! overlay's `build-relic-scripts.py` discovered one assemble-error / one live
//! NPE at a time. The rules are DATA tables keyed off the crate's opcode-book
//! registries so the diff stays in sync with the books.

pub mod lint;
