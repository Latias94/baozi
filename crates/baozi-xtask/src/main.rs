use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut arguments = env::args().skip(1);
    match (arguments.next().as_deref(), arguments.next()) {
        (Some("adr-check"), None) => run_adr_check(),
        _ => {
            eprintln!("usage: cargo run -p baozi-xtask -- adr-check");
            ExitCode::from(2)
        }
    }
}

fn run_adr_check() -> ExitCode {
    let current_directory = match env::current_dir() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("failed to read the current directory: {error}");
            return ExitCode::FAILURE;
        }
    };
    let Some(repository_root) = find_repository_root(&current_directory) else {
        eprintln!("failed to find a Baozi repository root from {current_directory:?}");
        return ExitCode::FAILURE;
    };

    let report = baozi_xtask::adr::check_repository(&repository_root);
    if report.violations.is_empty() {
        println!("ADR governance check passed ({} ADRs)", report.adrs_checked);
        ExitCode::SUCCESS
    } else {
        for violation in &report.violations {
            eprintln!("{violation}");
        }
        eprintln!(
            "ADR governance check failed ({} violations across {} ADRs)",
            report.violations.len(),
            report.adrs_checked
        );
        ExitCode::FAILURE
    }
}

fn find_repository_root(start: &Path) -> Option<PathBuf> {
    start.ancestors().find_map(|candidate| {
        let has_workspace = candidate.join("Cargo.toml").is_file();
        let has_adr_directory = candidate.join("docs").join("adr").is_dir();
        (has_workspace && has_adr_directory).then(|| candidate.to_path_buf())
    })
}
