//! The function types and the refusal that more than one stage shares, defined once (spec 0017).
//! `owd-format::types` re-exports them beside the serialisable value types.

/// A terrain sampler: heights for the points `(xs[i], ys[i])`, NaN where unknown.
pub type TerrainSampler<'a> = &'a dyn Fn(&[f64], &[f64]) -> Vec<f64>;

/// A terrain sampler that is also `Sync`, for a stage that samples from several threads.
pub type SyncTerrainSampler<'a> = &'a (dyn Fn(&[f64], &[f64]) -> Vec<f64> + Sync);

/// A refusal: a Python `ValueError`, with its text. A stage refuses with a [`Refusal`] until its own
/// typed error replaces it.
pub type Refusal = String;

/// What a grid asks of the ground (COG-1). A stage that samples the ground on a grid of cells
/// `cell_m` across (the optimise stage's search grid, its flood field) calls [`sample_grid`]; a
/// sampler that holds coarser copies of the ground (the overviews of a sectioned DEM) may then read
/// the coarsest one whose cell is no larger than `cell_m`, and says which with
/// [`note_grid_read`]. A sampler that holds one level ignores both, so its numbers do not change.
pub mod grid {
    use super::TerrainSampler;
    use core::cell::Cell;

    std::thread_local! {
        static ASK: Cell<Option<f64>> = const { Cell::new(None) };
        static READ: Cell<Option<f64>> = const { Cell::new(None) };
    }

    /// Sample `terrain` at the points of a grid of `cell_m` cells.
    pub fn sample_grid(terrain: TerrainSampler, xs: &[f64], ys: &[f64], cell_m: f64) -> Vec<f64> {
        let before = ASK.with(|a| a.replace(Some(cell_m)));
        let z = terrain(xs, ys);
        ASK.with(|a| a.set(before));
        z
    }

    /// The cell size of the grid being sampled now on this thread, if any.
    pub fn asked_cell() -> Option<f64> {
        ASK.with(Cell::get)
    }

    /// Say that a grid's ground was read from a coarser level, whose cell is `cell_m` metres.
    pub fn note_grid_read(cell_m: f64) {
        READ.with(|r| r.set(Some(r.get().map_or(cell_m, |c| c.max(cell_m)))));
    }

    /// The coarsest cell any grid's ground was read at since the last call, and start again.
    /// `None`: every grid read the full resolution.
    pub fn take_grid_read() -> Option<f64> {
        READ.with(|r| r.replace(None))
    }
}
