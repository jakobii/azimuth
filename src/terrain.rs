//! Ground elevation from AWS Terrain Tiles (Mapzen "Terrarium" encoding).
//!
//! Tiles are 256×256 PNGs on the usual web-mercator grid; each pixel encodes
//! metres as `R * 256 + G + B / 256 - 32768`.

use std::collections::HashMap;
use std::sync::Arc;

/// Zoom for looking up ground under standing spots: ~110 m pixels at 45°N,
/// ~28 km tiles, so a day's path needs a few dozen tiles at most.
pub const GROUND_ZOOM: u8 = 10;
/// Zoom for locating a summit's height: ~27 m pixels.
pub const SUMMIT_ZOOM: u8 = 12;

const SIZE: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileId {
    pub z: u8,
    pub x: u32,
    pub y: u32,
}

impl TileId {
    pub fn url(self) -> String {
        format!(
            "https://s3.amazonaws.com/elevation-tiles-prod/terrarium/{}/{}/{}.png",
            self.z, self.x, self.y
        )
    }
}

/// Fractional web-mercator tile coordinates of a point.
fn tile_xy(lat: f64, lon: f64, z: u8) -> (f64, f64) {
    let n = f64::from(1u32 << z);
    let lat = lat.clamp(-85.05, 85.05).to_radians();
    let x = (lon + 180.0) / 360.0 * n;
    let y = (1.0 - lat.tan().asinh() / std::f64::consts::PI) / 2.0 * n;
    (x, y)
}

/// The tile containing a point, and the pixel within it.
pub fn locate(lat: f64, lon: f64, z: u8) -> (TileId, usize, usize) {
    let (x, y) = tile_xy(lat, lon, z);
    let max = (1u32 << z) - 1;
    let (tx, ty) = ((x.floor() as u32).min(max), (y.floor() as u32).min(max));
    let px = (((x - f64::from(tx)) * SIZE as f64) as usize).min(SIZE - 1);
    let py = (((y - f64::from(ty)) * SIZE as f64) as usize).min(SIZE - 1);
    (TileId { z, x: tx, y: ty }, px, py)
}

/// A decoded tile: row-major elevations in metres.
pub type Grid = Arc<[f32]>;

/// Decode a Terrarium PNG into elevations.
pub fn decode(png_bytes: &[u8]) -> Result<Grid, String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or("tile too large")?];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    if info.width as usize != SIZE || info.height as usize != SIZE {
        return Err(format!("unexpected tile size {}x{}", info.width, info.height));
    }
    let channels = match info.color_type {
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        other => return Err(format!("unexpected colour type {other:?}")),
    };
    let row = info.line_size;
    let mut out = Vec::with_capacity(SIZE * SIZE);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let p = &buf[y * row + x * channels..];
            out.push(f32::from(p[0]) * 256.0 + f32::from(p[1]) + f32::from(p[2]) / 256.0 - 32768.0);
        }
    }
    Ok(out.into())
}

#[derive(Clone, Debug)]
pub enum TileState {
    Loading,
    Ready(Grid),
    Failed,
}

/// Ground height at a point, as far as the loaded tiles know.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ground {
    Known(f64),
    /// The tile isn't loaded yet; fetch it and ask again.
    Pending(TileId),
    /// The tile couldn't be loaded (offline, or no coverage).
    Unknown,
}

/// Loaded terrain tiles.
#[derive(Clone, Debug, Default)]
pub struct Terrain {
    tiles: HashMap<TileId, TileState>,
}

impl Terrain {
    pub fn state(&self, id: TileId) -> Option<&TileState> {
        self.tiles.get(&id)
    }

    pub fn set(&mut self, id: TileId, state: TileState) {
        self.tiles.insert(id, state);
    }

    pub fn loading(&self) -> usize {
        self.tiles.values().filter(|s| matches!(s, TileState::Loading)).count()
    }

    /// Ground under a point at [`GROUND_ZOOM`].
    pub fn ground(&self, lat: f64, lon: f64) -> Ground {
        let (id, px, py) = locate(lat, lon, GROUND_ZOOM);
        match self.tiles.get(&id) {
            Some(TileState::Ready(g)) => Ground::Known(f64::from(g[py * SIZE + px])),
            Some(TileState::Failed) => Ground::Unknown,
            Some(TileState::Loading) | None => Ground::Pending(id),
        }
    }
}

/// Ground size of one tile pixel, in metres.
pub fn pixel_m(lat: f64, z: u8) -> f64 {
    156_543.034 * lat.to_radians().cos() / f64::from(1u32 << z)
}

/// Centre of a tile pixel.
pub fn pixel_latlon(id: TileId, px: usize, py: usize) -> (f64, f64) {
    let n = f64::from(1u32 << id.z);
    let x = f64::from(id.x) + (px as f64 + 0.5) / SIZE as f64;
    let y = f64::from(id.y) + (py as f64 + 0.5) / SIZE as f64;
    let lon = x / n * 360.0 - 180.0;
    let lat = (std::f64::consts::PI * (1.0 - 2.0 * y / n)).sinh().atan().to_degrees();
    (lat, lon)
}

/// Highest pixel within `radius` pixels of (`px`, `py`), for snapping a tapped
/// point to the summit it was aimed at: `(elevation, px, py)`. Searches a
/// circle, within the one tile.
pub fn local_max(grid: &[f32], px: usize, py: usize, radius: usize) -> (f64, usize, usize) {
    let (x0, x1) = (px.saturating_sub(radius), (px + radius).min(SIZE - 1));
    let (y0, y1) = (py.saturating_sub(radius), (py + radius).min(SIZE - 1));
    let r2 = (radius * radius) as isize;
    let mut best = (f32::MIN, px, py);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (dx, dy) = (x as isize - px as isize, y as isize - py as isize);
            if dx * dx + dy * dy <= r2 && grid[y * SIZE + x] > best.0 {
                best = (grid[y * SIZE + x], x, y);
            }
        }
    }
    (f64::from(best.0), best.1, best.2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locates_mount_hood_tile() {
        // Verified against the live tile service: z12/663/1467 contains Hood.
        let (id, _, _) = locate(45.37362, -121.69591, 12);
        assert_eq!(id, TileId { z: 12, x: 663, y: 1467 });
    }

    #[test]
    fn decodes_terrarium_encoding() {
        // Build a 256×256 RGB PNG whose every pixel encodes 1234.5 m.
        let v = 1234.5_f32 + 32768.0;
        let (r, g) = ((v / 256.0).floor(), v.floor() % 256.0);
        let b = (v.fract() * 256.0).round();
        let px = [r as u8, g as u8, b as u8];
        let mut png_bytes = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut png_bytes, 256, 256);
            enc.set_color(png::ColorType::Rgb);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().unwrap();
            w.write_image_data(&px.repeat(256 * 256)).unwrap();
        }
        let grid = decode(&png_bytes).unwrap();
        assert_eq!(grid.len(), 256 * 256);
        assert!((grid[12345] - 1234.5).abs() < 0.01, "{}", grid[12345]);
    }

    #[test]
    fn pixel_latlon_inverts_locate() {
        let (id, px, py) = locate(45.37362, -121.69591, SUMMIT_ZOOM);
        let (lat, lon) = pixel_latlon(id, px, py);
        // Within one ~27 m pixel.
        assert!((lat - 45.37362).abs() < 0.0003 && (lon + 121.69591).abs() < 0.0004, "{lat}, {lon}");
    }

    #[test]
    fn local_max_finds_nearby_peak_within_radius() {
        let mut g = vec![100.0_f32; SIZE * SIZE];
        g[10 * SIZE + 13] = 900.0; // 3 px right of (10, 10)
        g[10 * SIZE + 40] = 2000.0; // far outside the radius
        assert_eq!(local_max(&g, 10, 10, 5), (900.0, 13, 10));
    }

    #[test]
    fn ground_reports_pending_then_known() {
        let mut t = Terrain::default();
        let (id, _, _) = locate(45.4, -121.9, GROUND_ZOOM);
        assert_eq!(t.ground(45.4, -121.9), Ground::Pending(id));
        t.set(id, TileState::Ready(vec![812.0; SIZE * SIZE].into()));
        assert_eq!(t.ground(45.4, -121.9), Ground::Known(812.0));
        t.set(id, TileState::Failed);
        assert_eq!(t.ground(45.4, -121.9), Ground::Unknown);
    }
}
