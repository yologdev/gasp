//! Records which `yoagent-state` the checker folds with, so the report can say so.
//!
//! The checker answers "can a conformant runtime fold and restore this store?"
//! by folding the store itself. That answer is only meaningful if the fold it
//! uses matches the fold runtimes use — and when it drifted, the skew was
//! discoverable only by reading two lockfiles.
//!
//! A certificate that does not say what produced it cannot be audited.
//!
//! Reads the workspace lockfile rather than guessing: cargo exposes no env var
//! for a dependency's resolved version, and the requirement in `Cargo.toml`
//! ("0.5") is not what was actually built.

use std::path::PathBuf;

fn main() {
    let lock = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|p| p.join("Cargo.lock"));

    let version = lock
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|body| resolved_version(&body, "yoagent-state"))
        // Not a build failure: a missing or restructured lockfile should not
        // stop the checker from running, only from naming its fold version.
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=GASP_STATE_VERSION={version}");
    if let Some(p) = lock {
        println!("cargo:rerun-if-changed={}", p.display());
    }
}

/// The `version = "x"` line following `name = "<crate>"` in a Cargo.lock.
fn resolved_version(lock: &str, crate_name: &str) -> Option<String> {
    let needle = format!("name = \"{crate_name}\"");
    let mut lines = lock.lines().skip_while(|l| l.trim() != needle);
    lines.next()?;
    lines
        .next()
        .and_then(|l| l.trim().strip_prefix("version = "))
        .map(|v| v.trim_matches('"').to_string())
}
