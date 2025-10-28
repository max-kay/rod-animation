use std::{collections::HashMap, fs::File, path::PathBuf, sync::LazyLock, time::Instant};

use anyhow::{Result, anyhow};
use log::{info, trace};
use serde::{Deserialize, Serialize};
use skia_safe::{OwnedCanvas, PathFillType};

use geo_types::{LineString, Polygon, geometry::Geometry};
use mvt_reader::{Reader, feature::Value};

use crate::{
    CACHE_PATH, STYLE_PATH,
    draw::{DrawInstructions, LayerStyle},
    vec::{Transform, Vector},
};

mod cache;
pub use cache::MvtGetter;

const TILE_URL: &'static str = "https://vector.openstreetmap.org/shortbread_v1/{z}/{x}/{y}.mvt";
pub const TILE_SIZE: u32 = 2048 * 3;

#[derive(Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum MyValue {
    String(String),
    Float(f32),
    Double(f64),
    Int(i64),
    UInt(u64),
    SInt(i64),
    Bool(bool),
    Null,
}

impl From<Value> for MyValue {
    fn from(value: Value) -> Self {
        match value {
            Value::String(val) => Self::String(val),
            Value::Float(val) => Self::Float(val),
            Value::Double(val) => Self::Double(val),
            Value::Int(val) => Self::Int(val),
            Value::UInt(val) => Self::UInt(val),
            Value::SInt(val) => Self::SInt(val),
            Value::Bool(val) => Self::Bool(val),
            Value::Null => Self::Null,
        }
    }
}

impl Into<Value> for MyValue {
    fn into(self) -> Value {
        match self {
            Self::String(val) => Value::String(val),
            Self::Float(val) => Value::Float(val),
            Self::Double(val) => Value::Double(val),
            Self::Int(val) => Value::Int(val),
            Self::UInt(val) => Value::UInt(val),
            Self::SInt(val) => Value::SInt(val),
            Self::Bool(val) => Value::Bool(val),
            Self::Null => Value::Null,
        }
    }
}

#[derive(Hash, Debug, Clone, Copy, PartialEq, Eq)]
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
        format!("{}_{}_{}.mvt", self.z, self.x, self.y)
    }

    fn to_path(&self) -> PathBuf {
        CACHE_PATH.join(self.to_file_name())
    }

    pub fn valid(&self) -> bool {
        let n_tiles = 1 << self.z;
        self.x < n_tiles && self.y < n_tiles
    }
}

#[derive(Debug, Clone)]
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

pub struct Area {
    pub outer: Path,
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

        build_contour(&self.outer);

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
        if self.outer.get_signed_area_sum() < 0.0 {
            self.outer.reverse();
            had_flip = true;
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

pub struct MapData {
    pub descr: TileDescr,
    layers: Vec<Layer>,
}

impl MapData {
    pub fn get_layer(&self, layer_idx: u8) -> Option<&Layer> {
        for layer in &self.layers {
            if layer.id == layer_idx {
                return Some(layer);
            }
        }
        None
    }
}

#[derive(Deserialize)]
#[serde(transparent)]
pub struct Style(Vec<LayerSorter>);

impl Style {
    pub fn get_layer_idx(&self, name: &str) -> Option<u8> {
        self.0.iter().enumerate().find_map(|(i, l)| {
            if l.layer_name == name {
                Some(i as u8)
            } else {
                None
            }
        })
    }

    pub fn get_sorter(&self, idx: u8) -> &LayerSorter {
        &self.0[idx as usize]
    }

    pub fn max_layer_idx(&self) -> u8 {
        (self.0.len() - 1) as u8
    }

    pub fn retain_non_empty(&mut self) {
        self.0.retain(|sorter| !sorter.is_empty());
    }
}

#[derive(Deserialize)]
pub struct LayerSorter {
    layer_name: String,
    sub_types: Vec<TypeConditions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fall_back: Option<LayerStyle>,
}

impl LayerSorter {
    fn apply(&self, props: Option<&HashMap<String, Value>>, zoom: u32) -> Option<&LayerStyle> {
        if self.sub_types.is_empty() {
            return self.fall_back.as_ref();
        }
        let props = props?;
        for conditions in &self.sub_types {
            if conditions.apply(props, zoom) {
                return Some(&conditions.style);
            }
        }
        return None;
    }

    fn is_empty(&self) -> bool {
        for ty in &self.sub_types {
            if ty.style.fill.is_some() || ty.style.stroke.is_some() {
                return false;
            }
        }
        self.fall_back.is_none()
    }
}

/// This struct represent one type of displayable thing in the map.
#[derive(Deserialize)]
struct TypeConditions {
    conditions: Vec<Condition>,
    style: LayerStyle,
    #[serde(skip_serializing_if = "Option::is_none")]
    min_zoomlevel: Option<u32>,
}

/// this represents a statement which needs to be true for the layer to be displayed.
#[derive(Serialize, Deserialize)]
struct Condition {
    key: String,
    values: Vec<MyValue>,
    white_list: bool,
}

impl Condition {
    fn apply(&self, props: &HashMap<String, Value>) -> Option<bool> {
        if let Some(val) = props.get(&self.key) {
            let contained = self.values.contains(&MyValue::from(val.clone()));
            return Some(!(self.white_list ^ contained));
        }
        return None;
    }
}

impl TypeConditions {
    /// returns true if all inner statements are true
    pub fn apply(&self, props: &HashMap<String, Value>, zoom: u32) -> bool {
        if let Some(z) = self.min_zoomlevel
            && zoom < z
        {
            return false;
        }
        for filter in &self.conditions {
            if let Some(b) = filter.apply(props)
                && !b
            {
                return false;
            }
        }
        true
    }
}

pub static SORTERS: LazyLock<Style> = LazyLock::new(|| {
    let file = File::open(&*STYLE_PATH).expect("could not decode style");
    let mut sorter: Style = serde_json::from_reader(file).expect("could not decode style");
    sorter.retain_non_empty();
    sorter
});

impl MapData {
    pub fn from_reader(tile: TileDescr, reader: Reader) -> Result<Self> {
        let start = Instant::now();
        let mut layers = Vec::new();
        for meta in reader
            .get_layer_metadata()
            .map_err(|_| anyhow!("could not get layer names"))?
        {
            let layer_idx = SORTERS.get_layer_idx(&&*meta.name);
            if layer_idx.is_none() {
                continue;
            }
            let layer_idx = layer_idx.expect("checked above");

            let mut paths = Vec::new();
            let mut areas = Vec::new();

            for feat in reader
                .get_features(meta.layer_index)
                .map_err(|_| anyhow!("could not get layer names"))?
            {
                if let Some(typ) = SORTERS
                    .get_sorter(layer_idx)
                    .apply(feat.properties.as_ref(), tile.z)
                {
                    convert_geometry(
                        feat.geometry,
                        meta.extent as f32,
                        &mut paths,
                        &mut areas,
                        typ,
                    );
                }
            }

            let mut rewound_area = false;

            for area in &mut areas {
                rewound_area |= area.1.enforce_winding();
            }

            if rewound_area {
                info!("had to rewind area")
            }

            layers.push(Layer {
                id: layer_idx,
                paths,
                areas,
            })
        }
        trace!(
            "took {} ms to parse map data from mvt",
            start.elapsed().as_secs_f64() * 1000.0
        );
        layers.sort_by(|a, b| a.id.cmp(&b.id));

        Ok(MapData {
            descr: tile,
            layers,
        })
    }
}

fn convert_polygon<'a>(
    polygon: Polygon<f32>,
    extent: f32,
    areas: &mut Vec<(&'a LayerStyle, Area)>,
    typ: &'a LayerStyle,
) {
    areas.push((
        typ,
        Area {
            outer: Path(
                polygon
                    .exterior()
                    .coords()
                    .map(|p| Vector::from(p) / extent)
                    .collect(),
            ),
            inner: polygon
                .interiors()
                .iter()
                .map(|path| Path(path.coords().map(|p| Vector::from(p) / extent).collect()))
                .collect(),
        },
    ))
}

fn convert_path<'a>(
    path: LineString<f32>,
    extent: f32,
    paths: &mut Vec<(&'a LayerStyle, Path)>,
    typ: &'a LayerStyle,
) {
    paths.push((
        typ,
        Path(path.coords().map(|p| Vector::from(p) / extent).collect()),
    ))
}

fn convert_geometry<'a>(
    geometry: Geometry<f32>,
    extent: f32,
    paths: &mut Vec<(&'a LayerStyle, Path)>,
    areas: &mut Vec<(&'a LayerStyle, Area)>,
    typ: &'a LayerStyle,
) {
    match geometry {
        Geometry::Line(line) => convert_path(line.into(), extent, paths, typ),
        Geometry::LineString(path) => convert_path(path, extent, paths, typ),
        Geometry::MultiLineString(multi_line_string) => {
            for path in multi_line_string.0 {
                convert_path(path, extent, paths, typ);
            }
        }

        Geometry::Polygon(polygon) => convert_polygon(polygon, extent, areas, typ),
        Geometry::MultiPolygon(multi_polygon) => {
            for polygon in multi_polygon.0 {
                convert_polygon(polygon, extent, areas, typ);
            }
        }
        Geometry::Rect(rect) => convert_polygon(rect.to_polygon(), extent, areas, typ),
        Geometry::Triangle(triangle) => convert_polygon(triangle.to_polygon(), extent, areas, typ),

        Geometry::GeometryCollection(collection) => {
            for geom in collection {
                convert_geometry(geom, extent, paths, areas, typ);
            }
        }

        Geometry::Point(_) => (),
        Geometry::MultiPoint(_) => (),
    }
}

pub struct Layer {
    id: u8,
    paths: Vec<(&'static LayerStyle, Path)>,
    areas: Vec<(&'static LayerStyle, Area)>,
}

impl Layer {
    pub fn draw(&self, canvas: &mut OwnedCanvas, tile_to_screen: Transform, opacity: f32) {
        for (style, path) in &self.paths {
            path.draw(&style.to_draw_instructions(tile_to_screen, opacity), canvas);
        }
        for (style, area) in &self.areas {
            area.draw(&style.to_draw_instructions(tile_to_screen, opacity), canvas);
        }
    }
}
