use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = concat!(
    "usage: cargo run -p baozi-xtask -- ",
    "<crate-graph-check|native-fs-check|runtime-admission-check>\n",
    "       cargo run -p baozi-xtask -- u4c-metrics --spec <path>"
);

fn main() -> ExitCode {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    match arguments.as_slice() {
        [help] if matches!(help.as_str(), "help" | "-h" | "--help") => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        [command] if command == "crate-graph-check" => run_crate_graph_check(),
        [command] if command == "native-fs-check" => run_native_fs_check(),
        [command] if command == "runtime-admission-check" => run_runtime_admission_check(),
        [command, flag, specification] if command == "u4c-metrics" && flag == "--spec" => {
            run_u4c_metrics(Path::new(specification))
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn run_u4c_metrics(specification_argument: &Path) -> ExitCode {
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
    match baozi_xtask::u4c_metrics::collect_repository(&repository_root, specification_argument) {
        Ok(metrics) => {
            println!("{metrics}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run_runtime_admission_check() -> ExitCode {
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

    let report = baozi_xtask::runtime_admission::check_repository(&repository_root);
    if report.violations.is_empty() {
        println!(
            "Runtime admission check passed ({} production packages, {} source files)",
            report.production_packages_checked, report.production_source_files_checked
        );
        ExitCode::SUCCESS
    } else {
        for violation in &report.violations {
            eprintln!("{violation}");
        }
        eprintln!(
            "Runtime admission check failed ({} violations across {} production packages and {} source files)",
            report.violations.len(),
            report.production_packages_checked,
            report.production_source_files_checked
        );
        ExitCode::FAILURE
    }
}

fn run_native_fs_check() -> ExitCode {
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

    let report = baozi_xtask::native_fs::check_repository(&repository_root);
    if report.violations.is_empty() {
        println!(
            "Native filesystem dependency check passed ({} target profiles)",
            report.profiles_checked
        );
        ExitCode::SUCCESS
    } else {
        for violation in &report.violations {
            eprintln!("{violation}");
        }
        eprintln!(
            "Native filesystem dependency check failed ({} violations across {} target profiles)",
            report.violations.len(),
            report.profiles_checked
        );
        ExitCode::FAILURE
    }
}

fn run_crate_graph_check() -> ExitCode {
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

    let report = baozi_xtask::crate_graph::check_repository(&repository_root);
    if report.violations.is_empty() {
        println!(
            "Crate graph check passed ({} workspace packages)",
            report.packages_checked
        );
        ExitCode::SUCCESS
    } else {
        for violation in &report.violations {
            eprintln!("{violation}");
        }
        eprintln!(
            "Crate graph check failed ({} violations across {} workspace packages)",
            report.violations.len(),
            report.packages_checked
        );
        ExitCode::FAILURE
    }
}

fn find_repository_root(start: &Path) -> Option<PathBuf> {
    start.ancestors().find_map(|candidate| {
        let has_workspace = candidate.join("Cargo.toml").is_file();
        let has_xtask = candidate
            .join("crates")
            .join("baozi-xtask")
            .join("Cargo.toml")
            .is_file();
        (has_workspace && has_xtask).then(|| candidate.to_path_buf())
    })
}
