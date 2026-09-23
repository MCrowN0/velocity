use crate::{
    vulkan::{
        Buffer, Device, IMAGE_FORMAT, Image, Result, barrier, color_layers, color_range, error,
    },
    *,
};
use ash::vk;
use std::{
    collections::HashMap,
    rc::Rc,
    sync::{
        Arc, Weak,
        atomic::{AtomicU64, Ordering},
    },
};

static NEXT_RENDERER: AtomicU64 = AtomicU64::new(1);
const FRAMES: usize = 2;

#[derive(Default, Debug, Clone, Copy)]
pub struct RenderStats {
    pub drawn: usize,
    pub culled: usize,
    pub draw_calls: usize,
    pub texture_uploads: usize,
    pub uploaded_bytes: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct SpriteInstance {
    position: Vec2,
    size: Vec2,
    uv: [f32; 4],
    color: Color,
}
impl SpriteInstance {
    fn sprite(sprite: &Sprite) -> Self {
        let signed = sprite.signed_size();
        Self {
            position: sprite.position,
            size: signed.abs(),
            color: sprite.color,
            uv: [
                if signed.x < 0. { 1. } else { 0. },
                if signed.y < 0. { 1. } else { 0. },
                if signed.x < 0. { -1. } else { 1. },
                if signed.y < 0. { -1. } else { 1. },
            ],
        }
    }
}
const _: () = assert!(std::mem::size_of::<SpriteInstance>() == 48);

#[derive(Clone, Copy)]
struct Batch {
    descriptor: vk::DescriptorSet,
    first: u32,
    count: u32,
    premultiplied: bool,
}
#[derive(Default)]
struct DrawList {
    instances: Vec<SpriteInstance>,
    batches: Vec<Batch>,
    offset: u64,
}
impl DrawList {
    fn clear(&mut self) {
        self.instances.clear();
        self.batches.clear();
    }
    fn push(
        &mut self,
        instance: SpriteInstance,
        descriptor: vk::DescriptorSet,
        premultiplied: bool,
    ) {
        let first = self.instances.len() as u32;
        self.instances.push(instance);
        if let Some(last) = self.batches.last_mut()
            && last.descriptor == descriptor
            && last.premultiplied == premultiplied
        {
            last.count += 1;
            return;
        }
        self.batches.push(Batch {
            descriptor,
            first,
            count: 1,
            premultiplied,
        });
    }
    fn bytes(&self) -> &[u8] {
        // repr(C), all-f32 fields, and a compile-time size check ensure no padding.
        unsafe {
            std::slice::from_raw_parts(self.instances.as_ptr().cast(), self.instances.len() * 48)
        }
    }
}
struct SurfaceTarget {
    image: Image,
    position: Vec2,
    logical: Vec2,
    origin: Vec2,
    scale: Vec2,
    list: DrawList,
    initialized: bool,
    visible: bool,
}
struct CachedTexture {
    source: Weak<Texture>,
    image: Image,
}

struct Frame {
    device: Rc<Device>,
    pool: vk::CommandPool,
    cmd: vk::CommandBuffer,
    fence: vk::Fence,
    acquired: vk::Semaphore,
    instances: Option<Buffer>,
    garbage: Vec<Image>,
}
impl Frame {
    fn new(device: &Rc<Device>) -> Result<Self> {
        unsafe {
            let mut frame = Self {
                device: device.clone(),
                pool: vk::CommandPool::null(),
                cmd: vk::CommandBuffer::null(),
                fence: vk::Fence::null(),
                acquired: vk::Semaphore::null(),
                instances: None,
                garbage: Vec::new(),
            };
            frame.pool = device
                .raw
                .create_command_pool(
                    &vk::CommandPoolCreateInfo::default().queue_family_index(device.family),
                    None,
                )
                .map_err(error)?;
            frame.cmd = device
                .raw
                .allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default()
                        .command_pool(frame.pool)
                        .level(vk::CommandBufferLevel::PRIMARY)
                        .command_buffer_count(1),
                )
                .map_err(error)?[0];
            frame.fence = device
                .raw
                .create_fence(
                    &vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED),
                    None,
                )
                .map_err(error)?;
            frame.acquired = device
                .raw
                .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
                .map_err(error)?;
            Ok(frame)
        }
    }
}
impl Drop for Frame {
    fn drop(&mut self) {
        unsafe {
            self.device.raw.destroy_command_pool(self.pool, None);
            self.device.raw.destroy_fence(self.fence, None);
            self.device.raw.destroy_semaphore(self.acquired, None);
        }
    }
}

struct Swapchain {
    device: Rc<Device>,
    api: ash::khr::swapchain::Device,
    raw: vk::SwapchainKHR,
    extent: vk::Extent2D,
    format: vk::Format,
    images: Vec<vk::Image>,
    views: Vec<vk::ImageView>,
    // Presentation can outlive a submission fence; use one semaphore per image.
    finished: Vec<vk::Semaphore>,
    initialized: Vec<bool>,
    can_capture: bool,
}
impl Swapchain {
    fn new(device: &Rc<Device>, size: [u32; 2], vsync: bool) -> Result<Self> {
        unsafe {
            let instance = &device.instance;
            let cap = instance
                .surface_api
                .get_physical_device_surface_capabilities(device.physical, instance.surface)
                .map_err(error)?;
            let formats = instance
                .surface_api
                .get_physical_device_surface_formats(device.physical, instance.surface)
                .map_err(error)?;
            let format = formats
                .iter()
                .find(|f| {
                    (f.format == vk::Format::B8G8R8A8_UNORM
                        || f.format == vk::Format::R8G8B8A8_UNORM)
                        && f.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
                })
                .copied()
                .ok_or("surface has no RGBA8/BGRA8 UNORM format")?;
            let extent = if cap.current_extent.width == u32::MAX {
                vk::Extent2D {
                    width: size[0].clamp(cap.min_image_extent.width, cap.max_image_extent.width),
                    height: size[1].clamp(cap.min_image_extent.height, cap.max_image_extent.height),
                }
            } else {
                cap.current_extent
            };
            let modes = instance
                .surface_api
                .get_physical_device_surface_present_modes(device.physical, instance.surface)
                .map_err(error)?;
            let mode = if !vsync && modes.contains(&vk::PresentModeKHR::IMMEDIATE) {
                vk::PresentModeKHR::IMMEDIATE
            } else if !vsync && modes.contains(&vk::PresentModeKHR::MAILBOX) {
                vk::PresentModeKHR::MAILBOX
            } else {
                vk::PresentModeKHR::FIFO
            };
            let count = if cap.max_image_count > 0 {
                (cap.min_image_count + 1).min(cap.max_image_count)
            } else {
                cap.min_image_count + 1
            };
            let alpha = [
                vk::CompositeAlphaFlagsKHR::OPAQUE,
                vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED,
                vk::CompositeAlphaFlagsKHR::POST_MULTIPLIED,
                vk::CompositeAlphaFlagsKHR::INHERIT,
            ]
            .into_iter()
            .find(|a| cap.supported_composite_alpha.contains(*a))
            .ok_or("unsupported composite alpha")?;
            let can_capture = cap
                .supported_usage_flags
                .contains(vk::ImageUsageFlags::TRANSFER_SRC);
            let mut usage = vk::ImageUsageFlags::COLOR_ATTACHMENT;
            if can_capture {
                usage |= vk::ImageUsageFlags::TRANSFER_SRC;
            }
            let mut swap = Self {
                device: device.clone(),
                api: ash::khr::swapchain::Device::new(&instance.raw, &device.raw),
                raw: vk::SwapchainKHR::null(),
                extent,
                format: format.format,
                images: Vec::new(),
                views: Vec::new(),
                finished: Vec::new(),
                initialized: Vec::new(),
                can_capture,
            };
            swap.raw = swap
                .api
                .create_swapchain(
                    &vk::SwapchainCreateInfoKHR::default()
                        .surface(instance.surface)
                        .min_image_count(count)
                        .image_format(format.format)
                        .image_color_space(format.color_space)
                        .image_extent(extent)
                        .image_array_layers(1)
                        .image_usage(usage)
                        .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
                        .pre_transform(cap.current_transform)
                        .composite_alpha(alpha)
                        .present_mode(mode)
                        .clipped(true),
                    None,
                )
                .map_err(error)?;
            swap.images = swap.api.get_swapchain_images(swap.raw).map_err(error)?;
            for &image in &swap.images {
                swap.views.push(
                    device
                        .raw
                        .create_image_view(
                            &vk::ImageViewCreateInfo::default()
                                .image(image)
                                .format(format.format)
                                .view_type(vk::ImageViewType::TYPE_2D)
                                .subresource_range(color_range()),
                            None,
                        )
                        .map_err(error)?,
                );
                swap.finished.push(
                    device
                        .raw
                        .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
                        .map_err(error)?,
                );
            }
            swap.initialized.resize(swap.images.len(), false);
            device.pipeline(format.format)?;
            Ok(swap)
        }
    }
}
impl Drop for Swapchain {
    fn drop(&mut self) {
        unsafe {
            for view in self.views.drain(..) {
                self.device.raw.destroy_image_view(view, None);
            }
            for semaphore in self.finished.drain(..) {
                self.device.raw.destroy_semaphore(semaphore, None);
            }
            self.api.destroy_swapchain(self.raw, None);
        }
    }
}

pub struct Renderer {
    pub sprites: Vec<Sprite>,
    /// The 1280×720 logical surface used by sprites with no explicit surface.
    pub default_surface: Surface,
    pub clear_color: Color,
    device: Rc<Device>,
    window: Window,
    id: u64,
    vsync: bool,
    canvas_size: [u32; 2],
    swap: Option<Swapchain>,
    frames: Vec<Frame>,
    frame: usize,
    textures: HashMap<usize, CachedTexture>,
    surfaces: Vec<SurfaceTarget>,
    window_list: DrawList,
    instance_bytes: usize,
    recreate: bool,
    failed: bool,
}
impl Renderer {
    pub fn new(window: &Window) -> Result<Self> {
        Self::with_vsync(window, true)
    }
    pub fn with_vsync(window: &Window, vsync: bool) -> Result<Self> {
        if window.size().contains(&0) {
            return Err("canvas dimensions must be nonzero".into());
        }
        let device = Device::new(window)?;
        device.pipeline(IMAGE_FORMAT)?;
        let id = NEXT_RENDERER.fetch_add(1, Ordering::Relaxed);
        let mut renderer = Self {
            sprites: Vec::new(),
            default_surface: Surface {
                renderer: id,
                index: 0,
            },
            clear_color: Color::BLACK,
            device,
            window: window.clone(),
            id,
            vsync,
            canvas_size: window.size(),
            swap: None,
            frames: Vec::new(),
            frame: 0,
            textures: HashMap::new(),
            surfaces: Vec::new(),
            window_list: DrawList::default(),
            instance_bytes: 0,
            recreate: true,
            failed: false,
        };
        for _ in 0..FRAMES {
            renderer.frames.push(Frame::new(&renderer.device)?);
        }
        renderer.default_surface = renderer.create_surface(1280, 720, Vec2::ZERO)?;
        Ok(renderer)
    }
    pub fn gpu_name(&self) -> &str {
        &self.device.name
    }
    pub fn cached_texture_count(&self) -> usize {
        self.textures.len()
    }
    pub fn add_sprite(&mut self, sprite: Sprite) {
        self.sprites.push(sprite);
    }
    pub fn create_surface(&mut self, width: u16, height: u16, position: Vec2) -> Result<Surface> {
        if width == 0 || height == 0 || !position.is_finite() {
            return Err("surface dimensions must be nonzero and offset finite".into());
        }
        let image = Image::new(&self.device, width as u32, height as u32, true)?;
        let handle = Surface {
            renderer: self.id,
            index: self.surfaces.len(),
        };
        self.surfaces.push(SurfaceTarget {
            image,
            position,
            logical: vec2(width as f32, height as f32),
            origin: Vec2::ZERO,
            scale: Vec2::ONE,
            list: DrawList::default(),
            initialized: false,
            visible: false,
        });
        Ok(handle)
    }
    pub fn set_surface_position(&mut self, surface: Surface, position: Vec2) -> Result<()> {
        if surface.renderer != self.id {
            return Err("surface belongs to another renderer".into());
        }
        if !position.is_finite() {
            return Err("surface offset must be finite".into());
        }
        self.surfaces[surface.index].position = position;
        Ok(())
    }
    pub fn read_surface(&self, surface: Surface) -> Result<Vec<u8>> {
        if surface.renderer != self.id {
            return Err("surface belongs to another renderer".into());
        }
        let surface = &self.surfaces[surface.index];
        if !surface.initialized {
            return Err("surface has not been rendered yet".into());
        }
        surface.image.read()
    }
    pub fn surface_size(&self, surface: Surface) -> Result<[u32; 2]> {
        if surface.renderer != self.id {
            return Err("surface belongs to another renderer".into());
        }
        let image = &self.surfaces[surface.index].image;
        Ok([image.width, image.height])
    }
    pub fn surface_position(&self, surface: Surface, point: Vec2) -> Result<Vec2> {
        if surface.renderer != self.id {
            return Err("surface belongs to another renderer".into());
        }
        let target = &self.surfaces[surface.index];
        let [w, h] = self.window.size();
        if w == 0 || h == 0 {
            return Err("window is minimized".into());
        }
        let canvas = canvas_rect(
            self.canvas_size,
            vk::Extent2D {
                width: w,
                height: h,
            },
        );
        let (origin, pixels) = surface_layout(
            target.logical,
            vec2(canvas.extent.width as f32, canvas.extent.height as f32),
            target.position,
        );
        let origin = vec2(
            origin.x + canvas.offset.x as f32,
            origin.y + canvas.offset.y as f32,
        );
        Ok(vec2(
            (point.x - origin.x) * target.logical.x / pixels.x,
            (point.y - origin.y) * target.logical.y / pixels.y,
        ))
    }
    pub(crate) fn clear_scene(&mut self) -> Result<()> {
        unsafe {
            self.device.raw.device_wait_idle().map_err(error)?;
        }
        self.sprites.clear();
        self.surfaces.clear();
        self.id = NEXT_RENDERER.fetch_add(1, Ordering::Relaxed);
        self.default_surface = self.create_surface(1280, 720, Vec2::ZERO)?;
        Ok(())
    }
    pub fn render(&mut self) -> Result<RenderStats> {
        self.render_internal(false).map(|(stats, _)| stats)
    }
    pub fn render_capture(&mut self) -> Result<(RenderStats, Vec<u8>)> {
        self.render_internal(true)
    }

    fn prepare(&mut self, size: Vec2) -> Result<RenderStats> {
        let mut stats = RenderStats::default();
        self.window_list.clear();
        for surface in &mut self.surfaces {
            surface.list.clear();
            let (origin, pixels) = surface_layout(surface.logical, size, surface.position);
            surface.origin = origin;
            surface.scale = vec2(pixels.x / surface.logical.x, pixels.y / surface.logical.y);
            surface.visible = intersects(origin, pixels, size);
            if surface.visible
                && (surface.image.width != pixels.x as u32
                    || surface.image.height != pixels.y as u32)
            {
                let image = Image::new(&self.device, pixels.x as u32, pixels.y as u32, true)?;
                self.frames[self.frame]
                    .garbage
                    .push(std::mem::replace(&mut surface.image, image));
                surface.initialized = false;
            }
        }
        self.frames[self.frame].garbage.extend(
            self.textures
                .extract_if(|_, cached| cached.source.strong_count() == 0)
                .map(|(_, cached)| cached.image),
        );
        for sprite in &self.sprites {
            let handle = sprite.surface.unwrap_or(self.default_surface);
            if handle.renderer != self.id {
                stats.culled += 1;
                continue;
            }
            let target = &self.surfaces[handle.index];
            if !target.visible {
                stats.culled += 1;
                continue;
            }
            let bounds = target.logical;
            let Some(sprite_size) = drawable_size(sprite, bounds) else {
                stats.culled += 1;
                continue;
            };
            {
                let target = &self.surfaces[handle.index];
                let origin = target.origin;
                let local = sprite.position * target.scale;
                if !intersects(
                    vec2(origin.x + local.x, origin.y + local.y),
                    sprite_size * target.scale,
                    size,
                ) {
                    stats.culled += 1;
                    continue;
                }
            }
            let key = Arc::as_ptr(&sprite.texture) as usize;
            let descriptor = match self.textures.entry(key) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.get().image.descriptor,
                std::collections::hash_map::Entry::Vacant(entry) => {
                    let image = Image::new(
                        &self.device,
                        sprite.texture.width() as u32,
                        sprite.texture.height() as u32,
                        false,
                    )?;
                    image.upload(sprite.texture.pixels())?;
                    let descriptor = image.descriptor;
                    entry.insert(CachedTexture {
                        source: Arc::downgrade(&sprite.texture),
                        image,
                    });
                    stats.texture_uploads += 1;
                    stats.uploaded_bytes += sprite.texture.pixels().len();
                    descriptor
                }
            };
            let mut instance = SpriteInstance::sprite(sprite);
            instance.position = instance.position * self.surfaces[handle.index].scale;
            instance.size = instance.size * self.surfaces[handle.index].scale;
            let list = &mut self.surfaces[handle.index].list;
            list.push(instance, descriptor, false);
            stats.drawn += 1;
        }
        for surface in &self.surfaces {
            if surface.visible && !surface.list.instances.is_empty() {
                self.window_list.push(
                    SpriteInstance {
                        position: surface.origin,
                        size: vec2(surface.image.width as f32, surface.image.height as f32),
                        uv: [0., 0., 1., 1.],
                        color: Color::WHITE,
                    },
                    surface.image.descriptor,
                    true,
                );
            }
        }
        self.instance_bytes = 0;
        for surface in &mut self.surfaces {
            surface.list.offset = self.instance_bytes as u64;
            self.instance_bytes += surface.list.bytes().len();
            stats.draw_calls += surface.list.batches.len();
        }
        self.window_list.offset = self.instance_bytes as u64;
        self.instance_bytes += self.window_list.bytes().len();
        stats.draw_calls += self.window_list.batches.len();
        stats.uploaded_bytes += self.instance_bytes;
        Ok(stats)
    }

    fn render_internal(&mut self, capture: bool) -> Result<(RenderStats, Vec<u8>)> {
        if self.failed {
            return Err("renderer stopped after a Vulkan error; recreate it".into());
        }
        let result = self.render_frame(capture);
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn render_frame(&mut self, capture: bool) -> Result<(RenderStats, Vec<u8>)> {
        unsafe {
            let size = self.window.size();
            if size[0] == 0 || size[1] == 0 {
                return Ok((RenderStats::default(), Vec::new()));
            }
            if self.recreate
                || self
                    .swap
                    .as_ref()
                    .is_none_or(|s| s.extent.width != size[0] || s.extent.height != size[1])
            {
                self.device.raw.device_wait_idle().map_err(error)?;
                self.swap = None;
                self.swap = Some(Swapchain::new(&self.device, size, self.vsync)?);
                self.recreate = false;
            }
            let frame = &mut self.frames[self.frame];
            self.device
                .raw
                .wait_for_fences(&[frame.fence], true, u64::MAX)
                .map_err(error)?;
            frame.garbage.clear();
            let extent = self.swap.as_ref().unwrap().extent;
            let canvas = canvas_rect(self.canvas_size, extent);
            let stats = self.prepare(vec2(
                canvas.extent.width as f32,
                canvas.extent.height as f32,
            ))?;
            let frame = &mut self.frames[self.frame];
            let required = self.instance_bytes.max(48);
            if frame
                .instances
                .as_ref()
                .is_none_or(|b| b.capacity < required)
            {
                frame.instances = Some(Buffer::new(
                    &self.device,
                    required.next_power_of_two(),
                    vk::BufferUsageFlags::VERTEX_BUFFER,
                )?);
            }
            let instances = frame.instances.as_mut().unwrap();
            for surface in &self.surfaces {
                instances.write_at(surface.list.offset as usize, surface.list.bytes());
            }
            instances.write_at(self.window_list.offset as usize, self.window_list.bytes());
            let swap = self.swap.as_mut().unwrap();
            if capture && !swap.can_capture {
                return Err("swapchain does not support diagnostic readback".into());
            }
            let readback = if capture {
                Some(Buffer::new(
                    &self.device,
                    extent.width as usize * extent.height as usize * 4,
                    vk::BufferUsageFlags::TRANSFER_DST,
                )?)
            } else {
                None
            };
            let (index, suboptimal) = match swap.api.acquire_next_image(
                swap.raw,
                u64::MAX,
                frame.acquired,
                vk::Fence::null(),
            ) {
                Ok(result) => result,
                Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                    self.recreate = true;
                    return Ok((RenderStats::default(), Vec::new()));
                }
                Err(e) => return Err(error(e)),
            };
            self.recreate |= suboptimal;
            let index = index as usize;
            let device = &self.device;
            let cmd = frame.cmd;
            device
                .raw
                .reset_command_pool(frame.pool, vk::CommandPoolResetFlags::empty())
                .map_err(error)?;
            device
                .raw
                .begin_command_buffer(
                    cmd,
                    &vk::CommandBufferBeginInfo::default()
                        .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
                )
                .map_err(error)?;
            let vertex = frame.instances.as_ref().unwrap().raw;
            for surface in &mut self.surfaces {
                if !surface.visible {
                    continue;
                }
                let old = if surface.initialized {
                    vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL
                } else {
                    vk::ImageLayout::UNDEFINED
                };
                barrier(
                    &device.raw,
                    cmd,
                    surface.image.raw,
                    old,
                    vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                );
                draw_pass(
                    device,
                    cmd,
                    surface.image.view,
                    vk::Extent2D {
                        width: surface.image.width,
                        height: surface.image.height,
                    },
                    IMAGE_FORMAT,
                    Color::TRANSPARENT,
                    None,
                    vertex,
                    &surface.list,
                )?;
                barrier(
                    &device.raw,
                    cmd,
                    surface.image.raw,
                    vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                    vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                );
                surface.initialized = true;
            }
            let old = if swap.initialized[index] {
                vk::ImageLayout::PRESENT_SRC_KHR
            } else {
                vk::ImageLayout::UNDEFINED
            };
            barrier(
                &device.raw,
                cmd,
                swap.images[index],
                old,
                vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            );
            draw_pass(
                device,
                cmd,
                swap.views[index],
                extent,
                swap.format,
                self.clear_color,
                Some(canvas),
                vertex,
                &self.window_list,
            )?;
            if let Some(readback) = &readback {
                barrier(
                    &device.raw,
                    cmd,
                    swap.images[index],
                    vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                );
                let region = vk::BufferImageCopy::default()
                    .image_subresource(color_layers())
                    .image_extent(vk::Extent3D {
                        width: extent.width,
                        height: extent.height,
                        depth: 1,
                    });
                device.raw.cmd_copy_image_to_buffer(
                    cmd,
                    swap.images[index],
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    readback.raw,
                    &[region],
                );
                barrier(
                    &device.raw,
                    cmd,
                    swap.images[index],
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    vk::ImageLayout::PRESENT_SRC_KHR,
                );
            } else {
                barrier(
                    &device.raw,
                    cmd,
                    swap.images[index],
                    vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                    vk::ImageLayout::PRESENT_SRC_KHR,
                );
            }
            device.raw.end_command_buffer(cmd).map_err(error)?;
            let waits = [frame.acquired];
            let stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
            let commands = [cmd];
            let signals = [swap.finished[index]];
            device.raw.reset_fences(&[frame.fence]).map_err(error)?;
            device
                .raw
                .queue_submit(
                    device.queue,
                    &[vk::SubmitInfo::default()
                        .wait_semaphores(&waits)
                        .wait_dst_stage_mask(&stages)
                        .command_buffers(&commands)
                        .signal_semaphores(&signals)],
                    frame.fence,
                )
                .map_err(error)?;
            swap.initialized[index] = true;
            let chains = [swap.raw];
            let indices = [index as u32];
            match swap.api.queue_present(
                device.queue,
                &vk::PresentInfoKHR::default()
                    .wait_semaphores(&signals)
                    .swapchains(&chains)
                    .image_indices(&indices),
            ) {
                Ok(suboptimal) => self.recreate |= suboptimal,
                Err(vk::Result::ERROR_OUT_OF_DATE_KHR | vk::Result::SUBOPTIMAL_KHR) => {
                    self.recreate = true
                }
                Err(e) => {
                    // Ensure a diagnostic staging buffer cannot be freed in flight.
                    let _ = device.raw.device_wait_idle();
                    return Err(error(e));
                }
            }
            let pixels = if let Some(readback) = readback {
                device
                    .raw
                    .wait_for_fences(&[frame.fence], true, u64::MAX)
                    .map_err(error)?;
                let mut pixels = readback.read();
                if swap.format == vk::Format::B8G8R8A8_UNORM {
                    for pixel in pixels.chunks_exact_mut(4) {
                        pixel.swap(0, 2);
                    }
                }
                pixels
            } else {
                Vec::new()
            };
            self.frame = (self.frame + 1) % FRAMES;
            Ok((stats, pixels))
        }
    }
}
impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.raw.device_wait_idle();
        }
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn draw_pass(
    device: &Device,
    cmd: vk::CommandBuffer,
    view: vk::ImageView,
    extent: vk::Extent2D,
    format: vk::Format,
    clear: Color,
    canvas: Option<vk::Rect2D>,
    vertex: vk::Buffer,
    list: &DrawList,
) -> Result<()> {
    unsafe {
        let background = if canvas.is_some() {
            Color::BLACK
        } else {
            clear
        };
        let area = canvas.unwrap_or(vk::Rect2D {
            offset: vk::Offset2D::default(),
            extent,
        });
        let attachments = [vk::RenderingAttachmentInfo::default()
            .image_view(view)
            .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .clear_value(vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: [background.r, background.g, background.b, background.a],
                },
            })];
        device.raw.cmd_begin_rendering(
            cmd,
            &vk::RenderingInfo::default()
                .render_area(vk::Rect2D {
                    offset: vk::Offset2D::default(),
                    extent,
                })
                .layer_count(1)
                .color_attachments(&attachments),
        );
        if canvas.is_some() {
            device.raw.cmd_clear_attachments(
                cmd,
                &[vk::ClearAttachment::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .color_attachment(0)
                    .clear_value(vk::ClearValue {
                        color: vk::ClearColorValue {
                            float32: [clear.r, clear.g, clear.b, clear.a],
                        },
                    })],
                &[vk::ClearRect::default().rect(area).layer_count(1)],
            );
        }
        if !list.batches.is_empty() {
            device.raw.cmd_bind_pipeline(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                device.pipeline(format)?,
            );
            device.raw.cmd_set_viewport(
                cmd,
                0,
                &[vk::Viewport {
                    x: area.offset.x as f32,
                    y: area.offset.y as f32,
                    width: area.extent.width as f32,
                    height: area.extent.height as f32,
                    min_depth: 0.,
                    max_depth: 1.,
                }],
            );
            device.raw.cmd_set_scissor(cmd, 0, &[area]);
            device
                .raw
                .cmd_bind_vertex_buffers(cmd, 0, &[vertex], &[list.offset]);
            for batch in &list.batches {
                let push = [
                    (area.extent.width as f32).to_bits(),
                    (area.extent.height as f32).to_bits(),
                    batch.premultiplied as u32,
                    0,
                ];
                let bytes = std::slice::from_raw_parts(push.as_ptr().cast(), 16);
                device.raw.cmd_push_constants(
                    cmd,
                    device.pipeline_layout,
                    vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                    0,
                    bytes,
                );
                device.raw.cmd_bind_descriptor_sets(
                    cmd,
                    vk::PipelineBindPoint::GRAPHICS,
                    device.pipeline_layout,
                    0,
                    &[batch.descriptor],
                    &[],
                );
                device.raw.cmd_draw(cmd, 6, batch.count, 0, batch.first);
            }
        }
        device.raw.cmd_end_rendering(cmd);
        Ok(())
    }
}

pub(crate) fn surface_layout(logical: Vec2, window: Vec2, offset: Vec2) -> (Vec2, Vec2) {
    let scale = (window.x / logical.x).min(window.y / logical.y);
    let pixels = vec2(
        (logical.x * scale).round().max(1.),
        (logical.y * scale).round().max(1.),
    );
    (
        vec2(
            (window.x - pixels.x) * 0.5 + offset.x * scale,
            (window.y - pixels.y) * 0.5 + offset.y * scale,
        ),
        pixels,
    )
}

// Integer arithmetic rounds inward so no fractional edge can expose the bars.
pub(crate) fn canvas_rect(canvas: [u32; 2], extent: vk::Extent2D) -> vk::Rect2D {
    let [cw, ch] = canvas.map(u64::from);
    let (w, h) = (u64::from(extent.width), u64::from(extent.height));
    let (width, height) = if w * ch <= h * cw {
        (w, (w * ch / cw).max(1))
    } else {
        ((h * cw / ch).max(1), h)
    };
    vk::Rect2D {
        offset: vk::Offset2D {
            x: ((w - width) / 2) as i32,
            y: ((h - height) / 2) as i32,
        },
        extent: vk::Extent2D {
            width: width as u32,
            height: height as u32,
        },
    }
}
