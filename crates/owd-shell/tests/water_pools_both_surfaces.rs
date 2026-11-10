//! A table of pools gives the same water level through the command line and through the app shell.
//!
//! Both start from the web example. The command line reads the table from a file
//! (`owd import --water-pools FILE`); the shell takes the same text.
//!
//! Label: property

use owd_cli::commands::{self, WaterFlags};
use owd_shell::state::AppState;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const TABLE: &str = "chainage_m,level_m\n0,2\n1000,1\n2000,0\n";

fn example() -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/web/river-example.owd"),
    )
    .unwrap()
}

/// A fresh folder under the system's temporary folder, removed at the end of the test.
struct Dir(PathBuf);
impl Dir {
    fn new(tag: &str) -> Dir {
        let p = std::env::temp_dir().join(format!("owd-water-pools-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Dir(p)
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn read(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// `owd import DOC` with water flags only.
fn cli_import(doc: &Path, water: WaterFlags) -> commands::Result {
    commands::import(
        &doc.to_string_lossy(),
        [
            ("left_bank", None),
            ("right_bank", None),
            ("centreline", None),
            ("river_polygon", None),
        ],
        None,
        water,
        &None,
        true,
    )
}

#[test]
fn a_pool_table_gives_equal_documents_through_the_shell_and_the_command_line() {
    let dir = Dir::new("equal");
    let doc = dir.0.join("d.owd");
    let table = dir.0.join("pools.csv");
    std::fs::write(&doc, example()).unwrap();
    std::fs::write(&table, TABLE).unwrap();

    let done = cli_import(
        &doc,
        WaterFlags {
            pools: Some(table.to_string_lossy().into_owned()),
            ..WaterFlags::default()
        },
    );
    if let Err(f) = done {
        panic!("`owd import --water-pools` failed: {}", f.message);
    }
    let from_cli = read(&doc);
    assert_eq!(
        from_cli["inputs"]["water_level"],
        json!({"points": [[0.0, 2.0], [1000.0, 2.0], [1000.0, 1.0], [2000.0, 1.0], [2000.0, 0.0]]})
    );

    let mut shell = AppState::open(&doc, &example(), None).unwrap();
    shell.set_water_pools(TABLE).unwrap();
    assert!(shell.is_dirty());
    let from_shell: Value = serde_json::from_str(&shell.save_text_for(&doc)).unwrap();
    assert_eq!(from_shell, from_cli);
}

#[test]
fn a_bad_pool_table_is_refused_on_both_surfaces_and_nothing_changes() {
    let dir = Dir::new("refused");
    let doc = dir.0.join("d.owd");
    let table = dir.0.join("pools.csv");
    std::fs::write(&doc, example()).unwrap();
    // Two rows at one chainage: a pool with no length.
    let bad = "0,2\n1000,1\n1000,0\n";
    std::fs::write(&table, bad).unwrap();
    let before = read(&doc);

    let pools = || Some(table.to_string_lossy().into_owned());
    let refused = cli_import(
        &doc,
        WaterFlags {
            pools: pools(),
            ..WaterFlags::default()
        },
    )
    .expect_err("the command line takes a pool with no length");
    assert!(refused.message.contains("chainage"), "{}", refused.message);
    assert_eq!(read(&doc), before);

    // A pool table and a level are two water levels: one must go.
    std::fs::write(&table, TABLE).unwrap();
    let both = cli_import(
        &doc,
        WaterFlags {
            pools: pools(),
            level: Some(3.0),
            ..WaterFlags::default()
        },
    )
    .expect_err("the command line takes a pool table and a level together");
    assert!(both.message.contains("not both"), "{}", both.message);
    assert_eq!(read(&doc), before);

    let mut shell = AppState::open(&doc, &example(), None).unwrap();
    // Opening brings an older document up to date, so compare with the shell's own text.
    let opened = shell.save_text_for(&doc);
    let e = shell
        .set_water_pools(bad)
        .expect_err("the shell takes a pool with no length");
    assert!(e.contains("chainage"), "{e}");
    assert!(!shell.is_dirty());
    assert_eq!(shell.save_text_for(&doc), opened);
}
