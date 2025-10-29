use std::{fs::read_to_string, path::Path};

use anyhow::{Result, anyhow};
use log::error;

use crate::{
    OneOrTwo, PEOPLE, WORLD,
    draw::{Fixed, Renderable, StillFrame, Sweep},
    lat_long_to_vec,
    vec::Vector,
};

macro_rules! error_on_none {
($val:expr, $($arg:tt)+) => {
    match $val {
        Some(val) => val,
        None => {(error!($($arg)+)); return None;}
    }
}
}

/// panics if path has no file name or is not a txt
pub fn from_path(path: impl AsRef<Path>) -> Result<Box<dyn Renderable>> {
    let name = path
        .as_ref()
        .iter()
        .last()
        .expect("path in renderable from path is allways valid by caller")
        .to_string_lossy()
        .to_string();
    let s = read_to_string(path)?;

    let this = from_str(
        name.strip_suffix(".txt")
            .expect("is always txt from caller"),
        &s,
    )
    .ok_or(anyhow!("could not read file"))?;
    Ok(this)
}

fn from_str(name: &str, s: &str) -> Option<Box<dyn Renderable>> {
    let valid_keys = &[
        "mitte",
        "zoom",
        "zeit",
        "dauer",
        "pins",
        "checkpoints",
        "pingrösse",
    ];

    let lines: Vec<_> = s
        .lines()
        .enumerate()
        .filter_map(|(i, s)| {
            let new = s
                .trim()
                .split('#')
                .next()
                .expect("split has allways one element");
            if !new.is_empty() {
                Some((i + 1, new))
            } else {
                None
            }
        })
        .collect();
    let map: Vec<_> = lines
        .iter()
        .skip(1)
        .map(|(line_nr, s)| {
            if let Some(split_index) = s.find(char::is_whitespace) {
                let (first_part, rest) = s.split_at(split_index);
                let trimmed_rest = rest.trim_start();
                (*line_nr, first_part.to_lowercase(), trimmed_rest)
            } else {
                (*line_nr, s.to_lowercase(), "")
            }
        })
        .collect();

    for p in &map {
        if !valid_keys.contains(&&*p.1) {
            error!(
                "auf Zeile {} ist ein ungültiger Schlüssel:\n{} in Kleinbuchstaben gibt es nicht",
                p.0, p.1
            );
            return None;
        }
    }

    match &*lines[0].1.to_lowercase() {
        "bild" => new_still_frame(name, &*map).map(|still| Box::new(still) as Box<dyn Renderable>),
        "animation" => new_animation(name, &*map),
        _ => {
            error!("Modus '{}' wurde nicht verstanden", lines[0].1);
            None
        }
    }
}

fn new_animation(name: &str, map: &[(usize, String, &str)]) -> Option<Box<dyn Renderable>> {
    let zoom_str = error_on_none!(find_key(map, "zoom"), "Zoom wurde nicht gefunden");
    let zoom_tup = error_on_none!(
        process_tuple(zoom_str.1),
        "Konnte die Liste für Zoom (Zeile: {}) nicht verstehen",
        zoom_str.0
    );
    let zoom = error_on_none!(
        zoom_tup.map(|s| s.parse().ok()).as_opt(),
        "Zoom (Zeile {}) wurde nicht verstanden",
        zoom_str.0
    )
    .splat();

    let time_str = error_on_none!(find_key(map, "zeit"), "Zeit wurde nicht gefunded");
    let time_tup = error_on_none!(
        process_tuple(time_str.1),
        "Konnte die Liste für Zeile (Zeile: {}) nicht verstehen",
        time_str.0
    );
    let time = error_on_none!(
        time_tup.map(|s| process_time(s)).as_opt(),
        "Zeit (Zeile {}) wurde nicht verstanden",
        time_str.0
    )
    .splat();

    let duration_str = error_on_none!(find_key(map, "dauer"), "duration wurde nicht gefunden");
    let duration = error_on_none!(
        duration_str.1.parse().ok(),
        "duration (Zeile {}) wurde nicht verstanden",
        duration_str.0
    );

    let people = match find_key(map, "pins") {
        Some(people_str) => error_on_none!(
            process_people(people_str.1),
            "Pins (Zeile: {}) benutzt Personen die nicht existieren",
            people_str.0
        ),
        None => Vec::new(),
    };

    let pin_h_str = error_on_none!(find_key(map, "pingrösse"), "Pingrösse wurde nicht gefunden");
    let pin_height = error_on_none!(
        pin_h_str.1.parse().ok(),
        "Pingrösse (Zeile {}) wurde nicht verstanden",
        pin_h_str.0
    );

    let center_str = error_on_none!(find_key(map, "mitte"), "Mitte wurde nicht gefunden");
    let center_tup = error_on_none!(
        process_tuple(center_str.1),
        "Konnte die Liste für Mitte (Zeile: {}) nicht verstehen",
        center_str.0
    );
    let center = error_on_none!(
        center_tup.map(process_coord).as_opt(),
        "Konnte die Liste für Mitte (Zeile: {}) nicht verstehen",
        center_str.0
    );

    match center {
        OneOrTwo::One(center) => Some(Box::new(Fixed {
            name: name.to_string(),
            center,
            zoom,
            time,
            duration_s: duration,
            people,
            pin_height,
            checkpoints: find_key(map, "checkpoints").is_some(),
        }) as Box<dyn Renderable>),

        OneOrTwo::Two(center0, center1) => Some(Box::new(Sweep {
            name: name.to_string(),
            center: (center0, center1),
            zoom,
            time,
            duration_s: duration,
            people,
            pin_height,
            checkpoints: find_key(map, "checkpoints").is_some(),
        }) as Box<dyn Renderable>),
    }
}

fn new_still_frame(name: &str, map: &[(usize, String, &str)]) -> Option<StillFrame> {
    let center_str = error_on_none!(find_key(map, "mitte"), "Mitte wurde nicht gefunden");
    let center = error_on_none!(
        process_coord(center_str.1),
        "Mitte (Zeile {}) wurde nicht verstanden!",
        center_str.0
    );

    let zoom_str = error_on_none!(find_key(map, "zoom"), "Zoom wurde nicht gefunden");
    let zoom = error_on_none!(
        zoom_str.1.parse().ok(),
        "Zoom (Zeile {}) wurde nicht verstanden",
        zoom_str.0
    );

    let time_str = error_on_none!(find_key(map, "zeit"), "Zeit wurde nicht gefunded");
    let time = error_on_none!(
        process_time(time_str.1),
        "Zeit (Zeile {}) wurde nicht verstanden",
        time_str.0
    );

    let people = match find_key(map, "pins") {
        Some(people_str) => error_on_none!(
            process_people(people_str.1),
            "Pins (Zeile: {}) benutzt Personen die nicht existieren",
            people_str.0
        ),
        None => Vec::new(),
    };

    let pin_h_str = error_on_none!(find_key(map, "pingrösse"), "Pingrösse wurde nicht gefunden");
    let pin_height = error_on_none!(
        pin_h_str.1.parse().ok(),
        "Pingrösse (Zeile {}) wurde nicht verstanden",
        pin_h_str.0
    );

    Some(StillFrame {
        name: name.to_string(),
        center,
        zoom,
        time,
        people,
        pin_height,
        checkpoints: find_key(map, "checkpoints").is_some(),
    })
}

fn find_key<'a, 'b>(map: &'a [(usize, String, &'b str)], key: &str) -> Option<(usize, &'b str)>
where
    'b: 'a,
{
    for (line, this_key, val) in map {
        if key == this_key {
            return Some((*line, val));
        }
    }
    None
}

fn process_tuple(s: &str) -> Option<OneOrTwo<&str>> {
    let mut split = s.split(';');
    let a = split.next()?;
    match (split.next(), split.next()) {
        (None, _) => Some(OneOrTwo::One(a.trim())),
        (Some(b), None) => Some(OneOrTwo::Two(a.trim(), b.trim())),
        _ => None,
    }
}

fn process_coord(s: &str) -> Option<Vector> {
    if s.contains('[') {
        let mut split = s.split('[');
        let name = split.next()?.trim();
        let s_time = split.next()?.trim().strip_suffix(']')?;
        if split.next().is_some() {
            return None;
        }
        let time = process_time(s_time)?;
        match WORLD.get_track(name) {
            Some(track) => match track.get_position(time) {
                Some(pos) => return Some(pos),
                None => {
                    error!("die Zeit {s_time}, ist für Person {name} ungültig");
                    error!("gültig ist: {}", track.valid_times());
                    return None;
                }
            },
            None => {
                error!("person '{}' wurde nicht gefunden", name);
                return None;
            }
        }
    }
    let mut split = s.strip_prefix('(')?.strip_suffix(')')?.split(',');
    let lat = split.next()?.trim().parse().ok()?;
    if !(20.0..=50.0).contains(&lat) {
        error!("Breitengrad ungültig");
        return None;
    }
    let lon = split.next()?.trim().parse().ok()?;
    if !(0.0..=10.0).contains(&lon) {
        error!("Längengrad ungültig");
        return None;
    }
    if split.next().is_some() {
        return None;
    }
    return Some(lat_long_to_vec(lat, lon));
}

fn process_time(s: &str) -> Option<u32> {
    let mut split = s.trim().split('T');
    let day: u32 = split.next()?.trim().parse().ok()?;
    let mut time_split = split.next()?.split(':');
    if split.next().is_some() {
        return None;
    }
    let hour: u32 = time_split.next()?.trim().parse().ok()?;
    let minute: u32 = time_split.next()?.trim().parse().ok()?;
    if time_split.next().is_some() {
        return None;
    }
    return Some(day * 24 * 60 * 60 + hour * 60 * 60 + minute * 60);
}

fn process_people(s: &str) -> Option<Vec<String>> {
    s.split(';')
        .filter_map(|mut s| {
            s = s.trim();
            if !s.is_empty() { Some(s) } else { None }
        })
        .map(|s| {
            if PEOPLE.contains(&s) {
                Some(s.to_string())
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod test {
    use super::*;

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    #[test]
    fn parse() {
        init();
        let s = include_str!("../../test_files/animation.txt");
        from_str("example", s).expect("in test");
        let s = include_str!("../../test_files/image.txt");
        from_str("example", s).expect("in test");
    }

    #[test]
    fn parse_fail() {
        init();
        let s1 = include_str!("../../test_files/image_failing.txt");
        let s2 = include_str!("../../test_files/dumb.txt");
        assert!(from_str("example", s1).is_none() && from_str("example", s2).is_none());
    }
}
