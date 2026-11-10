//! The two banks as a pair, at import: both run the same way (downstream when a centreline or a
//! downstream point says which way that is), left and right are named looking downstream, two banks
//! that cross are refused, and two banks give the river polygon (ticket B2).
//!
//! Pure planar code in project metres.

pub type Pt = [f64; 2];

fn d2(a: Pt, b: Pt) -> f64 {
    let (dx, dy) = (a[0] - b[0], a[1] - b[1]);
    dx * dx + dy * dy
}

fn ends(line: &[Pt]) -> Option<(Pt, Pt)> {
    Some((*line.first()?, *line.last()?))
}

/// True when `line` runs the other way to `reference`: its ends lie nearer the far ends.
pub fn runs_against(line: &[Pt], reference: &[Pt]) -> bool {
    let (Some((a0, a1)), Some((r0, r1))) = (ends(line), ends(reference)) else {
        return false;
    };
    let same = d2(a0, r0).sqrt() + d2(a1, r1).sqrt();
    let other = d2(a0, r1).sqrt() + d2(a1, r0).sqrt();
    other < same
}

/// True when `line` runs away from `point`: its start lies nearer the point than its end.
pub fn runs_away_from(line: &[Pt], point: Pt) -> bool {
    match ends(line) {
        Some((a0, a1)) => d2(a0, point) < d2(a1, point),
        None => false,
    }
}

/// Twice the signed area of the ring `left`, then `right` backwards. Both lines run downstream: a
/// left bank that lies on the left gives a clockwise ring, so a negative area.
pub fn ring_area(left: &[Pt], right: &[Pt]) -> f64 {
    let ring: Vec<Pt> = left.iter().chain(right.iter().rev()).copied().collect();
    let n = ring.len();
    (0..n)
        .map(|i| {
            let (p, q) = (ring[i], ring[(i + 1) % n]);
            p[0] * q[1] - q[0] * p[1]
        })
        .sum()
}

/// True when `left` lies on the right of `other` (a bank or the centreline), looking the way both
/// run.
pub fn on_the_right_of(left: &[Pt], other: &[Pt]) -> bool {
    ring_area(left, other) > 0.0
}

fn cross(o: Pt, a: Pt, b: Pt) -> f64 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

fn on_segment(a: Pt, b: Pt, p: Pt) -> bool {
    p[0] >= a[0].min(b[0])
        && p[0] <= a[0].max(b[0])
        && p[1] >= a[1].min(b[1])
        && p[1] <= a[1].max(b[1])
}

/// The point the closed segments `a-b` and `c-d` share, if any (touching counts).
fn meet(a: Pt, b: Pt, c: Pt, d: Pt) -> Option<Pt> {
    if a[0].max(b[0]) < c[0].min(d[0])
        || c[0].max(d[0]) < a[0].min(b[0])
        || a[1].max(b[1]) < c[1].min(d[1])
        || c[1].max(d[1]) < a[1].min(b[1])
    {
        return None;
    }
    let (d1, d2, d3, d4) = (
        cross(c, d, a),
        cross(c, d, b),
        cross(a, b, c),
        cross(a, b, d),
    );
    if ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
    {
        let t = d1 / (d1 - d2);
        return Some([a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])]);
    }
    if d1 == 0.0 && on_segment(c, d, a) {
        return Some(a);
    }
    if d2 == 0.0 && on_segment(c, d, b) {
        return Some(b);
    }
    if d3 == 0.0 && on_segment(a, b, c) {
        return Some(c);
    }
    if d4 == 0.0 && on_segment(a, b, d) {
        return Some(d);
    }
    None
}

/// The first point, along `a`, where the lines `a` and `b` meet.
pub fn first_crossing(a: &[Pt], b: &[Pt]) -> Option<Pt> {
    for s in a.windows(2) {
        let hits: Vec<Pt> = b
            .windows(2)
            .filter_map(|t| meet(s[0], s[1], t[0], t[1]))
            .collect();
        if let Some(p) = hits
            .into_iter()
            .min_by(|p, q| d2(s[0], *p).total_cmp(&d2(s[0], *q)))
        {
            return Some(p);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;

    #[test]
    fn crossing_lines_meet_at_the_point_where_they_cross() {
        let a = [[0.0, 0.0], [10.0, 10.0]];
        let b = [[0.0, 10.0], [10.0, 0.0]];
        assert_eq!(first_crossing(&a, &b), Some([5.0, 5.0]));
        assert_eq!(first_crossing(&a, &[[0.0, 1.0], [9.0, 10.0]]), None);
    }

    #[test]
    fn the_first_crossing_is_the_first_along_the_line() {
        let a = [[0.0, 0.0], [10.0, 0.0]];
        let b = [[8.0, -1.0], [8.0, 1.0], [2.0, 1.0], [2.0, -1.0]];
        assert_eq!(first_crossing(&a, &b), Some([2.0, 0.0]));
    }

    #[test]
    fn the_left_bank_looking_downstream_gives_a_clockwise_ring() {
        let north = [[0.0, 1.0], [10.0, 1.0]];
        let south = [[0.0, -1.0], [10.0, -1.0]];
        assert!(!on_the_right_of(&north, &south));
        assert!(on_the_right_of(&south, &north));
    }

    #[test]
    fn direction_is_read_from_the_ends() {
        let east = [[0.0, 0.0], [10.0, 0.0]];
        let west = [[10.0, 1.0], [0.0, 1.0]];
        assert!(runs_against(&west, &east));
        assert!(!runs_against(&east, &east));
        assert!(runs_away_from(&east, [-5.0, 0.0]));
        assert!(!runs_away_from(&east, [15.0, 0.0]));
    }
}
