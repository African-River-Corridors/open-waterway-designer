# The project folder

A project folder holds one study in one place: the design, its DEM, the source data and what OWD writes out. A small text file at the top, `owd-project.json`, says what is there. The web app opens the folder in one step. Every `owd` command that takes a design file takes the folder in its place.

## The folders

```
<project folder>/
  owd-project.json        the small text file that says what is here
  design/<name>.owd       the design file(s)
  terrain/<name>.tif      the DEM (GeoTIFF)
  inputs/                 source data: centreline, river polygon, bank lines, water levels by pool
  exports/                what OWD writes out: GeoJSON, CSV, sheets, reports
```

Make one with `owd project init <folder> --title "Reach A screening"`. It makes the four folders and the text file. It never overwrites a text file that is there. If the folder already holds one design and one DEM in the standard places, the new text file names them.

## The text file

```json
{
  "owd_project": 1,
  "title": "Reach A screening",
  "design": "design/reach-a.owd",
  "dem": "terrain/reach-a-dem.tif",
  "inputs": {
    "centreline": "inputs/centreline.geojson",
    "river_polygon": "inputs/river-polygon.geojson",
    "left_bank": null,
    "right_bank": null,
    "water_pools": "inputs/pools.csv"
  },
  "notes": ""
}
```

- `owd_project` is the version of the file. It must be `1`. Every other key is optional.
- A path is from the project folder, with forward slashes. An absolute path is refused. A path that leaves the folder (`..`) is refused.
- `design` ends in `.owd`. `dem` ends in `.tif` or `.tiff`.
- `null`, or a key left out, means "none".
- A key OWD does not know is ignored, and kept when OWD writes the file again. You can add your own, such as a client reference.
- `water_pools` is a table of `chainage_m,level_m`, one line per pool, as `owd import --water-pools` reads it.

OWD reads and checks the file in one place (`owd_format::project`), for the web app and the command line alike. A file it cannot use is refused with the key, the value and what to write instead.

## How the design names its DEM

A design file records its DEM by a path from **its own folder**, and by the SHA-256 of the file. This is unchanged. In a project the design is in `design/` and the DEM in `terrain/`, so the design records `../terrain/reach-a-dem.tif`. The command line finds the DEM from that path, as it always has.

OWD keeps the relation right for you:

- **`owd project init`** on a folder that already holds a design and a DEM: if the design names a DEM that is not where it says, and the project's DEM is that same file (the same hash), the design is pointed at it. Only the path changes. If the design finds a DEM where it says, `init` leaves the design alone.
- **Open project** in the web app: a design that names the same DEM under another path is pointed at the project's DEM. The design then shows as unsaved, with a notice. Press Save to keep it. A design with no DEM takes the project's DEM, as Load… does.
- **`owd project status`** says when the design looks for its DEM somewhere other than the project's DEM.

The design file never holds the project folder's name or its place on your disk. Nor does any export.

## In the web app

**Open project** in the toolbar asks for the folder (Chrome and Edge have the folder dialog; allow editing, so Save and the exports can write). OWD then:

1. reads `owd-project.json`;
2. opens the design it names;
3. loads the DEM it names. A large sectioned DEM (a tiled GeoTIFF) is not read whole: the browser keeps the file and reads the blocks it needs as you work, so it opens at once. See [Large DEMs](large-dems.md).

After that:

- **Save** writes back to the design file in the folder. No dialog.
- **Export GeoJSON**, **Export CSV**, the **Sheets** PDFs and the reports go into `exports/` (made if it is not there). One notice says where each file went.
- The **Project** panel, above Data in the left panel, shows the title and each listed item with its state: *loaded*, *found, not loaded*, or *missing*. An input that is found but not in the design has a **Load** button. It uses the same import as Load… and `owd import`.
- A DEM that is not the file the design recorded shows the usual warning. The panel offers **Use this DEM**, which records it as Load… does.
- **Save as** saves a copy somewhere else and leaves the project. Use Open project to go back.

**A folder with no text file.** OWD looks in `design/`, `terrain/` and `inputs/`, then at the top of the folder, and shows what it found. With one design and one DEM it offers **Use these and write owd-project.json**. With more than one, you pick. With nothing, it says what a project folder holds.

**Next time.** OWD remembers the folder in this browser and offers **Reopen** *title* at the start. The browser asks you to allow the folder again; that is its rule. "Forget it" removes the memory. The memory is on your machine only.

**Firefox and Safari** have no folder dialog. The button says so, and Open and Load… work as before.

Nothing is uploaded. The folder is read and written by your browser, on your machine.

## On the command line

```
owd project init reach-a --title "Reach A screening"
owd project status reach-a
owd status reach-a
owd run reach-a
owd export reach-a --csv -o reach-a/exports/reach-a.csv
```

- `owd project init <folder> [--title T]` makes the folders and the text file.
- `owd project status <folder>` lists what the text file names and whether each file is there, and checks the DEM against the hash the design recorded. It exits 1 when a named file is missing, the DEM is another file, or the design looks for its DEM somewhere else. Add `--json` for a machine-readable answer.
- Every command that takes a design file takes the project folder in its place. It reads the design the text file names. A command that writes the design writes that file. A folder with no text file, or a text file that names no design, is refused with what to do.

The command line does not put exports into `exports/` by itself: give `-o`, as before.
