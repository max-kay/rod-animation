use std::{
    collections::{HashMap, HashSet},
    f32::consts::{FRAC_PI_4, PI, TAU},
    io::Read as _,
    sync::{LazyLock, RwLock},
    time::Instant,
};

use anyhow::{Result, anyhow};

mod draw;
mod map;
mod track;
mod vec;

use draw::{ScenePos, Style};
use map::MvtGetter;
use skia_safe::Image;
use track::Track;
use vec::{Transform, Vector};

use crate::{draw::Renderable, map::TileDescr, track::get_tracks};

const WIDTH: usize = 1920;
const HEIGHT: usize = 1080;
const FRAME_RATE: f32 = 30.0;

const OUT_PATH: &'static str = "./out";
const PINS_PATH: &'static str = "./pins";
const TRACK_PATH: &'static str = "./tracks";

struct World {
    map: &'static RwLock<MvtGetter>,
    style: Style,
    tracks: Vec<Track>,
    pins: HashMap<String, Image>,
}

const FADE_WIDTH: f32 = 0.1;
const FADE_OFFSET: f32 = 0.5;
const FADE_MIN: f32 = FADE_OFFSET - FADE_WIDTH;
const FADE_MAX: f32 = FADE_OFFSET + FADE_WIDTH;

pub fn fade_function(x: f32) -> f32 {
    let x = x - FADE_OFFSET;
    assert!(
        -FADE_WIDTH <= x && x <= FADE_WIDTH,
        "fade function used outside of its designed interval"
    );
    -1.0 / 4.0 * x.powi(3) / FADE_WIDTH.powi(3) + 3.0 / 4.0 * x / FADE_WIDTH + 0.5
}

/// Takes latiude and longitude in degrees and returns world coordinates
pub fn lat_long_to_vec(lat: f32, lon: f32) -> Vector {
    Vector::new(
        0.5 + lon / 360.0,
        (PI - (FRAC_PI_4 - lat.to_radians() / 2.0).tan().ln()) / TAU,
    )
}

fn get_pins() -> Result<HashMap<String, Image>> {
    let mut pins = HashMap::new();
    for file in std::fs::read_dir(PINS_PATH)? {
        let path = file?.path();
        match path.extension() {
            Some(val) if val == "png" => (),
            _ => continue,
        }
        let name = path
            .iter()
            .last()
            .unwrap()
            .to_string_lossy()
            .split('.')
            .next()
            .unwrap()
            .to_owned();
        let mut file = std::fs::File::open(&path)?;
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer)?;

        let data = skia_safe::Data::new_copy(&buffer);

        let image = Image::from_encoded(data).ok_or(anyhow!("Failed to decode image: {}", name))?;

        pins.insert(name, image);
    }
    Ok(pins)
}

impl World {
    pub fn new() -> Self {
        LazyLock::force(&MAP_DATA);
        let this = World {
            map: &MAP_DATA,
            style: Style::default(),
            tracks: get_tracks().expect("could not load tracks"),
            pins: get_pins().expect("could not load pins"),
        };

        let names: HashSet<_> = this.tracks.iter().map(|val| val.name.to_string()).collect();
        assert!(names.len() <= 7, "to many tracks found");
        assert!(this.pins.len() <= 7, "to many pins found");
        for val in &[
            "Clarissa", "Luca", "Ivo", "Takashi", "Marc", "Louis", "Flavio",
        ] {
            if !names.contains(*val) {
                panic!("No track found for {}", val);
            }
            if !this.pins.contains_key(*val) {
                panic!("No pin found for {}", val);
            }
        }
        println!("World is initialized");
        this
    }
}

pub enum OneOrTwo<T> {
    One(T),
    Two(T, T),
}

impl<T> OneOrTwo<T> {
    pub fn map<S, F: Fn(T) -> S>(self, func: F) -> OneOrTwo<S> {
        match self {
            OneOrTwo::One(val) => OneOrTwo::One(func(val)),
            OneOrTwo::Two(a, b) => OneOrTwo::Two(func(a), func(b)),
        }
    }
}

impl World {
    pub fn get_tiles_at(&self, scene: ScenePos) -> OneOrTwo<Vec<TileDescr>> {
        let floor_zoom = scene.zoom.floor();
        let frac_zoom = scene.zoom - floor_zoom;
        match frac_zoom {
            0.0..=FADE_MIN => OneOrTwo::One(self.get_tiles_fixed(scene, floor_zoom as u32)),
            FADE_MIN..=FADE_MAX => OneOrTwo::Two(
                self.get_tiles_fixed(scene, floor_zoom as u32),
                self.get_tiles_fixed(scene, floor_zoom as u32),
            ),
            FADE_MAX..=1.0 => OneOrTwo::One(self.get_tiles_fixed(scene, floor_zoom as u32 + 1)),
            _ => unreachable!("all values of the fractionals are covered"),
        }
    }

    pub fn get_tiles_fixed(&self, scene: ScenePos, zoom: u32) -> Vec<TileDescr> {
        let min_x = (scene.world_min().x * 2f32.powi(zoom as i32).floor()) as u32;
        let min_y = (scene.world_min().y * 2f32.powi(zoom as i32).floor()) as u32;
        let max_x = (scene.world_max().x * 2f32.powi(zoom as i32).floor()) as u32;
        let max_y = (scene.world_max().y * 2f32.powi(zoom as i32).floor()) as u32;
        let mut tiles = Vec::new();
        for x in min_x..=max_x {
            for y in min_y..=max_y {
                let tile = TileDescr { z: zoom, x, y };
                if !tile.valid() {
                    eprintln!("encountered invalid tile: {:?}", tile);
                    continue;
                }
                tiles.push(tile)
            }
        }
        tiles
    }

    pub fn load_tiles_at(&self, scene: ScenePos) -> Result<()> {
        let mut lock = self.map.write().expect("RwLock not poisoned");

        match self.get_tiles_at(scene) {
            OneOrTwo::One(tiles) => lock.load_tiles(&tiles)?,
            OneOrTwo::Two(a, b) => {
                lock.load_tiles(&a)?;
                lock.load_tiles(&b)?;
            }
        }
        drop(lock);
        Ok(())
    }
}

static WORLD: LazyLock<World> = LazyLock::new(World::new);
static MAP_DATA: LazyLock<RwLock<MvtGetter>> =
    LazyLock::new(|| RwLock::new(MvtGetter::new().expect("failed to initialize MvtGetter")));

fn main() {
    let start = Instant::now();
    LazyLock::force(&WORLD);
    println!(
        "took {}s to initialize world",
        start.elapsed().as_secs_f32()
    );
    let start = Instant::now();
    Renderable::Image {
        center: lat_long_to_vec(45.024183710835956, 4.765212115427184),
        zoomlevel: 7.0,
        time: 60 * 60 * 12 * 3,
        people: Vec::new(),
        pin_height: 250.0,
    }
    .make_file()
    .unwrap();

    // Renderable::Fixed {
    //     center: Position::LatLong(LatLong::from_float(45.024183710835956, 4.765212115427184)),
    //     zoomlevel: 1.0,
    //     start: TimeStamp::Time(TIME_ZERO.clone()),
    //     end: TimeStamp::Time(TIME_END.clone()),
    //     duration_s: 5.0,
    //     people: Vec::new(),
    //     pin_height: 250.0,
    // }
    // .make_file()
    // .unwrap();
    Renderable::Fixed {
        center: lat_long_to_vec(45.18838548473186, 5.719852490686185),
        zoomlevel: 10.0,
        start: 60 * 60 * 15,
        end: 60 * 60 * 48,
        duration_s: 10.0,
        people: Vec::new(),
        pin_height: 250.0,
    }
    .make_file()
    .unwrap();
    println!("took {}s", start.elapsed().as_secs_f32());
}
