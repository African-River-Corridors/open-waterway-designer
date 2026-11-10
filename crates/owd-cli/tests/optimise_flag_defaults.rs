//! The flags of `owd optimise` carry no default of their own.
//!
//! The default of an optimise setting lives in one place, in `owd-format`. The command line does
//! not type it again: not as a flag default and not as a number in the help text. The help still
//! shows each default, so it must read it from the shared settings.
//!
//! Label: property

use serde_json::Value;
use std::path::Path;
use std::process::Command;

/// The flags of `owd optimise` whose default is a number or a list, with the setting that holds
/// it. `--sites` sets the list of sites; with no list, the stage puts a site every `site_step_m`.
const FLAGS: [(&str, &str); 4] = [
    ("--sites", "site_step_m"),
    ("--lifts", "lifts_m"),
    ("--wet-weight", "wet_weight"),
    ("--dry-weight", "dry_weight"),
];

/// The lines of the `Optimise { ... }` variant of `enum Command` in `crates/owd-cli/src/cli.rs`.
fn variant_lines() -> Vec<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/cli.rs");
    let text = std::fs::read_to_string(path).unwrap();
    let block: Vec<String> = text
        .lines()
        .skip_while(|l| *l != "    Optimise {")
        .take_while(|l| *l != "    },")
        .map(str::to_string)
        .collect();
    assert!(
        block.len() > 1,
        "`enum Command` in src/cli.rs holds no `    Optimise {{` block"
    );
    block
}

/// The rule. No line of the variant holds `default_value`. No line holds the word `default` with
/// a digit after it on the same line.
#[test]
fn no_flag_of_owd_optimise_types_a_default() {
    let mut found = Vec::new();
    for line in variant_lines() {
        let lower = line.to_lowercase();
        let typed = lower
            .split_once("default")
            .is_some_and(|(_, rest)| rest.chars().any(|c| c.is_ascii_digit()));
        if lower.contains("default_value") || typed {
            found.push(line.trim().to_string());
        }
    }
    assert!(
        found.is_empty(),
        "`owd optimise` types a default of its own in src/cli.rs:\n{}",
        found.join("\n")
    );
}

/// A number as the help shows it (`500`, not `500.0`); a list with commas (`0.5,1`).
fn shown(v: &Value) -> String {
    match v.as_array() {
        Some(list) => list.iter().map(shown).collect::<Vec<_>>().join(","),
        None => format!("{}", v.as_f64().expect("a number")),
    }
}

/// The help text of one flag: from the line that names it to the next flag.
fn flag_help(help: &str, flag: &str) -> String {
    help.lines()
        .skip_while(|l| !l.trim_start().starts_with(flag))
        .enumerate()
        .take_while(|(i, l)| *i == 0 || !l.trim_start().starts_with('-'))
        .map(|(_, l)| l.trim())
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn the_help_of_owd_optimise_shows_the_default_the_stage_uses() {
    // The settings the optimise stage wrote into an example. The user set the sites only.
    let example =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/web/river-rolling-circle.owd");
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(example).unwrap()).unwrap();
    let defaults = &doc["stages"]["optimise"]["settings"];

    let out = Command::new(env!("CARGO_BIN_EXE_owd"))
        .args(["optimise", "--help"])
        .output()
        .expect("owd runs");
    let help = String::from_utf8_lossy(&out.stdout).into_owned();
    for (flag, key) in FLAGS {
        let text = flag_help(&help, flag);
        let value = shown(&defaults[key]);
        assert!(
            text.split_once("default")
                .is_some_and(|(_, rest)| rest.contains(&value)),
            "the help of `{flag}` does not show the default {value} of `{key}`: {text}"
        );
    }
}
