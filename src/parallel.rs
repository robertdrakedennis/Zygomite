use anyhow::{Context, Result, bail};
use std::sync::OnceLock;

static RAYON_THREADS: OnceLock<usize> = OnceLock::new();

pub fn init_global_rayon() -> Result<usize> {
    if let Some(&threads) = RAYON_THREADS.get() {
        return Ok(threads);
    }

    let threads = match std::env::var("RS3_CACHE_RS_THREADS") {
        Ok(raw) => {
            let parsed = raw
                .parse::<usize>()
                .with_context(|| format!("parsing RS3_CACHE_RS_THREADS={raw}"))?;
            if parsed == 0 {
                bail!("RS3_CACHE_RS_THREADS must be >= 1");
            }
            parsed
        }
        Err(std::env::VarError::NotPresent) => std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1),
        Err(err) => bail!("reading RS3_CACHE_RS_THREADS: {err}"),
    };

    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .thread_name(|idx| format!("rs3-cache-rs-{idx}"))
        .build_global()
        .context("building global rayon thread pool")?;
    let _ = RAYON_THREADS.set(threads);
    Ok(threads)
}
