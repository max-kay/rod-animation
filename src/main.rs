use std::{
    collections::HashMap,
    f32::consts::{FRAC_PI_4, PI, TAU},
    fs::{self, File, read_dir},
    io::{self},
    path::PathBuf,
    sync::{LazyLock, Mutex, RwLock},
    time::Instant,
};

use anyhow::Result;
use log::{error, info};

mod draw;
mod map;
mod track;
mod vec;

use draw::ScenePos;
use map::MvtGetter;
use sha2::{Digest, Sha256};
use track::Track;
use vec::{Transform, Vector};

use crate::{
    draw::{Pin, Renderable, parse},
    map::TileDescr,
};

const WIDTH: usize = 1920 * 2;
const HEIGHT: usize = 1080 * 2;
const FRAME_RATE: f32 = 30.0;
//
// My default path (when no custom flag is passed)
#[cfg(not(feature = "luca_build"))]
const BASE_RES_PATH: LazyLock<PathBuf> = LazyLock::new(|| "./res".into());

// Friend's custom path (when the 'friend_build' flag is passed)
#[cfg(feature = "luca_build")]
const BASE_RES_PATH: LazyLock<PathBuf> = LazyLock::new(|| "/Users/luca/rod".into());

const IN_PATH: LazyLock<PathBuf> = LazyLock::new(|| {
    std::fs::read_to_string("./res/in")
        .unwrap()
        .trim()
        .to_string()
        .into()
});
const OUT_PATH: LazyLock<PathBuf> = LazyLock::new(|| {
    std::fs::read_to_string("./res/out")
        .unwrap()
        .trim()
        .to_string()
        .into()
});

const PINS_PATH: LazyLock<PathBuf> = LazyLock::new(|| {
    let mut path = BASE_RES_PATH.clone();
    path.push("pins");
    path
});
const TRACK_PATH: LazyLock<PathBuf> = LazyLock::new(|| {
    let mut path = BASE_RES_PATH.clone();
    path.push("tracks");
    path
});
const CACHE_PATH: LazyLock<PathBuf> = LazyLock::new(|| {
    let mut path = BASE_RES_PATH.clone();
    path.push("cache");
    path
});
const STYLE_PATH: LazyLock<PathBuf> = LazyLock::new(|| {
    let mut path = BASE_RES_PATH.clone();
    path.push("style.json");
    path
});
const HASHES_PATH: LazyLock<PathBuf> = LazyLock::new(|| {
    let mut path = BASE_RES_PATH.clone();
    path.push("hashes.json");
    path
});

const PEOPLE: &'static [&'static str] = &[
    "Clarissa", "Luca", "Flavio", "Louis", "Takashi", "Marc", "Ivo",
];

const FADE_MIN: f32 = 0.25;
const FADE_MID: f32 = 0.5;
const FADE_MAX: f32 = 0.75;

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

    pub fn one(self) -> Option<T> {
        match self {
            OneOrTwo::One(val) => Some(val),
            OneOrTwo::Two(_, _) => None,
        }
    }

    pub fn two(self) -> Option<(T, T)> {
        match self {
            OneOrTwo::One(_) => None,
            OneOrTwo::Two(a, b) => Some((a, b)),
        }
    }
}

impl<T> OneOrTwo<Option<T>> {
    pub fn as_opt(self) -> Option<OneOrTwo<T>> {
        match self {
            OneOrTwo::One(Some(val)) => Some(OneOrTwo::One(val)),
            OneOrTwo::Two(Some(a), Some(b)) => Some(OneOrTwo::Two(a, b)),
            _ => None,
        }
    }
}

impl<T: Clone> OneOrTwo<T> {
    pub fn splat(self) -> (T, T) {
        match self {
            OneOrTwo::One(val) => (val.clone(), val),
            OneOrTwo::Two(a, b) => (a, b),
        }
    }
}

fn smooth_step(x: f32, edge0: f32, edge1: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - t * 2.0)
}

fn smoother_step(x: f32, edge0: f32, edge1: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    return t * t * t * (t * (6.0 * t - 15.0) + 10.0);
}

pub fn fade_in_function(x: f32) -> f32 {
    assert!(
        FADE_MIN <= x && x <= FADE_MAX,
        "fade in function used outside of its designed interval\n x was {x}"
    );
    if x > FADE_MID {
        return 1.0;
    }
    smooth_step(x, FADE_MIN, FADE_MID)
}

pub fn fade_out_function(x: f32) -> f32 {
    assert!(
        FADE_MIN <= x && x <= FADE_MAX,
        "fade in function used outside of its designed interval\n x was {x}"
    );
    if x < FADE_MID {
        return 1.0;
    }
    1.0 - smooth_step(x, FADE_MID, FADE_MAX)
}

/// Takes latiude and longitude in degrees and returns world coordinates
pub fn lat_long_to_vec(lat: f32, lon: f32) -> Vector {
    Vector::new(
        0.5 + lon / 360.0,
        (PI - (FRAC_PI_4 + lat.to_radians() / 2.0).tan().ln()) / TAU,
    )
}

struct World {
    map: &'static RwLock<MvtGetter>,
    tracks: HashMap<String, Track>,
    checkpoints: HashMap<String, (Vector, Pin)>,
}

impl World {
    pub fn new() -> Self {
        World {
            map: &MAP_DATA,
            tracks: track::get_tracks().expect("could not load tracks"),
            checkpoints: track::get_checkpoints().expect("could not load checkpoints"),
        }
    }
}

impl World {
    pub fn get_tiles_at(&self, scene: ScenePos) -> OneOrTwo<Vec<TileDescr>> {
        let floor_zoom = scene.zoom.floor();
        let frac_zoom = scene.zoom - floor_zoom;
        if floor_zoom as u32 >= 14 {
            return OneOrTwo::One(self.get_tiles_fixed(scene, 14));
        }
        match frac_zoom {
            0.0..=FADE_MIN => OneOrTwo::One(self.get_tiles_fixed(scene, floor_zoom as u32)),
            FADE_MIN..=FADE_MAX => OneOrTwo::Two(
                self.get_tiles_fixed(scene, floor_zoom as u32),
                self.get_tiles_fixed(scene, floor_zoom as u32 + 1),
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
                    error!("encountered invalid tile: {:?}", tile);
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

impl World {
    pub fn get_track(&self, name: &str) -> Option<&Track> {
        self.tracks.get(name)
    }
}

static WORLD: LazyLock<World> = LazyLock::new(World::new);

static MAP_DATA: LazyLock<RwLock<MvtGetter>> =
    LazyLock::new(|| RwLock::new(MvtGetter::new().expect("failed to initialize MvtGetter")));

static FILE_HASHES: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| match File::open(&*HASHES_PATH) {
        Ok(file) => Mutex::new(serde_json::from_reader(file).expect("could not load file hashes")),
        Err(_) => Mutex::new(HashMap::new()),
    });

fn hash_file(path: impl AsRef<std::path::Path>) -> String {
    let buf = fs::read(path).expect("path is always valid");
    let hash = Sha256::digest(&buf);
    hex::encode(hash)
}

fn process_renderable(path: std::path::PathBuf, renderable: Box<dyn Renderable>) {
    let name = renderable.name().to_string();
    let start = Instant::now();
    info!("rendering {}", name);
    match renderable.make_file() {
        Ok(_) => {
            info!(
                "took {}s to render: {}",
                start.elapsed().as_secs_f32(),
                name
            );
            (*FILE_HASHES.lock().expect("not poisoned"))
                .insert(path.to_string_lossy().into_owned(), hash_file(&path));
        }
        Err(err) => error!("could not render: {} reason: {}", name, err),
    };
}

fn init() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp(None)
        .init();

    let start = Instant::now();
    LazyLock::force(&MAP_DATA);
    LazyLock::force(&WORLD);
    LazyLock::force(&FILE_HASHES);
    LazyLock::force(&map::SORTERS);
    LazyLock::force(&OUT_PATH);
    assert!(
        std::path::Path::exists(&OUT_PATH.as_ref()),
        "out path not found"
    );
    LazyLock::force(&IN_PATH);
    assert!(
        std::path::Path::exists(&IN_PATH.as_ref()),
        "in path not found"
    );

    info!(
        "took {}s to initialize world",
        start.elapsed().as_secs_f32()
    );
}

fn main() {
    init();
    info!("ready");
    loop {
        let mut input = String::new();
        match io::stdin().read_line(&mut input) {
            Ok(_) => {
                if input == "end\n" {
                    info!("program beendet");
                    break;
                }
                for file in read_dir(&*IN_PATH).expect("could not read input dir") {
                    if file.is_err() {
                        continue;
                    }
                    let path = file.expect("checked above").path();
                    if !(path.extension().and_then(|s| s.to_str()) == Some("txt")) {
                        continue;
                    }
                    info!("reading file: {:?}", path.iter().last().unwrap());
                    match parse::from_path(&path) {
                        Ok(r) => {
                            {
                                if let Some(val) = FILE_HASHES
                                    .lock()
                                    .expect("not poisoned")
                                    .get(&*path.to_string_lossy())
                                    && &*hash_file(&path) == val
                                    && std::path::Path::new(&r.get_file_name()).exists()
                                {
                                    continue;
                                }
                            }
                            process_renderable(path, r)
                        }
                        Err(err) => {
                            error!("could not read file: {}", err);
                            continue;
                        }
                    }
                }
            }
            Err(error) => {
                error!("An error occurred while reading input: {}", error);
                break;
            }
        }

        info!("finished loop waiting for enter");
        info!("write 'end' to quit")
    }

    let mut file = File::create(&*HASHES_PATH).unwrap();
    serde_json::to_writer_pretty::<_, HashMap<String, String>>(
        &mut file,
        &*FILE_HASHES.lock().unwrap(),
    )
    .unwrap();
}
