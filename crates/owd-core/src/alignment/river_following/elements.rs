//! A line of exact elements: straights and circular arcs, each starting where the one before ends,
//! at its heading. Tangent at every join by construction: an element's start pose is the end pose
//! of the one before, worked out once.

use crate::geom::Pt;

/// A pose: `(x, y, heading)`, the heading in radians counter-clockwise from +x.
pub type Pose = (f64, f64, f64);

/// The pose `t` metres along a path of curvature `k` (+ left) from `p`, exactly.
pub fn advance(p: Pose, k: f64, t: f64) -> Pose {
    let half = 0.5 * k * t;
    // The chord is `t · sin(half) / half`; written so a near-straight arc keeps its precision.
    let chord = if half.abs() < 1e-6 {
        t * (1.0 - half * half / 6.0)
    } else {
        t * libm::sin(half) / half
    };
    let dir = p.2 + half;
    (
        p.0 + chord * libm::cos(dir),
        p.1 + chord * libm::sin(dir),
        p.2 + k * t,
    )
}

/// One element of a line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Element {
    /// Where it starts.
    pub start: Pose,
    /// Its chainage along the line at the start, in metres.
    pub s0: f64,
    /// Its length in metres (along the element).
    pub length: f64,
    /// Its curvature in 1/m, + left; 0 for a straight.
    pub curvature: f64,
}

impl Element {
    /// True for an arc.
    pub fn is_arc(&self) -> bool {
        self.curvature != 0.0
    }

    /// The radius in metres; infinity for a straight.
    pub fn radius(&self) -> f64 {
        if self.is_arc() {
            1.0 / self.curvature.abs()
        } else {
            f64::INFINITY
        }
    }

    /// `1` for a left arc, `-1` for a right arc, `0` for a straight.
    pub fn hand(&self) -> i8 {
        if self.curvature > 0.0 {
            1
        } else if self.curvature < 0.0 {
            -1
        } else {
            0
        }
    }

    /// The pose `t` metres into the element.
    pub fn pose_at(&self, t: f64) -> Pose {
        advance(self.start, self.curvature, t)
    }

    /// Where it ends.
    pub fn end(&self) -> Pose {
        self.pose_at(self.length)
    }

    /// Its chainage at the end.
    pub fn s1(&self) -> f64 {
        self.s0 + self.length
    }

    /// The centre of an arc.
    pub fn centre(&self) -> Option<Pt> {
        self.is_arc().then(|| {
            let r = 1.0 / self.curvature;
            [
                self.start.0 - r * libm::sin(self.start.2),
                self.start.1 + r * libm::cos(self.start.2),
            ]
        })
    }

    /// The distance from `p` to the element, and the element's own chainage (0 to its length) of
    /// the nearest point.
    pub fn distance(&self, p: Pt) -> (f64, f64) {
        let (x0, y0, th) = self.start;
        if let Some(c) = self.centre() {
            let r = self.radius();
            let hand = f64::from(self.hand());
            // The angle swept from the start to `p`, in the element's own direction.
            let a0 = libm::atan2(y0 - c[1], x0 - c[0]);
            let a = libm::atan2(p[1] - c[1], p[0] - c[0]);
            let mut sweep = hand * (a - a0);
            sweep = sweep.rem_euclid(core::f64::consts::TAU);
            let span = self.length / r;
            let t = if sweep <= span {
                sweep * r
            } else {
                // Past the end: nearer end, measured round the missing part of the circle.
                let past = sweep - span;
                let before = core::f64::consts::TAU - sweep;
                if past < before { self.length } else { 0.0 }
            };
            if t > 0.0 && t < self.length {
                let d = libm::hypot(p[0] - c[0], p[1] - c[1]);
                return ((d - r).abs(), t);
            }
            let q = self.pose_at(t);
            return (libm::hypot(p[0] - q.0, p[1] - q.1), t);
        }
        let (u0, u1) = (libm::cos(th), libm::sin(th));
        let t = ((p[0] - x0) * u0 + (p[1] - y0) * u1).clamp(0.0, self.length);
        let q = (x0 + t * u0, y0 + t * u1);
        (libm::hypot(p[0] - q.0, p[1] - q.1), t)
    }
}

/// A line of elements, in order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Chain {
    pub elements: Vec<Element>,
}

/// The step of the dense line along an arc, in metres: at 2 m an arc of R 150 m reads within about
/// 0.1 % of its radius on the chord check (as the lattice's own arcs).
pub const ARC_STEP_M: f64 = 2.0;
/// The step of the dense line along a straight, in metres.
pub const STRAIGHT_STEP_M: f64 = 10.0;

impl Chain {
    /// A chain from a start pose and pieces `(curvature, length)`; a piece of no length is left
    /// out, and two pieces of the same curvature in a row become one element.
    pub fn from_pieces(start: Pose, pieces: &[(f64, f64)]) -> Chain {
        let mut out: Vec<Element> = Vec::with_capacity(pieces.len());
        let mut pose = start;
        let mut s = 0.0;
        for &(k, len) in pieces {
            if len.is_nan() || len <= 1e-9 {
                continue;
            }
            if let Some(last) = out.last_mut()
                && last.curvature == k
            {
                last.length += len;
                pose = last.end();
                s = last.s1();
                continue;
            }
            let e = Element {
                start: pose,
                s0: s,
                length: len,
                curvature: k,
            };
            pose = e.end();
            s += len;
            out.push(e);
        }
        Chain { elements: out }
    }

    /// The pieces `(curvature, length)` of the chain.
    pub fn pieces(&self) -> Vec<(f64, f64)> {
        self.elements
            .iter()
            .map(|e| (e.curvature, e.length))
            .collect()
    }

    /// The length in metres.
    pub fn length(&self) -> f64 {
        self.elements.last().map_or(0.0, Element::s1)
    }

    /// The start pose.
    pub fn start(&self) -> Pose {
        self.elements.first().map_or((0.0, 0.0, 0.0), |e| e.start)
    }

    /// The end pose.
    pub fn end(&self) -> Pose {
        self.elements.last().map_or(self.start(), Element::end)
    }

    /// The index of the element at chainage `s` (clamped to the line).
    pub fn element_at(&self, s: f64) -> usize {
        let n = self.elements.len();
        if n == 0 {
            return 0;
        }
        let i = self.elements.partition_point(|e| e.s0 <= s);
        i.saturating_sub(1).min(n - 1)
    }

    /// The pose at chainage `s` (clamped to the line).
    pub fn pose_at(&self, s: f64) -> Pose {
        if self.elements.is_empty() {
            return (0.0, 0.0, 0.0);
        }
        let e = &self.elements[self.element_at(s)];
        e.pose_at((s - e.s0).clamp(0.0, e.length))
    }

    /// The curvature at chainage `s`: of the element there.
    pub fn curvature_at(&self, s: f64) -> f64 {
        if self.elements.is_empty() {
            return 0.0;
        }
        self.elements[self.element_at(s)].curvature
    }

    /// The dense line: every join exactly, arcs every [`ARC_STEP_M`] at most, straights every
    /// [`STRAIGHT_STEP_M`] at most.
    pub fn points(&self) -> Vec<Pt> {
        let mut out = Vec::new();
        if let Some(e) = self.elements.first() {
            out.push([e.start.0, e.start.1]);
        }
        for e in &self.elements {
            let step = if e.is_arc() {
                ARC_STEP_M
            } else {
                STRAIGHT_STEP_M
            };
            let n = ((e.length / step).ceil() as usize).max(1);
            for i in 1..=n {
                let p = e.pose_at(e.length * i as f64 / n as f64);
                out.push([p.0, p.1]);
            }
        }
        out
    }

    /// Stations about every `h` metres (the last at the end): `(chainage, pose, curvature)`.
    pub fn stations(&self, h: f64) -> Vec<(f64, Pose, f64)> {
        let total = self.length();
        let n = ((total / h).round() as usize).max(1);
        (0..=n)
            .map(|i| {
                let s = total * i as f64 / n as f64;
                (s, self.pose_at(s), self.curvature_at(s))
            })
            .collect()
    }

    /// The pieces from chainage `a` to `b` (`a < b`): the pose at `a` and the pieces after it.
    pub fn slice(&self, a: f64, b: f64) -> (Pose, Vec<(f64, f64)>) {
        let start = self.pose_at(a);
        let mut out = Vec::new();
        for e in &self.elements {
            let lo = e.s0.max(a);
            let hi = e.s1().min(b);
            if hi > lo {
                out.push((e.curvature, hi - lo));
            }
        }
        (start, out)
    }

    /// The largest jump of heading (radians) and of position (metres) at any join.
    pub fn largest_join_jump(&self) -> (f64, f64) {
        let mut th: f64 = 0.0;
        let mut pos: f64 = 0.0;
        for w in self.elements.windows(2) {
            let a = w[0].end();
            let b = w[1].start;
            th = th.max((a.2 - b.2).abs());
            pos = pos.max(libm::hypot(a.0 - b.0, a.1 - b.1));
        }
        (th, pos)
    }

    /// The least radius of any arc; infinity with none.
    pub fn least_radius(&self) -> f64 {
        self.elements
            .iter()
            .map(Element::radius)
            .fold(f64::INFINITY, f64::min)
    }

    /// Pairs of arcs in a row of opposite hand, either tighter than `tight_r`, with a straight
    /// between them shorter than `min_straight`: `(chainage of the first arc's end, straight
    /// length)`.
    pub fn short_reversals(&self, tight_r: f64, min_straight: f64) -> Vec<(f64, f64)> {
        let mut out = Vec::new();
        let mut last_arc: Option<&Element> = None;
        let mut run = 0.0;
        for e in &self.elements {
            if e.is_arc() {
                if let Some(a) = last_arc
                    && a.hand() == -e.hand()
                    && (a.radius() < tight_r || e.radius() < tight_r)
                    && run < min_straight - 1e-6
                {
                    out.push((a.s1(), run));
                }
                last_arc = Some(e);
                run = 0.0;
            } else {
                run += e.length;
            }
        }
        out
    }
}

/// Walk `pts` (in order along the chain) and give each its distance from the chain and the chain's
/// chainage of its nearest point. The walk keeps an element pointer that only moves on, looking a
/// few elements either side, so a long line costs one pass.
pub fn walk_distances(chain: &Chain, pts: &[Pt]) -> Vec<(f64, f64)> {
    let n = chain.elements.len();
    if n == 0 {
        return pts.iter().map(|_| (f64::INFINITY, 0.0)).collect();
    }
    let mut at = 0usize;
    let mut out = Vec::with_capacity(pts.len());
    for &p in pts {
        let lo = at.saturating_sub(2);
        let hi = (at + 4).min(n);
        let mut best = (f64::INFINITY, 0.0, at);
        for (j, e) in chain.elements.iter().enumerate().take(hi).skip(lo) {
            let (d, t) = e.distance(p);
            if d < best.0 - 1e-12 {
                best = (d, e.s0 + t, j);
            }
        }
        // Move on only: a point never pulls the pointer back.
        at = best.2.max(at);
        out.push((best.0, best.1));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arc_ends_where_the_circle_says() {
        let r = 300.0;
        let e = Element {
            start: (0.0, 0.0, 0.0),
            s0: 0.0,
            length: r * core::f64::consts::FRAC_PI_2,
            curvature: 1.0 / r,
        };
        let p = e.end();
        assert!((p.0 - r).abs() < 1e-9 && (p.1 - r).abs() < 1e-9);
        assert!((p.2 - core::f64::consts::FRAC_PI_2).abs() < 1e-12);
        assert_eq!(e.centre(), Some([0.0, r]));
        // A point on the circle half way round the arc is on it.
        let (d, t) = e.distance([r * libm::sin(0.4), r - r * libm::cos(0.4)]);
        assert!(d < 1e-9 && (t - 0.4 * r).abs() < 1e-6);
    }

    #[test]
    fn chain_is_tangent_and_merges_like_pieces() {
        let c = Chain::from_pieces(
            (10.0, 5.0, 0.3),
            &[
                (0.0, 100.0),
                (0.0, 50.0),
                (1.0 / 400.0, 300.0),
                (-1.0 / 350.0, 200.0),
            ],
        );
        assert_eq!(c.elements.len(), 3);
        assert_eq!(c.largest_join_jump(), (0.0, 0.0));
        assert!((c.length() - 650.0).abs() < 1e-9);
        assert_eq!(c.short_reversals(700.0, 75.0).len(), 1);
    }
}
