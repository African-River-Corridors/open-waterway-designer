# Flat terrain twin: where each value comes from

Synthetic geometry. Every value here is worked by hand from the rules below. No value comes from a
run of this tool.

## Inputs
- The banks and the river polygon of the straight river twin (`line-straight`): two straight
  lines 3,200 m long and 400 m apart, in EPSG:32630, and the rectangle between them.
- `inputs/flat.tif`: terrain that is flat at 8.75 m. It is a GeoTIFF of 38 by 10 pixels of 100 m,
  in EPSG:32630, with its corner at (499,700, 600,500). It covers the river and 300 m around it.
  The repository's own writer made it (`owd_format::design::dem_source::write_geotiff`).
- A level water surface at 10.00 m.

The terrain is the river bed. The water is 10.00 − 8.75 = 1.25 m deep everywhere.

## The channel
The Rule Set sizes the channel for a vessel 9.5 m wide with a draught of 2.5 m:

- bottom width: 3.2 beams = 3.2 × 9.5 = **30.4 m**, so the half-width is 15.2 m;
- design depth: 1.3 draughts = 1.3 × 2.5 = **3.25 m**.

The channel bottom is at 10.00 − 3.25 = 6.75 m. The bed is at 8.75 m. The cut is
8.75 − 6.75 = 2.00 m deep.

## The section
Below the water the sides of the cut rise 1 m in 3 m. The cut is a trapezoid 2.00 m deep:

- bottom: 30.4 m wide;
- each side runs 3 × 2.00 = 6.0 m out before it meets the bed;
- area = 30.4 × 2.00 + 3 × 2.00² = 60.8 + 12.0 = **72.8 m²**.

The bed is below the water, so no ground stands above the water line. There is no bench, no cut
above the water and no face: the cut model (A or B) makes no difference.

## The volume
The river is straight, so the design line is the centreline, 3,200 m long.

Volume = 72.8 m² × 3,200 m = **232,960 m³**.

## Dredge and dry cut
The cut is 30.4 + 2 × 6.0 = 42.4 m wide at the bed. The river is 400 m wide and the line runs down
its middle. All of the cut is inside the river polygon: all of it is dredge. The dry cut is
**0 m³**. No ground is above the water, so the tallest face is **0 m**.

## The tolerance
The tool does not integrate. It adds up samples. Three things can move its answer.

1. **Samples across the section.** It reads the bed every 1 m across, from −200 m to +200 m, and
   adds cut depth × 1 m. By hand: the 31 samples from −15 m to +15 m are on the bottom,
   31 × 2.00 = 62.0 m². On each side the samples at 16 m to 21 m are 0.8 m to 5.8 m up the side;
   their cut depths are 2.00 − 0.8/3 … 2.00 − 5.8/3, which add up to 5.4 m. Both sides: 10.8 m².
   The sample at 22 m is past the cut. Sum = 62.0 + 10.8 = 72.8 m². For this section the samples
   give the exact area. Error: none.
2. **Stations along the line.** It takes a section every 25 m and counts each as 25 m of channel.
   A line of length L has floor(L / 25) + 1 stations, so the tool's length is between L and
   L + 25 m. On 3,200 m that is at most +0.8 %.
3. **The ends of the design line.** The rolling circle may stop short of each end of the river
   by a step. The straight river twin allows 1 % on the length for this.

The terrain file adds nothing: the height between pixels of a flat surface is the same height, and
8.75 is exact in the file's 32-bit numbers.

So the tool's volume lies between −1 % and +0.8 % of 232,960 m³. The band is **2 %**. The dry cut
may differ from zero only by rounding: the band is 1 m³.
