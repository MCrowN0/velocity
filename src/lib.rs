pub mod audio;
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
pub struct Texture {
    width: u16,
    height: u16,
    pixels: Box<[u8]>,
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
        })
    }
    pub fn from_file_bytes(bytes: &[u8]) -> Result<Self, String> {
        let img = image::load_from_memory(bytes).unwrap();
        let (width, height) = img.dimensions();
        let rgba = img.into_rgba8().into_vec();
        Self::from_rgba(width as u16, height as u16, rgba)
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
}
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
    pub(crate) fn signed_size(&self) -> Vec2 {
        self.size
            .unwrap_or(vec2(self.texture.width as f32, self.texture.height as f32) * self.scale)
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
    let c = sprite.color;
    if !sprite.visible || c.a <= 0. || ![c.r, c.g, c.b, c.a].iter().all(|v| v.is_finite()) {
        return None;
    }
    let size = sprite.signed_size().abs();
    intersects(sprite.position, size, bounds).then_some(size)
}
#[cfg(test)]
mod tests;

mod game;
pub use game::{
    Game, Scene, get_fps, get_key_just_pressed, get_key_pressed, get_mouse_button_pressed,
    get_mouse_position, quit, switch_scene,
};
