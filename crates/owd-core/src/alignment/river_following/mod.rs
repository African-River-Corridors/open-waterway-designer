//! The river-following line (spec OPT-4): a line built from the river first, then moved by cost.
//!
//! 1. [`seed`]: arcs and straights that follow the river centreline as closely as R_min allows.
//! 2. Cut-throughs: short arc-straight-arc paths across a meander's neck, each an explicit choice.
//! 3. The sideways shift: dynamic programming on the offset from the line, the shifted line's own
//!    curvature a hard limit.

pub mod choice;
pub mod cuts;
pub mod elements;
#[doc(hidden)]
pub mod fixtures;
pub mod mask;
pub mod score;
pub mod seed;
