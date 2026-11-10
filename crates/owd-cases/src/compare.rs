//! Read each expected value from the two documents, judge it, and compare the documents with each
//! other (the one-path check).

use crate::case::{Case, Class, Expect, Measure, Rule, Tolerance};
use serde_json::{Map, Value, json};

/// Two numbers in the two documents are the same when they differ by at most this much, or by at
/// most this fraction of the larger.
const ONE_PATH_TOLERANCE: f64 = 1e-9;

/// The most differences the report lists per case.
const MAX_DIFFERENCES: usize = 50;

pub struct Row {
    pub name: String,
    pub class: &'static str,
    pub expected: Option<f64>,
    pub cli: Option<f64>,
    pub shell: Option<f64>,
    pub verdict: &'static str,
}

pub struct CaseResult {
    pub id: String,
    pub rows: Vec<Row>,
    pub one_path_equal: bool,
    pub differences: Vec<String>,
    /// A surface that could not play the steps, and why.
    pub errors: Vec<String>,
}

/// The number `e` reads from `doc`, if the pointer is there and holds what the measure needs.
fn read(doc: Option<&Value>, e: &Expect) -> Option<f64> {
    let v = doc?.pointer(&e.pointer)?;
    match e.measure {
        Measure::Value => v.as_f64(),
        Measure::Count => v.as_array().map(|a| a.len() as f64),
        Measure::LengthM => {
            let coords = if v.is_object() { &v["coordinates"] } else { v };
            let pts: Vec<(f64, f64)> = coords
                .as_array()?
                .iter()
                .map(|p| Some((p.get(0)?.as_f64()?, p.get(1)?.as_f64()?)))
                .collect::<Option<_>>()?;
            Some(
                pts.windows(2)
                    .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
                    .sum(),
            )
        }
    }
}

fn within(got: f64, expected: f64, t: Tolerance) -> bool {
    let band = match t {
        Tolerance::Absolute(a) => a,
        Tolerance::Relative(r) => r * expected.abs(),
    };
    (got - expected).abs() <= band
}

/// One surface's value against the row: inside its tolerance, or meeting its rule.
fn passes(got: Option<f64>, e: &Expect) -> bool {
    let Some(got) = got else { return false };
    match e.class {
        Class::Exact(t) | Class::SameRecipe(t) | Class::Outcome(t) => {
            within(got, e.value.expect("checked when read"), t)
        }
        Class::KnownDefect(Rule::AtLeast(x)) => got >= x,
        Class::KnownDefect(Rule::AtMost(x)) => got <= x,
        Class::NotYetPossible => true,
    }
}

pub fn run_case(
    case: &Case,
    cli: Result<Value, String>,
    shell: Result<Value, String>,
) -> CaseResult {
    let mut errors = Vec::new();
    if let Err(e) = &cli {
        errors.push(format!("command line: {e}"));
    }
    if let Err(e) = &shell {
        errors.push(format!("shell: {e}"));
    }
    let (cli, shell) = (cli.ok(), shell.ok());
    let rows = case
        .expect
        .iter()
        .map(|e| {
            let (c, s) = (read(cli.as_ref(), e), read(shell.as_ref(), e));
            // A value passes only when both surfaces give it.
            let verdict = match e.class {
                Class::NotYetPossible => "not_yet_possible",
                _ if passes(c, e) && passes(s, e) => "pass",
                _ => "fail",
            };
            Row {
                name: e.name.clone(),
                class: e.class.name(),
                expected: e.value,
                cli: c,
                shell: s,
                verdict,
            }
        })
        .collect();
    let (one_path_equal, differences) = match (&cli, &shell) {
        (Some(a), Some(b)) => {
            let mut diffs = Vec::new();
            differ(&comparable(a), &comparable(b), "", &mut diffs);
            (diffs.is_empty(), diffs)
        }
        _ => (false, errors.clone()),
    };
    CaseResult {
        id: case.id.clone(),
        rows,
        one_path_equal,
        differences,
        errors,
    }
}

/// The document as the one-path check compares it.
fn comparable(doc: &Value) -> Value {
    let mut doc = doc.clone();
    // UNTIL-A2: the shell writes provenance records for imported inputs that the command line's
    // import does not. Until the one-import ticket (A2) makes the two imports one, the top-level
    // `provenance` member is left out of the comparison. Its follow-up removes these lines.
    if let Some(o) = doc.as_object_mut() {
        o.remove("provenance");
    }
    doc
}

fn same_number(a: f64, b: f64) -> bool {
    let d = (a - b).abs();
    d <= ONE_PATH_TOLERANCE || d <= ONE_PATH_TOLERANCE * a.abs().max(b.abs())
}

/// Every JSON pointer where `a` and `b` differ, numbers compared by [`ONE_PATH_TOLERANCE`].
fn differ(a: &Value, b: &Value, at: &str, out: &mut Vec<String>) {
    if out.len() >= MAX_DIFFERENCES {
        return;
    }
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for (k, va) in x {
                let p = format!("{at}/{}", k.replace('~', "~0").replace('/', "~1"));
                match y.get(k) {
                    Some(vb) => differ(va, vb, &p, out),
                    None => out.push(format!("{p}: only in the command line's document")),
                }
            }
            for k in y.keys().filter(|k| !x.contains_key(*k)) {
                out.push(format!(
                    "{at}/{}: only in the shell's document",
                    k.replace('~', "~0").replace('/', "~1")
                ));
            }
        }
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                out.push(format!("{at}: {} items against {}", x.len(), y.len()));
                return;
            }
            for (i, (va, vb)) in x.iter().zip(y).enumerate() {
                differ(va, vb, &format!("{at}/{i}"), out);
            }
        }
        (Value::Number(x), Value::Number(y)) => match (x.as_f64(), y.as_f64()) {
            (Some(p), Some(q)) if same_number(p, q) => {}
            _ => out.push(format!("{at}: {x} against {y}")),
        },
        _ if a == b => {}
        _ => out.push(format!("{at}: {a} against {b}")),
    }
}

fn num(x: Option<f64>) -> Value {
    x.map_or(Value::Null, |v| json!(v))
}

pub fn report(results: &[CaseResult]) -> Value {
    let (mut pass, mut fail, mut nyp, mut one_path_failures) = (0u64, 0u64, 0u64, 0u64);
    let cases: Vec<Value> = results
        .iter()
        .map(|r| {
            let values: Vec<Value> = r
                .rows
                .iter()
                .map(|row| {
                    match row.verdict {
                        "pass" => pass += 1,
                        "fail" => fail += 1,
                        _ => nyp += 1,
                    }
                    let gap = |got: Option<f64>| match (got, row.expected) {
                        (Some(g), Some(e)) => json!(g - e),
                        _ => Value::Null,
                    };
                    json!({
                        "name": row.name,
                        "class": row.class,
                        "expected": num(row.expected),
                        "cli": num(row.cli),
                        "shell": num(row.shell),
                        "gap": {"cli": gap(row.cli), "shell": gap(row.shell)},
                        "verdict": row.verdict,
                    })
                })
                .collect();
            if !r.one_path_equal {
                one_path_failures += 1;
            }
            let mut case = Map::new();
            case.insert("id".into(), json!(r.id));
            case.insert("values".into(), json!(values));
            case.insert(
                "one_path".into(),
                json!({"equal": r.one_path_equal, "differences": r.differences}),
            );
            if !r.errors.is_empty() {
                case.insert("errors".into(), json!(r.errors));
            }
            Value::Object(case)
        })
        .collect();
    json!({
        "cases": cases,
        "totals": {
            "pass": pass,
            "fail": fail,
            "not_yet_possible": nyp,
            "one_path_failures": one_path_failures,
        },
    })
}

/// The short table printed on stdout.
pub fn table(results: &[CaseResult]) -> String {
    let show = |x: Option<f64>| x.map_or("-".to_string(), |v| format!("{v:.3}"));
    let mut out = String::new();
    for r in results {
        out.push_str(&format!("case {}\n", r.id));
        out.push_str(&format!(
            "  {:<28} {:<16} {:>14} {:>14} {:>14}  {}\n",
            "value", "class", "expected", "cli", "shell", "verdict"
        ));
        for row in &r.rows {
            out.push_str(&format!(
                "  {:<28} {:<16} {:>14} {:>14} {:>14}  {}\n",
                row.name,
                row.class,
                show(row.expected),
                show(row.cli),
                show(row.shell),
                row.verdict
            ));
        }
        for e in &r.errors {
            out.push_str(&format!("  error: {e}\n"));
        }
        out.push_str(&format!(
            "  one path: {}\n",
            if r.one_path_equal {
                "the two documents are equal".to_string()
            } else {
                format!("{} difference(s)", r.differences.len())
            }
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;

    fn diffs(a: Value, b: Value) -> Vec<String> {
        let mut out = Vec::new();
        differ(&comparable(&a), &comparable(&b), "", &mut out);
        out
    }

    #[test]
    fn the_one_path_check_finds_a_real_difference_and_names_its_pointer() {
        let a = json!({"options": [{"line": {"coordinates": [[500000.0, 600000.0]]}}]});
        let b = json!({"options": [{"line": {"coordinates": [[500000.5, 600000.0]]}}]});
        let found = diffs(a, b);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].starts_with("/options/0/line/coordinates/0/0:"),
            "{found:?}"
        );
        assert_eq!(
            diffs(
                json!({"a": 1, "b": [1, 2]}),
                json!({"a": 1, "b": [1], "c": "x"})
            )
            .len(),
            2
        );
    }

    #[test]
    fn the_one_path_check_ignores_last_bit_noise_and_until_a2_the_provenance() {
        assert!(diffs(json!({"x": 600000.0}), json!({"x": 600000.0 + 1e-7})).is_empty());
        assert!(!diffs(json!({"x": 600000.0}), json!({"x": 600000.01})).is_empty());
        assert!(diffs(json!({"x": 1, "provenance": {"a": 1}}), json!({"x": 1})).is_empty());
    }
}
