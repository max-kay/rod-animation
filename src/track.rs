use std::{io, sync::LazyLock};

use anyhow::Result;
use chrono::NaiveDateTime;

use crate::{TRACK_PATH, lat_long_to_vec, vec::Vector};

pub fn get_tracks() -> Result<Vec<Track>> {
    let mut tracks = Vec::new();
    for file in std::fs::read_dir(TRACK_PATH)? {
        let path = file?.path();
        if path.extension().and_then(|s| s.to_str()) != Some("txt") {
            continue;
        }
        let name = path
            .iter()
            .last()
            .unwrap()
            .to_str()
            .unwrap()
            .split(".")
            .next()
            .unwrap()
            .to_string();
        tracks.push(Track::from_file(&name)?)
    }
    Ok(tracks)
}

pub const TIME_ZERO: LazyLock<NaiveDateTime> = LazyLock::new(|| {
    NaiveDateTime::parse_from_str("2025-04-14T19:00:00", "%Y-%m-%dT%H:%M:%S").unwrap()
});

// pub const TIME_END: LazyLock<NaiveDateTime> = LazyLock::new(|| {
//     NaiveDateTime::parse_from_str("2025-04-19T16:05:28", "%Y-%m-%dT%H:%M:%S").unwrap()
// });
// pub const TIME_END_S: LazyLock<u32> =
//     LazyLock::new(|| (*TIME_END - *TIME_ZERO).num_seconds() as u32);

pub struct TrackingPoint {
    pub time: u32,
    pub position: Vector,
}

pub struct Track {
    pub name: String,
    pub points: Vec<TrackingPoint>,
}

impl Track {
    pub fn from_file(name: &str) -> Result<Self> {
        let file = std::fs::File::open(format!("{TRACK_PATH}/{name}.txt"))?;
        let s = io::read_to_string(file)?;
        let mut points = Vec::new();
        for line in s.lines() {
            let mut split = line.split(",");
            let lat = split.next().unwrap().parse()?;
            let lon = split.next().unwrap().parse()?;
            let position = lat_long_to_vec(lat, lon);
            let time = (NaiveDateTime::parse_from_str(split.next().unwrap(), "%Y-%m-%dT%H:%M:%S")?
                - *TIME_ZERO)
                .num_seconds() as u32;

            points.push(TrackingPoint { time, position })
        }
        Ok(Self {
            name: name.to_string(),
            points,
        })
    }

    pub fn get_position(&self, time: u32) -> Option<Vector> {
        match self.points.binary_search_by_key(&time, |pt| pt.time) {
            Ok(idx) => Some(self.points[idx].position),
            Err(idx) => {
                if idx == 0 {
                    return Some(self.points[0].position);
                }
                if idx == self.points.len() {
                    return None;
                    // TODO: how long should they stay at end?
                }
                let t0 = self.points[idx - 1].time;
                let t1 = self.points[idx].time;
                let fraction = (time - t0) as f32 / (t1 - t0) as f32;
                let v0 = self.points[idx - 1].position;
                let v1 = self.points[idx].position;
                Some(v0 + (v1 - v0) * fraction)
            }
        }
    }
}
