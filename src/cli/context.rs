//! The shared per-invocation [`CommandContext`].
//!
//! Historically every `run_*` handler in `cli.rs` received the resolved
//! environment piecemeal as positional arguments (`cache`, `tar_path`,
//! `data_dir`, `build`, `subbuild`, …). `CommandContext` bundles that
//! environment once so handlers can take `(&ctx, opts)` instead — the
//! command-specific fields stay in their own `Opts` struct.
//!
//! The cache-dependent fields are resolved once by the dispatcher (see
//! [`crate::cli::run`]); the expensive derived artefacts (the opcode book) are
//! built lazily on first use so cheap commands never pay for them.

use std::cell::OnceCell;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::cache::FlatCache;
use crate::script::OpcodeBook;

/// The cache build + sub-build the command runs against.
#[derive(Clone, Copy, Debug)]
pub struct RuntimeVersion {
    pub build: u32,
    pub subbuild: u32,
}

/// The shared environment every cache-backed command needs, resolved once in
/// the dispatcher and passed to each handler by reference.
pub struct CommandContext {
    /// The opened flat cache (`--cache-dir` / `--cache-tar`).
    pub cache: FlatCache,
    /// Path to the canonical cache `.tar` (`--cache-tar` or the default).
    pub tar_path: PathBuf,
    /// The build / sub-build to decode against (`--build` / `--subbuild`).
    pub version: RuntimeVersion,
    /// The crate data directory (`--data-dir`).
    pub data_dir: PathBuf,
    /// Memoized opcode book at `version.build` / `version.subbuild`, built on
    /// first use. Handlers that need a *different* build's book (e.g. the
    /// 948→910 port) load that one directly.
    opcode_book: OnceCell<OpcodeBook>,
}

impl CommandContext {
    /// Build the context from the resolved cache + environment.
    pub fn new(
        cache: FlatCache,
        tar_path: PathBuf,
        version: RuntimeVersion,
        data_dir: PathBuf,
    ) -> Self {
        Self {
            cache,
            tar_path,
            version,
            data_dir,
            opcode_book: OnceCell::new(),
        }
    }

    /// The opened flat cache.
    pub fn cache(&self) -> &FlatCache {
        &self.cache
    }

    /// The canonical cache `.tar` path.
    pub fn tar_path(&self) -> &Path {
        &self.tar_path
    }

    /// The crate data directory.
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// The build number to decode against.
    pub fn build(&self) -> u32 {
        self.version.build
    }

    /// The sub-build number to decode against.
    pub fn subbuild(&self) -> u32 {
        self.version.subbuild
    }

    /// The opcode book at the command's `version`, loaded once on first use.
    pub fn opcode_book(&self) -> Result<&OpcodeBook> {
        if let Some(book) = self.opcode_book.get() {
            return Ok(book);
        }
        let book = OpcodeBook::load(&self.data_dir, self.version.build, self.version.subbuild)?;
        // `OnceCell::get_or_try_init` is unstable; this set cannot race (no
        // interior `&mut`/threads) and only runs when `get` above missed.
        let _ = self.opcode_book.set(book);
        Ok(self
            .opcode_book
            .get()
            .expect("opcode book was just initialised"))
    }
}
