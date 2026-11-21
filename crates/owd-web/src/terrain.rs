//! The terrain tiles of the session's DEM, for the map's 3D view (MAP-2). The tile maker is kept
//! with the DEM it was built for: a new DEM gets a new maker and a new number, so the page can
//! tell that every tile it holds is out of date. The contour tiles of the same DEM (MAP-3) are
//! made from the same maker. Nothing here is written to the design.

use owd_format::design::contours::ContourCache;
use owd_format::design::pool_water::{DEPTH_STOPS, PoolWater};
use owd_format::design::structures::PoolWaterIn;
use owd_format::design::terrain_tile::TileMaker;
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
    /// The pool water of the maker's DEM (MAP-4): the flood mask, and what it was made from.
    water: Option<(PoolWater, PoolWaterIn)>,
    /// Counts the flood masks made: the `id` in [`TerrainTiles::pool_water`].
    water_id: u32,
}

impl TerrainTiles {
    /// The maker for `dem`, built now if the DEM is not the one the last maker was built for.
    /// `None` with no DEM.
    fn for_dem(&mut self, dem: Option<Arc<Dem>>) -> Result<Option<&mut TileMaker>, String> {
        let Some(dem) = dem else {
            self.maker = None;
            self.contours.clear();
            self.water = None;
            return Ok(None);
        };
        if !self
            .maker
            .as_ref()
            .is_some_and(|m| Arc::ptr_eq(m.dem(), &dem))
        {
            self.maker = None;
            self.contours.clear();
            self.water = None;
            self.maker = Some(TileMaker::new(dem).map_err(|e| e.to_string())?);
            self.id += 1;
        }
        Ok(self.maker.as_mut())
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
        let maker = self
            .for_dem(dem)?
            .ok_or_else(|| "there is no DEM in memory to cut terrain tiles from".to_string())?;
        let z = u8::try_from(z).map_err(|_| format!("there is no map tile {z}/{x}/{y}"))?;
        maker.tile(z, x, y).map_err(|e| e.to_string())
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
        self.for_dem(dem)?;
        let maker = self
            .maker
            .as_ref()
            .ok_or_else(|| "there is no DEM in memory to draw contours from".to_string())?;
        let z = u8::try_from(z).map_err(|_| format!("there is no map tile {z}/{x}/{y}"))?;
        self.contours
            .tile(maker, z, x, y, interval_m)
            .map_err(|e| e.to_string())
    }

    /// The pool water of `pools` on the DEM (MAP-4): the flood mask is made now unless the one
    /// in memory is of the same DEM and the same pools. Answers `null` with no DEM or no pools,
    /// else `{id, made, mask: {factor, width, height, cell_m, bytes, peak_bytes, flooded_cells},
    /// pools: [{number, level_m, flooded_ha, outside_river_ha}], depth_stops: [[depth_m, [r, g,
    /// b, a]], ...]}`. `id` changes with the mask: tiles of another number are out of date.
    /// `made` says whether this call made the mask.
    pub fn pool_water(
        &mut self,
        dem: Option<Arc<Dem>>,
        pools: Option<PoolWaterIn>,
    ) -> Result<Value, String> {
        let (Some(_), Some(pools)) = (self.for_dem(dem)?, pools) else {
            self.water = None;
            return Ok(Value::Null);
        };
        let made = !self.water.as_ref().is_some_and(|(_, of)| *of == pools);
        if made {
            self.water = None;
            let maker = self.maker.as_ref().expect("the maker was just made");
            let water = PoolWater::build(maker, &pools).map_err(|e| e.to_string())?;
            self.water = Some((water, pools));
            self.water_id += 1;
        }
        let (water, _) = self.water.as_ref().expect("the mask was just made");
        Ok(json!({
            "id": self.water_id,
            "made": made,
            "mask": water.stats(),
            "pools": water.areas(),
            "depth_stops": DEPTH_STOPS.iter().map(|(d, c)| json!([d, c])).collect::<Vec<_>>(),
        }))
    }

    /// The flood mask numbered `id` and the maker of its DEM. Refused when the mask in memory
    /// is another one: the tile asked for is out of date.
    fn water_of(&mut self, id: u32) -> Result<(&mut PoolWater, &TileMaker), String> {
        match (&mut self.water, &self.maker) {
            (Some((water, _)), Some(maker)) if id == self.water_id => Ok((water, maker)),
            _ => Err("the pool water asked for is out of date".to_string()),
        }
    }

    /// The water of tile `z/x/y` of flood mask `id`: a PNG coloured by depth, clear where dry.
    pub fn pool_water_tile(
        &mut self,
        id: u32,
        z: u32,
        x: u32,
        y: u32,
    ) -> Result<Arc<Vec<u8>>, String> {
        let (water, maker) = self.water_of(id)?;
        let z = u8::try_from(z).map_err(|_| format!("there is no map tile {z}/{x}/{y}"))?;
        water.depth_tile(maker, z, x, y).map_err(|e| e.to_string())
    }

    /// The terrain of tile `z/x/y` with the water of flood mask `id` on it: a Terrarium PNG
    /// whose height is the pool's level where there is water and the ground elsewhere.
    pub fn pool_surface_tile(
        &mut self,
        id: u32,
        z: u32,
        x: u32,
        y: u32,
    ) -> Result<Arc<Vec<u8>>, String> {
        let (water, maker) = self.water_of(id)?;
        let z = u8::try_from(z).map_err(|_| format!("there is no map tile {z}/{x}/{y}"))?;
        water
            .surface_tile(maker, z, x, y)
            .map_err(|e| e.to_string())
    }

    /// What a click reads at a longitude and latitude: `{pool, water_level_m, ground_m,
    /// depth_m}`, or `null` on dry ground.
    pub fn pool_water_at(&mut self, id: u32, lon: f64, lat: f64) -> Result<Value, String> {
        let (water, maker) = self.water_of(id)?;
        serde_json::to_value(water.at(maker, lon, lat)).map_err(|e| e.to_string())
    }
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

    /// A valley along the DEM's rows, lowest on row 25, and a pool at `level` along that row.
    fn valley() -> Arc<Dem> {
        Arc::new(Dem {
            data: (0..2500)
                .map(|i| 10.0 + ((i / 50) as f64 - 24.5).abs())
                .collect(),
            ..(*dem(0.0)).clone()
        })
    }

    fn pool(level: f64) -> PoolWaterIn {
        PoolWaterIn {
            line: vec![[-1.995, 5.0255], [-1.955, 5.0255]],
            chainage_m: vec![0.0, 4000.0],
            pools: vec![(1, 0.0, 4000.0, level)],
            polygon: Vec::new(),
        }
    }

    #[test]
    fn the_pool_water_is_made_once_for_a_dem_and_its_pools_and_old_tiles_are_refused() {
        let mut t = TerrainTiles::default();
        assert_eq!(t.pool_water(None, Some(pool(15.0))).unwrap(), Value::Null);
        assert_eq!(t.pool_water(Some(valley()), None).unwrap(), Value::Null);
        assert!(
            t.pool_water_tile(1, 12, 2025, 1990)
                .unwrap_err()
                .contains("out of date")
        );
        let a = valley();
        let first = t.pool_water(Some(a.clone()), Some(pool(15.0))).unwrap();
        assert_eq!((&first["id"], &first["made"]), (&json!(1), &json!(true)));
        assert_eq!(first["mask"]["factor"], 1);
        assert_eq!(first["pools"][0]["number"], 1);
        assert!(first["pools"][0]["flooded_ha"].as_f64().unwrap() > 0.0);
        assert_eq!(first["pools"][0]["outside_river_ha"], Value::Null);
        assert_eq!(first["depth_stops"][0], json!([0.0, DEPTH_STOPS[0].1]));
        // Asked again: the same mask. Its tiles: a picture, a terrain tile, a click.
        let again = t.pool_water(Some(a.clone()), Some(pool(15.0))).unwrap();
        assert_eq!((&again["id"], &again["made"]), (&json!(1), &json!(false)));
        let png = t.pool_water_tile(1, 12, 2025, 1990).unwrap();
        assert!(png.len() > 100);
        let heights = decode_png(&t.pool_surface_tile(1, 12, 2025, 1990).unwrap()).unwrap();
        assert!(
            heights.iter().any(|h| (h - 15.0).abs() < 0.01),
            "water at the pool's level"
        );
        let wet = t.pool_water_at(1, -1.975, 5.0255).unwrap();
        assert_eq!(wet["water_level_m"], 15.0);
        assert_eq!(wet["pool"], 1);
        assert_eq!(
            t.pool_water_at(1, -1.975, 5.045).unwrap(),
            Value::Null,
            "high ground"
        );
        // Another level: another mask, and a tile of the old one is refused.
        let higher = t.pool_water(Some(a.clone()), Some(pool(20.0))).unwrap();
        assert_eq!((&higher["id"], &higher["made"]), (&json!(2), &json!(true)));
        assert!(
            higher["pools"][0]["flooded_ha"].as_f64().unwrap()
                > first["pools"][0]["flooded_ha"].as_f64().unwrap()
        );
        assert!(
            t.pool_water_tile(1, 12, 2025, 1990)
                .unwrap_err()
                .contains("out of date")
        );
        assert!(t.pool_water_tile(2, 12, 2025, 1990).is_ok());
        // Another DEM: the mask goes with the old one.
        t.info(Some(dem(10.0))).unwrap();
        assert!(t.pool_water_tile(2, 12, 2025, 1990).is_err());
    }
}
