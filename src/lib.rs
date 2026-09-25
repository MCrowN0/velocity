pub mod assets;
pub mod audio;
mod key;
pub use key::Key;
#[allow(dead_code, unused_imports, clippy::all)]
#[path = "../vendor/avif/mod.rs"]
mod avif;
pub use assets::{Asset, Assets, TextureCompression, TextureLoadOptions, get_assets};
mod renderer;
pub use audio::{AudioBackend, AudioEngine, AudioLoadMode, AudioPlayer, AudioSource, play_audio};
mod vulkan;
pub mod window;
use image::GenericImageView;
pub use renderer::{RenderStats, Renderer};
use std::sync::Arc;
pub use window::{Input, Window};

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}
pub const fn vec2(x: f32, y: f32) -> Vec2 {
    Vec2 { x, y }
}
impl Vec2 {
    pub const ZERO: Self = vec2(0., 0.);
    pub const ONE: Self = vec2(1., 1.);
    pub fn abs(self) -> Self {
        vec2(self.x.abs(), self.y.abs())
    }
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}
impl std::ops::Mul for Vec2 {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self {
        vec2(self.x * rhs.x, self.y * rhs.y)
    }
}
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}
impl Color {
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }
    pub const WHITE: Self = Self::new(1., 1., 1., 1.);
    pub const BLACK: Self = Self::new(0., 0., 0., 1.);
    pub const TRANSPARENT: Self = Self::new(0., 0., 0., 0.);
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TextureFilter {
    #[default]
    Linear,
    Nearest,
}
pub struct Texture {
    pub filter: TextureFilter,
    width: u16,
    height: u16,
    pixels: Box<[u8]>,
    pub(crate) gpu: Option<assets::GpuHandle>,
}
impl Drop for Texture {
    fn drop(&mut self) {
        if let Some(gpu) = &self.gpu
            && let Some(retired) = gpu.retired.upgrade()
        {
            retired.lock().unwrap().push(gpu.key);
        }
    }
}
impl Texture {
    pub fn from_rgba(width: u16, height: u16, pixels: Vec<u8>) -> Result<Self, String> {
        if width == 0 || height == 0 || pixels.len() != usize::from(width) * usize::from(height) * 4
        {
            return Err("expected nonzero dimensions and width * height * 4 RGBA bytes".into());
        }
        Ok(Self {
            width,
            height,
            pixels: pixels.into_boxed_slice(),
            filter: TextureFilter::default(),
            gpu: None,
        })
    }
    pub fn from_file_bytes(bytes: &[u8]) -> Result<Self, String> {
        let img = assets::decode(bytes)?;
        let (width, height) = img.dimensions();
        let rgba = img.into_rgba8().into_vec();
        Self::from_rgba(
            width
                .try_into()
                .map_err(|_| "texture width exceeds 65535")?,
            height
                .try_into()
                .map_err(|_| "texture height exceeds 65535")?,
            rgba,
        )
    }
    pub fn width(&self) -> u16 {
        self.width
    }
    pub fn height(&self) -> u16 {
        self.height
    }
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Surface {
    pub(crate) renderer: u64,
    pub(crate) index: usize,
    pub(crate) logical_size: [u16; 2],
}
mod animation;
pub use animation::{AnimatedSprite, Frame, SpriteFrames};
#[derive(Clone)]
pub struct Sprite {
    pub texture: Arc<Texture>,
    pub position: Vec2,
    pub scale: Vec2,
    pub size: Option<Vec2>,
    pub visible: bool,
    pub color: Color,
    /// `None` renders to `Renderer::default_surface`.
    pub surface: Option<Surface>,
}
impl Sprite {
    pub fn new(texture: Arc<Texture>) -> Self {
        Self {
            texture,
            position: Vec2::ZERO,
            scale: Vec2::ONE,
            size: None,
            visible: true,
            color: Color::WHITE,
            surface: None,
        }
    }
    pub fn set_position(&mut self, position: Vec2) {
        self.position = position;
    }
    /// Center once on `"X"`, `"Y"`, or `"XY"` within the assigned surface,
    /// or the default 1280×720 logical surface when none is assigned.
    /// Size overrides scale; negative dimensions are treated as flips.
    /// Panics if `axes` is not one of the three supported values.
    pub fn center(&mut self, axes: &str) {
        self.center_size(axes, self.signed_size().abs());
    }
    pub(crate) fn center_size(&mut self, axes: &str, size: Vec2) {
        assert!(
            matches!(axes, "X" | "Y" | "XY"),
            "center axes must be X, Y, or XY"
        );
        let [width, height] = self.surface.map_or([1280, 720], |s| s.logical_size);
        if axes != "Y" {
            self.position.x = (f32::from(width) - size.x) * 0.5;
        }
        if axes != "X" {
            self.position.y = (f32::from(height) - size.y) * 0.5;
        }
    }
    pub(crate) fn geometry(&self, frame: Option<Frame>) -> (Vec2, Vec2, [f32; 4]) {
        let signed = self.size.unwrap_or_else(|| {
            frame.map_or_else(
                || vec2(self.texture.width as f32, self.texture.height as f32),
                |f| f.source_size,
            ) * self.scale
        });
        let size = signed.abs();
        let Some(f) = frame else {
            return (
                self.position,
                size,
                [
                    if signed.x < 0. { 1. } else { 0. },
                    if signed.y < 0. { 1. } else { 0. },
                    if signed.x < 0. { -1. } else { 1. },
                    if signed.y < 0. { -1. } else { 1. },
                ],
            );
        };
        if f.empty || f.source_size.x == 0. || f.source_size.y == 0. {
            return (self.position, Vec2::ZERO, [0.; 4]);
        }
        let scale = vec2(size.x / f.source_size.x, size.y / f.source_size.y);
        let trimmed = if f.rotated {
            vec2(f.rect[3], f.rect[2])
        } else {
            vec2(f.rect[2], f.rect[3])
        };
        let flip_x = (signed.x < 0.) ^ f.flip_x;
        let flip_y = (signed.y < 0.) ^ f.flip_y;
        let offset = vec2(
            if flip_x {
                f.source_size.x - f.offset.x - trimmed.x
            } else {
                f.offset.x
            },
            if flip_y {
                f.source_size.y - f.offset.y - trimmed.y
            } else {
                f.offset.y
            },
        ) * scale;
        let tw = self.texture.width as f32;
        let th = self.texture.height as f32;
        let (fx, fy) = if f.rotated {
            (flip_y, flip_x)
        } else {
            (flip_x, flip_y)
        };
        let uv = [
            (f.rect[0] + if fx { f.rect[2] } else { 0. }) / tw,
            (f.rect[1] + if fy { f.rect[3] } else { 0. }) / th,
            f.rect[2] / tw * if fx { -1. } else { 1. },
            f.rect[3] / th * if fy { -1. } else { 1. },
        ];
        (
            vec2(self.position.x + offset.x, self.position.y + offset.y),
            trimmed * scale,
            uv,
        )
    }
    pub(crate) fn signed_size(&self) -> Vec2 {
        self.size.unwrap_or_else(|| {
            vec2(self.texture.width as f32, self.texture.height as f32) * self.scale
        })
    }
}
pub(crate) fn intersects(position: Vec2, size: Vec2, bounds: Vec2) -> bool {
    position.is_finite()
        && size.is_finite()
        && size.x > 0.
        && size.y > 0.
        && position.x < bounds.x
        && position.y < bounds.y
        && position.x + size.x > 0.
        && position.y + size.y > 0.
}
pub(crate) fn drawable_size(sprite: &Sprite, bounds: Vec2) -> Option<Vec2> {
    // Positive extents cannot reach back into the viewport from its right/bottom.
    // Reject before touching the texture, scale, or the remaining color channels.
    if !sprite.visible || sprite.position.x >= bounds.x || sprite.position.y >= bounds.y {
        return None;
    }
    let c = sprite.color;
    if c.a <= 0. || ![c.r, c.g, c.b, c.a].iter().all(|v| v.is_finite()) {
        return None;
    }
    let size = sprite.signed_size().abs();
    intersects(sprite.position, size, bounds).then_some(size)
}
#[cfg(test)]
mod tests;

mod game;
pub use game::{
    Game, SceneBehavior, get_fps, get_key_just_pressed, get_key_pressed, get_mouse_button_pressed,
    get_mouse_position, quit, switch_scene,
};

mod scene;
pub use scene::{Handle, Scene, SceneObject};
