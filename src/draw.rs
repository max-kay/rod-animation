use std::{
    fmt, fs,
    io::Read,
    path::{self, PathBuf},
    process::Command,
    sync::LazyLock,
};

use anyhow::{Result, anyhow};
use hsv::hsv_to_rgb;
use log::{error, info};
use rayon::iter::{ParallelBridge, ParallelIterator};
use serde::Deserialize;
use skia_safe::{
    Bitmap, Canvas, Color4f, ColorType, FilterMode, Image, ImageInfo, OwnedCanvas, Paint,
    PaintStyle, SamplingOptions,
    canvas::{SaveLayerRec, SrcRectConstraint},
};

use crate::{
    BASE_RES_PATH, FRAME_RATE, HEIGHT, OUT_PATH, OneOrTwo, PEOPLE, PINS_PATH, Transform, Vector,
    WIDTH, WORLD, fade_in_function, fade_out_function,
    map::{SORTERS, TILE_SIZE, TileDescr},
    smoother_step,
};

pub mod parse;

#[derive(Clone, Copy, Deserialize)]
#[serde(from = "usize")]
pub struct Color {
    r: u8,
    g: u8,
    b: u8,
}

impl fmt::Debug for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Color {{ #{:>0X}{:>0X}{:>0X} }}", self.r, self.g, self.b)
    }
}

const COLORS: LazyLock<Vec<Color>> = LazyLock::new(|| {
    vec![
        // Siedlungsgebiet
        Color::from_hsv(235.0, 0.18, 0.02),
        // Hintergrund
        Color::from_hsv(235.0, 0.18, 0.08),
        // Häuser
        Color::from_hsv(235.0, 0.18, 0.12),
        // Strassen
        Color::from_hsv(235.0, 0.18, 0.24),
        // Grenzen
        Color::from_hsv(235.0, 0.18, 0.40),
        // Wasser
        Color::from_hsv(235.0, 0.3, 0.19),
        // Wald
        Color::from_hsv(131.0, 0.24, 0.09),
    ]
});

impl From<usize> for Color {
    fn from(value: usize) -> Color {
        COLORS[value]
    }
}

impl Color {
    fn from_hsv(hue: f64, sat: f64, value: f64) -> Self {
        let (r, g, b) = hsv_to_rgb(hue, sat, value);
        Self::new(r, g, b)
    }

    pub fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    fn to_skia(&self) -> Color4f {
        Color4f::from(skia_safe::Color::from_rgb(self.r, self.g, self.b))
    }

    fn with_opacity(&self, opacity: f32) -> Color4f {
        let mut col = Color4f::from(skia_safe::Color::from_rgb(self.r, self.g, self.b));
        col.a = opacity;
        col
    }
}

#[derive(Deserialize)]
pub struct LayerStyle {
    pub fill: Option<Color>,
    pub stroke: Option<(f32, Color)>,
}

impl LayerStyle {
    pub fn to_draw_instructions(&self, transform: Transform, opacity: f32) -> DrawInstructions {
        let Self { fill, stroke } = self;
        DrawInstructions {
            fill: *fill,
            stroke: *stroke,
            transform,
            opacity,
        }
    }
}

pub struct DrawInstructions {
    pub fill: Option<Color>,
    pub stroke: Option<(f32, Color)>,
    pub transform: Transform,
    pub opacity: f32,
}

impl DrawInstructions {
    pub fn path_style(&self) -> Option<Paint> {
        if let Some((line_width, stroke_color)) = self.stroke {
            let color = stroke_color.with_opacity(self.opacity);
            let mut paint = Paint::new(&color, None);
            paint.set_stroke(true);
            paint.set_style(PaintStyle::Stroke);
            paint.set_stroke_width(line_width);
            paint.set_stroke_cap(skia_safe::PaintCap::Round);
            paint.set_stroke_join(skia_safe::PaintJoin::Round);
            paint.set_anti_alias(true);
            Some(paint)
        } else {
            None
        }
    }

    pub fn area_style(&self) -> Option<Paint> {
        if let Some(color) = self.fill {
            let color = color.with_opacity(self.opacity);
            let mut paint = Paint::new(&color, None);
            paint.set_style(PaintStyle::Fill);
            paint.set_anti_alias(true);
            Some(paint)
        } else if let Some((width, color)) = self.stroke {
            let color = color.with_opacity(self.opacity);
            let mut paint = Paint::new(&color, None);
            paint.set_stroke(true);
            paint.set_style(PaintStyle::Stroke);
            paint.set_stroke_width(width);
            paint.set_anti_alias(true);
            Some(paint)
        } else {
            None
        }
    }
}

pub struct Pin {
    pin: Image,
    pin_tip_x: f32,
    pin_tip_y: f32,
    img_width: f32,
    img_height: f32,
}

impl Pin {
    pub fn new(image: Image, pin_tip_x: f32, pin_tip_y: f32) -> Self {
        Self {
            img_width: image.width() as f32,
            img_height: image.height() as f32,
            pin: image,
            pin_tip_x,
            pin_tip_y,
        }
    }

    pub fn load(name: &str, pin_tip_x: f32, pin_tip_y: f32) -> Result<Pin> {
        let mut file = std::fs::File::open(PINS_PATH.join(format!("{}.png", name)))?;
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer)?;

        let data = skia_safe::Data::new_copy(&buffer);

        Ok(Self::new(
            Image::from_encoded(data).ok_or(anyhow!("Failed to decode image: {}", name))?,
            pin_tip_x,
            pin_tip_y,
        ))
    }

    fn draw(&self, target_location: Vector, pin_height: f32, canvas: &mut OwnedCanvas) {
        let scale_factor = pin_height / self.img_height as f32;

        let scaled_size = Vector::new(self.img_width as f32 * scale_factor, pin_height);

        let offset = Vector::new(
            self.pin_tip_x as f32 * scale_factor,
            self.pin_tip_y as f32 * scale_factor,
        );

        let dest = target_location - offset;
        let dest_bottom_right = dest + scaled_size;

        let src_rect = skia_safe::Rect::from_wh(self.img_width, self.img_height);
        let dest_rect =
            skia_safe::Rect::new(dest.x, dest.y, dest_bottom_right.x, dest_bottom_right.y);

        if dest_rect.intersects(skia_safe::Rect::from_xywh(
            0.0,
            0.0,
            WIDTH as f32,
            HEIGHT as f32,
        )) {
            let off_screen_width = (0.0 - dest_rect.left())
                .max(dest_rect.right() - WIDTH as f32)
                .max(0.0);
            let off_screen_height = (0.0 - dest_rect.top())
                .max(dest_rect.bottom() - HEIGHT as f32)
                .max(0.0);
            let shown_frac = (off_screen_width - dest_rect.width())
                * (off_screen_height - dest_rect.height())
                / (dest_rect.width() * dest_rect.height());

            let sampling = SamplingOptions::new(FilterMode::Linear, skia_safe::MipmapMode::Linear);
            let mut paint = Paint::default();
            paint.set_anti_alias(true);
            paint.set_alpha_f(shown_frac * shown_frac * shown_frac);

            canvas.draw_image_rect_with_sampling_options(
                &self.pin,
                Some((&src_rect, SrcRectConstraint::Fast)),
                dest_rect,
                sampling,
                &paint,
            );
        }
    }
}

#[derive(Copy, Clone)]
pub struct ScenePos {
    pub center: Vector,
    pub zoom: f32,
    pub time: u32,
}

impl ScenePos {
    pub fn new(center: Vector, zoom: f32, time: u32) -> Self {
        Self { center, zoom, time }
    }

    pub fn world_to_screen(&self) -> Transform {
        let scale = 2f32.powf(self.zoom) * TILE_SIZE as f32;
        let scaled_center = self.center * scale;
        let screen_center = Vector::new(WIDTH as f32 / 2.0, HEIGHT as f32 / 2.0);
        let translation = screen_center - scaled_center;
        Transform::new(scale, translation)
    }

    pub fn screen_to_world(&self) -> Transform {
        let scale = 2f32.powf(-self.zoom) / TILE_SIZE as f32;
        let scaled_screen_center = scale * Vector::new(WIDTH as f32 / 2.0, HEIGHT as f32 / 2.0);
        Transform::new(scale, -scaled_screen_center + self.center)
    }

    pub fn tile_to_screen(&self, tile: TileDescr) -> Transform {
        let scale = TILE_SIZE as f32 * 2f32.powf(self.zoom - tile.z as f32);
        let translation = (scale * Vector::new(tile.x as f32, tile.y as f32))
            - (TILE_SIZE as f32 * 2f32.powf(self.zoom) * self.center)
            + Vector::new(WIDTH as f32 / 2.0, HEIGHT as f32 / 2.0);
        Transform::new(scale, translation)
    }

    pub fn world_min(&self) -> Vector {
        self.screen_to_world() * Vector::zeros()
    }

    pub fn world_max(&self) -> Vector {
        self.screen_to_world() * Vector::new(WIDTH as f32, HEIGHT as f32)
    }
}

pub struct Frame {
    scene_pos: ScenePos,
    people: Vec<String>,
    pin_height: f32,
    checkpoints: bool,
}

impl Frame {
    pub fn render_background(&self, canvas: &mut OwnedCanvas) {
        canvas.clear(COLORS[1].to_skia());
        let tiles = WORLD.get_tiles_at(self.scene_pos);
        let map = WORLD.map.read().expect("RwLock not poisoned");
        match tiles {
            OneOrTwo::One(tiles) => {
                let tiles: Option<Vec<_>> = tiles.iter().map(|tile| map.get_tile(*tile)).collect();
                if tiles.is_none() {
                    error!("some tiles needed were not loaded");
                    return;
                }
                let tiles = tiles.expect("checked above");
                for id in 0..=SORTERS.max_layer_idx() {
                    for tile in &tiles {
                        if let Some(layer) = tile.get_layer(id) {
                            layer.draw(canvas, self.scene_pos.tile_to_screen(tile.descr), 1.0)
                        }
                    }
                }
            }
            OneOrTwo::Two(less_detail, more_detail) => {
                let less_detail: Option<Vec<_>> =
                    less_detail.iter().map(|tile| map.get_tile(*tile)).collect();
                if less_detail.is_none() {
                    error!("some tiles needed were not loaded");
                }
                let less_detail = less_detail.expect("checked above");

                canvas.save_layer(&SaveLayerRec::default());
                for id in 0..=SORTERS.max_layer_idx() {
                    for tile in &less_detail {
                        if let Some(layer) = tile.get_layer(id) {
                            let opacity = fade_out_function(self.scene_pos.zoom.fract());
                            layer.draw(canvas, self.scene_pos.tile_to_screen(tile.descr), opacity)
                        }
                    }
                }
                canvas.restore();

                let more_detail: Option<Vec<_>> =
                    more_detail.iter().map(|tile| map.get_tile(*tile)).collect();
                if more_detail.is_none() {
                    error!("some tiles needed were not loaded");
                }
                let more_detail = more_detail.expect("checked above");
                canvas.save_layer(&SaveLayerRec::default());
                for id in 0..=SORTERS.max_layer_idx() {
                    for tile in &more_detail {
                        if let Some(layer) = tile.get_layer(id) {
                            let opacity = fade_in_function(self.scene_pos.zoom.fract());
                            layer.draw(canvas, self.scene_pos.tile_to_screen(tile.descr), opacity)
                        }
                    }
                }
                canvas.restore();
            }
        }
    }

    pub fn render(self) -> Image {
        let info = ImageInfo::new(
            (WIDTH as i32, HEIGHT as i32),
            ColorType::N32,
            skia_safe::AlphaType::Opaque,
            None,
        );
        let mut bitmap = Bitmap::new();
        if !bitmap.set_info(&info, None) {
            panic!("could not set image info while rendering")
        };
        bitmap.alloc_pixels();
        let mut canvas =
            Canvas::from_bitmap(&bitmap, None).expect("Failed to create canvas from bitmap");

        self.render_background(&mut canvas);

        let people = if self.people.is_empty() {
            PEOPLE.iter().map(|s| s.to_string()).collect()
        } else {
            self.people
        };

        if self.checkpoints {
            for (_name, (position, pin)) in WORLD.checkpoints.iter() {
                pin.draw(
                    self.scene_pos.world_to_screen() * position,
                    self.pin_height,
                    &mut canvas,
                );
            }
        }

        for name in people {
            let track = WORLD
                .get_track(&name)
                .expect("here the list of people is valid");
            if let Some(position) = track.get_position(self.scene_pos.time) {
                track.pin.draw(
                    self.scene_pos.world_to_screen() * position,
                    self.pin_height,
                    &mut canvas,
                );
            }
        }
        bitmap.as_image()
    }
}

pub trait Renderable {
    fn get_file_name(&self) -> PathBuf;
    fn name(&self) -> &str;
    fn make_file(self: Box<Self>) -> Result<()>;
}

pub struct StillFrame {
    name: String,
    center: Vector,
    zoom: f32,
    time: u32,
    people: Vec<String>,
    checkpoints: bool,
    pin_height: f32,
}

impl Renderable for StillFrame {
    fn get_file_name(&self) -> PathBuf {
        OUT_PATH.join(format!("{}.png", self.name)).to_path_buf()
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn make_file(self: Box<Self>) -> Result<()> {
        let frame = Frame {
            scene_pos: ScenePos::new(self.center, self.zoom, self.time),
            people: self.people.clone(),
            checkpoints: self.checkpoints,
            pin_height: self.pin_height,
        };
        info!("loading tiles for {}", self.name);
        WORLD.load_tiles_at(frame.scene_pos)?;
        info!("finished loading tiles for {}", self.name);
        let image: skia_safe::Image = frame.render();
        let mut file = std::fs::File::create(&self.get_file_name())?;
        skia_safe::png_encoder::encode(
            &image.peek_pixels().expect("failed to get pixels."),
            &mut file,
            &skia_safe::png_encoder::Options::default(),
        );
        return Ok(());
    }
}

pub struct Fixed {
    name: String,
    center: Vector,
    zoom: (f32, f32),
    time: (u32, u32),
    duration_s: f32,
    people: Vec<String>,
    checkpoints: bool,
    pin_height: f32,
}

impl Fixed {
    pub fn as_frames(&self) -> Vec<Frame> {
        let Fixed {
            name: _,
            center,
            zoom,
            time,
            duration_s,
            people,
            pin_height,
            checkpoints,
        } = self;
        let frames_tot = (duration_s * FRAME_RATE).round() as u32;
        let mut frames = Vec::new();
        for i in 0..frames_tot {
            let zoom = zoom.0 + (zoom.1 - zoom.0) * (i as f32 / frames_tot as f32);
            let time = time.0
                + (((time.1 - time.0) as f32) * (i as f32 / frames_tot as f32)).round() as u32;
            frames.push(Frame {
                scene_pos: ScenePos::new(*center, zoom, time),
                people: people.clone(),
                checkpoints: *checkpoints,
                pin_height: *pin_height,
            });
        }
        frames
    }
}

impl Renderable for Fixed {
    fn name(&self) -> &str {
        &self.name
    }

    fn get_file_name(&self) -> PathBuf {
        OUT_PATH.join(format!("{}.mp4", self.name)).to_path_buf()
    }

    fn make_file(self: Box<Self>) -> Result<()> {
        make_video(self.as_frames(), &self.name, self.get_file_name())
    }
}
pub struct Sweep {
    name: String,
    center: (Vector, Vector),
    zoom: (f32, f32),
    time: (u32, u32),
    duration_s: f32,
    people: Vec<String>,
    checkpoints: bool,
    pin_height: f32,
}

impl Sweep {
    pub fn as_frames(&self) -> Vec<Frame> {
        let Sweep {
            name: _,
            center,
            zoom,
            time,
            duration_s,
            people,
            pin_height,
            checkpoints,
        } = self;
        let frames_tot = (duration_s * FRAME_RATE).round() as u32;
        let mut frames = Vec::new();
        let dist = (center.0 - center.1).norm();
        let max_zoom = -dist.log2();
        let zoomlevels: Vec<f32> = if max_zoom < zoom.0 || max_zoom < zoom.1 {
            (0..frames_tot)
                .map(|i| {
                    if 2 * i < frames_tot - 1 {
                        (max_zoom - zoom.0)
                            * smoother_step((i as f32) / (frames_tot - 1) as f32, 0.0, 0.5)
                            + zoom.0
                    } else {
                        (zoom.1 - max_zoom)
                            * smoother_step((i as f32) / (frames_tot - 1) as f32, 0.5, 1.0)
                            + max_zoom
                    }
                })
                .collect()
        } else {
            (0..frames_tot)
                .map(|i| zoom.0 + (zoom.1 - zoom.0) * (i as f32 / (frames_tot - 1) as f32))
                .collect()
        };
        let lin_zoom = (0..frames_tot)
            .map(|i| zoom.0 + (zoom.1 - zoom.0) * (i as f32) / (frames_tot - 1) as f32);

        let pin_heights: Vec<f32> = zoomlevels
            .iter()
            .zip(lin_zoom)
            .map(|(zoom, lin_zoom)| pin_height * 2f32.powf((zoom - lin_zoom) * 0.2))
            .collect();

        let vec_scales: Vec<f32> = zoomlevels
            .iter()
            .scan(0.0, |state, z| {
                *state += 2f32.powf(-z).powf(3.0);
                Some(*state)
            })
            .collect();

        let last = *vec_scales.last().unwrap();
        let centers: Vec<_> = vec_scales
            .iter()
            .map(|x| {
                let scale = x / last;
                center.0 + ((center.1 - center.0) * scale)
            })
            .collect();

        for (((i, zoom), center), pin_height) in (0..frames_tot)
            .zip(zoomlevels.iter())
            .zip(centers.iter())
            .zip(pin_heights.iter())
        {
            let time = time.0
                + (((time.1 - time.0) as f32) * (i as f32 / (frames_tot - 1) as f32)).round()
                    as u32;
            frames.push(Frame {
                scene_pos: ScenePos::new(*center, *zoom, time),
                people: people.clone(),
                checkpoints: *checkpoints,
                pin_height: *pin_height,
            });
        }
        frames
    }
}

impl Renderable for Sweep {
    fn name(&self) -> &str {
        &self.name
    }

    fn get_file_name(&self) -> PathBuf {
        OUT_PATH.join(format!("{}.mp4", self.name)).to_path_buf()
    }

    fn make_file(self: Box<Self>) -> Result<()> {
        make_video(self.as_frames(), &self.name, self.get_file_name())
    }
}

fn make_video(frames: Vec<Frame>, name: &str, file_name: impl AsRef<path::Path>) -> Result<()> {
    let tmp_path = BASE_RES_PATH.join("tmp");
    if tmp_path.exists() {
        fs::remove_dir_all(&tmp_path)?;
        fs::create_dir_all(&tmp_path)?;
    } else {
        fs::create_dir_all(&tmp_path)?;
    }

    info!("loading tiles for {name}");
    for frame in &frames {
        WORLD.load_tiles_at(frame.scene_pos)?;
    }
    info!("finished loading tiles for {name}");
    info!("start rendering {name}");
    frames
        .into_iter()
        .enumerate()
        .par_bridge()
        .for_each(|(i, frame)| {
            let image: skia_safe::Image = frame.render();
            let mut file =
                std::fs::File::create(tmp_path.join(format!("frame{i:0>8}.png"))).unwrap();
            skia_safe::png_encoder::encode(
                &image.peek_pixels().expect("failed to get pixels."),
                &mut file,
                &skia_safe::png_encoder::Options::default(),
            );
        });
    info!("finished rendering {name}");
    info!("making video for {name}");
    Command::new("ffmpeg")
        .arg("-y")
        .arg("-framerate")
        .arg(FRAME_RATE.to_string())
        .arg("-i")
        .arg(tmp_path.join("frame%08d.png"))
        .arg("-c:v")
        .arg("libx264")
        .arg("-pix_fmt")
        .arg("yuv420p")
        .arg(file_name.as_ref())
        .output()?;

    fs::remove_dir_all(tmp_path)?;
    info!(
        "finished {name} output_file: {}",
        file_name.as_ref().iter().last().unwrap().to_string_lossy()
    );
    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;

    use std::sync::LazyLock;

    use crate::{MAP_DATA, WORLD, lat_long_to_vec};

    #[test]
    fn load_test() {
        let poss = [
            lat_long_to_vec(47.55503577206553, 7.5869946379106254),
            lat_long_to_vec(46.90307463711658, 6.786078097356981),
        ];

        LazyLock::force(&MAP_DATA);
        LazyLock::force(&WORLD);
        let mut data_lock = MAP_DATA.write().unwrap();
        for zoom_level in 4..=5 {
            for pos in poss {
                let scene_pos = ScenePos {
                    center: pos,
                    zoom: zoom_level as f32,
                    time: 0,
                };
                let tiles = WORLD.get_tiles_fixed(scene_pos, zoom_level);
                for tile in tiles {
                    println!("{:?}", tile);
                    data_lock.load_tile(tile).unwrap();
                    data_lock.get_tile(tile).unwrap();
                }
            }
        }
    }
}
