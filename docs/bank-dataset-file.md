# The bank dataset file

A bank dataset file holds what a bank source knows about the banks of a river, in one file:

- the two bank lines;
- the river polygon, if there is one;
- who made the banks, how accurate they are, and where the source is unsure.

It is the hand-over format between a bank source and Open Waterway Designer. `owd import --banks FILE` reads it in one go. `owd export --banks FILE` writes it (add `--wgs84` for longitude and latitude). The app shell gives the same text, with the same words when it refuses one.

## The form

One GeoJSON FeatureCollection:

```json
{
  "type": "FeatureCollection",
  "crs": {"type": "name", "properties": {"name": "urn:ogc:def:crs:EPSG::32630"}},
  "bank_dataset": {
    "version": 1,
    "source": {"kind": "bank-source", "name": "a bank source", "date": "2026-11-10", "licence": "CC BY 4.0"},
    "accuracy_m": 2.5,
    "low_confidence": [{"from_m": 400.0, "to_m": 650.0, "reason": "trees over the bank"}]
  },
  "features": [
    {"type": "Feature", "properties": {"role": "left_bank"},     "geometry": {"type": "LineString", "coordinates": [[500000.0, 600200.0], [503200.0, 600200.0]]}},
    {"type": "Feature", "properties": {"role": "right_bank"},    "geometry": {"type": "LineString", "coordinates": [[500000.0, 599800.0], [503200.0, 599800.0]]}},
    {"type": "Feature", "properties": {"role": "river_polygon"}, "geometry": {"type": "Polygon", "coordinates": [[[500000.0, 599800.0], [503200.0, 599800.0], [503200.0, 600200.0], [500000.0, 600200.0], [500000.0, 599800.0]]]}}
  ]
}
```

## The rules

- **The CRS.** With no `crs` member, the positions are longitude and latitude (WGS84). A `crs` member that names WGS84 / UTM is honoured. Any other CRS is refused.
- **The `bank_dataset` member is required**, with `version` equal to 1.
  - `source`: who made the banks. Its `kind` is `file`, `edit` or `bank-source`, with a `name`, a `date` (`YYYY-MM-DD`) and a `licence` (text or `null`). It may be `null`.
  - `accuracy_m`: the accuracy of the bank lines in metres, zero or more, or `null` when unknown.
  - `low_confidence`: stretches of river chainage where the source is unsure. Each has `from_m`, `to_m` and a `reason`.

  `source`, `accuracy_m` and `low_confidence` may be left out. They mean what they mean in a design's `inputs.banks`.
- **The features.** `left_bank` and `right_bank` are required, once each, as lines. `river_polygon` is optional, at most once. The order of the features does not matter. Any other role is refused.

## What import does

Import reads the whole file first. If it refuses the file, nothing changes, and the message names what is wrong. The banks go through the same import as separate bank files:

- they are turned to run the same way, and named looking downstream;
- banks that cross are refused;
- a river polygon is built from the banks when the file has none;
- the bank checks report what they find.

Import replaces the design's `inputs.banks` whole: a file with no accuracy leaves no old accuracy behind.

Export always writes the four members of `bank_dataset`, and the features in the order left bank, right bank, river polygon. A file exported and imported again gives the same dataset, and its next export is the same text.
