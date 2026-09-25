use crate::{
    Texture, TextureFilter,
    vulkan::{Device, Image},
};
use ash::vk;
use std::{
    any::{Any, TypeId},
    cell::{Cell, RefCell},
    collections::HashMap,
    path::{Path, PathBuf},
    rc::{Rc, Weak as LocalWeak},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TextureCompression {
    BC1,
    BC2,
    BC3,
    BC4,
    BC5,
    BC6H,
    #[default]
    BC7,
    RGBA8,
}

impl TextureCompression {
    pub(crate) fn format(self) -> vk::Format {
        match self {
            Self::BC1 => vk::Format::BC1_RGB_UNORM_BLOCK,
            Self::BC2 => vk::Format::BC2_UNORM_BLOCK,
            Self::BC3 => vk::Format::BC3_UNORM_BLOCK,
            Self::BC4 => vk::Format::BC4_UNORM_BLOCK,
            Self::BC5 => vk::Format::BC5_UNORM_BLOCK,
            Self::BC6H => vk::Format::BC6H_UFLOAT_BLOCK,
            Self::BC7 => vk::Format::BC7_UNORM_BLOCK,
            Self::RGBA8 => vk::Format::R8G8B8A8_UNORM,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TextureLoadOptions {
    pub compression: TextureCompression,
    pub filter: TextureFilter,
}

mod sealed {
    pub trait Sealed {}
}
pub trait Asset: sealed::Sealed + Any + Send + Sync {
    fn load_asset(
        assets: &Assets,
        path: &Path,
        options: TextureLoadOptions,
    ) -> Result<Arc<Self>, String>;
}
impl sealed::Sealed for Texture {}
impl Asset for Texture {
    fn load_asset(
        assets: &Assets,
        path: &Path,
        options: TextureLoadOptions,
    ) -> Result<Arc<Self>, String> {
        let id = assets.inner.device.load(Ordering::Acquire);
        let uploader = UPLOADERS
            .with(|u| u.borrow().get(&id).and_then(LocalWeak::upgrade))
            .ok_or("texture loading requires Game::run's main thread or Renderer::attach_assets")?;
        uploader
            .device
            .check_texture_format(options.compression.format())?;
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let decoded = decode(&bytes)?;
        drop(bytes);
        let width = u16::try_from(decoded.width()).map_err(|_| "texture width exceeds 65535")?;
        let height = u16::try_from(decoded.height()).map_err(|_| "texture height exceeds 65535")?;
        if width == 0 || height == 0 {
            return Err("empty texture".into());
        }
        if u32::from(width.max(height)) > uploader.device.properties.limits.max_image_dimension2_d {
            return Err("image dimensions exceed GPU limits".into());
        }
        let data = encode(decoded, options.compression);
        let image = Image::with_format(
            &uploader.device,
            width.into(),
            height.into(),
            false,
            options.compression.format(),
        )?;
        image.upload(&data)?;
        uploader.collect_unused();
        uploader.garbage.borrow_mut().clear();
        let key = uploader.next_key.get();
        uploader
            .next_key
            .set(key.checked_add(1).ok_or("texture handle space exhausted")?);
        let texture = Arc::new(Texture {
            width,
            height,
            pixels: Box::default(),
            filter: options.filter,
            gpu: Some(GpuHandle {
                owner: id,
                key,
                retired: Arc::downgrade(&uploader.retired),
            }),
        });
        uploader.textures.borrow_mut().insert(key, image);
        Ok(texture)
    }
}

impl sealed::Sealed for crate::SpriteFrames {}
impl Asset for crate::SpriteFrames {
    fn load_asset(
        assets: &Assets,
        path: &Path,
        options: TextureLoadOptions,
    ) -> Result<Arc<Self>, String> {
        let xml = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let (image_path, entries) = crate::animation::parse_sparrow(&xml)?;
        let image_path = normalize(&image_path)?;
        let image = path.parent().ok_or("atlas has no parent")?.join(image_path);
        let index = assets.inner.index.as_ref().map_err(Clone::clone)?;
        let key = index
            .iter()
            .find(|(_, p)| **p == image)
            .map(|(key, _)| key)
            .ok_or("atlas image is not indexed")?;
        let texture = assets.load_with::<Texture>(key, options)?;
        Ok(Arc::new(Self::from_entries(texture, entries)?))
    }
}

type CacheKey = (String, TypeId, TextureLoadOptions);
struct Inner {
    index: Result<HashMap<String, PathBuf>, String>,
    cache: Mutex<HashMap<CacheKey, Arc<dyn Any + Send + Sync>>>,
    device: AtomicU64,
}

#[derive(Clone)]
pub struct Assets {
    inner: Arc<Inner>,
}

impl Default for Assets {
    fn default() -> Self {
        Self::new()
    }
}
impl Assets {
    pub fn new() -> Self {
        let index = (|| {
            let root = std::env::current_dir()
                .map_err(|e| e.to_string())?
                .join("assets");
            let mut index = HashMap::new();
            match std::fs::metadata(&root) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(index),
                Err(e) => return Err(e.to_string()),
                Ok(_) => {}
            }
            index_folder(&root, &root, &mut index)?;
            Ok(index)
        })();
        Self {
            inner: Arc::new(Inner {
                index,
                cache: Mutex::new(HashMap::new()),
                device: AtomicU64::new(0),
            }),
        }
    }
    pub fn load<T: Asset>(&self, key: &str) -> Result<Arc<T>, String> {
        self.load_with(key, TextureLoadOptions::default())
    }
    pub fn load_with<T: Asset>(
        &self,
        key: &str,
        options: TextureLoadOptions,
    ) -> Result<Arc<T>, String> {
        self.get(key, options, false)
    }
    pub fn preload<T: Asset>(&self, key: &str) -> Result<Arc<T>, String> {
        self.preload_with(key, TextureLoadOptions::default())
    }
    pub fn preload_with<T: Asset>(
        &self,
        key: &str,
        options: TextureLoadOptions,
    ) -> Result<Arc<T>, String> {
        self.get(key, options, true)
    }
    fn get<T: Asset>(
        &self,
        key: &str,
        options: TextureLoadOptions,
        retain: bool,
    ) -> Result<Arc<T>, String> {
        let key = normalize(key)?;
        let index = self.inner.index.as_ref().map_err(Clone::clone)?;
        let path = index
            .get(&key)
            .ok_or_else(|| format!("asset not indexed: {key}"))?;
        let cache_key = (key, TypeId::of::<T>(), options);
        if let Some(value) = self.inner.cache.lock().unwrap().get(&cache_key) {
            return value
                .clone()
                .downcast()
                .map_err(|_| "asset type mismatch".into());
        }
        let asset = T::load_asset(self, path, options)?;
        if retain {
            self.inner
                .cache
                .lock()
                .unwrap()
                .insert(cache_key, asset.clone());
        }
        Ok(asset)
    }
    pub fn deload(&self, key: &str) {
        if let Ok(key) = normalize(key) {
            self.inner
                .cache
                .lock()
                .unwrap()
                .retain(|(name, _, _), _| name != &key);
        }
    }
    pub fn len(&self) -> usize {
        self.inner.index.as_ref().map_or(0, HashMap::len)
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub(crate) fn attach(&self, uploader: &Rc<Uploader>) -> Result<(), String> {
        self.inner.index.as_ref().map_err(Clone::clone)?;
        if let Err(old) =
            self.inner
                .device
                .compare_exchange(0, uploader.id, Ordering::AcqRel, Ordering::Acquire)
            && old != uploader.id
        {
            return Err("assets already attached to another renderer".into());
        }
        UPLOADERS.with(|u| {
            let mut u = u.borrow_mut();
            u.retain(|_, value| value.strong_count() != 0);
            u.insert(uploader.id, Rc::downgrade(uploader));
        });
        Ok(())
    }
}

pub fn get_assets() -> Result<Assets, String> {
    crate::game::assets()
}

fn normalize(key: &str) -> Result<String, String> {
    let key = key.replace('\\', "/");
    if key.is_empty()
        || key
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == ".." || p.contains(':'))
    {
        return Err("asset keys must be relative paths inside assets".into());
    }
    Ok(key)
}
fn index_folder(
    root: &Path,
    folder: &Path,
    index: &mut HashMap<String, PathBuf>,
) -> Result<(), String> {
    for entry in std::fs::read_dir(folder).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        let path = entry.path();
        if kind.is_dir() {
            index_folder(root, &path, index)?;
        } else if kind.is_file() {
            let key = path
                .strip_prefix(root)
                .unwrap()
                .to_str()
                .ok_or("asset path is not UTF-8")?
                .replace('\\', "/");
            index.insert(key, path);
        }
    }
    Ok(())
}

pub(crate) struct GpuHandle {
    pub owner: u64,
    pub key: usize,
    pub retired: std::sync::Weak<Mutex<Vec<usize>>>,
}
pub(crate) struct Uploader {
    pub id: u64,
    pub device: Rc<Device>,
    pub textures: RefCell<HashMap<usize, Image>>,
    pub retired: Arc<Mutex<Vec<usize>>>,
    pub garbage: RefCell<Vec<Image>>,
    next_key: Cell<usize>,
}
impl Drop for Uploader {
    fn drop(&mut self) {
        let _ = UPLOADERS.try_with(|u| {
            u.borrow_mut().remove(&self.id);
        });
    }
}
impl Uploader {
    pub fn new(device: Rc<Device>) -> Rc<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Rc::new(Self {
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            device,
            textures: RefCell::new(HashMap::new()),
            retired: Arc::default(),
            garbage: RefCell::default(),
            next_key: Cell::new(0),
        })
    }
    pub fn collect_unused(&self) {
        let mut retired = self.retired.lock().unwrap();
        if retired.is_empty() {
            return;
        }
        let mut textures = self.textures.borrow_mut();
        let mut garbage = self.garbage.borrow_mut();
        for key in retired.drain(..) {
            if let Some(image) = textures.remove(&key) {
                garbage.push(image);
            }
        }
    }
}
thread_local! { static UPLOADERS: RefCell<HashMap<u64, LocalWeak<Uploader>>> = RefCell::new(HashMap::new()); }

pub(crate) fn decode(bytes: &[u8]) -> Result<image::DynamicImage, String> {
    if image::guess_format(bytes).ok() != Some(image::ImageFormat::Avif) {
        return image::load_from_memory(bytes).map_err(|e| e.to_string());
    }
    use crate::avif::Image as A;
    let image = crate::avif::Decoder::from_avif(bytes)
        .and_then(|d| d.to_image())
        .map_err(|e| e.to_string())?;
    macro_rules! convert {
        ($img:expr, $pixel:expr) => {{
            let img = $img;
            let (w, h) = (img.width() as u32, img.height() as u32);
            let data: Vec<u8> = img.pixels().flat_map($pixel).collect();
            image::DynamicImage::ImageRgba8(
                image::RgbaImage::from_raw(w, h, data).ok_or("invalid AVIF dimensions")?,
            )
        }};
    }
    Ok(match image {
        A::Rgb8(i) => convert!(i, |p| [p.r, p.g, p.b, 255]),
        A::Rgba8(i) => convert!(i, |p| [p.r, p.g, p.b, p.a]),
        A::Rgb16(i) => convert!(i, |p| [
            (p.r >> 8) as u8,
            (p.g >> 8) as u8,
            (p.b >> 8) as u8,
            255
        ]),
        A::Rgba16(i) => convert!(i, |p| [
            (p.r >> 8) as u8,
            (p.g >> 8) as u8,
            (p.b >> 8) as u8,
            (p.a >> 8) as u8
        ]),
        A::Gray8(i) => convert!(i, |p| [*p, *p, *p, 255]),
        A::Gray16(i) => convert!(i, |p| [
            (*p >> 8) as u8,
            (*p >> 8) as u8,
            (*p >> 8) as u8,
            255
        ]),
    })
}

pub(crate) fn premultiply(pixels: &mut [u8]) {
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = u16::from(pixel[3]);
        for channel in &mut pixel[..3] {
            *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
        }
    }
}

fn encode(image: image::DynamicImage, compression: TextureCompression) -> Vec<u8> {
    use TextureCompression::*;
    use intel_tex_2::*;
    if compression == RGBA8 {
        let mut pixels = image.into_rgba8().into_raw();
        premultiply(&mut pixels);
        return pixels;
    }
    let width = image.width().next_multiple_of(4);
    let height = image.height().next_multiple_of(4);
    if compression == BC6H {
        let image = image.into_rgba32f();
        let mut data = Vec::with_capacity(width as usize * height as usize * 8);
        for y in 0..height {
            for x in 0..width {
                for c in image
                    .get_pixel(x.min(image.width() - 1), y.min(image.height() - 1))
                    .0
                {
                    data.extend_from_slice(&half::f16::from_f32(c.clamp(0., 65504.)).to_le_bytes());
                }
            }
        }
        return bc6h::compress_blocks(
            &bc6h::very_fast_settings(),
            &RgbaSurface {
                data: &data,
                width,
                height,
                stride: width * 8,
            },
        );
    }
    let mut image = image.into_rgba8();
    if matches!(compression, BC2 | BC3 | BC7) {
        if compression == BC2 {
            for pixel in image.pixels_mut() {
                pixel[3] = ((u16::from(pixel[3]) + 8) / 17 * 17) as u8;
            }
        }
        premultiply(image.as_mut());
    }
    let image = if image.width() == width && image.height() == height {
        image
    } else {
        image::RgbaImage::from_fn(width, height, |x, y| {
            *image.get_pixel(x.min(image.width() - 1), y.min(image.height() - 1))
        })
    };
    let surface = RgbaSurface {
        data: image.as_raw(),
        width,
        height,
        stride: width * 4,
    };
    match compression {
        BC1 => bc1::compress_blocks(&surface),
        BC2 => {
            let mut blocks = bc3::compress_blocks(&surface);
            for (n, block) in blocks.chunks_exact_mut(16).enumerate() {
                let bx = n as u32 % (width / 4) * 4;
                let by = n as u32 / (width / 4) * 4;
                for i in 0..8 {
                    let a = image.get_pixel(bx + (i * 2 % 4), by + i * 2 / 4)[3];
                    let b = image.get_pixel(bx + ((i * 2 + 1) % 4), by + (i * 2 + 1) / 4)[3];
                    block[i as usize] = ((u16::from(a) * 15 + 127) / 255) as u8
                        | (((u16::from(b) * 15 + 127) / 255) as u8) << 4;
                }
            }
            blocks
        }
        BC3 => bc3::compress_blocks(&surface),
        BC4 => {
            let data: Vec<u8> = image.pixels().map(|p| p[0]).collect();
            bc4::compress_blocks(&RSurface {
                data: &data,
                width,
                height,
                stride: width,
            })
        }
        BC5 => {
            let data: Vec<u8> = image.pixels().flat_map(|p| [p[0], p[1]]).collect();
            bc5::compress_blocks(&RgSurface {
                data: &data,
                width,
                height,
                stride: width * 2,
            })
        }
        BC7 => bc7::compress_blocks(&bc7::alpha_ultra_fast_settings(), &surface),
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestAsset;
    impl sealed::Sealed for TestAsset {}
    impl Asset for TestAsset {
        fn load_asset(_: &Assets, _: &Path, _: TextureLoadOptions) -> Result<Arc<Self>, String> {
            Ok(Arc::new(Self))
        }
    }
    fn assets() -> Assets {
        Assets {
            inner: Arc::new(Inner {
                index: Ok(HashMap::from([("nested/test.png".into(), "unused".into())])),
                cache: Mutex::default(),
                device: AtomicU64::new(0),
            }),
        }
    }
    #[test]
    fn cache_is_explicit_and_options_are_part_of_the_key() {
        let assets = assets();
        let a = assets.load::<TestAsset>("nested/test.png").unwrap();
        let b = assets.load::<TestAsset>("nested/test.png").unwrap();
        assert!(!Arc::ptr_eq(&a, &b));
        let cached = assets.preload::<TestAsset>("nested/test.png").unwrap();
        assert!(Arc::ptr_eq(
            &cached,
            &assets.load::<TestAsset>("nested\\test.png").unwrap()
        ));
        let other = assets
            .preload_with::<TestAsset>(
                "nested/test.png",
                TextureLoadOptions {
                    compression: TextureCompression::RGBA8,
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(!Arc::ptr_eq(&cached, &other));
        let nearest = assets
            .preload_with::<TestAsset>(
                "nested/test.png",
                TextureLoadOptions {
                    filter: TextureFilter::Nearest,
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(!Arc::ptr_eq(&cached, &nearest));
        assert!(Arc::ptr_eq(
            &nearest,
            &assets
                .load_with::<TestAsset>(
                    "nested/test.png",
                    TextureLoadOptions {
                        filter: TextureFilter::Nearest,
                        ..Default::default()
                    },
                )
                .unwrap()
        ));
        assets.deload("nested\\test.png");
        assert_eq!(Arc::strong_count(&cached), 1);
        assert_eq!(Arc::strong_count(&other), 1);
        let weak = Arc::downgrade(&a);
        drop(a);
        assert!(weak.upgrade().is_none());
    }
    #[test]
    fn invalid_keys_and_images_return_errors() {
        for key in [
            "",
            "../test.png",
            "/test.png",
            "C:\\test.png",
            "nested/../test.png",
            "nested//test.png",
        ] {
            assert!(assets().load::<Texture>(key).is_err());
        }
        assert!(assets().load::<Texture>("missing.png").is_err());
        assert!(Texture::from_file_bytes(b"not an image").is_err());
    }
    #[test]
    fn all_encoders_handle_partial_blocks() {
        use TextureCompression::*;
        for (w, h) in [(1, 1), (3, 5), (4, 4), (8, 12)] {
            for format in [BC1, BC2, BC3, BC4, BC5, BC6H, BC7, RGBA8] {
                let image = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
                    w,
                    h,
                    image::Rgba([255, 128, 64, 128]),
                ));
                let data = encode(image, format);
                let expected = match format {
                    RGBA8 => w * h * 4,
                    BC1 | BC4 => w.div_ceil(4) * h.div_ceil(4) * 8,
                    _ => w.div_ceil(4) * h.div_ceil(4) * 16,
                };
                assert_eq!(data.len(), expected as usize, "{format:?} {w}x{h}");
                if format == BC2 {
                    assert_eq!(&data[..8], &[0x88; 8]);
                }
            }
        }
    }
    #[test]
    fn index_includes_nested_files_without_decoding() {
        let root = std::env::temp_dir().join(format!("velocity-index-{}", std::process::id()));
        std::fs::create_dir_all(root.join("nested")).unwrap();
        std::fs::write(root.join("nested/test.png"), b"not decoded during indexing").unwrap();
        let mut index = HashMap::new();
        index_folder(&root, &root, &mut index).unwrap();
        assert!(index.contains_key("nested/test.png"));
        std::fs::remove_file(root.join("nested/test.png")).unwrap();
        std::fs::remove_dir(root.join("nested")).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
