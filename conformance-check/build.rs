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
    // Both layouts, in this order. `cargo package` ships the lockfile at the
    // *package* root, beside this build script — so looking only at the
    // workspace parent worked in development and printed "unknown" for every
    // installed build, which is the one place the line has to work. Cargo also
    // regenerates the workspace lock before build scripts run, so the failure
    // was unreachable in-tree.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let lock = [manifest.join("Cargo.lock"), manifest.join("../Cargo.lock")]
        .into_iter()
        .find(|p| p.exists());

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

/// Every `version = "x"` following a `name = "<crate>"` in a Cargo.lock.
///
/// All of them, not the first. Cargo sorts by name then version ascending, so
/// taking the first reports the *lowest* when two versions resolve — and this
/// line exists precisely because 0.4 and 0.5 fold differently, so naming the
/// wrong one is worse than naming none. Ambiguity is reported as such.
fn resolved_version(lock: &str, crate_name: &str) -> Option<String> {
    let needle = format!("name = \"{crate_name}\"");
    let mut found: Vec<String> = Vec::new();
    let lines: Vec<&str> = lock.lines().collect();
    let mut in_package = false;
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        // Only `[[package]]` entries describe what was resolved. Cargo also
        // emits `[[patch.unused]]` in the same shape, so collecting every
        // matching name reported `ambiguous(...)` when a single version had
        // resolved — reachable with `[patch.crates-io] yoagent-state = { path
        // = ... }`, which is the obvious way to test a local fix.
        if trimmed.starts_with("[[") {
            in_package = trimmed == "[[package]]";
        }
        if !in_package || trimmed != needle {
            continue;
        }
        if let Some(v) = lines
            .get(i + 1)
            .and_then(|l| l.trim().strip_prefix("version = "))
        {
            found.push(v.trim_matches('"').to_string());
        }
    }
    match found.len() {
        0 => None,
        1 => found.pop(),
        _ => Some(format!("ambiguous({})", found.join(","))),
    }
}
