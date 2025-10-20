use anyhow::{Result, anyhow};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
};

use super::{CACHE_PATH, MapData, TileDescr};

pub struct MvtGetter {
    pub file_cache: HashSet<TileDescr>,
    pub mem_cache: HashMap<TileDescr, MapData>,
}

impl MvtGetter {
    pub fn new() -> Result<Self> {
        let mut file_cache = HashSet::new();
        if !fs::exists(CACHE_PATH)? {
            fs::create_dir(CACHE_PATH)?;
        }
        for entry in fs::read_dir(CACHE_PATH)? {
            if let Ok(entry) = entry {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) != Some("tile") {
                    continue;
                }
                let mut split = path
                    .iter()
                    .last()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .split(".")
                    .next()
                    .unwrap()
                    .split("_");
                file_cache.insert(TileDescr {
                    z: split.next().unwrap().parse()?,
                    x: split.next().unwrap().parse()?,
                    y: split.next().unwrap().parse()?,
                });
            }
        }
        Ok(Self {
            file_cache,
            mem_cache: HashMap::new(),
        })
    }
}

impl MvtGetter {
    pub fn get_tile(&self, tile: TileDescr) -> Option<&MapData> {
        self.mem_cache.get(&tile)
    }

    fn try_load_from_file(&mut self, tile: TileDescr) -> Result<()> {
        let data = bincode::decode_from_std_read(
            &mut File::open(&tile.to_path())?,
            bincode::config::standard(),
        )?;
        self.mem_cache.insert(tile, data);
        return Ok(());
    }

    pub fn load_tile(&mut self, tile: TileDescr) -> Result<()> {
        if self.mem_cache.contains_key(&tile) {
            return Ok(());
        }

        if self.file_cache.contains(&tile) {
            match self.try_load_from_file(tile) {
                Ok(_) => return Ok(()),
                Err(_) => {
                    self.file_cache.remove(&tile);
                }
            }
        }

        // return Ok(());

        let response = reqwest::blocking::get(dbg!(&tile.to_url()))?;
        let bytes = response.bytes()?;
        let buf = bytes.to_vec();
        let data = MapData::from_reader(
            tile,
            mvt_reader::Reader::new(buf).map_err(|_| anyhow!("could not creat Mvt Reader"))?,
        )?;
        let mut file = File::create(&tile.to_path())?;
        bincode::encode_into_std_write(&data, &mut file, bincode::config::standard())?;
        self.file_cache.insert(tile);
        self.mem_cache.insert(tile, data);
        Ok(())
    }

    pub fn load_tiles(&mut self, tiles: &[TileDescr]) -> Result<()> {
        for tile in tiles {
            self.load_tile(*tile)?
        }
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn cache() {
        let mut getter = MvtGetter::new().unwrap();
        let tile = TileDescr { z: 7, x: 66, y: 44 };
        getter.load_tile(tile).expect("could not get tile");
        drop(getter);

        let new_getter = MvtGetter::new().expect("could not create cached getter");
        assert!(new_getter.file_cache.contains(&tile));
    }
}
