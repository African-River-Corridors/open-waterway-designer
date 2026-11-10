//! The flags of `owd volumes` carry no default of their own.
//!
//! The default of a volumes setting lives in one place, in `owd-format`. The command line does
//! not type it again: not as a flag default and not as a number in the help text. The help still
//! shows each default, so it must read it from the shared settings.
//!
//! Label: property

use serde_json::Value;
use std::path::Path;
use std::process::Command;

/// The flags of `owd volumes` that set a number, with the setting each one sets.
const FLAGS: [(&str, &str); 2] = [("--station", "station_m"), ("--maxw", "maxw_m")];

/// The lines of the `Volumes { ... }` variant of `enum Command` in `crates/owd-cli/src/cli.rs`.
fn variant_lines() -> Vec<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/cli.rs");
    let text = std::fs::read_to_string(path).unwrap();
    let block: Vec<String> = text
        .lines()
        .skip_while(|l| *l != "    Volumes {")
        .take_while(|l| *l != "    },")
        .map(str::to_string)
        .collect();
    assert!(
        block.len() > 1,
        "`enum Command` in src/cli.rs holds no `    Volumes {{` block"
    );
    block
}

/// The rule. No line of the variant holds `default_value`. No line holds the word `default` with
/// a digit after it on the same line.
#[test]
fn no_flag_of_owd_volumes_types_a_default() {
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
        "`owd volumes` types a default of its own in src/cli.rs:\n{}",
        found.join("\n")
    );
}

/// A number as the help shows it: `25`, not `25.0`.
fn shown(v: &Value) -> String {
    format!("{}", v.as_f64().expect("a number"))
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
fn the_help_of_owd_volumes_shows_the_default_the_stage_uses() {
    // The settings the volumes stage wrote into the example, with nothing set by the user.
    let example =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/web/river-example.owd");
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(example).unwrap()).unwrap();
    let defaults = &doc["stages"]["volumes"]["settings"];

    let out = Command::new(env!("CARGO_BIN_EXE_owd"))
        .args(["volumes", "--help"])
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
