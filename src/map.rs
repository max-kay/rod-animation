use anyhow::{Result, anyhow};
use bincode::{Decode, Encode};
use geo_types::{Polygon, geometry::Geometry};
use mvt_reader::Reader;
use skia_safe::{OwnedCanvas, PathFillType};

use crate::{Area, Path, Transform, Vector, draw::DrawInstructions};

mod cache;
pub use cache::MvtGetter;

const CACHE_PATH: &'static str = "./.cache";
const TILE_URL: &'static str = "https://vector.openstreetmap.org/shortbread_v1/{z}/{x}/{y}.mvt";
pub const TILE_SIZE: u32 = 4096;

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
    pub x: i32,
    pub y: i32,
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

    pub fn tile_to_world(&self) -> Transform {
        Transform::new(
            2_f32.powi(-(self.z as i32)),
            Vector::new(self.x as f32, self.y as f32),
        )
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
        for (name, idx) in reader
            .get_layer_metadata()
            .map_err(|_| anyhow!("could not get layer names"))?
            .iter()
            .filter_map(|meta| {
                if LAYER_NAMES.contains(&&*meta.name) {
                    println!("extent{}", meta.extent);
                    Some((meta.name.clone(), meta.layer_index))
                } else {
                    None
                }
            })
        {
            let mut paths = Vec::new();
            let mut areas = Vec::new();

            for feat in reader
                .get_features(idx)
                .map_err(|_| anyhow!("could not get layer names"))?
            {
                convert_geometry(feat.geometry, &mut paths, &mut areas);
            }
            areas
                .iter_mut()
                .for_each(|a: &mut Area| a.enforce_winding());

            layers.push(Layer {
                id: name,
                paths,
                areas,
            })
        }

        Ok(MapData { descr, layers })
    }
}

fn convert_polygon(polygon: Polygon<f32>, areas: &mut Vec<Area>) {
    areas.push(Area {
        outer: vec![Path(
            polygon.exterior().coords().map(|p| p.into()).collect(),
        )],
        inner: polygon
            .interiors()
            .iter()
            .map(|path| Path(path.coords().map(|p| p.into()).collect()))
            .collect(),
    })
}

fn convert_geometry(geometry: Geometry<f32>, paths: &mut Vec<Path>, areas: &mut Vec<Area>) {
    match geometry {
        Geometry::Line(line) => paths.push(Path(vec![line.start.into(), line.end.into()])),
        Geometry::LineString(path) => paths.push(Path(path.coords().map(|p| p.into()).collect())),
        Geometry::MultiLineString(multi_line_string) => {
            for path in multi_line_string.0 {
                paths.push(Path(path.coords().map(|p| p.into()).collect()))
            }
        }

        Geometry::Polygon(polygon) => convert_polygon(polygon, areas),
        Geometry::MultiPolygon(multi_polygon) => {
            for polygon in multi_polygon.0 {
                convert_polygon(polygon, areas);
            }
        }
        Geometry::Rect(rect) => convert_polygon(rect.to_polygon(), areas),
        Geometry::Triangle(triangle) => convert_polygon(triangle.to_polygon(), areas),

        Geometry::GeometryCollection(collection) => {
            for geom in collection {
                convert_geometry(geom, paths, areas);
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
    fn draw_area(area: &Area, instructions: &DrawInstructions, canvas: &mut OwnedCanvas) {
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

        for path_data in &area.outer {
            build_contour(path_data);
        }

        for path_data in &area.inner {
            build_contour(path_data);
        }

        if let Some(style) = instructions.area_style() {
            canvas.draw_path(&path, &style);
        }
    }

    fn draw_path(path_data: &Path, instructions: &DrawInstructions, canvas: &mut OwnedCanvas) {
        let mut path = skia_safe::Path::new();
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
        if let Some(style) = instructions.path_style() {
            canvas.draw_path(&path, &style);
        }
    }

    pub fn draw(&self, canvas: &mut OwnedCanvas, instructions: DrawInstructions) {
        for path in &self.paths {
            Self::draw_path(path, &instructions, canvas);
        }
        for area in &self.areas {
            Self::draw_area(area, &instructions, canvas);
        }
    }
}
