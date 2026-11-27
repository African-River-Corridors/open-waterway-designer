//! The cut-through choice (spec OPT-4, 3.3): each candidate costed against staying in the river,
//! accepted by the rule or by the user's force list, and the line rebuilt with the cuts taken.

use super::cuts::{Candidate, splice};
use super::elements::Chain;
use super::score::{StationScorer, Totals, totals};
use std::collections::BTreeMap;

/// The rule's settings.
#[derive(Debug, Clone, PartialEq)]
pub struct ChoiceParams {
    /// A cut saves at least this length, m.
    pub min_save_m: f64,
    /// A cut is at least this share cheaper than staying (a tie goes to the river).
    pub min_gain: f64,
    /// A cut's tallest face is at most this, or no taller than staying's, m.
    pub face_max_m: f64,
    /// The costing station step, m.
    pub step_m: f64,
}

/// How a candidate was decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecidedBy {
    Rule,
    ForcedOn,
    ForcedOff,
}

impl DecidedBy {
    pub fn as_str(self) -> &'static str {
        match self {
            DecidedBy::Rule => "rule",
            DecidedBy::ForcedOn => "forced on",
            DecidedBy::ForcedOff => "forced off",
        }
    }
}

/// A candidate with its numbers and its decision.
#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    pub candidate: Candidate,
    /// Staying in the river over the stretch.
    pub stay: Totals,
    /// Each option of the cut.
    pub options: Vec<Totals>,
    /// The option offered: the cheapest that saves enough.
    pub best: usize,
    /// True when the offered option passes the rule.
    pub passes: bool,
    pub accepted: bool,
    pub decided_by: DecidedBy,
    /// Why it was not taken, in words.
    pub refused_because: Option<String>,
    /// Line chainage of the cut on the line after the cuts, when accepted.
    pub line_from: Option<f64>,
    pub line_to: Option<f64>,
}

/// What the rule says about one option against staying: `Ok` when it passes, else why not.
pub fn rule(
    cand: &Candidate,
    k: usize,
    cut: &Totals,
    stay: &Totals,
    p: &ChoiceParams,
) -> Result<(), String> {
    let saved = cand.saved_m(k);
    if saved < p.min_save_m {
        return Err(format!(
            "saves {saved:.0} m, under the {:.0} m it must",
            p.min_save_m
        ));
    }
    if cut.cost.is_nan() || cut.cost > (1.0 - p.min_gain) * stay.cost || cut.cost >= stay.cost {
        return Err(format!(
            "costs {:.0}, not {:.0} % under staying in the river ({:.0})",
            cut.cost,
            100.0 * p.min_gain,
            stay.cost
        ));
    }
    if cut.face_max_m > p.face_max_m && cut.face_max_m > stay.face_max_m {
        return Err(format!(
            "its tallest face is {:.1} m, over the limit of {:.1} m and over staying's {:.1} m",
            cut.face_max_m, p.face_max_m, stay.face_max_m
        ));
    }
    Ok(())
}

/// Cost every candidate against staying, decide each, and build the line with the cuts taken.
/// `force` maps an id to on (`true`) or off (`false`); a forced id that matches no candidate is
/// returned in the last list. With no ground model no cut passes by rule: only a forced one is
/// taken.
pub fn choose(
    chain: &Chain,
    cands: Vec<Candidate>,
    scorer: &dyn StationScorer,
    force: &BTreeMap<String, bool>,
    p: &ChoiceParams,
) -> (Vec<Choice>, Chain, Vec<String>) {
    let ground = scorer.has_ground();
    let mut out: Vec<Choice> = cands
        .into_iter()
        .map(|c| {
            let stay = totals(chain, c.s_from, c.s_to, p.step_m, scorer, p.face_max_m);
            let start = chain.pose_at(c.s_from);
            let options: Vec<Totals> = c
                .options
                .iter()
                .map(|o| {
                    let cut = Chain::from_pieces(start, &o.pieces);
                    totals(&cut, 0.0, cut.length(), p.step_m, scorer, p.face_max_m)
                })
                .collect();
            // The option offered: the first that passes, the cheapest of those; else the cheapest.
            let pass: Vec<bool> = options
                .iter()
                .enumerate()
                .map(|(k, t)| rule(&c, k, t, &stay, p).is_ok())
                .collect();
            let cheapest = |only_passing: bool| {
                (0..options.len())
                    .filter(|&k| !only_passing || pass[k])
                    .min_by(|&a, &b| options[a].cost.total_cmp(&options[b].cost))
            };
            let best = cheapest(true).or_else(|| cheapest(false)).unwrap_or(0);
            let verdict = rule(&c, best, &options[best], &stay, p);
            let passes = ground && verdict.is_ok();
            let refused_because = if !ground {
                Some(
                    "no DEM: cut-throughs are your choice (listed and costed on plan area; force \
                     one on by its id)"
                        .to_string(),
                )
            } else {
                verdict.err()
            };
            Choice {
                candidate: c,
                stay,
                options,
                best,
                passes,
                accepted: false,
                decided_by: DecidedBy::Rule,
                refused_because,
                line_from: None,
                line_to: None,
            }
        })
        .collect();
    let mut unmatched: Vec<String> = force
        .keys()
        .filter(|id| !out.iter().any(|c| &c.candidate.id == *id))
        .cloned()
        .collect();
    unmatched.sort();
    for c in &mut out {
        match force.get(&c.candidate.id) {
            Some(true) => c.decided_by = DecidedBy::ForcedOn,
            Some(false) => {
                c.decided_by = DecidedBy::ForcedOff;
                c.refused_because = Some("forced off".into());
            }
            None => {}
        }
    }
    // Forced on first, then the passing ones; each in order of the length saved.
    let mut order: Vec<usize> = (0..out.len())
        .filter(|&i| {
            out[i].decided_by == DecidedBy::ForcedOn
                || (out[i].decided_by == DecidedBy::Rule && out[i].passes)
        })
        .collect();
    order.sort_by(|&a, &b| {
        let fa = out[a].decided_by == DecidedBy::ForcedOn;
        let fb = out[b].decided_by == DecidedBy::ForcedOn;
        fb.cmp(&fa)
            .then(
                out[b]
                    .candidate
                    .saved_m(out[b].best)
                    .total_cmp(&out[a].candidate.saved_m(out[a].best)),
            )
            .then(a.cmp(&b))
    });
    let mut taken: Vec<usize> = Vec::new();
    for i in order {
        if let Some(&j) = taken
            .iter()
            .find(|&&j| out[j].candidate.overlaps(&out[i].candidate))
        {
            out[i].refused_because = Some(format!(
                "{}, but overlaps {}, taken first",
                if out[i].decided_by == DecidedBy::ForcedOn {
                    "forced on"
                } else {
                    "passes"
                },
                out[j].candidate.id
            ));
            continue;
        }
        out[i].accepted = true;
        out[i].refused_because = None;
        taken.push(i);
    }
    // The line with the cuts, the last first so the earlier chainages hold.
    taken.sort_by(|&a, &b| out[b].candidate.s_from.total_cmp(&out[a].candidate.s_from));
    let mut line = chain.clone();
    for &i in &taken {
        let c = &out[i];
        line = splice(
            &line,
            c.candidate.s_from,
            c.candidate.s_to,
            &c.candidate.options[c.best].pieces,
        );
    }
    // Each accepted cut's chainage on the new line: its own start, less what the cuts before it
    // saved.
    let mut by_start: Vec<usize> = taken.clone();
    by_start.sort_by(|&a, &b| out[a].candidate.s_from.total_cmp(&out[b].candidate.s_from));
    let mut saved = 0.0;
    for i in by_start {
        let c = &mut out[i];
        let from = c.candidate.s_from - saved;
        let len = c.candidate.options[c.best].length_m;
        c.line_from = Some(from);
        c.line_to = Some(from + len);
        saved += c.candidate.saved_m(c.best);
    }
    out.sort_by(|a, b| a.candidate.s_from.total_cmp(&b.candidate.s_from));
    (out, line, unmatched)
}
