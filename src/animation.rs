use crate::{Sprite, Texture, Vec2, vec2};
use std::{
    ops::{Deref, DerefMut},
    sync::Arc,
};

/// Decoded atlas frame. `rect` is the packed texture rectangle, before unrotation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub rect: [f32; 4],
    pub source_size: Vec2,
    pub offset: Vec2,
    pub rotated: bool,
    pub flip_x: bool,
    pub flip_y: bool,
    pub empty: bool,
}
impl Frame {
    pub fn new(rect: [f32; 4]) -> Self {
        Self {
            rect,
            source_size: vec2(rect[2], rect[3]),
            offset: Vec2::ZERO,
            rotated: false,
            flip_x: false,
            flip_y: false,
            empty: false,
        }
    }
}
// Flags occupy the high four bits of the name length. Geometry retains full f32 precision.
#[derive(Clone, Copy)]
struct Name {
    start: u32,
    len_flags: u32,
}
const NAME_MASK: u32 = (1 << 28) - 1;
const _: () = assert!(std::mem::size_of::<[f32; 8]>() == 32);

/// Shared atlas: contiguous geometry and UTF-8 names, one texture, no per-frame allocation.
pub struct SpriteFrames {
    pub texture: Arc<Texture>,
    geometry: Vec<[f32; 8]>,
    names: Vec<Name>,
    text: String,
    // True only if every nonempty frame stays inside its source rectangle,
    // including after either flip. Enables conservative anchor rejection.
    pub(crate) contained: bool,
}
impl SpriteFrames {
    pub fn new(texture: Arc<Texture>) -> Self {
        Self {
            texture,
            geometry: Vec::new(),
            names: Vec::new(),
            text: String::new(),
            contained: true,
        }
    }
    pub fn len(&self) -> usize {
        self.geometry.len()
    }
    pub fn is_empty(&self) -> bool {
        self.geometry.is_empty()
    }
    pub fn name(&self, index: usize) -> Option<&str> {
        let n = self.names.get(index)?;
        self.text
            .get(n.start as usize..n.start as usize + (n.len_flags & NAME_MASK) as usize)
    }
    pub fn find(&self, name: &str) -> Option<usize> {
        (0..self.len())
            .find(|&i| self.names[i].len_flags & (1 << 31) == 0 && self.name(i) == Some(name))
    }
    pub fn get(&self, index: usize) -> Option<Frame> {
        let g = self.geometry.get(index)?;
        let f = self.names[index].len_flags >> 28;
        Some(Frame {
            rect: [g[0], g[1], g[2], g[3]],
            source_size: vec2(g[4], g[5]),
            offset: vec2(g[6], g[7]),
            rotated: f & 1 != 0,
            flip_x: f & 2 != 0,
            flip_y: f & 4 != 0,
            empty: f & 8 != 0,
        })
    }
    /// Duplicate nonempty names keep their first frame, matching Flixel.
    pub fn add_frame(&mut self, name: &str, frame: Frame) -> Result<usize, String> {
        if !frame.empty
            && let Some(i) = self.find(name)
        {
            return Ok(i);
        }
        self.push_frame(name, frame)
    }
    fn push_frame(&mut self, name: &str, mut frame: Frame) -> Result<usize, String> {
        let [x, y, w, h] = frame.rect;
        if ![
            x,
            y,
            w,
            h,
            frame.source_size.x,
            frame.source_size.y,
            frame.offset.x,
            frame.offset.y,
        ]
        .iter()
        .all(|v| v.is_finite())
            || w < 0.
            || h < 0.
            || frame.source_size.x < 0.
            || frame.source_size.y < 0.
        {
            return Err("invalid frame geometry".into());
        }
        if name.len() > NAME_MASK as usize
            || self
                .text
                .len()
                .checked_add(name.len())
                .is_none_or(|n| n > u32::MAX as usize)
        {
            return Err("atlas names exceed 4 GiB".into());
        }
        let tw = self.texture.width() as f32;
        let th = self.texture.height() as f32;
        frame.rect = if frame.empty {
            [0.; 4]
        } else {
            let left = x.clamp(0., tw);
            let top = y.clamp(0., th);
            [
                left,
                top,
                (x + w).clamp(0., tw) - left,
                (y + h).clamp(0., th) - top,
            ]
        };
        if !frame.empty {
            let (w, h) = if frame.rotated {
                (frame.rect[3], frame.rect[2])
            } else {
                (frame.rect[2], frame.rect[3])
            };
            self.contained &= frame.offset.x >= 0.
                && frame.offset.y >= 0.
                && frame.source_size.x - frame.offset.x - w >= 0.
                && frame.source_size.y - frame.offset.y - h >= 0.;
        }
        let flags = u32::from(frame.rotated)
            | u32::from(frame.flip_x) << 1
            | u32::from(frame.flip_y) << 2
            | u32::from(frame.empty) << 3;
        self.names.push(Name {
            start: self.text.len() as u32,
            len_flags: name.len() as u32 | flags << 28,
        });
        self.text.push_str(name);
        self.geometry.push([
            frame.rect[0],
            frame.rect[1],
            frame.rect[2],
            frame.rect[3],
            frame.source_size.x,
            frame.source_size.y,
            frame.offset.x,
            frame.offset.y,
        ]);
        Ok(self.len() - 1)
    }
    pub fn shrink_to_fit(&mut self) {
        self.geometry.shrink_to_fit();
        self.names.shrink_to_fit();
        self.text.shrink_to_fit();
    }
    /// Heap storage excluding texture, allocator bookkeeping, and the collection header.
    pub fn metadata_bytes(&self) -> usize {
        self.geometry.capacity() * 32 + self.names.capacity() * 8 + self.text.capacity()
    }
    pub fn from_sparrow(texture: Arc<Texture>, xml: &str) -> Result<Self, String> {
        let (_, entries) = parse_sparrow(xml)?;
        Self::from_entries(texture, entries)
    }
    pub(crate) fn from_entries(
        texture: Arc<Texture>,
        entries: Vec<(String, Frame)>,
    ) -> Result<Self, String> {
        let mut result = Self::new(texture);
        let mut seen = std::collections::HashSet::new();
        for (name, frame) in &entries {
            if !frame.empty && !seen.insert(name.as_str()) {
                continue;
            }
            result.push_frame(name, *frame)?;
        }
        result.shrink_to_fit();
        Ok(result)
    }
}

pub(crate) type Sparrow = (String, Vec<(String, Frame)>);
pub(crate) fn parse_sparrow(xml: &str) -> Result<Sparrow, String> {
    use quick_xml::{Reader, events::Event};
    let mut reader = Reader::from_str(xml);
    let mut image = None;
    let mut entries = Vec::new();
    let mut depth = 0usize;
    let mut closed = false;
    loop {
        let event = reader.read_event().map_err(|e| e.to_string())?;
        let empty = matches!(&event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let tag = e.name();
                let attrs = e
                    .attributes()
                    .map(|a| {
                        let a = a.map_err(|e| e.to_string())?;
                        Ok((
                            a.key.into_inner(),
                            a.decode_and_unescape_value(reader.decoder())
                                .map_err(|e| e.to_string())?,
                        ))
                    })
                    .collect::<Result<std::collections::HashMap<_, _>, String>>()?;
                if depth == 0 {
                    if closed || image.is_some() || tag.as_ref() != b"TextureAtlas" {
                        return Err("expected one TextureAtlas root".into());
                    }
                    image = Some(
                        attrs
                            .get(b"imagePath".as_slice())
                            .map(|s| s.to_string())
                            .unwrap_or_default(),
                    );
                    if empty {
                        closed = true;
                    }
                } else if depth == 1 && tag.as_ref() == b"SubTexture" {
                    let required = |key: &str| {
                        attrs
                            .get(key.as_bytes())
                            .ok_or_else(|| format!("missing Sparrow attribute {key}"))
                    };
                    let number = |key: &str| -> Result<f32, String> {
                        let value = required(key)?
                            .parse::<f32>()
                            .map_err(|_| format!("invalid Sparrow number {key}"))?;
                        if !value.is_finite() {
                            return Err(format!("nonfinite Sparrow number {key}"));
                        }
                        Ok(value)
                    };
                    let integer = |key: &str| -> Result<f32, String> {
                        haxe_int(required(key)?.as_ref())
                            .map(|n| n as f32)
                            .ok_or_else(|| format!("invalid Sparrow integer {key}"))
                    };
                    let yes = |key: &str| {
                        attrs
                            .get(key.as_bytes())
                            .is_some_and(|v| v.as_ref() == "true")
                    };
                    let mut frame = Frame::new([
                        number("x")?,
                        number("y")?,
                        number("width")?,
                        number("height")?,
                    ]);
                    let trimmed = attrs.contains_key(b"frameX".as_slice());
                    frame.rotated = yes("rotated");
                    frame.flip_x = yes("flipX");
                    frame.flip_y = yes("flipY");
                    if trimmed {
                        frame.offset = vec2(-integer("frameX")?, -integer("frameY")?);
                        frame.source_size = vec2(integer("frameWidth")?, integer("frameHeight")?);
                    } else if frame.rotated {
                        frame.source_size = vec2(frame.rect[3], frame.rect[2]);
                    }
                    if frame.rect[2] == 0. || frame.rect[3] == 0. {
                        frame.empty = true;
                        frame.rotated = false;
                        frame.flip_x = false;
                        frame.flip_y = false;
                        if !trimmed {
                            frame.source_size = Vec2::ONE;
                        }
                    }
                    entries.push((required("name")?.to_string(), frame));
                }
                if !empty {
                    depth += 1;
                }
            }
            Event::End(_) => {
                depth = depth.checked_sub(1).ok_or("unexpected XML closing tag")?;
                if depth == 0 {
                    closed = true;
                }
            }
            Event::Text(t) if depth == 0 && t.as_ref().iter().any(|b| !b.is_ascii_whitespace()) => {
                return Err("text outside TextureAtlas".into());
            }
            Event::DocType(_) => return Err("DTD is not supported in Sparrow atlases".into()),
            Event::Eof => break,
            _ => {}
        }
    }
    if !closed || depth != 0 {
        return Err("incomplete TextureAtlas".into());
    }
    Ok((image.ok_or("missing TextureAtlas")?, entries))
}

// Common sequential clips need no index allocation. Arbitrary/repeated/reversed
// sequences retain their exact order in a boxed slice.
enum FrameSequence {
    Range { start: u32, len: usize },
    Indices(Box<[u32]>),
}
impl FrameSequence {
    fn new(frames: &[u32]) -> Self {
        if frames
            .windows(2)
            .all(|pair| pair[0].checked_add(1) == Some(pair[1]))
        {
            Self::Range {
                start: frames[0],
                len: frames.len(),
            }
        } else {
            Self::Indices(frames.into())
        }
    }
    fn len(&self) -> usize {
        match self {
            Self::Range { len, .. } => *len,
            Self::Indices(f) => f.len(),
        }
    }
    fn get(&self, index: usize) -> u32 {
        match self {
            Self::Range { start, .. } => *start + index as u32,
            Self::Indices(f) => f[index],
        }
    }
}
struct Animation {
    name: Box<str>,
    frames: FrameSequence,
    fps: f32,
    looping: bool,
}
/// A Sprite plus shared atlas and a small playback controller. Sprite fields/methods
/// are available through Deref. Game advances scene-owned animations automatically; standalone users call update(dt).
pub struct AnimatedSprite {
    sprite: Sprite,
    pub frames: Arc<SpriteFrames>,
    animations: Vec<Animation>,
    current: Option<usize>,
    cursor: usize,
    elapsed: f64,
    playing: bool,
    active_len: usize,
    active_fps: f32,
    active_looping: bool,
}
impl Deref for AnimatedSprite {
    type Target = Sprite;
    fn deref(&self) -> &Sprite {
        &self.sprite
    }
}
impl DerefMut for AnimatedSprite {
    fn deref_mut(&mut self) -> &mut Sprite {
        &mut self.sprite
    }
}
impl AnimatedSprite {
    pub fn new(frames: Arc<SpriteFrames>) -> Self {
        let sprite = Sprite::new(frames.texture.clone());
        Self {
            sprite,
            frames,
            animations: Vec::new(),
            current: None,
            cursor: 0,
            elapsed: 0.,
            playing: false,
            active_len: 0,
            active_fps: 0.,
            active_looping: false,
        }
    }
    /// Center once on `"X"`, `"Y"`, or `"XY"`, using the current frame's
    /// untrimmed source size and scale (or the explicit size override).
    /// Uses the assigned surface, or the default 1280×720 logical surface.
    /// Panics for unsupported axes, as with `Sprite::center`.
    pub fn center(&mut self, axes: &str) {
        let size = self
            .sprite
            .size
            .unwrap_or_else(|| self.atlas_frame().source_size * self.sprite.scale)
            .abs();
        self.sprite.center_size(axes, size);
    }
    /// Current frame within the current animation (zero when none is selected).
    pub fn frame(&self) -> usize {
        self.cursor
    }
    pub fn animation(&self) -> Option<&str> {
        self.current.map(|i| self.animations[i].name.as_ref())
    }
    pub fn playing(&self) -> bool {
        self.playing
    }
    pub fn framerate(&self) -> f32 {
        self.active_fps
    }
    pub fn add_animation(
        &mut self,
        name: &str,
        frames: &[u32],
        fps: f32,
        looping: bool,
    ) -> Result<(), String> {
        if frames.is_empty()
            || frames.iter().any(|&i| i as usize >= self.frames.len())
            || !fps.is_finite()
            || fps < 0.
        {
            return Err("invalid animation frames or framerate".into());
        }
        let a = Animation {
            name: name.into(),
            frames: FrameSequence::new(frames),
            fps,
            looping,
        };
        if let Some(i) = self.animations.iter().position(|a| a.name.as_ref() == name) {
            self.animations[i] = a;
            if self.current == Some(i) {
                self.cursor = 0;
                self.elapsed = 0.;
                self.refresh_active();
                self.sync();
            }
        } else {
            self.animations.push(a);
        }
        Ok(())
    }
    pub fn add_by_prefix(
        &mut self,
        name: &str,
        prefix: &str,
        fps: f32,
        looping: bool,
    ) -> Result<(), String> {
        let mut indices: Vec<u32> = (0..self.frames.len())
            .filter(|&i| self.frames.name(i).unwrap().starts_with(prefix))
            .map(|i| i as u32)
            .collect();
        // Flixel parses the number immediately after the prefix and uses its absolute
        // value. Stable sorting preserves XML order for equal/non-numeric suffixes.
        indices.sort_by_key(|&i| {
            let suffix = &self.frames.name(i as usize).unwrap()[prefix.len()..];
            haxe_int(suffix).unwrap_or(0).unsigned_abs()
        });
        self.add_animation(name, &indices, fps, looping)
    }
    pub fn play(&mut self, name: &str) -> Result<(), String> {
        let i = self
            .animations
            .iter()
            .position(|a| a.name.as_ref() == name)
            .ok_or_else(|| format!("unknown animation: {name}"))?;
        self.current = Some(i);
        self.refresh_active();
        self.cursor = 0;
        self.elapsed = 0.;
        self.playing = true;
        self.sync();
        Ok(())
    }
    pub fn pause(&mut self) {
        self.playing = false;
    }
    pub fn resume(&mut self) {
        self.playing = self.current.is_some();
    }
    pub fn stop(&mut self) {
        self.playing = false;
        self.cursor = 0;
        self.elapsed = 0.;
        self.sync();
    }
    pub fn set_frame(&mut self, frame: usize) -> Result<(), String> {
        let i = self.current.ok_or("no animation selected")?;
        if frame >= self.animations[i].frames.len() {
            return Err("animation frame out of range".into());
        }
        self.cursor = frame;
        self.elapsed = 0.;
        self.sync();
        Ok(())
    }
    pub fn set_framerate(&mut self, name: &str, fps: f32) -> Result<(), String> {
        if !fps.is_finite() || fps < 0. {
            return Err("framerate must be finite and nonnegative".into());
        }
        self.animation_mut(name)?.fps = fps;
        self.refresh_active();
        Ok(())
    }
    pub fn set_looping(&mut self, name: &str, looping: bool) -> Result<(), String> {
        self.animation_mut(name)?.looping = looping;
        self.refresh_active();
        Ok(())
    }
    pub fn rename_animation(&mut self, old: &str, new: &str) -> Result<(), String> {
        if self.animations.iter().any(|a| a.name.as_ref() == new) {
            return Err("animation name already exists".into());
        }
        self.animation_mut(old)?.name = new.into();
        Ok(())
    }
    fn animation_mut(&mut self, name: &str) -> Result<&mut Animation, String> {
        self.animations
            .iter_mut()
            .find(|a| a.name.as_ref() == name)
            .ok_or_else(|| "unknown animation".into())
    }
    pub fn update(&mut self, dt: f32) {
        if !dt.is_finite() || dt < 0. {
            return;
        }
        if self.playing {
            let ticks = self.elapsed + f64::from(dt) * f64::from(self.active_fps);
            if ticks < 1. {
                self.elapsed = ticks;
            } else if ticks < 2. {
                // Nearly all game updates cross at most one boundary. Avoid floor,
                // float-to-integer conversion and remainder in this common case.
                self.elapsed = ticks - 1.;
                self.cursor += 1;
                if self.cursor == self.active_len {
                    if self.active_looping {
                        self.cursor = 0;
                    } else {
                        self.cursor -= 1;
                        self.playing = false;
                        self.elapsed = 0.;
                    }
                }
            } else {
                let steps = ticks.floor();
                self.elapsed = ticks - steps;
                let remaining = self.active_len - self.cursor;
                if steps < remaining as f64 {
                    self.cursor += steps as usize;
                } else if self.active_looping {
                    self.cursor = ((steps - remaining as f64) % self.active_len as f64) as usize;
                } else {
                    self.cursor = self.active_len - 1;
                    self.playing = false;
                    self.elapsed = 0.;
                }
            }
        }
        self.sync();
    }
    fn refresh_active(&mut self) {
        if let Some(i) = self.current {
            let a = &self.animations[i];
            self.active_len = a.frames.len();
            self.active_fps = a.fps;
            self.active_looping = a.looping;
        }
    }
    pub(crate) fn sync(&mut self) {
        if !Arc::ptr_eq(&self.sprite.texture, &self.frames.texture) {
            self.sprite.texture = self.frames.texture.clone();
        }
    }
    pub(crate) fn atlas_frame(&self) -> Frame {
        let index = self
            .current
            .map_or(0, |i| self.animations[i].frames.get(self.cursor) as usize);
        self.frames.get(index).unwrap_or_else(|| {
            let mut f = Frame::new([0.; 4]);
            f.empty = true;
            f
        })
    }
}

// Std.parseInt accepts a numeric prefix (including hexadecimal), ignoring suffixes.
fn haxe_int(value: &str) -> Option<i32> {
    let s = value.trim_start();
    let (negative, s) = if let Some(s) = s.strip_prefix('-') {
        (true, s)
    } else {
        (false, s.strip_prefix('+').unwrap_or(s))
    };
    let (radix, s) = if s.starts_with("0x") || s.starts_with("0X") {
        (16, &s[2..])
    } else {
        (10, s)
    };
    let mut digits = 0;
    let mut value = 0u32;
    for c in s.chars() {
        let Some(digit) = c.to_digit(radix) else {
            break;
        };
        value = value.wrapping_mul(radix).wrapping_add(digit);
        digits += 1;
    }
    (digits != 0).then(|| {
        if negative {
            (value as i32).wrapping_neg()
        } else {
            value as i32
        }
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn texture() -> Arc<Texture> {
        Arc::new(Texture::from_rgba(16, 16, vec![255; 16 * 16 * 4]).unwrap())
    }
    #[test]
    fn compressed_sequences_and_active_mutations() {
        for sequence in [
            &[3, 4, 5][..],
            &[2, 2, 1],
            &[5, 4, 3],
            &[u32::MAX],
            &[0, 2, 4],
        ] {
            let packed = FrameSequence::new(sequence);
            assert_eq!(packed.len(), sequence.len());
            for (i, &index) in sequence.iter().enumerate() {
                assert_eq!(packed.get(i), index);
            }
        }
        assert!(matches!(
            FrameSequence::new(&[3, 4, 5]),
            FrameSequence::Range { .. }
        ));
        assert!(matches!(
            FrameSequence::new(&[3, 3, 5]),
            FrameSequence::Indices(_)
        ));
        let mut atlas = SpriteFrames::new(texture());
        for i in 0..6 {
            atlas
                .add_frame(&i.to_string(), Frame::new([i as f32, 0., 1., 1.]))
                .unwrap();
        }
        let mut s = AnimatedSprite::new(Arc::new(atlas));
        s.add_animation("a", &[2, 3, 4], 4., true).unwrap();
        s.play("a").unwrap();
        s.update(0.25);
        assert_eq!(s.atlas_frame().rect[0], 3.);
        s.set_looping("a", false).unwrap();
        s.update(1.);
        assert!(!s.playing());
        assert_eq!(s.frame(), 2);
        s.add_animation("a", &[1, 0], 8., true).unwrap();
        assert_eq!(s.framerate(), 8.);
        s.resume();
        s.update(0.125);
        assert_eq!(s.atlas_frame().rect[0], 0.);
        s.set_framerate("a", 0.).unwrap();
        s.update(2.);
        assert_eq!(s.frame(), 1);
    }
    #[test]
    fn fast_advance_matches_general_algorithm() {
        let mut atlas = SpriteFrames::new(texture());
        for i in 0..7 {
            atlas
                .add_frame(&i.to_string(), Frame::new([i as f32, 0., 1., 1.]))
                .unwrap();
        }
        let atlas = Arc::new(atlas);
        for looping in [false, true] {
            let mut s = AnimatedSprite::new(atlas.clone());
            s.add_animation("a", &[0, 1, 2, 3, 4, 5, 6], 24., looping)
                .unwrap();
            s.play("a").unwrap();
            let (mut cursor, mut elapsed, mut playing) = (0usize, 0f64, true);
            for dt in [0., 1. / 60., 1. / 24., 0.08, 0.125, 1., 100., f32::MAX] {
                if playing {
                    let ticks = elapsed + f64::from(dt) * 24.;
                    elapsed = ticks;
                    if ticks >= 1. {
                        let steps = ticks.floor();
                        elapsed = ticks - steps;
                        let remaining = 7 - cursor;
                        if steps < remaining as f64 {
                            cursor += steps as usize;
                        } else if looping {
                            cursor = ((steps - remaining as f64) % 7.) as usize;
                        } else {
                            cursor = 6;
                            playing = false;
                            elapsed = 0.;
                        }
                    }
                }
                s.update(dt);
                assert_eq!(
                    (s.frame(), s.playing(), s.elapsed),
                    (cursor, playing, elapsed)
                );
            }
        }
    }
    #[test]
    fn prefix_sort_is_numeric_and_stable() {
        let mut f = SpriteFrames::new(texture());
        for (i, name) in [
            "idle10.png",
            "idle2.png",
            "idle-1.png",
            "idlefoo.png",
            "idle01.png",
        ]
        .iter()
        .enumerate()
        {
            f.add_frame(name, Frame::new([i as f32, 0., 1., 1.]))
                .unwrap();
        }
        let mut s = AnimatedSprite::new(Arc::new(f));
        s.add_by_prefix("idle", "idle", 1., true).unwrap();
        s.play("idle").unwrap();
        for expected in [3., 2., 4., 1., 0., 3.] {
            assert_eq!(s.atlas_frame().rect[0], expected);
            s.update(1.);
        }
    }
    #[test]
    fn size_override_trim_culling_and_replacing_frames() {
        let mut f = SpriteFrames::new(texture());
        let mut frame = Frame::new([0., 0., 2., 3.]);
        frame.offset = vec2(-4., 1.);
        frame.source_size = vec2(10., 10.);
        f.add_frame("a", frame).unwrap();
        let mut s = AnimatedSprite::new(Arc::new(f));
        s.position = vec2(20., 0.);
        s.scale = vec2(9., 9.);
        s.size = Some(vec2(20., 30.));
        let (position, size, _) = s.geometry(Some(s.atlas_frame()));
        assert_eq!(position, vec2(12., 3.));
        assert_eq!(size, vec2(4., 9.));
        assert!(crate::intersects(position, size, vec2(20., 20.)));
        s.add_animation("a", &[0], 1., false).unwrap();
        s.play("a").unwrap();
        s.frames = Arc::new(SpriteFrames::new(texture()));
        s.update(0.);
        assert!(s.atlas_frame().empty);
        assert!(Arc::ptr_eq(&s.texture, &s.frames.texture));
    }
    #[test]
    fn empty_names_do_not_hide_nonempty_duplicates() {
        let xml = "<TextureAtlas><SubTexture name='a' x='0' y='0' width='0' height='4'/><SubTexture name='a' x='1' y='2' width='3' height='4'/><SubTexture name='a' x='4' y='5' width='2' height='3'/></TextureAtlas>";
        let f = SpriteFrames::from_sparrow(texture(), xml).unwrap();
        assert_eq!(f.len(), 2);
        assert_eq!(f.find("a"), Some(1));
        assert_eq!(f.get(1).unwrap().rect, [1., 2., 3., 4.]);
    }
}
