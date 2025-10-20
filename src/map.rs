use anyhow::{Result, anyhow};
use skia_safe::{OwnedCanvas, PathFillType};

use bincode::{Decode, Encode};
use geo_types::{LineString, Polygon, geometry::Geometry};
use mvt_reader::Reader;

use crate::{draw::DrawInstructions, vec::Vector};

mod cache;
pub use cache::MvtGetter;

const CACHE_PATH: &'static str = "./.cache";
const TILE_URL: &'static str = "https://vector.openstreetmap.org/shortbread_v1/{z}/{x}/{y}.mvt";
pub const TILE_SIZE: u32 = 256;

const LAYER_NAMES: &'static [&'static str] = &[
    "ocean",
    "water_polygons",
    "water_lines",
    "dam_lines",
    "dam_polygons",
    "pier_lines",
    "pier_polygons",
    "boundaries",
    "land",
    "sites",
    "buildings",
    "streets",
    "street_polygons",
    "bridges",
    "aerialways",
];

#[derive(Hash, Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct TileDescr {
    pub z: u32,
    pub x: u32,
    pub y: u32,
}

impl TileDescr {
    fn to_url(&self) -> String {
        TILE_URL
            .replace("{z}", &self.z.to_string())
            .replace("{x}", &self.x.to_string())
            .replace("{y}", &self.y.to_string())
    }

    fn to_file_name(&self) -> String {
        format!("{}_{}_{}.tile", self.z, self.x, self.y)
    }

    fn to_path(&self) -> String {
        format!("{CACHE_PATH}/{}", self.to_file_name())
    }

    pub fn valid(&self) -> bool {
        let n_tiles = 1 << self.z;
        self.x < n_tiles && self.y < n_tiles
    }
}

#[derive(Encode, Decode, Debug, Clone)]
pub struct Path(pub Vec<Vector>);

impl Path {
    pub fn draw(&self, instructions: &DrawInstructions, canvas: &mut OwnedCanvas) {
        let mut path = skia_safe::Path::new();
        if self.0.is_empty() {
            return;
        }
        let first = instructions.transform * self.0[0];
        path.move_to((first.x, first.y));
        for point in self.0.iter().skip(1) {
            let trans_point = instructions.transform * point;
            path.line_to((trans_point.x, trans_point.y));
        }
        path.close();
        if let Some(style) = instructions.path_style() {
            canvas.draw_path(&path, &style);
        }
    }

    fn get_signed_area_sum(&self) -> f32 {
        let n = self.0.len();
        if n < 3 {
            return 0.0;
        }

        let mut signed_area_sum = 0.0;

        for i in 0..n {
            let p_i = &self.0[i];
            let p_next = &self.0[(i + 1) % n];
            signed_area_sum += (p_i.x * p_next.y) - (p_next.x * p_i.y);
        }
        signed_area_sum
    }

    fn reverse(&mut self) {
        self.0.reverse();
    }
}

#[derive(Encode, Decode, Default, Debug, Clone)]
pub struct Area {
    pub outer: Vec<Path>,
    pub inner: Vec<Path>,
}

impl Area {
    pub fn draw(&self, instructions: &DrawInstructions, canvas: &mut OwnedCanvas) {
        let mut path = skia_safe::Path::new();
        path.set_fill_type(PathFillType::Winding);

        let mut build_contour = |path_data: &Path| {
            if path_data.0.is_empty() {
                return;
            }
            let first = instructions.transform * path_data.0[0];
            path.move_to((first.x, first.y));
            for point in path_data.0.iter().skip(1) {
                let trans_point = instructions.transform * point;
                path.line_to((trans_point.x, trans_point.y));
            }
            path.close();
        };

        for path_data in &self.outer {
            build_contour(path_data);
        }

        for path_data in &self.inner {
            build_contour(path_data);
        }

        if let Some(style) = instructions.area_style() {
            canvas.draw_path(&path, &style);
        }
    }

    /// enforce winding rules
    /// return true if winding rule was not followed
    pub fn enforce_winding(&mut self) -> bool {
        let mut had_flip = false;
        for path in &mut self.outer {
            if path.get_signed_area_sum() < 0.0 {
                path.reverse();
                had_flip = true;
            }
        }
        for path in &mut self.inner {
            if path.get_signed_area_sum() > 0.0 {
                path.reverse();
                had_flip = true;
            }
        }
        had_flip
    }
}
#[derive(Encode, Decode)]
pub struct MapData {
    pub descr: TileDescr,
    layers: Vec<Layer>,
}

impl MapData {
    pub fn get_layer(&self, id: &str) -> Option<&Layer> {
        for layer in &self.layers {
            if layer.id == id {
                return Some(layer);
            }
        }
        None
    }
}

impl MapData {
    pub fn from_reader(descr: TileDescr, reader: Reader) -> Result<Self> {
        let mut layers = Vec::new();
        for meta in reader
            .get_layer_metadata()
            .map_err(|_| anyhow!("could not get layer names"))?
        {
            if !LAYER_NAMES.contains(&&*meta.name) {
                continue;
            }
            let mut paths = Vec::new();
            let mut areas = Vec::new();

            for feat in reader
                .get_features(meta.layer_index)
                .map_err(|_| anyhow!("could not get layer names"))?
            {
                convert_geometry(feat.geometry, meta.extent as f32, &mut paths, &mut areas);
            }

            let mut rewound_area = false;

            for area in &mut areas {
                rewound_area |= area.enforce_winding();
            }

            if rewound_area {
                eprintln!("had to rewind area")
            }

            layers.push(Layer {
                id: meta.name,
                paths,
                areas,
            })
        }

        Ok(MapData { descr, layers })
    }
}

fn convert_polygon(polygon: Polygon<f32>, extent: f32, areas: &mut Vec<Area>) {
    areas.push(Area {
        outer: vec![Path(
            polygon
                .exterior()
                .coords()
                .map(|p| Vector::from(p) / extent)
                .collect(),
        )],
        inner: polygon
            .interiors()
            .iter()
            .map(|path| Path(path.coords().map(|p| Vector::from(p) / extent).collect()))
            .collect(),
    })
}

fn convert_path(path: LineString<f32>, extent: f32, paths: &mut Vec<Path>) {
    paths.push(Path(
        path.coords().map(|p| Vector::from(p) / extent).collect(),
    ))
}

fn convert_geometry(
    geometry: Geometry<f32>,
    extent: f32,
    paths: &mut Vec<Path>,
    areas: &mut Vec<Area>,
) {
    match geometry {
        Geometry::Line(line) => convert_path(line.into(), extent, paths),
        Geometry::LineString(path) => convert_path(path, extent, paths),
        Geometry::MultiLineString(multi_line_string) => {
            for path in multi_line_string.0 {
                convert_path(path, extent, paths);
            }
        }

        Geometry::Polygon(polygon) => convert_polygon(polygon, extent, areas),
        Geometry::MultiPolygon(multi_polygon) => {
            for polygon in multi_polygon.0 {
                convert_polygon(polygon, extent, areas);
            }
        }
        Geometry::Rect(rect) => convert_polygon(rect.to_polygon(), extent, areas),
        Geometry::Triangle(triangle) => convert_polygon(triangle.to_polygon(), extent, areas),

        Geometry::GeometryCollection(collection) => {
            for geom in collection {
                convert_geometry(geom, extent, paths, areas);
            }
        }

        Geometry::Point(_) => (),
        Geometry::MultiPoint(_) => (),
    }
}

#[derive(Encode, Decode)]
pub struct Layer {
    id: String,
    paths: Vec<Path>,
    areas: Vec<Area>,
}

impl Layer {
    pub fn draw(&self, canvas: &mut OwnedCanvas, instructions: DrawInstructions) {
        for path in &self.paths {
            path.draw(&instructions, canvas);
        }
        for area in &self.areas {
            area.draw(&instructions, canvas);
        }
    }
}
