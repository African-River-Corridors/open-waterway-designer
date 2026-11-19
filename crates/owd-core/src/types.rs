//! The function types and the refusal that more than one stage shares, defined once (spec 0017).
//! `owd-format::types` re-exports them beside the serialisable value types.

/// A terrain sampler: heights for the points `(xs[i], ys[i])`, NaN where unknown.
pub type TerrainSampler<'a> = &'a dyn Fn(&[f64], &[f64]) -> Vec<f64>;

/// A terrain sampler that is also `Sync`, for a stage that samples from several threads.
pub type SyncTerrainSampler<'a> = &'a (dyn Fn(&[f64], &[f64]) -> Vec<f64> + Sync);

/// A refusal: a Python `ValueError`, with its text. A stage refuses with a [`Refusal`] until its own
/// typed error replaces it.
pub type Refusal = String;
