//! `owd-cases run [--cases DIR] [--report FILE]`: play each alignment case through the command line
//! and the app shell, read each expected value from both documents, and write one report.
//!
//! Exit code: 0 when no value fails and the surfaces agree; 1 when a value fails or the surfaces
//! differ; 2 when the case store is refused (a broken `case.json`, or bad arguments). A refused
//! store writes no report.

mod case;
mod compare;
mod play;

use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "usage: owd-cases run [--cases DIR] [--report FILE]";

struct Args {
    cases: PathBuf,
    report: PathBuf,
}

fn args() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    if it.next().as_deref() != Some("run") {
        return Err(USAGE.into());
    }
    let (mut cases, mut report) = (None, None);
    while let Some(a) = it.next() {
        let mut value = || {
            it.next()
                .ok_or_else(|| format!("{a} needs a value\n{USAGE}"))
        };
        match a.as_str() {
            "--cases" => cases = Some(PathBuf::from(value()?)),
            "--report" => report = Some(PathBuf::from(value()?)),
            _ => return Err(format!("unknown argument `{a}`\n{USAGE}")),
        }
    }
    // The folder named, else the one in OWD_ALIGNMENT_CASES, else the repository's own cases.
    let cases = cases
        .or_else(|| {
            std::env::var_os("OWD_ALIGNMENT_CASES")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        })
        .unwrap_or_else(|| PathBuf::from("tests/alignment_cases"));
    let report = report.unwrap_or_else(|| cases.join("alignment-report.json"));
    Ok(Args { cases, report })
}

fn main() -> ExitCode {
    let args = match args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("owd-cases: {e}");
            return ExitCode::from(2);
        }
    };
    let cases = match case::read_store(&args.cases) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("owd-cases: refused: {e}");
            return ExitCode::from(2);
        }
    };
    // The commands run in process: their own lines would mix with the table.
    owd_cli::commands::set_quiet(true);
    let results: Vec<compare::CaseResult> = cases
        .iter()
        .map(|c| compare::run_case(c, play::command_line(c), play::shell(c)))
        .collect();
    let report = compare::report(&results);
    let text = serde_json::to_string_pretty(&report).expect("a report is JSON") + "\n";
    if let Err(e) = std::fs::write(&args.report, text) {
        eprintln!(
            "owd-cases: cannot write the report {}: {e}",
            args.report.display()
        );
        return ExitCode::from(2);
    }
    print!("{}", compare::table(&results));
    println!("report: {}", args.report.display());
    let totals = &report["totals"];
    let failed =
        totals["fail"].as_u64().unwrap_or(0) + totals["one_path_failures"].as_u64().unwrap_or(0);
    if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
