# Tight bend twin: where each value comes from

Synthetic geometry. No value here comes from a run of this tool.

## Banks
A river 60 m wide, in EPSG:32630, made of two straights joined by a half turn to the left.

- The first straight runs east for 1,000 m from (500000, 600000).
- The bend is a half circle. Its centre is (501000, 600100). The centreline radius is 100 m.
- The second straight runs west for 1,000 m, 200 m north of the first.

Each bank is the centreline moved 30 m to its side. The left bank is on the inside of the bend:
its radius there is 100 − 30 = 70 m. The right bank is on the outside: 100 + 30 = 130 m.

Each bank has 39 positions: 10 on each straight, 100 m apart, and 19 on the bend, 10° apart. The
coordinates are rounded to the millimetre. The river polygon is the area between the two banks.

## Required radius
The Rule Set gives a minimum bend radius of three vessel lengths. The vessel class is 85 m long.
3 × 85 = 255 m.

## The bend
A line that turns through a half turn, with a radius of R or more all the way, is at least 2 × R
wide across the turn. Across the turn the river is 2 × 130 = 260 m wide, from the outside bank of
one straight to the outside bank of the other. So no line inside the river can hold a radius of
more than 130 m through the bend.

130 m is less than the required 255 m. The radius check must fail: `radius_ok` is false. Any tool
that reports true here is wrong.
