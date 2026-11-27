//! The window optimiser (OPT-5): a volume-aware redesign of a design line in short windows.
//!
//! The starting line is cut into windows of about 1 to 2 km whose seams sit on straights where the
//! line hugs the river ([`seams`]). In each window an exact lattice search
//! ([`crate::optimise::lattice::solve_lattice_with`]) finds the least-cost line of arcs no tighter
//! than R_min and straights, from the line's exact pose at one seam to its exact pose at the other,
//! pricing every move with the volumes stage's own section ([`scorer`]: one scorer). A sideways
//! polish ([`polish`]) then slides both the found line and the starting stretch to whatever the full
//! measure says is cheapest. The window takes the found line only when the full measure says it is
//! at least `adopt_threshold` cheaper than the polished starting stretch over the same stretch,
//! padded the same at each end; else the polished starting stretch on the same rule; else it keeps
//! the starting line bit for bit, and says why. A second pass searches windows centred on the first
//! pass's free seams, on the first pass's line, so a seam does not lock in a poor join.
//!
//! Every window starts and ends on the current line's exact poses, so the joined line has no gap and
//! no kink at a seam; every lattice line holds R_min by construction; a polished line is used only
//! when it passes OWD's radius check; a window that would make the line cross itself is undone. The
//! result is never dearer than the starting line on the full measure, window by window.
//!
//! The core reads no clock: the caller passes one, and a stop for cancelling.

pub mod grid;
pub mod line;
pub mod polish;
pub mod run;
pub mod scorer;
pub mod seams;

pub use run::{WindowConfig, WindowInputs, WindowRecord, WindowsResult, optimise_windows};

/// A heading difference in (-pi, pi].
pub fn wrap(d: f64) -> f64 {
    let tau = 2.0 * core::f64::consts::PI;
    let m = (d + core::f64::consts::PI) % tau;
    let m = if m < 0.0 { m + tau } else { m };
    m - core::f64::consts::PI
}
