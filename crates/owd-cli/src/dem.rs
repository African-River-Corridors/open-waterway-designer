//! `owd dem info` and `owd dem prepare` (COG-1, part 4): what a DEM file holds, and a sectioned copy
//! of one that OWD reads a block at a time.

use crate::commands::{Failure, Result, input_error, say};
use owd_format::sectioned::layout::{
    Image, Layout, SampleType, Samples, compression_name, decode_block,
};
use owd_format::sectioned::source::FileSource;
use owd_format::sectioned::write::{Compression, Spec, Spool, write};
use owd_format::sectioned::{ByteSource, SectionedDem, Terrain};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

fn open(path: &str) -> core::result::Result<(FileSource, Layout), Failure> {
    let src = FileSource::open(Path::new(path)).map_err(input_error)?;
    let layout = Layout::read(&src).map_err(|e| input_error(format!("{path}: {e}")))?;
    Ok((src, layout))
}

/// Whether OWD reads the file in sections, and why not when it does not.
fn sectioned_verdict(path: &str) -> (bool, String) {
    let shared: owd_format::sectioned::SharedSource = match FileSource::open(Path::new(path)) {
        Ok(f) => Arc::new(f),
        Err(e) => return (false, e),
    };
    match SectionedDem::open(shared, 1 << 20) {
        Ok(_) => (true, "yes: it is in tiles, read a block at a time".into()),
        Err(e) => (false, format!("no: {e}")),
    }
}

/// The overviews a sectioned copy should have: down to about 256 cells a side.
fn has_full_overviews(l: &Layout) -> bool {
    let last = l.images.last().expect("one image");
    l.images.len() > 1 && last.width.max(last.height) <= 1024
}

fn describe(img: &Image) -> Value {
    json!({
        "width": img.width, "height": img.height,
        "layout": if img.tiled { "tiles" } else { "strips" },
        "block": [img.block_w, img.block_h],
        "bands": img.bands,
        "sample": img.sample.map_or_else(|| format!("{}-bit, format {}", img.bits, img.format), |s| s.name().to_string()),
        "compression": compression_name(img.compression),
        "predictor": img.predictor,
    })
}

/// `owd dem info FILE`.
pub fn info(path: &str, json_mode: bool) -> Result {
    let (src, layout) = open(path)?;
    let full = &layout.images[0];
    let (ok, why) = sectioned_verdict(path);
    let geo = layout.geo.as_ref().ok();
    let gt = geo.map(|g| g.geotransform);
    let overviews: Vec<Value> = layout.images[1..]
        .iter()
        .map(|i| {
            let mut v = describe(i);
            if let Some(g) = gt {
                v["cell"] = json!([
                    g[0].abs() * full.width as f64 / i.width as f64,
                    g[4].abs() * full.height as f64 / i.height as f64
                ]);
            }
            v
        })
        .collect();
    let value = json!({
        "file": path,
        "bytes": src.len(),
        "bigtiff": layout.big,
        "cells": full.width as u64 * full.height as u64,
        "image": describe(full),
        "cell": gt.map(|g| [g[0].abs(), g[4].abs()]),
        "geotransform": gt,
        "crs": geo.and_then(|g| g.epsg).map(|e| format!("EPSG:{e}")),
        "nodata": geo.and_then(|g| g.nodata),
        "georeferencing": layout.geo.as_ref().err().map(|e| e.to_string()),
        "overviews": overviews,
        "sectioned": ok,
        "sectioned_note": why,
    });
    let mut h = vec![
        format!("file: {path}"),
        format!(
            "size: {} bytes ({:.2} GiB){}",
            src.len(),
            src.len() as f64 / f64::from(1u32 << 30),
            if layout.big { ", BigTIFF" } else { "" }
        ),
        format!(
            "cells: {} x {} ({} cells)",
            full.width,
            full.height,
            full.width as u64 * full.height as u64
        ),
    ];
    if let Some(g) = gt {
        h.push(format!("cell size: {} x {}", g[0].abs(), g[4].abs()));
    }
    h.push(format!(
        "CRS: {}",
        geo.and_then(|g| g.epsg)
            .map_or("unknown".to_string(), |e| format!("EPSG:{e}"))
    ));
    if let Err(e) = &layout.geo {
        h.push(format!("georeferencing: {e}"));
    }
    h.push(if full.tiled {
        format!("layout: tiles of {} x {}", full.block_w, full.block_h)
    } else {
        format!("layout: strips of {} rows", full.block_h)
    });
    h.push(format!("bands: {}", full.bands));
    h.push(format!(
        "sample type: {}",
        full.sample.map_or_else(
            || format!("{}-bit (format {})", full.bits, full.format),
            |s| s.name().to_string()
        )
    ));
    h.push(format!(
        "compression: {}, predictor {}",
        compression_name(full.compression),
        full.predictor
    ));
    h.push(format!(
        "nodata: {}",
        geo.and_then(|g| g.nodata)
            .map_or("none".to_string(), |v| v.to_string())
    ));
    if layout.images.len() == 1 {
        h.push("overviews: none".into());
    } else {
        let sizes: Vec<String> = layout.images[1..]
            .iter()
            .map(|i| format!("{} x {}", i.width, i.height))
            .collect();
        h.push(format!("overviews: {} ({})", sizes.len(), sizes.join(", ")));
    }
    h.push(format!("read in sections by OWD: {why}"));
    say(false, json_mode, &h.join("\n"), value);
    Ok(())
}

/// The rows of the full image, a band at a time, decoded block by block: each block row is decoded
/// once and dropped when the bands have passed it.
struct Rows<'a> {
    src: &'a FileSource,
    little: bool,
    img: &'a Image,
    nodata: Option<f64>,
    held: HashMap<usize, Samples>,
}

impl Rows<'_> {
    fn band(&mut self, r0: usize, r1: usize) -> core::result::Result<Vec<f64>, String> {
        let (w, bw, bh) = (self.img.width, self.img.block_w, self.img.block_h);
        let across = self.img.across();
        let fill = self.nodata.unwrap_or(f64::NAN);
        let mut out = vec![fill; (r1 - r0) * w];
        let (b0, b1) = (r0 / bh, (r1 - 1) / bh);
        self.held.retain(|i, _| i / across >= b0);
        for by in b0..=b1 {
            for bx in 0..across {
                let i = by * across + bx;
                if !self.held.contains_key(&i) {
                    let s = decode_block(self.src, self.little, self.img, i, fill)
                        .map_err(|e| e.to_string())?;
                    self.held.insert(i, s);
                }
                let s = &self.held[&i];
                for r in (by * bh).max(r0)..((by + 1) * bh).min(r1) {
                    for c in bx * bw..((bx + 1) * bw).min(w) {
                        out[(r - r0) * w + c] = s.get((r - by * bh) * bw + (c - bx * bw));
                    }
                }
            }
        }
        Ok(out)
    }
}

/// `owd dem prepare IN -o OUT`.
pub fn prepare(input: &str, output: &str, keep_type: bool, force: bool, json_mode: bool) -> Result {
    let (src, layout) = open(input)?;
    let full = &layout.images[0];
    let geo = layout
        .geo
        .clone()
        .map_err(|e| input_error(format!("{input}: {e}")))?;
    let readable = full
        .readable()
        .map_err(|e| input_error(format!("{input}: {e}")))?;
    if full.bands != 1 {
        return Err(input_error(format!(
            "{input} has {} bands; a DEM has one",
            full.bands
        )));
    }
    if full.tiled && has_full_overviews(&layout) && !force {
        let msg = format!(
            "{input} is already sectioned: tiles of {} x {}, {} overviews down to {} x {}. Nothing to do (give --force to write a copy anyway).",
            full.block_w,
            full.block_h,
            layout.images.len() - 1,
            layout.images.last().map_or(0, |i| i.width),
            layout.images.last().map_or(0, |i| i.height),
        );
        say(
            false,
            json_mode,
            &msg,
            json!({"written": false, "already_sectioned": true, "message": msg}),
        );
        return Ok(());
    }
    if Path::new(input).canonicalize().ok() == Path::new(output).canonicalize().ok() {
        return Err(input_error(
            "the output must be another file than the input",
        ));
    }
    let sample = if keep_type { readable } else { SampleType::F32 };
    let spec = Spec {
        width: full.width,
        height: full.height,
        tile: 512,
        sample,
        compression: Compression::Deflate,
        predictor: Spec::default_predictor(sample),
        big: None,
        overviews: None,
        nodata: geo.nodata,
        geotransform: geo.geotransform,
        epsg: geo.epsg,
        gap: 0,
    };
    let started = std::time::Instant::now();
    let mut rows = Rows {
        src: &src,
        little: layout.little,
        img: full,
        nodata: geo.nodata,
        held: HashMap::new(),
    };
    let out_path = Path::new(output);
    let spool_path = |k: usize| {
        let mut p = out_path.as_os_str().to_owned();
        p.push(format!(".owd-spool-{k}"));
        std::path::PathBuf::from(p)
    };
    let mut spools = Vec::new();
    let mut out = std::fs::File::create(out_path)
        .map_err(|e| input_error(format!("cannot write {output}: {e}")))?;
    let mut last = 0usize;
    let result = write(
        &spec,
        |r0, r1| rows.band(r0, r1),
        |k| {
            let p = spool_path(k);
            spools.push(p.clone());
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(&p)
                .map(|f| Box::new(f) as Box<dyn Spool>)
        },
        &mut out,
        |done, total| {
            if !json_mode && (done * 20 / total.max(1) > last || done == total) {
                last = done * 20 / total.max(1);
                eprintln!("owd: prepare: {done} of {total} rows");
            }
        },
    );
    for p in &spools {
        let _ = std::fs::remove_file(p);
    }
    let size = result.map_err(|e| {
        let _ = std::fs::remove_file(out_path);
        input_error(format!("cannot write {output}: {e}"))
    })?;
    let secs = started.elapsed().as_secs_f64();
    let levels = spec.levels();
    let msg = format!(
        "wrote {output}: {size} bytes ({:.2} GiB), tiles of 512 x 512, DEFLATE with predictor {}, {}, {} overviews down to {} x {}, in {secs:.1} s",
        size as f64 / f64::from(1u32 << 30),
        spec.predictor,
        sample.name(),
        levels.len() - 1,
        levels.last().map_or(0, |l| l.0),
        levels.last().map_or(0, |l| l.1),
    );
    say(
        false,
        json_mode,
        &msg,
        json!({"written": true, "file": output, "bytes": size, "seconds": secs, "overviews": levels.len() - 1, "sample": sample.name()}),
    );
    Ok(())
}

/// Every cell of `a` (any readable GeoTIFF) equal to the full level of `b` (a sectioned one), the
/// way a test checks a prepared copy: `None` when equal, else the first cell that differs.
pub fn first_difference(
    a: &str,
    b: &str,
) -> core::result::Result<Option<(usize, usize, f64, f64)>, String> {
    let src = FileSource::open(Path::new(a))?;
    let layout = Layout::read(&src).map_err(|e| e.to_string())?;
    let geo = layout.geo.clone().map_err(|e| e.to_string())?;
    let sec = SectionedDem::open(Arc::new(FileSource::open(Path::new(b))?), 64 << 20)
        .map_err(|e| e.to_string())?;
    let img = &layout.images[0];
    let mut rows = Rows {
        src: &src,
        little: layout.little,
        img,
        nodata: geo.nodata,
        held: HashMap::new(),
    };
    let step = 256;
    let mut r0 = 0;
    while r0 < img.height {
        let r1 = (r0 + step).min(img.height);
        let band = rows.band(r0, r1)?;
        for r in r0..r1 {
            for c in 0..img.width {
                let v = band[(r - r0) * img.width + c];
                let want = if geo.nodata == Some(v) || v.is_nan() {
                    f64::NAN
                } else {
                    f64::from(v as f32)
                };
                let got = sec.value(r, c);
                if !(want.to_bits() == got.to_bits() || (want.is_nan() && got.is_nan())) {
                    return Ok(Some((r, c, want, got)));
                }
            }
        }
        r0 = r1;
    }
    Ok(None)
}
