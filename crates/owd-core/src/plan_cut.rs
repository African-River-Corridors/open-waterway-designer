//! The plan cut: the cut seen from above. The fairway along a design line, at the water surface,
//! less the river polygon. It needs a line, a width and the river; no terrain and no water level,
//! so a study has it before either exists. Every output says so ([`PLAN_ONLY_LABEL`]).
//!
//! Pure: no I/O, no clock. Built on the existing geometry: the stations of the volumes stage
//! (`station_frame`), the repaired polygon of two lines (`polygon_from_bank_pair`), the polygon
//! difference (`subtract_polygons`) and a point inside a polygon (`representative_point`).

use crate::geom::interior::representative_point;
use crate::geom::overlay::polygon_area;
use crate::geom::{Polygon, Pt};
use crate::volumes::footprint::{polygon_from_bank_pair, subtract_polygons};
use crate::volumes::sections::{HalfWidth, station_frame};

/// Said on every output of the plan cut.
pub const PLAN_ONLY_LABEL: &str =
    "plan only: areas from the river polygon, no terrain, no water level";

/// The fairway of a design line.
pub struct Fairway<'a> {
    /// The bottom half-width, as the volumes stage gives it.
    pub bottom_half_width: HalfWidth<'a>,
    /// The side slope under water, horizontal to vertical.
    pub wet_slope: f64,
    /// The design depth, in metres.
    pub depth_m: f64,
    /// The station spacing, in metres.
    pub station_m: f64,
}

/// The bank a cut piece is on, looking the way the river centreline runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bank {
    Left,
    Right,
}

/// One piece of the cut.
#[derive(Debug, Clone, PartialEq)]
pub struct Piece {
    pub polygon: Polygon,
    pub area_m2: f64,
    pub bank: Bank,
}

/// The plan cut of a design line.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanCut {
    /// The cut pieces, largest first.
    pub pieces: Vec<Piece>,
    /// Pieces under the smallest area: how many, and their area in all.
    pub dropped_pieces: usize,
    pub dropped_m2: f64,
}

impl PlanCut {
    /// The area of the cut: the sum of the pieces.
    pub fn area_m2(&self) -> f64 {
        self.pieces.iter().map(|p| p.area_m2).sum()
    }

    /// The area of the pieces on the left bank.
    pub fn left_m2(&self) -> f64 {
        self.on(Bank::Left)
    }

    /// The area of the pieces on the right bank.
    pub fn right_m2(&self) -> f64 {
        self.on(Bank::Right)
    }

    /// The area of the largest piece; 0 when there is no piece.
    pub fn largest_piece_m2(&self) -> f64 {
        self.pieces.first().map_or(0.0, |p| p.area_m2)
    }

    fn on(&self, bank: Bank) -> f64 {
        self.pieces
            .iter()
            .filter(|p| p.bank == bank)
            .map(|p| p.area_m2)
            .sum()
    }
}

/// The fairway polygon of `line` at the water surface: at each station the bottom half-width plus
/// `wet_slope × depth_m` either side, the left edge then the right edge backwards. It ends square
/// at the two ends of the line.
pub fn fairway_polygon(line: &[Pt], fairway: &Fairway) -> Result<Vec<Polygon>, String> {
    let (_, xy, heading) = station_frame(line, fairway.station_m)?;
    let bottom: Vec<f64> = match &fairway.bottom_half_width {
        HalfWidth::Fixed(v) => vec![*v; xy.len()],
        HalfWidth::PerStation(v) => v.clone(),
        HalfWidth::FromStations(f) => f(&xy),
    };
    if bottom.len() != xy.len() {
        return Err(format!(
            "the bottom half-width has {} values for {} stations: give one per station",
            bottom.len(),
            xy.len()
        ));
    }
    let widen = fairway.wet_slope * fairway.depth_m;
    let (mut left, mut right) = (Vec::with_capacity(xy.len()), Vec::with_capacity(xy.len()));
    for ((p, th), hb) in xy.iter().zip(&heading).zip(&bottom) {
        let h = hb + widen;
        let (nx, ny) = (-libm::sin(*th), libm::cos(*th));
        left.push([p[0] + h * nx, p[1] + h * ny]);
        right.push([p[0] - h * nx, p[1] - h * ny]);
    }
    polygon_from_bank_pair(&left, &right)
}

/// Whether `p` is on the left of the segment of `centreline` nearest to it, looking the way the
/// centreline runs. A point on the line counts as left.
fn on_the_left(p: Pt, centreline: &[Pt]) -> bool {
    let mut best: Option<(f64, f64)> = None;
    for w in centreline.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len2 = dx * dx + dy * dy;
        let t = if len2 > 0.0 {
            (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let (qx, qy) = (a[0] + t * dx, a[1] + t * dy);
        let d2 = (p[0] - qx) * (p[0] - qx) + (p[1] - qy) * (p[1] - qy);
        let side = dx * (p[1] - a[1]) - dy * (p[0] - a[0]);
        if best.is_none_or(|(bd, _)| d2 < bd) {
            best = Some((d2, side));
        }
    }
    best.is_none_or(|(_, side)| side >= 0.0)
}

/// The fairway of `line`, less `river`. `centreline` is the river centreline, for the bank of each
/// piece. A piece under `min_piece_m2` is not a cut: it is counted as dropped.
pub fn plan_cut(
    line: &[Pt],
    fairway: &Fairway,
    river: &[Polygon],
    centreline: &[Pt],
    min_piece_m2: f64,
) -> Result<PlanCut, String> {
    let channel = fairway_polygon(line, fairway)?;
    let cut = subtract_polygons(&channel, river, 0.0)?;
    let mut pieces = Vec::new();
    let (mut dropped_pieces, mut dropped_m2) = (0, 0.0);
    for polygon in cut {
        let area_m2 = polygon_area(&polygon);
        if area_m2 < min_piece_m2 {
            dropped_pieces += 1;
            dropped_m2 += area_m2;
            continue;
        }
        let point = representative_point(core::slice::from_ref(&polygon));
        let bank = match point {
            Some(p) if !on_the_left(p, centreline) => Bank::Right,
            _ => Bank::Left,
        };
        pieces.push(Piece {
            polygon,
            area_m2,
            bank,
        });
    }
    pieces.sort_by(|a, b| b.area_m2.total_cmp(&a.area_m2));
    Ok(PlanCut {
        pieces,
        dropped_pieces,
        dropped_m2,
    })
}
