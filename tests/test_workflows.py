"""The CI workflows: the required checks always run, the filtered jobs never block a merge, and every
scheduled workflow reports its failures.

Label: property. No pinned numbers: the tests read .github/workflows.
"""
import re
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[1]
WF = ROOT / ".github" / "workflows"
#: Branch protection requires these check names on main. Renaming one needs a settings change.
REQUIRED = ("test", "release-guard", "web-app")
#: Jobs in ci.yml that run on a pull request only when their inputs change.
FILTERED = {"web-app": "web"}


def load(name):
    doc = yaml.safe_load((WF / name).read_text(encoding="utf-8"))
    doc["on"] = doc.pop(True, doc.get("on"))  # YAML 1.1 reads the key `on` as true
    return doc


def test_the_required_jobs_exist_and_always_run_on_every_pull_request():
    ci = load("ci.yml")
    assert "paths" not in (ci["on"]["pull_request"] or {}), "a path filter on ci.yml could skip a required check"
    for name in REQUIRED:
        job = ci["jobs"][name]
        if name in FILTERED:  # a skipped job does not block a merge: web-app skips itself on other PRs
            continue
        assert job.get("if", "always()") == "always()", f"{name} must not be conditional"
        needs = job.get("needs", [])
        assert "changes" not in (needs if isinstance(needs, list) else [needs])
    for name, job in ci["jobs"].items():
        if name.startswith("gate-"):
            assert "if" not in job and "needs" not in job, f"{name} is part of `test`: it must always run"


def test_the_test_check_needs_every_part_of_the_gate_and_fails_unless_all_passed():
    jobs = load("ci.yml")["jobs"]
    parts = sorted(n for n in jobs if n.startswith("gate-"))
    test = jobs["test"]
    assert sorted(test["needs"]) == parts and test["if"] == "always()"
    script = test["steps"][-1]["run"]
    assert f"keys == {str(parts).replace(chr(39), chr(34))}" in script and '.result == "success"' in script


def test_the_parts_run_every_step_of_the_gate_and_every_rust_partition():
    jobs = load("ci.yml")["jobs"]
    steps, partitions = [], []
    # The pull-request gate (ci.yml) and the nightly (nightly.yml) together run every step once.
    for doc in (jobs, load("nightly.yml")["jobs"]):
        for name, job in doc.items():
            if not name.startswith("gate-"):
                continue
            env = job["steps"][-1]["env"]
            steps += env["GATES_STEPS"].split(",")
            if "GATES_PARTITION" in env:
                partitions = job["strategy"]["matrix"]["partition"]
    gates = (ROOT / "tools" / "gates.sh").read_text(encoding="utf-8")
    all_steps = re.search(r'ALL_STEPS="([^"]+)"', gates).group(1).split(",")
    assert sorted(steps) == sorted(all_steps), "each gate step runs in exactly one part"
    n = len(partitions)
    assert n >= 2 and partitions == [f"{k}/{n}" for k in range(1, n + 1)]


def test_a_filtered_job_runs_on_main_and_on_a_pull_request_that_changes_its_inputs():
    ci = load("ci.yml")
    outputs = ci["jobs"]["changes"]["outputs"]
    for name, group in FILTERED.items():
        job = ci["jobs"][name]
        assert job["needs"] == "changes"
        assert job["if"] == f"github.event_name != 'pull_request' || needs.changes.outputs.{group} == 'true'"
        assert group in outputs
    # web-app runs on any change under app/ (the front end it builds).
    flt = ci["jobs"]["changes"]["steps"][-1]["with"]["filters"]
    assert '"app/**"' in flt.split("web:")[1]


def test_the_path_filter_and_codeql_actions_are_pinned_to_a_commit():
    found = 0
    for path in WF.glob("*.yml"):
        for uses in re.findall(r"uses:\s*(\S+)", path.read_text(encoding="utf-8")):
            if uses.startswith(("dorny/paths-filter", "github/codeql-action")):
                found += 1
                assert re.search(r"@[0-9a-f]{40}$", uses), f"{path.name}: {uses} is not pinned to a commit"
    assert found >= 3


def test_every_scheduled_workflow_reports_a_failed_run():
    watched = set(load("scheduled-failure.yml")["on"]["workflow_run"]["workflows"])
    scheduled = {load(p.name)["name"] for p in WF.glob("*.yml") if "schedule" in (load(p.name)["on"] or {})}
    assert scheduled and scheduled <= watched, f"not reported: {scheduled - watched}"


def test_codeql_does_not_run_on_pull_requests():
    assert "pull_request" not in load("codeql.yml")["on"]


def test_the_rust_caches_of_ci_and_the_bundles_save_from_main_only():
    for name in ("ci.yml", "app-bundles.yml", "nightly.yml"):
        for job_name, job in load(name)["jobs"].items():
            for step in job.get("steps", []):
                if str(step.get("uses", "")).startswith("Swatinem/rust-cache"):
                    assert (step.get("with") or {}).get("save-if") == "${{ github.ref == 'refs/heads/main' }}", \
                        f"{name} {job_name}: a pull request must not save a Rust cache"


def test_the_desktop_app_is_out_of_the_workspace_and_the_pull_request_checks():
    members = re.search(r"^members = \[(.*?)\]", (ROOT / "Cargo.toml").read_text(encoding="utf-8"), re.M).group(1)
    assert "src-tauri" not in members and (ROOT / "app" / "src-tauri").is_dir(), "code kept, not a member"
    assert "pull_request" not in load("app-bundles.yml")["on"] and "push" not in load("app-bundles.yml")["on"]
    assert not {"app-linux", "app-e2e"} & set(load("ci.yml")["jobs"])


def test_the_pull_request_gate_has_only_the_lean_jobs_and_the_slow_ones_run_nightly():
    ci, nightly = load("ci.yml"), load("nightly.yml")
    assert set(ci["jobs"]) == {"changes", "gate-lint", "gate-rust", "test", "release-guard", "web-app"}
    assert "schedule" not in ci["on"]
    assert {"gate-python", "drawing-crosscheck", "web-parity", "upstream-main"} <= set(nightly["jobs"])


def test_design_impact_is_advisory():
    assert load("design-impact.yml")["jobs"]["design-impact"]["continue-on-error"] is True
