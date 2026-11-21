//! The terrain tiles of the session's DEM, for the map's 3D view (MAP-2). The tile maker is kept
//! with the DEM it was built for: a new DEM gets a new maker and a new number, so the page can
//! tell that every tile it holds is out of date. The contour tiles of the same DEM (MAP-3) are
//! made from the same maker. Nothing here is written to the design.

use owd_format::design::channel::{
    CutSurface, DEPTH_CLASSES_M, DREDGE_COLOURS, DRY_COLOURS,
};
use owd_format::design::contours::ContourCache;
use owd_format::design::terrain_tile::{TerrainState, TileMaker};
use std::collections::HashMap;
use owd_format::geotiff::Dem;
use serde_json::{Value, json};
use std::sync::Arc;

/// The tile maker of the DEM in memory, if there is one.
#[derive(Debug, Default)]
pub struct TerrainTiles {
    maker: Option<TileMaker>,
    /// The contour tiles made from the maker's DEM.
    contours: ContourCache,
    /// Counts the DEMs seen: the `id` in [`TerrainTiles::info`].
    id: u32,
    /// The cut of the channel over the DEM (MAP-5): the After terrain state and the cut-depth
    /// tiles are made from it. See [`TerrainTiles::set_cut`].
    cut: CutTiles,
}

/// The tiles of the cut-depth picture kept: each a PNG of at most about 30 kB.
const CUT_TILES: usize = 256;

/// The cut surface of the design line now, and the cut-depth tiles made from it.
#[derive(Debug, Default)]
struct CutTiles {
    surface: Option<Arc<CutSurface>>,
    /// The design the surface was made for: see [`TerrainTiles::cut_key`].
    key: String,
    /// Counts the cut surfaces seen: the `cut` in [`TerrainTiles::info`].
    rev: u32,
    /// By tile: when it was last asked for, and its PNG (no bytes: nothing is cut in it).
    tiles: HashMap<(u8, u32, u32), (u64, Arc<Vec<u8>>)>,
    clock: u64,
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

impl TerrainTiles {
    /// The maker for `dem`, built now if the DEM is not the one the last maker was built for.
    /// `None` with no DEM.
    fn for_dem(&mut self, dem: Option<Arc<Dem>>) -> Result<Option<&mut TileMaker>, String> {
        let Some(dem) = dem else {
            self.maker = None;
            self.contours.clear();
            return Ok(None);
        };
        if !self
            .maker
            .as_ref()
            .is_some_and(|m| Arc::ptr_eq(m.dem(), &dem))
        {
            self.maker = None;
            self.contours.clear();
            self.maker = Some(TileMaker::new(dem).map_err(|e| e.to_string())?);
            self.id += 1;
            // The cut was laid on the old DEM: it is made again for this one.
            self.drop_cut();
        }
        Ok(self.maker.as_mut())
    }

    // --- the cut (MAP-5) ----------------------------------------------------------------------

    fn drop_cut(&mut self) {
        if self.cut.surface.is_some() {
            self.cut.rev += 1;
        }
        self.cut.surface = None;
        self.cut.key.clear();
        self.cut.tiles.clear();
        self.contours.clear_state(TerrainState::After);
    }

    /// The design the cut surface held now was made for; empty for none.
    pub fn cut_key(&self) -> &str {
        &self.cut.key
    }

    /// Take the cut surface made for the design `key` (`None`: the design has no cut to show).
    /// The After tiles, their contours and the cut-depth tiles of the old one are forgotten, and
    /// the `cut` number in [`TerrainTiles::info`] goes up, so the page asks for none of them again.
    pub fn set_cut(&mut self, key: String, surface: Option<CutSurface>) {
        self.drop_cut();
        self.cut.key = key;
        self.cut.surface = surface.map(Arc::new);
        self.cut.rev += 1;
        if let Some(maker) = self.maker.as_mut() {
            let overlay = self
                .cut
                .surface
                .clone()
                .map(|s| s as Arc<dyn owd_format::design::terrain_tile::Overlay>);
            maker.set_overlay(TerrainState::After, overlay);
        }
    }

    /// The cut-depth picture of tile `z/x/y`: an RGBA PNG, with no bytes when nothing is cut in
    /// the tile or the design has no cut to show.
    pub fn cut_tile(
        &mut self,
        dem: Option<Arc<Dem>>,
        z: u32,
        x: u32,
        y: u32,
    ) -> Result<Arc<Vec<u8>>, String> {
        self.for_dem(dem)?;
        let z = u8::try_from(z).map_err(|_| format!("there is no map tile {z}/{x}/{y}"))?;
        let (Some(maker), Some(surface)) = (self.maker.as_ref(), self.cut.surface.clone()) else {
            return Ok(Arc::new(Vec::new()));
        };
        self.cut.clock += 1;
        if let Some(hit) = self.cut.tiles.get_mut(&(z, x, y)) {
            hit.0 = self.cut.clock;
            return Ok(hit.1.clone());
        }
        let png = Arc::new(
            surface
                .depth_tile(maker, z, x, y)
                .map_err(|e| e.to_string())?
                .unwrap_or_default(),
        );
        if self.cut.tiles.len() >= CUT_TILES
            && let Some(oldest) = self
                .cut
                .tiles
                .iter()
                .min_by_key(|(_, (used, _))| *used)
                .map(|(k, _)| *k)
        {
            self.cut.tiles.remove(&oldest);
        }
        self.cut
            .tiles
            .insert((z, x, y), (self.cut.clock, png.clone()));
        Ok(png)
    }

    /// What a click on the cut reads out at (`lon`, `lat`): `null` where nothing is cut.
    pub fn cut_at(&mut self, dem: Option<Arc<Dem>>, lon: f64, lat: f64) -> Result<Value, String> {
        self.for_dem(dem)?;
        let (Some(maker), Some(surface)) = (self.maker.as_ref(), self.cut.surface.as_ref()) else {
            return Ok(Value::Null);
        };
        Ok(surface
            .at_lon_lat(maker, lon, lat)
            .map_or(Value::Null, |c| json!(c)))
    }

    /// The key of the cut-depth picture: each depth class with its two colours.
    pub fn cut_legend() -> Value {
        let classes: Vec<Value> = DEPTH_CLASSES_M
            .iter()
            .enumerate()
            .map(|(i, from)| {
                json!({
                    "from_m": from,
                    "to_m": DEPTH_CLASSES_M.get(i + 1),
                    "dredge": hex(DREDGE_COLOURS[i]),
                    "dry": hex(DRY_COLOURS[i]),
                })
            })
            .collect();
        json!(classes)
    }

    /// What the map needs to add the terrain: `null` with no DEM in memory, else
    /// `{id, bounds: [west, south, east, north], min_m, max_m, base_m, max_zoom, tile_size,
    /// encoding}`.
    pub fn info(&mut self, dem: Option<Arc<Dem>>) -> Result<Value, String> {
        let Some(maker) = self.for_dem(dem)? else {
            return Ok(Value::Null);
        };
        let info = maker.info().clone();
        Ok(json!({
            "id": self.id,
            "bounds": info.bounds,
            "min_m": info.min_m,
            "max_m": info.max_m,
            "base_m": info.base_m,
            "max_zoom": info.max_zoom,
            "tile_size": info.tile_size,
            "encoding": "terrarium",
            // The cut over this DEM (MAP-5): a number that changes with it, or null for none.
            "cut": self.cut.surface.as_ref().map(|_| self.cut.rev),
        }))
    }

    /// Tile `z/x/y` of the DEM as a Terrarium PNG.
    pub fn tile(
        &mut self,
        dem: Option<Arc<Dem>>,
        z: u32,
        x: u32,
        y: u32,
    ) -> Result<Arc<Vec<u8>>, String> {
        self.tile_in(dem, TerrainState::Before.name(), z, x, y)
    }

    /// Tile `z/x/y` of the ground in the terrain state named `state` (`before`, `after`).
    pub fn tile_in(
        &mut self,
        dem: Option<Arc<Dem>>,
        state: &str,
        z: u32,
        x: u32,
        y: u32,
    ) -> Result<Arc<Vec<u8>>, String> {
        let state = state_named(state)?;
        let maker = self
            .for_dem(dem)?
            .ok_or_else(|| "there is no DEM in memory to cut terrain tiles from".to_string())?;
        let z = u8::try_from(z).map_err(|_| format!("there is no map tile {z}/{x}/{y}"))?;
        maker.tile_in(state, z, x, y).map_err(|e| e.to_string())
    }

    /// The contour lines of tile `z/x/y` of the DEM, every `interval_m` metres, as a Mapbox
    /// Vector Tile (no bytes when the tile has no line).
    pub fn contour_tile(
        &mut self,
        dem: Option<Arc<Dem>>,
        z: u32,
        x: u32,
        y: u32,
        interval_m: f64,
    ) -> Result<Arc<Vec<u8>>, String> {
        self.contour_tile_in(dem, TerrainState::Before.name(), z, x, y, interval_m)
    }

    /// [`TerrainTiles::contour_tile`] of the ground in the terrain state named `state`.
    pub fn contour_tile_in(
        &mut self,
        dem: Option<Arc<Dem>>,
        state: &str,
        z: u32,
        x: u32,
        y: u32,
        interval_m: f64,
    ) -> Result<Arc<Vec<u8>>, String> {
        let state = state_named(state)?;
        self.for_dem(dem)?;
        let maker = self
            .maker
            .as_ref()
            .ok_or_else(|| "there is no DEM in memory to draw contours from".to_string())?;
        let z = u8::try_from(z).map_err(|_| format!("there is no map tile {z}/{x}/{y}"))?;
        self.contours
            .tile_in(maker, state, z, x, y, interval_m)
            .map_err(|e| e.to_string())
    }
}

fn state_named(name: &str) -> Result<TerrainState, String> {
    TerrainState::from_name(name).ok_or_else(|| format!("there is no terrain state `{name}`"))
}

#[cfg(test)]
mod tests {
    //! Label: example
    use super::*;
    use owd_format::design::terrain_tile::decode_png;

    fn dem(level: f64) -> Arc<Dem> {
        Arc::new(Dem {
            width: 50,
            height: 50,
            data: (0..2500).map(|i| level + (i % 50) as f64).collect(),
            geotransform: [0.001, 0.0, -2.0, 0.0, -0.001, 5.05],
            nodata: None,
            epsg: Some(4326),
        })
    }

    #[test]
    fn no_dem_gives_null_and_a_tile_is_refused_plainly() {
        let mut t = TerrainTiles::default();
        assert_eq!(t.info(None).unwrap(), Value::Null);
        let e = t.tile(None, 3, 1, 1).unwrap_err();
        assert!(e.contains("no DEM"), "{e}");
    }

    #[test]
    fn the_same_dem_keeps_its_number_and_a_new_dem_gets_a_new_one() {
        let mut t = TerrainTiles::default();
        let a = dem(10.0);
        let first = t.info(Some(a.clone())).unwrap();
        assert_eq!(first["id"], 1);
        assert_eq!(first["encoding"], "terrarium");
        assert_eq!(first["tile_size"], 256);
        // Heights run 10 to 59 across each row: the base is the middle one.
        assert_eq!(first["base_m"], 35.0);
        assert_eq!(first["bounds"], json!([-2.0, 5.0, -1.95, 5.05]));
        t.tile(Some(a.clone()), 12, 2025, 1989).unwrap();
        assert_eq!(
            t.info(Some(a.clone())).unwrap()["id"],
            1,
            "asked again, same DEM"
        );
        // Another DEM: another number, and tiles of the new heights.
        let b = dem(200.0);
        assert_eq!(t.info(Some(b.clone())).unwrap()["id"], 2);
        let far = decode_png(&t.tile(Some(b), 2, 0, 0).unwrap()).unwrap();
        assert!(
            (far[0] - 225.0).abs() < 0.01,
            "off the DEM: its base height"
        );
        // The DEM goes (a new design): nothing to draw.
        assert_eq!(t.info(None).unwrap(), Value::Null);
        assert!(t.tile(None, 2, 0, 0).is_err());
    }

    #[test]
    fn contour_tiles_are_of_the_dem_in_memory_and_go_with_it() {
        let mut t = TerrainTiles::default();
        let e = t.contour_tile(None, 12, 2025, 1990, 1.0).unwrap_err();
        assert!(e.contains("no DEM"), "{e}");
        // Heights 10 to 59 across the DEM: contours every 5 m, and more of them every 1 m.
        let a = dem(10.0);
        let five = t
            .contour_tile(Some(a.clone()), 12, 2025, 1990, 5.0)
            .unwrap();
        let one = t
            .contour_tile(Some(a.clone()), 12, 2025, 1990, 1.0)
            .unwrap();
        assert!(!five.is_empty() && one.len() > five.len());
        assert!(Arc::ptr_eq(
            &five,
            &t.contour_tile(Some(a.clone()), 12, 2025, 1990, 5.0)
                .unwrap()
        ));
        assert_eq!(t.info(Some(a.clone())).unwrap()["id"], 1, "the same DEM");
        // A tile that holds none of the DEM: no lines, no bytes. No such tile or interval: refused.
        assert!(
            t.contour_tile(Some(a.clone()), 12, 0, 0, 5.0)
                .unwrap()
                .is_empty()
        );
        assert!(
            t.contour_tile(Some(a.clone()), 300, 0, 0, 5.0)
                .unwrap_err()
                .contains("300/0/0")
        );
        assert!(
            t.contour_tile(Some(a), 12, 2025, 1990, 0.0)
                .unwrap_err()
                .contains("interval")
        );
        // Flat ground has no contours; a new DEM never gets the old one's tile.
        let flat = Arc::new(Dem {
            data: vec![7.0; 2500],
            ..(*dem(0.0)).clone()
        });
        assert!(
            t.contour_tile(Some(flat), 12, 2025, 1990, 5.0)
                .unwrap()
                .is_empty()
        );
        assert!(t.contour_tile(None, 12, 2025, 1990, 5.0).is_err());
    }

    #[test]
    fn a_tile_that_does_not_exist_is_refused() {
        let mut t = TerrainTiles::default();
        assert!(
            t.tile(Some(dem(1.0)), 300, 0, 0)
                .unwrap_err()
                .contains("300/0/0")
        );
        assert!(
            t.tile(Some(dem(1.0)), 2, 4, 0)
                .unwrap_err()
                .contains("2/4/0")
        );
    }
}
