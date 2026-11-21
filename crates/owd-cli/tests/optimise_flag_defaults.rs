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

/// `--length-weight` (a cost per metre of design line) shows the stage's own default, 0 (off), read
/// from the shared settings: the setting is not written into a design while it is 0, so the example
/// above does not hold it.
#[test]
fn the_help_of_length_weight_shows_the_default_the_stage_uses() {
    let d = owd_format::design::OptimiseSettings::default().length_weight;
    let out = Command::new(env!("CARGO_BIN_EXE_owd"))
        .args(["optimise", "--help"])
        .output()
        .expect("owd runs");
    let help = String::from_utf8_lossy(&out.stdout).into_owned();
    let text = flag_help(&help, "--length-weight");
    assert!(
        text.contains(&format!("(default {d}: off)")),
        "the help of `--length-weight` does not show the default {d}: {text}"
    );
}

/// `owd optimise --length-weight` goes through the stage's own reading of its settings: a negative
/// weight is refused by name (exit 2) and the design is not written.
#[test]
fn a_negative_length_weight_is_refused_by_name() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("owd-length-weight");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("d.owd");
    let text = owd_format::design::Session::new("Length weight", "2026-11-20").to_text();
    std::fs::write(&path, &text).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_owd"))
        .args(["optimise", path.to_str().unwrap(), "--length-weight=-1"])
        .output()
        .expect("owd runs");
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{err}");
    assert!(err.contains("length_weight"), "{err}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
}

/// `--land-weight` (a cost per m2 of channel footprint outside the river polygon) shows the stage's
/// own default, 0 (off), and its unit, read from the shared settings.
#[test]
fn the_help_of_land_weight_shows_the_default_the_stage_uses() {
    let d = owd_format::design::OptimiseSettings::default().land_weight;
    let out = Command::new(env!("CARGO_BIN_EXE_owd"))
        .args(["optimise", "--help"])
        .output()
        .expect("owd runs");
    let help = String::from_utf8_lossy(&out.stdout).into_owned();
    let text = flag_help(&help, "--land-weight");
    assert!(text.contains("<M3_PER_M2>"), "{text}");
    assert!(
        text.contains(&format!("(default {d}: off)")),
        "the help of `--land-weight` does not show the default {d}: {text}"
    );
}

/// `owd optimise --land-weight` goes through the stage's own reading of its settings: a negative
/// weight is refused by name (exit 2) and the design is not written.
#[test]
fn a_negative_land_weight_is_refused_by_name() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("owd-land-weight");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("d.owd");
    let text = owd_format::design::Session::new("Land weight", "2026-11-20").to_text();
    std::fs::write(&path, &text).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_owd"))
        .args(["optimise", path.to_str().unwrap(), "--land-weight=-1"])
        .output()
        .expect("owd runs");
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{err}");
    assert!(
        err.contains("the setting `land_weight` must be a number >= 0"),
        "{err}"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
}

/// `--corridor-limit` goes through the stage's own reading of its settings. Its help says what it
/// does; a value that is not true or false is refused before anything runs (exit 2) and the design
/// is not written.
#[test]
fn the_corridor_limit_flag_is_documented_and_refuses_a_value_that_is_not_true_or_false() {
    let out = Command::new(env!("CARGO_BIN_EXE_owd"))
        .args(["optimise", "--help"])
        .output()
        .expect("owd runs");
    let help = String::from_utf8_lossy(&out.stdout).into_owned();
    let text = flag_help(&help, "--corridor-limit");
    assert!(text.contains("corridor_half_m"), "{text}");
    assert!(text.contains("keeps the baseline"), "{text}");

    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("owd-corridor-limit");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("d.owd");
    let text = owd_format::design::Session::new("Corridor limit", "2026-11-20").to_text();
    std::fs::write(&path, &text).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_owd"))
        .args([
            "optimise",
            path.to_str().unwrap(),
            "--corridor-limit",
            "maybe",
        ])
        .output()
        .expect("owd runs");
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    // The flag alone, and with a value, are both read: the run then stops at what the bare design
    // lacks (its sizing), not at the flag.
    for args in [vec!["--corridor-limit"], vec!["--corridor-limit", "false"]] {
        let out = Command::new(env!("CARGO_BIN_EXE_owd"))
            .arg("optimise")
            .arg(path.to_str().unwrap())
            .args(&args)
            .output()
            .expect("owd runs");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains("sizing"), "{args:?}: {err}");
    }
}

/// The flood flags show the stage's own defaults, read from the shared settings: the flood cost is
/// 0 (off), the distance 1500 m, and the limit above bank-full has none.
#[test]
fn the_help_of_the_flood_flags_shows_the_defaults_the_stage_uses() {
    let d = owd_format::design::OptimiseSettings::default();
    assert_eq!(d.flood_cost_per_ha, 0.0);
    assert_eq!(d.max_above_bank_m, None);
    let out = Command::new(env!("CARGO_BIN_EXE_owd"))
        .args(["optimise", "--help"])
        .output()
        .expect("owd runs");
    let help = String::from_utf8_lossy(&out.stdout).into_owned();
    let cost = flag_help(&help, "--flood-cost-per-ha");
    assert!(
        cost.contains(&format!("(default {}: off)", d.flood_cost_per_ha)),
        "{cost}"
    );
    assert!(cost.contains("3000"), "{cost}");
    let limit = flag_help(&help, "--max-above-bank");
    assert!(limit.contains("(default: no limit)"), "{limit}");
    assert!(limit.contains("estimate"), "{limit}");
    let dist = flag_help(&help, "--flood-distance");
    assert!(
        dist.contains(&format!("(default {})", d.flood_distance_m)),
        "{dist}"
    );
    assert!(help.contains("--flood-report"), "{help}");
}

/// The flood flags go through the stage's own reading of its settings: a bad value is refused by
/// the setting's name (exit 2) and the design is not written.
#[test]
fn a_bad_flood_flag_is_refused_by_name() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("owd-flood-flags");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("d.owd");
    let text = owd_format::design::Session::new("Flood flags", "2026-11-21").to_text();
    std::fs::write(&path, &text).unwrap();
    for (flag, name) in [
        ("--flood-cost-per-ha=-1", "flood_cost_per_ha"),
        ("--max-above-bank=-0.5", "max_above_bank_m"),
        ("--flood-distance=0", "flood_distance_m"),
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_owd"))
            .args(["optimise", path.to_str().unwrap(), flag])
            .output()
            .expect("owd runs");
        let err = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(2), "{flag}: {err}");
        assert!(err.contains(name), "{flag}: {err}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }
}

/// The flags of the joins (OPT-3) are documented with the stage's own defaults, and go through the
/// stage's own reading of its settings: a bad value is refused by name before anything runs (exit
/// 2) and the design is not written; a good one is read, and the run then stops at what the bare
/// design lacks (its sizing), not at the flag.
#[test]
fn the_join_flags_are_documented_and_refuse_bad_values_by_name() {
    let d = owd_format::design::OptimiseSettings::default();
    let out = Command::new(env!("CARGO_BIN_EXE_owd"))
        .args(["optimise", "--help"])
        .output()
        .expect("owd runs");
    let help = String::from_utf8_lossy(&out.stdout).into_owned();
    for (flag, says) in [
        ("--seams-on-straights", "straightest".to_string()),
        ("--free-seams", "gate".to_string()),
        ("--seam-window", format!("(default {})", d.seam_window_m)),
        (
            "--seam-heading-tol",
            format!("(default {})", d.seam_heading_tol_deg),
        ),
        ("--min-straight", format!("(default {})", d.min_straight_m)),
        ("--weir-straight", "two lengths of".to_string()),
        ("--seam-offset", "river".to_string()),
    ] {
        let text = flag_help(&help, flag);
        assert!(text.contains(&says), "{flag}: `{says}` is not in: {text}");
    }

    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("owd-join-flags");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("d.owd");
    let text = owd_format::design::Session::new("Joins", "2026-11-21").to_text();
    std::fs::write(&path, &text).unwrap();
    let run = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_owd"))
            .arg("optimise")
            .arg(path.to_str().unwrap())
            .args(args)
            .output()
            .expect("owd runs");
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    for (args, says) in [
        (
            vec!["--min-straight=-1"],
            "the setting `min_straight_m` must be a number of metres >= 0",
        ),
        (
            vec!["--seam-window=-1"],
            "the setting `seam_window_m` must be a number of metres >= 0",
        ),
        (
            vec!["--weir-straight=-1"],
            "the setting `weir_straight_m` must be a number of metres >= 0",
        ),
        (
            vec!["--seam-heading-tol", "95"],
            "the setting `seam_heading_tol_deg` must be a number of degrees",
        ),
    ] {
        let (code, err) = run(&args);
        assert_eq!(code, Some(2), "{args:?}: {err}");
        assert!(err.contains(says), "{args:?}: {err}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }
    for args in [
        vec!["--free-seams", "maybe"],
        vec!["--seams-on-straights", "maybe"],
    ] {
        let (code, _) = run(&args);
        assert_eq!(code, Some(2), "{args:?}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }
    for args in [
        vec!["--free-seams"],
        vec!["--free-seams", "false"],
        vec!["--seams-on-straights"],
        vec!["--seams-on-straights", "--seam-window", "900"],
        vec![
            "--free-seams",
            "--weir-straight",
            "180",
            "--min-straight",
            "90",
            "--seam-offset",
            "40",
        ],
    ] {
        let (_, err) = run(&args);
        assert!(err.contains("sizing"), "{args:?}: {err}");
    }
}
