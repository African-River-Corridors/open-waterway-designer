# Slow tests: nightly, not per pull request

Issue #151 (build-speed fix 3). The `test` job runs `tools/gates.sh`, which runs the Rust tests with
`cargo nextest run --profile pr`. That profile leaves out the 27 slowest tests (20 s or more each in
a debug build, about 920 s of test time together). `.github/workflows/nightly.yml` runs **every** test with
`--profile nightly`, daily and on demand (`workflow_dispatch`). The list lives in `.config/nextest.toml`.

On a pull request the `pr` profile runs in 3 hash partitions, one CI job each (`gate-rust`, since
2026-10-22). With 3 partitions the longest `pr` test (about 70 s) is shorter than its partition's run, so no
test moved for the split.

Nothing is deleted. A test that is renamed drops out of the list and so runs in the pull-request job again.
To move a test, edit the filter in `.config/nextest.toml` and the table below. To run all tests locally:
`GATES_NEXTEST_PROFILE=nightly bash tools/gates.sh`.

| Test (binary: name) | Debug time | Why it moved |
|---|---|---|
| owd-cli::volumes: `the_chain_gives_the_same_document_as_run_on_the_same_inputs_and_settings` | 101 s | Runs the whole chain twice through the CLI on the full river and DEM. |
| owd-format::optimise: `volumes_measure_the_lattice_line_under_the_pool_levels` | 63 s | Optimises a lattice option, then measures volumes at every pool level. |
| owd-cli::volumes: `export_writes_the_quantity_table_and_the_design_line_with_the_screening_label` | 54 s | Full alignment, optimise and volumes run before the export. |
| owd-cli::volumes: `optimise_adds_the_lattice_option_and_volumes_measure_it` | 53 s | Same full pipeline with the lattice option. |
| owd-core: `alignment::rolling_circle::water::tests::at_46_m_the_fairway_fit_places_every_bend` | 50 s | Fairway fit on every bend of the synthetic river. |
| owd-format::optimise: `weirs_pools_and_the_lattice_option_equal_the_core_called_directly` | 50 s | Runs the optimiser twice (stage and core) to compare. |
| owd-core: `alignment::fairway_fit::corridor_tests::a_narrow_creek_is_never_fitted_sharper_than_its_input` | 44 s | Long corridor sweep over a narrow creek. |
| owd-core: `alignment::rolling_circle::water::tests::at_40_m_every_apex_is_too_narrow_and_the_option_keeps_a_line` | 42 s | Fairway fit sweep at 40 m width. |
| owd-format::alignment: `a_stage_run_runs_the_rolling_circle_and_a_new_vessel_makes_it_stale_then_run_reruns_both` | 37 s | Runs the rolling circle twice end to end. |
| owd-cli::volumes: `run_reruns_a_stale_volumes_stage` | 36 s | Full run, edit, then full re-run through the CLI. |
| owd-format::volumes: `run_stale_reruns_volumes_when_alignment_changes` | 34 s | Full volumes run, edit, then re-run. |
| owd-cli::volumes: `a_changed_dem_warns_on_open_and_marks_volumes_stale` | 30 s | Full volumes run before the staleness check. |

| owd-format::volumes: `the_volumes_equal_section_volumes_called_directly` | 29 s | Long run on the full river or DEM in a debug build. |
| owd-cli::volumes: `volumes_equal_the_core_called_directly_on_the_same_dem` | 27 s | Long run on the full river or DEM in a debug build. |
| owd-core: `alignment::fairway_fit::property_tests::the_synthetic_river_fits_at_300_m_and_is_too_narrow_at_every_apex_at_40_m` | 25 s | Long run on the full river or DEM in a debug build. |
| owd-format::volumes: `a_new_dem_reference_marks_volumes_stale` | 24 s | Long run on the full river or DEM in a debug build. |
| owd-format::volumes: `a_saved_document_reloads_with_the_same_volumes` | 24 s | Long run on the full river or DEM in a debug build. |
| owd-format::volumes: `a_changed_dem_file_warns_on_open_and_marks_volumes_stale` | 23 s | Long run on the full river or DEM in a debug build. |
| owd-cli::owd: `align_makes_the_rolling_circle_design_line_and_replaces_it_when_run_again` | 23 s | Long run on the full river or DEM in a debug build. |
| owd-format::wgs84_river: `a_wgs84_river_is_reprojected_and_gives_the_volumes_of_the_utm_river` | 23 s | Long run on the full river or DEM in a debug build. |
| owd-format::sections: `the_long_section_sums_to_the_volumes_the_table_shows` | 23 s | Long run on the full river or DEM in a debug build. |
| owd-format::sections: `a_cross_section_is_the_walk_s_station_and_its_cut_agrees_with_the_long_section` | 22 s | Long run on the full river or DEM in a debug build. |
| owd-format::sections: `a_point_picked_on_the_map_gives_the_nearest_station` | 22 s | Long run on the full river or DEM in a debug build. |
| owd-format::volumes: `volumes_needs_a_water_level_and_a_fresh_alignment` | 22 s | Long run on the full river or DEM in a debug build. |
| owd-format::volumes: `volumes_refuses_without_a_dem_and_names_the_dem_it_cannot_read` | 21 s | Long run on the full river or DEM in a debug build. |
| owd-format::alignment: `a_second_method_replaces_the_design_line_and_the_reroute_starts_from_the_river` | 20 s | Long run on the full river or DEM in a debug build. |
| owd-reach::bankcheck: `the_recorded_whole_river_gives_a_centreline_about_as_long_as_its_seed` | 30 s | The bank checks on 144 km of recorded banks (issue #373); the gap rule's fast guard is `midline::tests::a_gap_in_a_bank_does_not_tangle_the_mid_line`. |
| owd-reach::bankcheck: `one_more_round_with_only_the_side_repairs_moves_nothing_at_the_ends` | 106 s | Runs the bank checks twice on 144 km of recorded banks (issue #411). |
| owd-core::lattice_long_pool: `a_31_km_pool_is_searched_in_pieces_of_the_default_length` | 17 s debug (was 182 s, and past the 600 s limit on a CI runner) | Searches a 31 km pool in six pieces on a coarser grain (issue #559); its short twin `a_long_pool_is_searched_in_pieces_that_join_at_r_min` stays in the pull-request job and also shows the whole pool hitting the cap. |
| owd-format::lattice_self_crossing: `a_self_crossing_lattice_line_never_passes` | 45 s | Sizing, the cascade and the pool lattice end to end on an 8 km synthetic meander (issue #569); the core tests in `owd-core::lattice_self_crossing` (the guard and the unchanged simple line) and the `no_self_crossing` unit test in `export.rs` stay in the pull-request job. |
| owd-core::lattice_length_weight: `a_length_weight_makes_the_searchs_own_line_simple` | 70 s | Two pools and one more search on the 8 km meander of issue #569, with a length weight. Its twins in the same file, the trade-off table and the heuristic check, stay in the pull-request job. |
| owd-core::lattice_length_weight: `at_weight_zero_the_search_is_unchanged` | 60 s | Two plain-Dijkstra searches of the looped pool of issue #569 (with no length weight the heuristic is 0). The tests of `owd-core::lattice_self_crossing` run the same pools with no weight in the pull-request job. |
| owd-format::optimise: `a_zero_flood_cost_changes_nothing_and_the_flood_terms_are_reported` | 70 s | Runs the optimise stage four times on the `optimise_pools` example (no flood setting, flood cost 0, the report, a flood cost). Its twins `the_flood_settings_are_off_by_default_round_trip_and_are_validated` and `a_flood_cost_or_a_bank_limit_without_a_dem_is_refused`, and the tests of `owd-core::flood_land`, stay in the pull-request job. |
| owd-format::optimise: `a_zero_length_weight_changes_nothing_and_a_positive_one_is_hashed_and_shown` | 35 s | Runs the optimise stage three times on the `optimise_pools` example (no weight, weight 0, weight 50). Its settings-only twin `the_length_weight_is_a_setting_off_by_default` stays in the pull-request job. |
| owd-core::lattice_land_cost: `a_land_weight_keeps_the_line_in_the_river_on_a_drowned_plain` | 40 s debug | Four runs of two pools on a 4.8 km synthetic meander over a drowned plain (land weights 0, 0.005, 0.02 and 3.5; OPT-1). Its twins in the same file (the scorer's land charge, the baseline's land cost, the refusals) stay in the pull-request job. |
| owd-format::optimise: `a_positive_land_weight_is_hashed_and_shown_and_never_reaches_the_volumes` | 60 s | Runs the optimise stage three times and the volumes stage twice on the `optimise_pools` example. Its settings-only twin `the_land_weight_is_a_setting_off_by_default` and the default-bytes test stay in the pull-request job. |
| owd-format::sectioned_large: `a_dem_of_1_2_billion_cells_is_opened_and_a_100_km_line_runs_within_the_cache` | several minutes on its first run (it writes a 20,000 x 60,000 cell DEM; kept afterwards in the target's temporary folder) | COG-1: proves a DEM far over the browser's memory opens at once and a 100 km line's volumes run within the default 256 MiB cache. It has a nightly limit of 1800 s in `.config/nextest.toml`. The ignored `measure_the_owners_raster` in the same file is a measurement run by hand in a release build. |
| owd-cli::optimisation_doc: the nine tests named `*_the_documents_numbers` (the filter is a pattern: `/_the_documents_numbers$/`) | about 100 s each, debug | Each runs one run of the worked example of `docs/optimisation/` again through the `owd` binary (sizing, rolling circle, the weir search and the pool search on a 7.5 km synthetic river, then volumes) and compares with the stored numbers. The fast tests of the same file (the settings tables, the tagged numbers against the stored ones, the anchors, the links) stay in the pull-request job. |

| owd-core::window_cases: `s2_a_bend_against_a_spur_moves_away_from_it` | 65 s debug | Three window-optimiser runs of the bend against a spur (OPT-5 S2, S10: as it is, mirrored, on four threads). Its twins in the same file (S3, S5, S6, S7, S8, S13, S14) and `owd-core::window_guarantees` stay in the pull-request job. |
| owd-format::optimise_windows: `the_window_method_makes_a_line_that_holds_r_min_and_is_never_dearer` | about 5 min debug | The optimise stage's window method end to end, twice (determinism), then the volumes, on the `optimise_pools` example (OPT-5). The settings test in the same file stays in the pull-request job. |
| owd-format::optimise_windows: `the_window_method_keeps_the_pools_of_a_fresh_lattice_run_and_its_weirs_are_seams` | about 4 min debug | The pool lattice, then the window method on its pools (OPT-5). |
Times are from one local nextest run (10 cores, tests in parallel, so inflated); CI times are in the pull request.
Python tests (about 33 s in total) all still run in the pull-request job.

**Longer nightly limit.** The nightly profile stops a test after 600 s (`terminate-after = 5` of 120 s). Three `owd-cli::exit_code_rule` tests (`optimise_follows_the_rule`, `volumes_follows_the_rule`, `run_follows_the_rule`) take about 470 s on a CI runner, so they have an override of 1200 s in `.config/nextest.toml` (issue #497). They also run in the pull-request job, which has no limit.
