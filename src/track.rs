use std::{collections::HashMap, io, path, sync::LazyLock};

use anyhow::{Result, anyhow};
use chrono::NaiveDateTime;

use crate::{PEOPLE, TRACK_PATH, draw::Pin, lat_long_to_vec, vec::Vector};

pub fn get_checkpoints() -> Result<HashMap<String, (Vector, Pin)>> {
    [
        ("Grenoble", lat_long_to_vec(45.242976, 5.644920)),
        ("Avignon", lat_long_to_vec(43.921494, 4.779126)),
        ("Perpignan", lat_long_to_vec(42.647380, 2.894101)),
        (
            "Barcelona",
            lat_long_to_vec(41.37875146251132, 2.1690145515198394),
        ),
    ]
    .iter()
    .map(|(name, pos)| match Pin::load(name, 1888.0, 4672.0) {
        Ok(pin) => Ok((name.to_string(), (*pos, pin))),
        Err(_) => Err(anyhow!("could not get pin")),
    })
    .collect()
}

pub fn get_tracks() -> Result<HashMap<String, Track>> {
    let mut tracks = HashMap::new();
    for name in PEOPLE {
        let pin = Pin::load(name, 1731.0, 5488.0)?;
        let path = TRACK_PATH.join(format!("{name}.txt"));
        tracks.insert(name.to_string(), Track::from_file(&path, pin)?);
    }
    Ok(tracks)
}

pub const TIME_ZERO: LazyLock<NaiveDateTime> = LazyLock::new(|| {
    NaiveDateTime::parse_from_str("2025-04-14T00:00:00", "%Y-%m-%dT%H:%M:%S")
        .expect("is valid format")
});

pub struct TrackingPoint {
    pub time: u32,
    pub position: Vector,
}

pub struct Track {
    pub points: Vec<TrackingPoint>,
    pub pin: Pin,
}

impl Track {
    pub fn from_file(path: impl AsRef<path::Path>, pin: Pin) -> Result<Self> {
        let file = std::fs::File::open(path)?;
        let s = io::read_to_string(file)?;
        let mut points = Vec::new();
        for line in s.lines() {
            let mut split = line.split(",");
            let lat = split.next().expect("tracks have valid format").parse()?;
            let lon = split.next().expect("tracks have valid format").parse()?;
            let position = lat_long_to_vec(lat, lon);
            let time = (NaiveDateTime::parse_from_str(
                split.next().expect("tracks have valid format"),
                "%Y-%m-%dT%H:%M:%S",
            )? - *TIME_ZERO)
                .num_seconds() as u32;

            points.push(TrackingPoint { time, position })
        }
        Ok(Self { pin, points })
    }

    pub fn get_position(&self, time: u32) -> Option<Vector> {
        match self.points.binary_search_by_key(&time, |pt| pt.time) {
            Ok(idx) => Some(self.points[idx].position),
            Err(idx) => {
                if idx == 0 {
                    return Some(self.points[0].position);
                }
                if idx == self.points.len() {
                    let last = self.points.last().expect("len is allways > 0");
                    if time - last.time < 60 * 60 * 5 {
                        return Some(last.position);
                    } else {
                        return None;
                    }
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

    pub fn valid_times(&self) -> String {
        let t_0 = chrono::Duration::seconds(self.points[0].time as i64);
        let t_1 =
            chrono::Duration::seconds(self.points.last().expect("len allways > 0").time as i64);
        format!(
            "{}T{}:{} bis {}T{}:{}",
            t_0.num_days(),
            t_0.num_hours() - t_0.num_days() * 24,
            t_0.num_minutes() - t_0.num_hours() * 60,
            t_1.num_days(),
            t_1.num_hours() - t_1.num_days() * 24,
            t_1.num_minutes() - t_1.num_hours() * 60,
        )
    }
}
