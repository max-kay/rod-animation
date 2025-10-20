use std::{fs, process::Command};

use anyhow::Result;
use log::{error, info};
use rayon::iter::{IndexedParallelIterator, IntoParallelRefIterator, ParallelIterator};
use skia_safe::{
    Bitmap, Canvas, Color, Color4f, ColorType, FilterMode, Image, ImageInfo, OwnedCanvas, Paint,
    PaintStyle, SamplingOptions, canvas::SrcRectConstraint,
};

use crate::{
    FRAME_RATE, HEIGHT, OUT_PATH, OneOrTwo, Transform, Vector, WIDTH, WORLD, fade_function,
    map::{TILE_SIZE, TileDescr},
};

pub struct Style {
    layers: Vec<(String, LayerStyle)>,
}

impl Style {
    pub fn get_layers(&self) -> &[(String, LayerStyle)] {
        &self.layers
    }
}

impl Default for Style {
    fn default() -> Self {
        Self {
            layers: vec![
                (
                    "ocean".to_string(),
                    LayerStyle {
                        fill: None,
                        stroke: None,
                    },
                ),
                (
                    "water_polygons".to_string(),
                    LayerStyle {
                        fill: Some(Color::from_rgb(0, 0, 127)),
                        stroke: None,
                    },
                ),
                (
                    "water_lines".to_string(),
                    LayerStyle {
                        fill: None,
                        stroke: Some((2.0, Color::from_rgb(0, 0, u8::MAX))),
                    },
                ),
                (
                    "dam_lines".to_string(),
                    LayerStyle {
                        fill: None,
                        stroke: None,
                    },
                ),
                (
                    "dam_polygons".to_string(),
                    LayerStyle {
                        fill: None,
                        stroke: None,
                    },
                ),
                (
                    "pier_lines".to_string(),
                    LayerStyle {
                        fill: None,
                        stroke: None,
                    },
                ),
                (
                    "pier_polygons".to_string(),
                    LayerStyle {
                        fill: None,
                        stroke: None,
                    },
                ),
                (
                    "boundaries".to_string(),
                    LayerStyle {
                        fill: None,
                        stroke: Some((5.0, Color::from_rgb(0, 0, 0))),
                    },
                ),
                (
                    "land".to_string(),
                    LayerStyle {
                        fill: None,
                        stroke: None,
                    },
                ),
                (
                    "sites".to_string(),
                    LayerStyle {
                        fill: None,
                        stroke: None,
                    },
                ),
                (
                    "buildings".to_string(),
                    LayerStyle {
                        fill: None,
                        stroke: None,
                    },
                ),
                (
                    "streets".to_string(),
                    LayerStyle {
                        fill: None,
                        stroke: None,
                    },
                ),
                (
                    "street_polygons".to_string(),
                    LayerStyle {
                        fill: None,
                        stroke: None,
                    },
                ),
                (
                    "bridges".to_string(),
                    LayerStyle {
                        fill: None,
                        stroke: None,
                    },
                ),
                (
                    "aerialways".to_string(),
                    LayerStyle {
                        fill: None,
                        stroke: None,
                    },
                ),
            ],
        }
    }
}

#[derive(Clone, Copy)]
pub struct LayerStyle {
    pub fill: Option<Color>,
    pub stroke: Option<(f32, Color)>,
}

impl LayerStyle {
    pub fn to_draw_instructions(&self, transform: Transform, opacity: f32) -> DrawInstructions {
        let Self { fill, stroke } = *self;
        DrawInstructions {
            fill,
            stroke,
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
            let mut color = Color4f::from(stroke_color);
            color.a = self.opacity;
            let mut paint = Paint::new(&color, None);
            paint.set_stroke(true);
            paint.set_style(PaintStyle::Stroke);
            paint.set_stroke_width(line_width);
            paint.set_anti_alias(true);
            Some(paint)
        } else {
            None
        }
    }

    pub fn area_style(&self) -> Option<Paint> {
        if let Some(color) = self.fill {
            let mut color = Color4f::from(color);
            color.a = self.opacity;
            let mut paint = Paint::new(&color, None);
            paint.set_style(PaintStyle::Fill);
            paint.set_anti_alias(true);
            if let Some((width, _color)) = self.stroke {
                // TODO: set style correctly
                paint.set_style(PaintStyle::StrokeAndFill);
                paint.set_stroke_width(width);
            }
            Some(paint)
        } else if let Some((width, color)) = self.stroke {
            let mut color = Color4f::from(color);
            color.a = self.opacity;
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
    position: Vector,
    pin: &'static Image,
}

impl Pin {
    const PIN_TIP_X: u32 = 1731;
    const PIN_TIP_Y: u32 = 5488;
    const IMG_WIDTH: u32 = 3513;
    const IMG_HEIGHT: u32 = 5868;

    fn draw(&self, transform: Transform, pin_height: f32, canvas: &mut OwnedCanvas) {
        let target_location = transform * self.position;
        let scale_factor = pin_height / Self::IMG_HEIGHT as f32;

        let scaled_size = Vector::new(Self::IMG_WIDTH as f32 * scale_factor, pin_height);

        let offset = Vector::new(
            Self::PIN_TIP_X as f32 * scale_factor,
            Self::PIN_TIP_Y as f32 * scale_factor,
        );

        let dest = target_location - offset;
        let dest_bottom_right = dest + scaled_size;

        let src_rect = skia_safe::Rect::from_wh(Self::IMG_WIDTH as f32, Self::IMG_HEIGHT as f32);
        let dest_rect =
            skia_safe::Rect::new(dest.x, dest.y, dest_bottom_right.x, dest_bottom_right.y);

        let sampling = SamplingOptions::new(FilterMode::Linear, skia_safe::MipmapMode::Linear);
        let mut paint = Paint::default();
        paint.set_anti_alias(true);

        canvas.draw_image_rect_with_sampling_options(
            self.pin,
            Some((&src_rect, SrcRectConstraint::Fast)),
            dest_rect,
            sampling,
            &paint,
        );
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
}

impl Frame {
    pub fn render_background(&self, canvas: &mut OwnedCanvas) {
        canvas.clear(Color::from_rgb(0x27, 0xAE, 0xB9));
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
                for (id, instr) in WORLD.style.get_layers() {
                    for tile in &tiles {
                        let instructions = instr
                            .to_draw_instructions(self.scene_pos.tile_to_screen(tile.descr), 1.0);
                        if let Some(layer) = tile.get_layer(id) {
                            layer.draw(canvas, instructions)
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
                let more_detail: Option<Vec<_>> =
                    more_detail.iter().map(|tile| map.get_tile(*tile)).collect();
                if more_detail.is_none() {
                    error!("some tiles needed were not loaded");
                }
                let more_detail = more_detail.expect("checked above");
                let opacity = fade_function(self.scene_pos.zoom.fract());
                for (id, instr) in WORLD.style.get_layers() {
                    for tile in &more_detail {
                        let instructions = instr.to_draw_instructions(
                            self.scene_pos.tile_to_screen(tile.descr),
                            opacity,
                        );
                        if let Some(layer) = tile.get_layer(id) {
                            layer.draw(canvas, instructions)
                        }
                    }
                    for tile in &less_detail {
                        let instructions = instr.to_draw_instructions(
                            self.scene_pos.tile_to_screen(tile.descr),
                            1.0 - opacity,
                        );
                        if let Some(layer) = tile.get_layer(id) {
                            layer.draw(canvas, instructions)
                        }
                    }
                }
            }
        }
    }

    pub fn render(&self) -> Image {
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

        for person in &self.people {
            if WORLD.pins.contains_key(person) {
                error!("{person} wurde nicht gefunden!")
            }
        }

        for track in &WORLD.tracks {
            if !self.people.is_empty() && !self.people.contains(&track.name) {
                continue;
            }
            if let Some(position) = track.get_position(self.scene_pos.time) {
                let pin = Pin {
                    position,
                    pin: WORLD.pins.get(&track.name).expect("failed to get pins"),
                };
                pin.draw(
                    self.scene_pos.world_to_screen(),
                    self.pin_height,
                    &mut canvas,
                );
            }
        }
        bitmap.as_image()
    }
}

pub enum Renderable {
    Image {
        name: String,
        center: Vector,
        zoomlevel: f32,
        time: u32,
        people: Vec<String>,
        pin_height: f32,
    },
    Fixed {
        name: String,
        center: Vector,
        zoomlevel: f32,
        start: u32,
        end: u32,
        duration_s: f32,
        people: Vec<String>,
        pin_height: f32,
    },
}

impl Renderable {
    pub fn name(&self) -> &str {
        match &self {
            Renderable::Image { name, .. } => name,
            Renderable::Fixed { name, .. } => name,
        }
    }

    pub fn to_frames(&self) -> Vec<Frame> {
        match self {
            Renderable::Image {
                name: _,
                center: _,
                zoomlevel: _,
                time: _,
                people: _,
                pin_height: _,
            } => unreachable!(),

            Renderable::Fixed {
                name: _,
                center,
                zoomlevel,
                start,
                end,
                duration_s,
                people,
                pin_height,
            } => {
                let frames_tot = (duration_s * FRAME_RATE).round() as u32;
                let mut frames = Vec::new();
                for i in 0..frames_tot {
                    let time = start
                        + (((end - start) as f32) * (i as f32 / frames_tot as f32)).round() as u32;
                    frames.push(Frame {
                        scene_pos: ScenePos::new(*center, *zoomlevel, time),
                        people: people.clone(),
                        pin_height: *pin_height,
                    });
                }
                frames
            }
        }
    }

    pub fn make_file(self) -> Result<()> {
        if let Self::Image {
            name,
            center,
            zoomlevel,
            time,
            people,
            pin_height,
        } = self
        {
            let frame = Frame {
                scene_pos: ScenePos::new(center, zoomlevel, time),
                people: people.clone(),
                pin_height,
            };
            info!("loading tiles for {name}");
            WORLD.load_tiles_at(frame.scene_pos)?;
            info!("finished loading tiles for {name}");
            let image: skia_safe::Image = frame.render();
            let mut file = std::fs::File::create(&format!("{OUT_PATH}/{name}.png"))?;
            skia_safe::png_encoder::encode(
                &image.peek_pixels().expect("failed to get pixels."),
                &mut file,
                &skia_safe::png_encoder::Options::default(),
            );
            return Ok(());
        }
        let path_string = &format!("{OUT_PATH}/tmp");
        let tmp_path = std::path::Path::new(&path_string);
        if tmp_path.exists() {
            fs::remove_dir_all(tmp_path)?;
            fs::create_dir_all(tmp_path)?;
        } else {
            fs::create_dir_all(tmp_path)?;
        }
        let name = self.name();

        let frames = self.to_frames();
        info!("loading tiles for {name}");
        for frame in &frames {
            WORLD.load_tiles_at(frame.scene_pos)?;
        }
        info!("finished loading tiles for {name}");
        frames.par_iter().enumerate().for_each(|(i, frame)| {
            let image: skia_safe::Image = frame.render();
            let mut file =
                std::fs::File::create(tmp_path.join(format!("frame{i:0>8}.png"))).unwrap();
            skia_safe::png_encoder::encode(
                &image.peek_pixels().expect("failed to get pixels."),
                &mut file,
                &skia_safe::png_encoder::Options::default(),
            );
        });
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
            .arg(format!("{OUT_PATH}/out.mp4"))
            .output()?;

        fs::remove_dir_all(tmp_path)?;

        Ok(())
    }
}
