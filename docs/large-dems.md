# Large DEMs: read in sections

A DEM of a whole river can be far larger than memory. A 2 m terrain of 11,850 x 77,275 cells has 916 million cells; read whole, as numbers of 8 bytes, it needs 7.3 GB, over the 4 GB a browser can hold. OWD reads such a DEM **in sections**: one block at a time, at the detail the work needs.

## What OWD reads in sections

A **sectioned** GeoTIFF: one file in tiles (blocks of, say, 512 x 512 cells), usually with **overviews** (the same ground at 2, 4, 8 ... times coarser cells) inside it. This is a cloud-optimised GeoTIFF (COG). OWD reads:

- classic TIFF and BigTIFF (a file over 4 GB);
- 8, 16 or 32-bit integers, or 32 or 64-bit floats, one band;
- DEFLATE, LZW or no compression, with the horizontal predictor (on floats too) or the floating-point one.

A tiled file of 64 MiB or more is read in sections. A smaller file is read whole, as before: it is quick, and every result is the same either way.

`owd dem info FILE` says what a file holds and whether OWD reads it in sections.

## A DEM in strips

A GeoTIFF in **strips** (rows), as many tools write by default, cannot be read in sections. If it fits in memory it is read whole, as before. If not, OWD says so and names the fix:

```
owd dem prepare terrain.tif -o terrain-sectioned.tif
```

This writes a sectioned copy: 512 x 512 tiles, DEFLATE with the predictor that suits the numbers, 32-bit floats (`--keep-type` keeps the input's type), overviews by averaging down to about 256 cells a side, nodata kept and left out of the averages. It never holds the whole raster in memory. A file that is already sectioned with overviews is left alone (`--force` writes a copy anyway).

## What reads which detail

| Reader | Detail it reads |
|---|---|
| Volumes, cross-sections, long section, the cut view, the drawings | Full resolution, along the line only. The numbers are the same as from the whole file. |
| The optimise stage's search grid and flood field | The coarsest overview whose cells are no larger than the grid's own (`cell_m`, `flood_cell_m`). The stage's details then say so (`ground_read_cell_m`). Weirs and pools are chosen on full-resolution sections. |
| The 3D view, contours, terrain shading, the depth of cut on the map | The coarsest overview no larger than a map pixel at that zoom; full resolution when zoomed in. |
| Pool water (which ground each pool floods) | Full resolution, only where the water spreads. |
| The hillshade | The coarsest overview no larger than its pixels. |

A DEM held whole has one level, so every reader gets full resolution, as before.

## Memory

Decoded blocks are kept in a cache of about 256 MB (128 MB in the browser). The **terrain memory limit** setting sets that cache for a sectioned DEM; for a DEM read whole it is still the most the whole grid may take.

## Which file the design was made on

The design records which DEM it was made on. A small file is recorded by the SHA-256 of all its bytes, as always. A large sectioned file is recorded by a **quick check** (`quick_check` in the design), so checking it does not read the whole file: the file's length, every byte of its header and directories (with the tables of where each block is and how long it is), and the stored bytes of a sample of blocks (every block of a small image; 32 blocks spaced evenly through a larger one).

**What the quick check misses:** a change made in place to a block it does not sample, which keeps the block's length and place and the file's length. A tool that rewrites a block nearly always moves it or changes its length, which the check sees; what is left is damage in place. `owd project status <folder> --full` hashes every byte, with progress, and compares it with a full hash when the design recorded one.

## In the browser

Load…, drag and drop and Open project give the browser's worker the file itself, not its bytes. It reads the blocks it needs from the file as you work, so the page never waits for the whole file.

After a page reload the worker has lost the file. **Open project** gets it back once you allow the folder again (the browser asks; that is its rule). A file you dropped or picked with Load… cannot be read again after a reload: load it again.
