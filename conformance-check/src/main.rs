use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut repo: Option<PathBuf> = None;
    let mut fixture_facts = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--fixture" => fixture_facts = true,
            "-h" | "--help" => {
                println!("usage: conformance-check <agent-repo> [--fixture]");
                println!("  --fixture   additionally assert the Part VI fixture graph facts");
                return ExitCode::SUCCESS;
            }
            flag if flag.starts_with('-') => {
                eprintln!("error: unknown flag `{flag}`");
                return ExitCode::from(2);
            }
            path => {
                if repo.is_some() {
                    eprintln!("error: multiple repo paths given (one at a time)");
                    return ExitCode::from(2);
                }
                repo = Some(PathBuf::from(path));
            }
        }
    }
    let Some(repo) = repo else {
        eprintln!("usage: conformance-check <agent-repo> [--fixture]");
        return ExitCode::from(2);
    };

    // Name the fold. The checker certifies a store by folding it, so the
    // verdict is only meaningful against a stated fold version — and when the
    // checker's `yoagent-state` drifted from the runtimes', the skew was
    // discoverable only by reading two lockfiles.
    println!(
        "conformance-check {} — folding with yoagent-state {}",
        env!("CARGO_PKG_VERSION"),
        yoagent_state::VERSION,
    );

    let mut failed = false;
    for report in conformance_check::run_all(&repo, fixture_facts) {
        let mark = if report.passed() { "PASS" } else { "FAIL" };
        println!("[{mark}] check {} — {}", report.number, report.name);
        for note in &report.notes {
            println!("       note: {note}");
        }
        for failure in &report.failures {
            println!("       {failure}");
            failed = true;
        }
    }
    if failed {
        ExitCode::FAILURE
    } else {
        println!("conformant: all checks passed");
        ExitCode::SUCCESS
    }
}
