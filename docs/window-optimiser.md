# The window optimiser

The window optimiser is a method of the optimise stage (`method: "windows"`, `owd optimise
--method windows`). It makes a design line cheaper to dig, a stretch of 1 to 2 km at a time, and it
judges every change with the same measure the volumes stage reports. It is off by default.

## What it does

1. **It takes a starting line.** The alignment's design line, or a line given with
   `owd design-line`. With no alignment, the river centreline.
2. **It cuts the line into windows.** A seam (the point where two windows meet) is forced at each
   end of the line, at each weir site of an earlier pool-lattice run, and at each end of a recorded
   cut-through. Between forced seams, free seams go every 800 to 2,500 m (1,500 m preferred) where
   the line is straight, inside the river and near its centreline. Of all the chains of seams that
   fit, the one whose worst seam is straightest is chosen, then the one nearest the preferred length.
   Where no seam can stand on such a straight, one is placed anyway and the record says so
   (`forced_off_river`).
3. **It searches each window.** A lattice search finds the cheapest line from the starting line's
   exact point and heading at one seam to its exact point and heading at the other. The line is made
   of arcs no tighter than R_min (12 radii from R_min to 8 R_min) and straights, with at least one
   vessel length of straight between a left and a right curve, also across the seams. Every move is
   priced with the volumes stage's own cross-section: its cut model, its slope, its bend widening at
   the arc's own radius, its ground with the bed model, and the weights of the optimise stage (wet,
   dry, land, length).
4. **It polishes.** The found line and the starting stretch are both slid sideways by a smooth
   offset that keeps their end points and headings, to whatever the full measure says is cheapest.
5. **It adopts only a real gain.** The window takes the found line when the full measure (the volumes
   stage's stations, over the window and the same stretch either side) says it is at least 1 %
   cheaper than the polished starting stretch, and it does not loop, break R_min or leave the
   permitted ground. Failing that, it takes the polished starting stretch on the same rule. Failing
   that, it keeps the starting line exactly as it was, and records why.
6. **A second pass** searches a window over each free seam, from the middle of the window before it
   to the middle of the window after it, so a seam does not lock in a poor join.
7. **It checks the whole line.** R_min on OWD's chord, no self-crossing, the minimum straight, and
   the cost measured end to end against the sum of the windows' savings.

## What it keeps

The weir cascade is not run. When the optimise stage has a fresh run of the pool lattice, its pools,
pool levels and weirs are kept, and each weir is a forced seam with a straight of two vessel lengths
through it. Otherwise the design's own water level is used. With no DEM the windows are optimised on
plan area: the channel's footprint outside the river polygon, times the land weight (1 when not set),
plus the length cost.

A river-following line (`method: "river_following"`, or a line whose details list `cut_throughs`)
may move only inside the river and inside the room of its recorded cut-throughs
(`stay_in_river: "auto"`). `"on"` holds any line to the river; `"off"` frees it.

## The record

The design line's details hold, for every window: its stretch, its two seams, the search's cost of
the starting stretch and of the found line, the full measure of the starting stretch, of the
polished starting stretch and of the found line (weighted cost, dredge, dry cut, land, length,
tallest face, share of ground unknown), the gain in per cent, what was adopted and why (`gain`,
`polished_start`, `under_threshold`, `dearer`, `loop`, `radius`, `left_river`, `not_closed`, `cap`,
`timeout`, `no_path`, `no_ground`, `too_short`, `crosses_line`), the search's expansions, states and
seconds, the polish of both lines, the loop test, the least radius, and the settings' hash. The
totals give the whole line's cost before and after, measured end to end.

## Settings

All under `windows` in the optimise settings. A setting left `null` takes its value from the sizing.

| Setting | Default | Meaning |
|---|---|---|
| `window_target_m` | 1,500 | Preferred window length (m) |
| `window_min_m`, `window_max_m` | 800, 2,500 | The band of window lengths (m) |
| `seam_step_m` | 20 | Seam candidates every this many metres |
| `seam_max_offset_m` | ¼ bottom width, ≥ 10 | A free seam is this close to the river centreline (m) |
| `seam_scales_m` | R_min × 1/6, 1/3, 2/3, 4/3 | Chords the straightness is read on (m) |
| `second_pass` | true | Search over the first pass's seams |
| `pitch_m`, `n_heading` | 12, 64 | Lattice pitch (m), heading bins |
| `radii_mode` | `geometric` | `geometric` (`n_radii` from R_min to `radius_max_factor` × R_min), `multiples` (`radius_multiples` × R_min) or `list` (`radii_m`) |
| `n_radii`, `radius_max_factor` | 12, 8 | |
| `nominal_arc_m` | 80 | Arcs turn the whole number of heading bins nearest this length |
| `straight_len_m` | 2 × pitch | The straight move |
| `min_straight_m` | one vessel length | Straight between a left and a right curve |
| `corridor_half_m`, `corridor_limit` | 350, true | The search stays within this of the starting line |
| `cell_m` | 2 | The search's ground grid |
| `stay_in_river` | `auto` | See above; `in_river_margin_m` 0, `cut_through_room_m` two bottom widths |
| `heuristic` | `c_min` | `c_min` (the length weight per metre: exact) or `none` |
| `max_expand`, `max_states` | 2,000,000, 8,000,000 | Caps per window |
| `window_time_s` | 300 | Time limit per window |
| `polish` | true | With `polish_spacings_m` (R_min / 3.6 … / 1.44), `polish_max_shift_m` (½ bottom width, ≥ 15), `polish_station_m` 20, `polish_radius_margin_m` (max 20 m, 6 % of R_min) |
| `adopt_threshold` | 0.01 | Adopt at 1 % cheaper |
| `turn_limit_deg`, `length_ratio_limit` | 1,080, 3 | The loop test |
| `search_rake_step_m`, `search_ds_m` | 2, 8 | The search's sampling across and along |
| `threads` | every core | Windows searched at once (one in the browser) |
| `memory_budget_mb` | 4,096 | Fewer windows run at once when their states could pass this |

The section (cut model, station step, reach, sampling, slope, half-width) is the volumes stage's,
so a change to the volumes settings makes a window run stale.

## How long it takes

See the change log entry for the measured table (window length against states, expansions,
seconds and memory, on synthetic rivers).
