//! The optimisation document (`docs/optimisation/`) against the engine and against its own worked
//! example. The document is a living record of the design: these tests fail when it drifts.
//!
//! Fast, on every change:
//! - the settings tables of appendix B hold the engine's names and defaults (a new setting fails
//!   to compile here until it is in the table; a changed default fails the test);
//! - every number the document tags (`<!--=key-->`) is the example's stored number
//!   (`docs/optimisation/example/numbers.json`) at the precision shown;
//! - every block of the tool's printed output is in the stored output of its run, line for line;
//! - every guarantee (section 6) and every item of section 7 carries an anchor
//!   (`<!--test: file::name-->`) and the test it names exists;
//! - the links between the two files resolve.
//!
//! Slow, nightly (`.config/nextest.toml`, `docs/agents/slow-tests.md`): each run of the example
//! is run again through the `owd` binary, by the commands of `docs/optimisation/example/runs.json`,
//! and compared with the stored numbers to 1 part in a million.
//!
//! Label: property (the document agrees with the engine); the re-runs are snapshots of the example.

use owd_core::alignment::radius_check::RADIUS_TOL;
use owd_core::cut_model::{ABOVE_SLOPE_H_PER_V, BERM_M, PRISM_INTRUDE_M};
use owd_core::optimise::flood::{BANK_MIN_CELLS, BANK_PERCENTILE, BANK_WINDOW_M};
use owd_core::optimise::lattice::{
    BOUNDARY_STEP_FRACTION, BOUNDARY_STEPS, CLOSE_GRACE_EXPANSIONS, DEFAULT_C_MIN_MAX_SAMPLES,
    DEFAULT_C_MIN_STRIDE_M, GATE_MIDDLE_BIAS, HalfWidthSpec, LINE_ARC_STEP_M, LatticeConfig,
    PIECE_BOUNDARY_SLACK, SectionSpec,
};
use owd_format::design::{OptimiseSettings, VolumesSettings};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn doc_dir() -> PathBuf {
    repo().join("docs/optimisation")
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

fn main_doc() -> String {
    read(&doc_dir().join("README.md"))
}

fn appendices() -> String {
    read(&doc_dir().join("appendices.md"))
}

fn numbers() -> BTreeMap<String, Value> {
    serde_json::from_str(&read(&doc_dir().join("example/numbers.json"))).unwrap()
}

fn runs() -> Value {
    serde_json::from_str(&read(&doc_dir().join("example/runs.json"))).unwrap()
}

// ----- the settings tables ----------------------------------------------------------------

/// The rows of the table between `<!-- settings:NAME -->` and `<!-- /settings -->`: the cells of
/// each body row, with code ticks removed.
fn table(text: &str, name: &str) -> Vec<Vec<String>> {
    let open = format!("<!-- settings:{name} -->");
    let body = text
        .split_once(&open)
        .unwrap_or_else(|| panic!("appendices.md holds no `{open}`"))
        .1
        .split_once("<!-- /settings -->")
        .expect("the table is closed by `<!-- /settings -->`")
        .0;
    body.lines()
        .filter(|l| l.starts_with('|') && !l.starts_with("|---"))
        .skip(1) // the header
        .map(|l| {
            l.trim_matches('|')
                .split('|')
                .map(|c| c.trim().trim_matches('`').to_string())
                .collect()
        })
        .collect()
}

/// Two JSON values are the same default: numbers by value (`500` is `500.0`), the rest exactly.
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same(p, q))
        }
        _ => a == b,
    }
}

/// Hold a table to `expected`: the same names, in any order, each with the same default. `name_col`
/// and `default_col` are the columns of the name and of the default.
fn hold(
    rows: &[Vec<String>],
    expected: &[(&str, Value)],
    name_col: usize,
    default_col: usize,
    what: &str,
) {
    let mut wrong = Vec::new();
    for (name, value) in expected {
        match rows.iter().find(|r| r[name_col] == *name) {
            None => wrong.push(format!(
                "`{name}` is not in the table (the engine has {value})"
            )),
            Some(r) => match serde_json::from_str::<Value>(&r[default_col]) {
                Ok(shown) if same(&shown, value) => {}
                _ => wrong.push(format!(
                    "`{name}`: the table says `{}`, the engine has {value}",
                    r[default_col]
                )),
            },
        }
    }
    for r in rows {
        if !expected.iter().any(|(n, _)| *n == r[name_col]) {
            wrong.push(format!(
                "`{}` is in the table and not in the engine",
                r[name_col]
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "docs/optimisation/appendices.md, {what}: update the table (and the text of the document \
         that names the setting):\n{}",
        wrong.join("\n")
    );
}

#[test]
fn the_optimise_settings_table_holds_the_engines_names_and_defaults() {
    // Every field, by name: a field added to `OptimiseSettings` does not compile here until it is
    // added to this list and to the table of appendix B.
    let OptimiseSettings {
        site_step_m,
        candidate_sites_m,
        lifts_m,
        terminal_min_level_m,
        min_spacing_m,
        base_level_m,
        k_best,
        wet_weight,
        dry_weight,
        length_weight,
        land_weight,
        flood_cost_per_ha,
        max_above_bank_m,
        flood_distance_m,
        flood_cell_m,
        flood_report,
        building_cost,
        building_min_depth_m,
        structure_fixed,
        structure_per_lift,
        structure_power,
        cut_model,
        station_m,
        maxw_m,
        half_width_m,
        side_slope,
        rake_half_m,
        rake_step_m,
        ds_step_m,
        pitch_m,
        n_heading,
        radius_multiples,
        corridor_half_m,
        corridor_limit,
        seams_on_straights,
        seam_window_m,
        free_seams,
        seam_offset_m,
        seam_heading_tol_deg,
        weir_straight_m,
        min_straight_m,
        cell_m,
        check_chord_m,
        // The window optimiser's (OPT-5): its own docs page holds them, not yet appendix B. Left
        // out here only so that the merge of the two lanes compiles; the lanes' merge decides.
        method: _,
        windows: _,
    } = OptimiseSettings::default();
    let expected = [
        ("site_step_m", json!(site_step_m)),
        ("candidate_sites_m", json!(candidate_sites_m)),
        ("lifts_m", json!(lifts_m)),
        ("terminal_min_level_m", json!(terminal_min_level_m)),
        ("min_spacing_m", json!(min_spacing_m)),
        ("base_level_m", json!(base_level_m)),
        ("k_best", json!(k_best)),
        ("wet_weight", json!(wet_weight)),
        ("dry_weight", json!(dry_weight)),
        ("length_weight", json!(length_weight)),
        ("land_weight", json!(land_weight)),
        ("flood_cost_per_ha", json!(flood_cost_per_ha)),
        ("max_above_bank_m", json!(max_above_bank_m)),
        ("flood_distance_m", json!(flood_distance_m)),
        ("flood_cell_m", json!(flood_cell_m)),
        ("flood_report", json!(flood_report)),
        ("building_cost", json!(building_cost)),
        ("building_min_depth_m", json!(building_min_depth_m)),
        ("structure_fixed", json!(structure_fixed)),
        ("structure_per_lift", json!(structure_per_lift)),
        ("structure_power", json!(structure_power)),
        ("cut_model", json!(cut_model)),
        ("station_m", json!(station_m)),
        ("maxw_m", json!(maxw_m)),
        ("half_width_m", json!(half_width_m)),
        ("side_slope", json!(side_slope)),
        ("rake_half_m", json!(rake_half_m)),
        ("rake_step_m", json!(rake_step_m)),
        ("ds_step_m", json!(ds_step_m)),
        ("pitch_m", json!(pitch_m)),
        ("n_heading", json!(n_heading)),
        ("radius_multiples", json!(radius_multiples)),
        ("corridor_half_m", json!(corridor_half_m)),
        ("corridor_limit", json!(corridor_limit)),
        ("seams_on_straights", json!(seams_on_straights)),
        ("seam_window_m", json!(seam_window_m)),
        ("free_seams", json!(free_seams)),
        ("seam_offset_m", json!(seam_offset_m)),
        ("seam_heading_tol_deg", json!(seam_heading_tol_deg)),
        ("weir_straight_m", json!(weir_straight_m)),
        ("min_straight_m", json!(min_straight_m)),
        ("cell_m", json!(cell_m)),
        ("check_chord_m", json!(check_chord_m)),
    ];
    hold(
        &table(&appendices(), "optimise"),
        &expected,
        0,
        3,
        "settings of the optimise stage",
    );
}

/// A flag the table names is a flag of the command: the help of `owd optimise` and of
/// `owd volumes` holds it.
#[test]
fn every_flag_in_the_settings_tables_is_a_flag_of_the_command() {
    let help = |cmd: &str| {
        let o = Command::new(env!("CARGO_BIN_EXE_owd"))
            .args([cmd, "--help"])
            .output()
            .expect("owd runs");
        String::from_utf8_lossy(&o.stdout).into_owned()
    };
    let text = appendices();
    for (name, cmd) in [("optimise", "optimise"), ("volumes", "volumes")] {
        let h = help(cmd);
        for row in table(&text, name) {
            let flag = &row[1];
            if flag == "none" {
                continue;
            }
            let found = h
                .lines()
                .any(|l| l.trim_start().split([' ', '=']).next() == Some(flag.as_str()));
            assert!(
                found,
                "`owd {cmd}` has no flag `{flag}` (setting `{}`)",
                row[0]
            );
        }
    }
}

#[test]
fn the_volumes_settings_table_holds_the_engines_names_and_defaults() {
    let VolumesSettings {
        cut_model,
        station_m,
        maxw_m,
        dx_m,
        wet_slope,
        half_width_m,
    } = VolumesSettings::default();
    let expected = [
        ("cut_model", json!(cut_model)),
        ("station_m", json!(station_m)),
        ("maxw_m", json!(maxw_m)),
        ("dx_m", json!(dx_m)),
        ("wet_slope", json!(wet_slope)),
        ("half_width_m", json!(half_width_m)),
    ];
    hold(
        &table(&appendices(), "volumes"),
        &expected,
        0,
        3,
        "settings of the volumes stage",
    );
}

#[test]
fn the_table_of_fixed_values_holds_the_engines() {
    let r_min = 255.0;
    // Every field, by name: a field added to the lattice's configuration does not compile here
    // until someone decides whether the document must say it.
    let LatticeConfig {
        r_min_m: _,
        // These are settings of the stage: the optimise table holds them.
        radius_multiples: _,
        pitch_m,
        n_heading: _,
        corridor_limit: _,
        min_straight_m: _,
        seams_on_straights: _,
        seam_window_m: _,
        free_seams: _,
        seam_offset_m: _,
        seam_heading_tol_deg: _,
        weir_straight_m: _,
        // These are fixed by the stage: the table of fixed values holds them.
        straight_len_m,
        arc_step_m,
        goal_pos_tol_m,
        goal_head_tol_bins,
        close_goal,
        close_reach_m,
        heuristic,
        heuristic_weight,
        prune_bound,
        max_expand,
        bounds_margin_m,
        max_pool_piece_m,
        // The window optimiser's (OPT-5), off by default: not yet in the table. Left out here only
        // so that the merge of the two lanes compiles.
        radii_m: _,
        nominal_arc_m: _,
        max_states: _,
    } = LatticeConfig::new(r_min).validated().unwrap();
    // Stated in the text of section 5, not in the table: the search always closes on the end
    // pose, and it drops no state by a bound.
    assert!(
        close_goal,
        "section 5 says every pool's search ends through a closing path"
    );
    assert!(
        prune_bound.is_infinite(),
        "section 5 names no pruning bound"
    );
    let spec = SectionSpec::new(HalfWidthSpec::Fixed(15.0));
    let expected = [
        ("straight_len_m", json!(straight_len_m.unwrap() / pitch_m)),
        ("arc_step_m", json!(arc_step_m)),
        ("close_reach_m", json!(close_reach_m.unwrap() / r_min)),
        ("goal_pos_tol_m", json!(goal_pos_tol_m.unwrap() / pitch_m)),
        ("goal_head_tol_bins", json!(goal_head_tol_bins)),
        ("CLOSE_GRACE_EXPANSIONS", json!(CLOSE_GRACE_EXPANSIONS)),
        ("heuristic", json!(heuristic)),
        ("heuristic_weight", json!(heuristic_weight)),
        ("max_expand", json!(max_expand)),
        ("max_pool_piece_m", json!(max_pool_piece_m)),
        ("PIECE_BOUNDARY_SLACK", json!(PIECE_BOUNDARY_SLACK)),
        ("bounds_margin_m", json!(bounds_margin_m)),
        ("min_valid_fraction", json!(spec.min_valid_fraction)),
        ("DEFAULT_C_MIN_STRIDE_M", json!(DEFAULT_C_MIN_STRIDE_M)),
        (
            "DEFAULT_C_MIN_MAX_SAMPLES",
            json!(DEFAULT_C_MIN_MAX_SAMPLES),
        ),
        ("GATE_MIDDLE_BIAS", json!(GATE_MIDDLE_BIAS)),
        ("BOUNDARY_STEP_FRACTION", json!(BOUNDARY_STEP_FRACTION)),
        ("BOUNDARY_STEPS", json!(BOUNDARY_STEPS)),
        ("LINE_ARC_STEP_M", json!(LINE_ARC_STEP_M)),
        ("RADIUS_TOL", json!(RADIUS_TOL)),
        ("BERM_M", json!(BERM_M)),
        ("ABOVE_SLOPE_H_PER_V", json!(ABOVE_SLOPE_H_PER_V)),
        ("PRISM_INTRUDE_M", json!(PRISM_INTRUDE_M)),
        ("BANK_WINDOW_M", json!(BANK_WINDOW_M)),
        ("BANK_PERCENTILE", json!(BANK_PERCENTILE)),
        ("BANK_MIN_CELLS", json!(BANK_MIN_CELLS)),
    ];
    hold(
        &table(&appendices(), "fixed"),
        &expected,
        1,
        3,
        "values fixed in the engine",
    );
}

// ----- the numbers ---------------------------------------------------------------------------

/// The number written just before byte `at` of `text`, and how many decimals it shows.
fn number_before(text: &str, at: usize) -> Option<(f64, usize)> {
    let head = &text[..at];
    let start = head
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_ascii_digit() || matches!(c, '.' | ',' | '-' | '\u{2212}'))
        .last()?
        .0;
    let shown = head[start..].replace(',', "").replace('\u{2212}', "-");
    let decimals = shown.split_once('.').map_or(0, |(_, d)| d.len());
    shown.parse().ok().map(|v| (v, decimals))
}

/// Every `<!--=key-->` of a file, with the number shown before it.
fn tagged(text: &str, file: &str) -> Vec<(String, f64, usize)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = text[from..].find("<!--=") {
        let at = from + i;
        let end = at + text[at..].find("-->").expect("a tag is closed");
        let key = text[at + 5..end].to_string();
        let (v, d) = number_before(text, at)
            .unwrap_or_else(|| panic!("{file}: no number stands before the tag `{key}`"));
        out.push((key, v, d));
        from = end;
    }
    out
}

#[test]
fn every_tagged_number_is_the_examples_at_the_precision_shown() {
    let numbers = numbers();
    let mut wrong = Vec::new();
    let mut count = 0;
    for (file, text) in [("README.md", main_doc()), ("appendices.md", appendices())] {
        for (key, shown, decimals) in tagged(&text, file) {
            count += 1;
            let Some(value) = numbers.get(&key).and_then(|e| e["value"].as_f64()) else {
                wrong.push(format!(
                    "{file}: `{key}` is not a number of example/numbers.json"
                ));
                continue;
            };
            // The number shown is the stored number rounded to the decimals shown.
            let half = 0.5 * 10f64.powi(-(decimals as i32));
            if (shown - value).abs() > half * (1.0 + 1e-9) + 1e-12 {
                wrong.push(format!(
                    "{file}: `{key}` is shown as {shown}; the example gives {value}"
                ));
            }
        }
    }
    assert!(
        count >= 150,
        "only {count} tagged numbers: the tags were lost"
    );
    assert!(
        wrong.is_empty(),
        "docs/optimisation: numbers that are not the example's (run \
         docs/optimisation/example/run.py and update the text):\n{}",
        wrong.join("\n")
    );
}

/// A block of printed output, marked `<!-- file: example/runs/NAME.txt -->`, is in that file: its
/// lines, in order (a block may leave lines out between the ones it shows).
#[test]
fn every_block_of_printed_output_is_in_the_stored_output_of_its_run() {
    let text = main_doc();
    let mut blocks = 0;
    let mut rest = text.as_str();
    while let Some((_, after)) = rest.split_once("<!-- file: ") {
        let (name, after) = after.split_once(" -->").expect("the marker is closed");
        let (_, after) = after
            .split_once("```text\n")
            .expect("a text block follows the marker");
        let (block, after) = after.split_once("```").expect("the block is closed");
        let stored = read(&doc_dir().join(name));
        let mut lines = stored.lines();
        for want in block.lines() {
            assert!(
                lines.any(|l| l == want),
                "docs/optimisation/README.md shows this line as the output of {name}, and the \
                 stored output does not hold it (in this order):\n{want}"
            );
        }
        blocks += 1;
        rest = after;
    }
    assert!(
        blocks >= 4,
        "only {blocks} blocks of printed output are marked"
    );
}

// ----- the anchors ---------------------------------------------------------------------------

/// The text of `text` from the heading that starts with `from` to the one that starts with `to`.
fn between<'a>(text: &'a str, from: &str, to: &str) -> &'a str {
    let a = text
        .find(from)
        .unwrap_or_else(|| panic!("no heading `{from}`"));
    let b = text[a..]
        .find(to)
        .unwrap_or_else(|| panic!("no heading `{to}`"))
        + a;
    &text[a..b]
}

#[test]
fn every_guarantee_and_every_doubt_names_a_test_that_exists() {
    let text = main_doc();
    let mut wrong = Vec::new();
    // Section 6: every row of its tables. Section 7: every numbered item.
    let six = between(&text, "\n## 6 ", "\n## 7 ");
    let rows = six
        .lines()
        .filter(|l| l.starts_with("| ") && !l.starts_with("| What ") && !l.starts_with("| Check "));
    let seven = between(&text, "\n## 7 ", "\n## 8 ");
    let items = seven
        .lines()
        .filter(|l| l.starts_with("**") && l[2..].starts_with(|c: char| c.is_ascii_digit()));
    let mut claims = 0;
    for line in rows.chain(items) {
        claims += 1;
        if !line.contains("<!--test: ") {
            wrong.push(format!("no anchor: {}", &line[..line.len().min(90)]));
        }
    }
    assert!(
        claims >= 25,
        "only {claims} guarantees and doubts were found"
    );
    let mut anchors = 0;
    let mut rest = text.as_str();
    while let Some((_, after)) = rest.split_once("<!--test: ") {
        let (anchor, after) = after.split_once("-->").expect("an anchor is closed");
        rest = after;
        anchors += 1;
        let Some((file, name)) = anchor.split_once("::") else {
            wrong.push(format!("`{anchor}` is not `file::test`"));
            continue;
        };
        let path = repo().join(file);
        let Ok(source) = std::fs::read_to_string(&path) else {
            wrong.push(format!("`{anchor}`: there is no file {file}"));
            continue;
        };
        if !source.contains(&format!("fn {name}(")) && !source.contains(&format!("def {name}(")) {
            wrong.push(format!("`{anchor}`: {file} holds no test `{name}`"));
        }
    }
    assert!(anchors >= claims, "{anchors} anchors for {claims} claims");
    assert!(
        wrong.is_empty(),
        "docs/optimisation/README.md: a guarantee or a doubt is no longer pinned by a test. Name \
         the test that pins it now, or correct the statement:\n{}",
        wrong.join("\n")
    );
}

// ----- the links -----------------------------------------------------------------------------

/// The anchor GitHub and the website give a heading: lower case, letters, digits, `-` and `_`
/// kept, spaces to `-`, the rest dropped.
fn slug(heading: &str) -> String {
    heading
        .trim()
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
            _ => None,
        })
        .collect()
}

fn anchors_of(text: &str) -> Vec<String> {
    let mut out: Vec<String> = text
        .lines()
        .filter(|l| l.starts_with('#'))
        .map(|l| slug(l.trim_start_matches('#')))
        .collect();
    let mut rest = text;
    while let Some((_, after)) = rest.split_once("<a id=\"") {
        let (id, after) = after.split_once('"').expect("an id is closed");
        out.push(id.to_string());
        rest = after;
    }
    out
}

#[test]
fn the_links_of_the_document_resolve() {
    let dir = doc_dir();
    let mut wrong = Vec::new();
    for file in ["README.md", "appendices.md"] {
        let text = read(&dir.join(file));
        let mut rest = text.as_str();
        while let Some((_, after)) = rest.split_once("](") {
            let (target, after) = after.split_once(')').expect("a link is closed");
            rest = after;
            if target.contains("://") {
                continue;
            }
            let (path, anchor) = target.split_once('#').unwrap_or((target, ""));
            let to = if path.is_empty() {
                dir.join(file)
            } else {
                dir.join(path)
            };
            if !to.exists() {
                wrong.push(format!("{file}: `{target}`: no such file"));
                continue;
            }
            if !anchor.is_empty() && !anchors_of(&read(&to)).iter().any(|a| a == anchor) {
                wrong.push(format!("{file}: `{target}`: no such heading"));
            }
        }
    }
    // The README's section links the document.
    let readme = read(&repo().join("README.md"));
    assert!(
        readme.contains("](docs/optimisation/README.md)"),
        "the README's optimise section links docs/optimisation/README.md"
    );
    assert!(
        wrong.is_empty(),
        "docs/optimisation: broken links:\n{}",
        wrong.join("\n")
    );
}

/// The status table names every part as in a release, in the development branch, or planned; and
/// section 8 opens with its notice.
#[test]
fn the_status_table_and_the_notice_of_the_planned_method_are_there() {
    let text = main_doc();
    let status = between(&text, "\n## Status", "\n## 1 ");
    let rows: Vec<&str> = status
        .lines()
        .filter(|l| l.starts_with("| ") && !l.starts_with("| Part "))
        .collect();
    assert!(rows.len() >= 15, "the status table has {} rows", rows.len());
    for r in &rows {
        assert!(
            r.contains("In release ")
                || r.contains("In the development branch")
                || r.contains("Planned, not built"),
            "a row of the status table has no status: {r}"
        );
    }
    let eight = between(&text, "\n## 8 ", "\n## 9 ");
    assert!(
        eight.contains("NOT IN ANY RELEASE. NOT BUILT."),
        "section 8 must open with its notice: the planned method is not built"
    );
}

// ----- the worked example, run again (slow) --------------------------------------------------

/// Fill the place-holders of a command of `runs.json`.
fn fill(args: &Value, doc: &Path, dem: &Path) -> Vec<String> {
    let inputs = doc_dir().join("example/inputs");
    args.as_array()
        .expect("a command is a list")
        .iter()
        .map(|a| {
            a.as_str()
                .expect("an argument is text")
                .replace("{doc}", doc.to_str().unwrap())
                .replace("{inputs}", inputs.to_str().unwrap())
                .replace("{dem}", dem.to_str().unwrap())
        })
        .collect()
}

fn owd(args: &[String]) {
    let o = Command::new(env!("CARGO_BIN_EXE_owd"))
        .args(args)
        .output()
        .expect("owd runs");
    // Exit 1 is a failed check on a saved design (the example's rolling circle fails one).
    let code = o.status.code().expect("exited normally");
    assert!(
        code == 0 || code == 1,
        "owd {args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
}

/// Run one run of the example again, by the commands of `runs.json`, and hold every number of
/// `numbers.json` that was read from it: numbers to 1 part in a million, the rest exactly.
fn rerun(name: &str) {
    let spec = runs();
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("owd-optimisation-doc-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let dem = dir.join("dem.tif");
    std::fs::copy(doc_dir().join("example/inputs/dem.tif"), &dem).unwrap();
    let doc = dir.join(format!("{name}.owd"));
    for step in spec["setup"].as_array().unwrap() {
        owd(&fill(step, &doc, &dem));
    }
    // The settings with no flag, as `set_setting.py` writes them.
    let mut design: Value = serde_json::from_str(&read(&doc)).unwrap();
    design["stages"]["optimise"]["settings"] = spec["settings"].clone();
    std::fs::write(&doc, design.to_string()).unwrap();
    let mut optimise = fill(&spec["optimise"], &doc, &dem);
    let extra = spec["runs"]
        .get(name)
        .unwrap_or_else(|| panic!("runs.json has no run `{name}`"));
    optimise.extend(fill(extra, &doc, &dem));
    owd(&optimise);
    owd(&fill(&spec["volumes"], &doc, &dem));

    let design: Value = serde_json::from_str(&read(&doc)).unwrap();
    let mut wrong = Vec::new();
    let mut held = 0;
    for (key, entry) in numbers() {
        if entry["run"].as_str() != Some(name) {
            continue;
        }
        held += 1;
        let pointer = entry["pointer"]
            .as_str()
            .expect("a number of a run has a pointer");
        let now = design.pointer(pointer).cloned().unwrap_or(Value::Null);
        let stored = &entry["value"];
        let ok = match (now.as_f64(), stored.as_f64()) {
            (Some(a), Some(b)) => (a - b).abs() <= 1e-6 * b.abs().max(1e-3),
            _ => now == *stored,
        };
        if !ok {
            wrong.push(format!(
                "{key} ({pointer}): the example stored {stored}, this build gives {now}"
            ));
        }
    }
    assert!(
        held > 20,
        "numbers.json holds only {held} numbers of the run `{name}`"
    );
    assert!(
        wrong.is_empty(),
        "the run `{name}` of the worked example no longer gives the document's numbers. If the \
         change is meant, run docs/optimisation/example/run.py and update the document:\n{}",
        wrong.join("\n")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_base_run_gives_the_documents_numbers() {
    rerun("base");
}

#[test]
fn a_cost_per_metre_of_300_gives_the_documents_numbers() {
    rerun("length-300");
}

#[test]
fn a_cost_per_metre_of_1000_gives_the_documents_numbers() {
    rerun("length-1000");
}

#[test]
fn a_land_cost_gives_the_documents_numbers() {
    rerun("land");
}

#[test]
fn a_flood_cost_of_3000_gives_the_documents_numbers() {
    rerun("flood-3000");
}

#[test]
fn a_flood_cost_of_20000_gives_the_documents_numbers() {
    rerun("flood-20000");
}

#[test]
fn a_bank_limit_gives_the_documents_numbers() {
    rerun("bank-0");
}

#[test]
fn free_seams_give_the_documents_numbers() {
    rerun("free-seams");
}

#[test]
fn free_seams_with_no_weir_straight_gives_the_documents_numbers() {
    rerun("free-seams-0");
}

/// Every run of `runs.json` has its re-run above, and its stored output.
#[test]
fn every_run_of_the_example_is_run_again_by_a_test() {
    let source = read(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/optimisation_doc.rs"));
    for name in runs()["runs"].as_object().unwrap().keys() {
        assert!(
            source.contains(&format!("rerun(\"{name}\");")),
            "the run `{name}` of runs.json has no re-run in this file"
        );
        assert!(
            doc_dir().join(format!("example/runs/{name}.txt")).exists(),
            "the run `{name}` has no stored output"
        );
    }
}
