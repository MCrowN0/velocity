use crate::Window;
use ash::{Entry, vk};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use std::{cell::RefCell, collections::HashMap, ffi::CStr, rc::Rc};

pub(crate) type Result<T> = std::result::Result<T, String>;
pub(crate) fn error(e: vk::Result) -> String {
    format!("Vulkan error {}", e.as_raw())
}
pub(crate) const IMAGE_FORMAT: vk::Format = vk::Format::R8G8B8A8_UNORM;

pub(crate) struct Instance {
    pub raw: ash::Instance,
    pub surface_api: ash::khr::surface::Instance,
    pub surface: vk::SurfaceKHR,
    _entry: Entry,
    _window: Window,
}
impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            self.surface_api.destroy_surface(self.surface, None);
            self.raw.destroy_instance(None);
        }
    }
}

pub(crate) struct Device {
    pub raw: ash::Device,
    pub instance: Rc<Instance>,
    pub physical: vk::PhysicalDevice,
    pub queue: vk::Queue,
    pub family: u32,
    pub name: String,
    pub properties: vk::PhysicalDeviceProperties,
    memory: vk::PhysicalDeviceMemoryProperties,
    pub sampler: vk::Sampler,
    nearest_sampler: vk::Sampler,
    pub descriptor_layout: vk::DescriptorSetLayout,
    pub pipeline_layout: vk::PipelineLayout,
    upload_pool: vk::CommandPool,
    descriptors: RefCell<Vec<vk::DescriptorPool>>,
    pipelines: RefCell<HashMap<vk::Format, vk::Pipeline>>,
}
impl Device {
    pub fn new(window: &Window) -> Result<Rc<Self>> {
        unsafe {
            let entry = Entry::load().map_err(|e| e.to_string())?;
            let display = window.display_handle().map_err(|e| e.to_string())?.as_raw();
            let handle = window.window_handle().map_err(|e| e.to_string())?.as_raw();
            let extensions = ash_window::enumerate_required_extensions(display).map_err(error)?;
            let app = vk::ApplicationInfo::default()
                .application_name(c"Velocity")
                .api_version(vk::API_VERSION_1_3);
            let raw = entry
                .create_instance(
                    &vk::InstanceCreateInfo::default()
                        .application_info(&app)
                        .enabled_extension_names(extensions),
                    None,
                )
                .map_err(error)?;
            let surface_api = ash::khr::surface::Instance::new(&entry, &raw);
            let mut instance = Instance {
                raw,
                surface_api,
                surface: vk::SurfaceKHR::null(),
                _entry: entry,
                _window: window.clone(),
            };
            instance.surface =
                ash_window::create_surface(&instance._entry, &instance.raw, display, handle, None)
                    .map_err(error)?;
            let instance = Rc::new(instance);
            let mut candidates = Vec::new();
            for physical in instance.raw.enumerate_physical_devices().map_err(error)? {
                let properties = instance.raw.get_physical_device_properties(physical);
                if properties.api_version < vk::API_VERSION_1_3 {
                    continue;
                }
                let mut features13 = vk::PhysicalDeviceVulkan13Features::default();
                let mut features =
                    vk::PhysicalDeviceFeatures2::default().push_next(&mut features13);
                instance
                    .raw
                    .get_physical_device_features2(physical, &mut features);
                if features13.dynamic_rendering == 0 {
                    continue;
                }
                let extensions = instance
                    .raw
                    .enumerate_device_extension_properties(physical)
                    .map_err(error)?;
                if !extensions
                    .iter()
                    .any(|e| CStr::from_ptr(e.extension_name.as_ptr()) == ash::khr::swapchain::NAME)
                {
                    continue;
                }
                for (index, family) in instance
                    .raw
                    .get_physical_device_queue_family_properties(physical)
                    .iter()
                    .enumerate()
                {
                    if family.queue_flags.contains(vk::QueueFlags::GRAPHICS)
                        && instance
                            .surface_api
                            .get_physical_device_surface_support(
                                physical,
                                index as u32,
                                instance.surface,
                            )
                            .map_err(error)?
                    {
                        let score =
                            if properties.device_type == vk::PhysicalDeviceType::DISCRETE_GPU {
                                2
                            } else {
                                1
                            };
                        candidates.push((score, physical, index as u32, properties));
                        break;
                    }
                }
            }
            let (_, physical, family, properties) = candidates
                .into_iter()
                .max_by_key(|c| c.0)
                .ok_or("a Vulkan 1.3 GPU with dynamic rendering and presentation is required")?;
            let priorities = [1.0];
            let queues = [vk::DeviceQueueCreateInfo::default()
                .queue_family_index(family)
                .queue_priorities(&priorities)];
            let extensions = [ash::khr::swapchain::NAME.as_ptr()];
            let mut features =
                vk::PhysicalDeviceVulkan13Features::default().dynamic_rendering(true);
            let supported = instance.raw.get_physical_device_features(physical);
            let core_features = vk::PhysicalDeviceFeatures::default()
                .texture_compression_bc(supported.texture_compression_bc != 0);
            let raw = instance
                .raw
                .create_device(
                    physical,
                    &vk::DeviceCreateInfo::default()
                        .enabled_features(&core_features)
                        .queue_create_infos(&queues)
                        .enabled_extension_names(&extensions)
                        .push_next(&mut features),
                    None,
                )
                .map_err(error)?;
            let queue = raw.get_device_queue(family, 0);
            let memory = instance.raw.get_physical_device_memory_properties(physical);
            let name = CStr::from_ptr(properties.device_name.as_ptr())
                .to_string_lossy()
                .into_owned();
            let mut device = Self {
                raw,
                instance,
                physical,
                queue,
                family,
                name,
                properties,
                memory,
                sampler: vk::Sampler::null(),
                nearest_sampler: vk::Sampler::null(),
                descriptor_layout: vk::DescriptorSetLayout::null(),
                pipeline_layout: vk::PipelineLayout::null(),
                upload_pool: vk::CommandPool::null(),
                descriptors: RefCell::new(Vec::new()),
                pipelines: RefCell::new(HashMap::new()),
            };
            device.sampler = device
                .raw
                .create_sampler(
                    &vk::SamplerCreateInfo::default()
                        .mag_filter(vk::Filter::LINEAR)
                        .min_filter(vk::Filter::LINEAR)
                        .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                        .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                        .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE),
                    None,
                )
                .map_err(error)?;
            device.nearest_sampler = device
                .raw
                .create_sampler(
                    &vk::SamplerCreateInfo::default()
                        .mag_filter(vk::Filter::NEAREST)
                        .min_filter(vk::Filter::NEAREST)
                        .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                        .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                        .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE),
                    None,
                )
                .map_err(error)?;
            let bindings = [
                vk::DescriptorSetLayoutBinding::default()
                    .binding(0)
                    .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                    .descriptor_count(1)
                    .stage_flags(vk::ShaderStageFlags::FRAGMENT),
                vk::DescriptorSetLayoutBinding::default()
                    .binding(1)
                    .descriptor_type(vk::DescriptorType::SAMPLER)
                    .descriptor_count(1)
                    .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            ];
            device.descriptor_layout = device
                .raw
                .create_descriptor_set_layout(
                    &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
                    None,
                )
                .map_err(error)?;
            let layouts = [device.descriptor_layout];
            let push = [vk::PushConstantRange::default()
                .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT)
                .size(16)];
            device.pipeline_layout = device
                .raw
                .create_pipeline_layout(
                    &vk::PipelineLayoutCreateInfo::default()
                        .set_layouts(&layouts)
                        .push_constant_ranges(&push),
                    None,
                )
                .map_err(error)?;
            device.upload_pool = device
                .raw
                .create_command_pool(
                    &vk::CommandPoolCreateInfo::default()
                        .queue_family_index(family)
                        .flags(vk::CommandPoolCreateFlags::TRANSIENT),
                    None,
                )
                .map_err(error)?;
            Ok(Rc::new(device))
        }
    }

    pub fn check_texture_format(&self, format: vk::Format) -> Result<()> {
        let features = unsafe {
            self.instance
                .raw
                .get_physical_device_format_properties(self.physical, format)
        };
        if !features
            .optimal_tiling_features
            .contains(vk::FormatFeatureFlags::SAMPLED_IMAGE | vk::FormatFeatureFlags::TRANSFER_DST)
        {
            return Err(format!(
                "GPU does not support texture format {}",
                format.as_raw()
            ));
        }
        Ok(())
    }
    fn memory_type(&self, bits: u32, flags: vk::MemoryPropertyFlags) -> Result<u32> {
        (0..self.memory.memory_type_count)
            .find(|&i| {
                bits & (1 << i) != 0
                    && self.memory.memory_types[i as usize]
                        .property_flags
                        .contains(flags)
            })
            .ok_or_else(|| "no suitable Vulkan memory type".into())
    }
    fn descriptor(
        &self,
        view: vk::ImageView,
        sampler: vk::Sampler,
    ) -> Result<(vk::DescriptorPool, vk::DescriptorSet)> {
        unsafe {
            let mut pools = self.descriptors.borrow_mut();
            let layouts = [self.descriptor_layout];
            let mut allocation = None;
            for &pool in pools.iter() {
                match self.raw.allocate_descriptor_sets(
                    &vk::DescriptorSetAllocateInfo::default()
                        .descriptor_pool(pool)
                        .set_layouts(&layouts),
                ) {
                    Ok(sets) => {
                        allocation = Some((pool, sets[0]));
                        break;
                    }
                    Err(
                        vk::Result::ERROR_OUT_OF_POOL_MEMORY | vk::Result::ERROR_FRAGMENTED_POOL,
                    ) => {}
                    Err(e) => return Err(error(e)),
                }
            }
            let (pool, set) = match allocation {
                Some(allocation) => allocation,
                None => {
                    let sizes = [
                        vk::DescriptorPoolSize {
                            ty: vk::DescriptorType::SAMPLED_IMAGE,
                            descriptor_count: 256,
                        },
                        vk::DescriptorPoolSize {
                            ty: vk::DescriptorType::SAMPLER,
                            descriptor_count: 256,
                        },
                    ];
                    let pool = self
                        .raw
                        .create_descriptor_pool(
                            &vk::DescriptorPoolCreateInfo::default()
                                .max_sets(256)
                                .pool_sizes(&sizes)
                                .flags(vk::DescriptorPoolCreateFlags::FREE_DESCRIPTOR_SET),
                            None,
                        )
                        .map_err(error)?;
                    pools.push(pool);
                    let sets = self
                        .raw
                        .allocate_descriptor_sets(
                            &vk::DescriptorSetAllocateInfo::default()
                                .descriptor_pool(pool)
                                .set_layouts(&layouts),
                        )
                        .map_err(error)?;
                    (pool, sets[0])
                }
            };
            let image = [vk::DescriptorImageInfo::default()
                .image_view(view)
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
            let sampler = [vk::DescriptorImageInfo::default().sampler(sampler)];
            self.raw.update_descriptor_sets(
                &[
                    vk::WriteDescriptorSet::default()
                        .dst_set(set)
                        .dst_binding(0)
                        .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                        .image_info(&image),
                    vk::WriteDescriptorSet::default()
                        .dst_set(set)
                        .dst_binding(1)
                        .descriptor_type(vk::DescriptorType::SAMPLER)
                        .image_info(&sampler),
                ],
                &[],
            );
            Ok((pool, set))
        }
    }

    pub fn pipeline(&self, format: vk::Format) -> Result<vk::Pipeline> {
        unsafe {
            if let Some(&pipeline) = self.pipelines.borrow().get(&format) {
                return Ok(pipeline);
            }
            let mut shaders = Vec::new();
            let result = (|| {
                for bytes in [
                    include_bytes!(concat!(env!("OUT_DIR"), "/vs_main.spv")).as_slice(),
                    include_bytes!(concat!(env!("OUT_DIR"), "/fs_main.spv")).as_slice(),
                ] {
                    let words = ash::util::read_spv(&mut std::io::Cursor::new(bytes))
                        .map_err(|e| e.to_string())?;
                    shaders.push(
                        self.raw
                            .create_shader_module(
                                &vk::ShaderModuleCreateInfo::default().code(&words),
                                None,
                            )
                            .map_err(error)?,
                    );
                }
                let stages = [
                    vk::PipelineShaderStageCreateInfo::default()
                        .stage(vk::ShaderStageFlags::VERTEX)
                        .module(shaders[0])
                        .name(c"vs_main"),
                    vk::PipelineShaderStageCreateInfo::default()
                        .stage(vk::ShaderStageFlags::FRAGMENT)
                        .module(shaders[1])
                        .name(c"fs_main"),
                ];
                let bindings = [vk::VertexInputBindingDescription {
                    binding: 0,
                    stride: 48,
                    input_rate: vk::VertexInputRate::INSTANCE,
                }];
                let attributes = [
                    vk::VertexInputAttributeDescription {
                        location: 0,
                        binding: 0,
                        format: vk::Format::R32G32_SFLOAT,
                        offset: 0,
                    },
                    vk::VertexInputAttributeDescription {
                        location: 1,
                        binding: 0,
                        format: vk::Format::R32G32_SFLOAT,
                        offset: 8,
                    },
                    vk::VertexInputAttributeDescription {
                        location: 2,
                        binding: 0,
                        format: vk::Format::R32G32B32A32_SFLOAT,
                        offset: 16,
                    },
                    vk::VertexInputAttributeDescription {
                        location: 3,
                        binding: 0,
                        format: vk::Format::R32G32B32A32_SFLOAT,
                        offset: 32,
                    },
                ];
                let vertex = vk::PipelineVertexInputStateCreateInfo::default()
                    .vertex_binding_descriptions(&bindings)
                    .vertex_attribute_descriptions(&attributes);
                let assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
                    .topology(vk::PrimitiveTopology::TRIANGLE_STRIP);
                let viewport = vk::PipelineViewportStateCreateInfo::default()
                    .viewport_count(1)
                    .scissor_count(1);
                let raster = vk::PipelineRasterizationStateCreateInfo::default()
                    .polygon_mode(vk::PolygonMode::FILL)
                    .cull_mode(vk::CullModeFlags::NONE)
                    .line_width(1.);
                let samples = vk::PipelineMultisampleStateCreateInfo::default()
                    .rasterization_samples(vk::SampleCountFlags::TYPE_1);
                let attachments = [vk::PipelineColorBlendAttachmentState::default()
                    .blend_enable(true)
                    .src_color_blend_factor(vk::BlendFactor::ONE)
                    .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
                    .color_blend_op(vk::BlendOp::ADD)
                    .src_alpha_blend_factor(vk::BlendFactor::ONE)
                    .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
                    .alpha_blend_op(vk::BlendOp::ADD)
                    .color_write_mask(vk::ColorComponentFlags::RGBA)];
                let blend =
                    vk::PipelineColorBlendStateCreateInfo::default().attachments(&attachments);
                let dynamic = vk::PipelineDynamicStateCreateInfo::default()
                    .dynamic_states(&[vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR]);
                let formats = [format];
                let mut rendering =
                    vk::PipelineRenderingCreateInfo::default().color_attachment_formats(&formats);
                let info = vk::GraphicsPipelineCreateInfo::default()
                    .stages(&stages)
                    .vertex_input_state(&vertex)
                    .input_assembly_state(&assembly)
                    .viewport_state(&viewport)
                    .rasterization_state(&raster)
                    .multisample_state(&samples)
                    .color_blend_state(&blend)
                    .dynamic_state(&dynamic)
                    .layout(self.pipeline_layout)
                    .push_next(&mut rendering);
                match self
                    .raw
                    .create_graphics_pipelines(vk::PipelineCache::null(), &[info], None)
                {
                    Ok(pipelines) => Ok(pipelines[0]),
                    Err((pipelines, e)) => {
                        for pipeline in pipelines {
                            self.raw.destroy_pipeline(pipeline, None);
                        }
                        Err(error(e))
                    }
                }
            })();
            for shader in shaders {
                self.raw.destroy_shader_module(shader, None);
            }
            let pipeline = result?;
            self.pipelines.borrow_mut().insert(format, pipeline);
            Ok(pipeline)
        }
    }

    pub fn immediate(&self, record: impl FnOnce(vk::CommandBuffer)) -> Result<()> {
        unsafe {
            let cmd = self
                .raw
                .allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default()
                        .command_pool(self.upload_pool)
                        .level(vk::CommandBufferLevel::PRIMARY)
                        .command_buffer_count(1),
                )
                .map_err(error)?[0];
            let result = (|| {
                self.raw
                    .begin_command_buffer(
                        cmd,
                        &vk::CommandBufferBeginInfo::default()
                            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
                    )
                    .map_err(error)?;
                record(cmd);
                self.raw.end_command_buffer(cmd).map_err(error)?;
                let commands = [cmd];
                self.raw
                    .queue_submit(
                        self.queue,
                        &[vk::SubmitInfo::default().command_buffers(&commands)],
                        vk::Fence::null(),
                    )
                    .map_err(error)?;
                self.raw.queue_wait_idle(self.queue).map_err(error)
            })();
            self.raw.free_command_buffers(self.upload_pool, &[cmd]);
            result
        }
    }
}
impl Drop for Device {
    fn drop(&mut self) {
        unsafe {
            let _ = self.raw.device_wait_idle();
            for (_, pipeline) in self.pipelines.get_mut().drain() {
                self.raw.destroy_pipeline(pipeline, None);
            }
            for pool in self.descriptors.get_mut().drain(..) {
                self.raw.destroy_descriptor_pool(pool, None);
            }
            self.raw.destroy_command_pool(self.upload_pool, None);
            self.raw.destroy_pipeline_layout(self.pipeline_layout, None);
            self.raw
                .destroy_descriptor_set_layout(self.descriptor_layout, None);
            self.raw.destroy_sampler(self.sampler, None);
            self.raw.destroy_sampler(self.nearest_sampler, None);
            self.raw.destroy_device(None);
        }
    }
}

pub(crate) struct Buffer {
    device: Rc<Device>,
    pub raw: vk::Buffer,
    memory: vk::DeviceMemory,
    pub capacity: usize,
    mapped: *mut u8,
}
impl Buffer {
    pub fn new(device: &Rc<Device>, capacity: usize, usage: vk::BufferUsageFlags) -> Result<Self> {
        unsafe {
            let mut buffer = Self {
                device: device.clone(),
                raw: vk::Buffer::null(),
                memory: vk::DeviceMemory::null(),
                capacity,
                mapped: std::ptr::null_mut(),
            };
            buffer.raw = device
                .raw
                .create_buffer(
                    &vk::BufferCreateInfo::default()
                        .size(capacity as u64)
                        .usage(usage)
                        .sharing_mode(vk::SharingMode::EXCLUSIVE),
                    None,
                )
                .map_err(error)?;
            let requirements = device.raw.get_buffer_memory_requirements(buffer.raw);
            let memory_type = device.memory_type(
                requirements.memory_type_bits,
                vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            )?;
            buffer.memory = device
                .raw
                .allocate_memory(
                    &vk::MemoryAllocateInfo::default()
                        .allocation_size(requirements.size)
                        .memory_type_index(memory_type),
                    None,
                )
                .map_err(error)?;
            device
                .raw
                .bind_buffer_memory(buffer.raw, buffer.memory, 0)
                .map_err(error)?;
            buffer.mapped = device
                .raw
                .map_memory(
                    buffer.memory,
                    0,
                    requirements.size,
                    vk::MemoryMapFlags::empty(),
                )
                .map_err(error)? as *mut u8;
            Ok(buffer)
        }
    }
    pub fn write(&mut self, bytes: &[u8]) {
        self.write_at(0, bytes);
    }
    pub fn write_at(&mut self, offset: usize, bytes: &[u8]) {
        assert!(offset <= self.capacity && bytes.len() <= self.capacity - offset);
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), self.mapped.add(offset), bytes.len());
        }
    }
    pub fn read(&self) -> Vec<u8> {
        unsafe { std::slice::from_raw_parts(self.mapped, self.capacity).to_vec() }
    }
}
impl Drop for Buffer {
    fn drop(&mut self) {
        unsafe {
            if !self.mapped.is_null() {
                self.device.raw.unmap_memory(self.memory);
            }
            self.device.raw.destroy_buffer(self.raw, None);
            self.device.raw.free_memory(self.memory, None);
        }
    }
}

pub(crate) struct Image {
    device: Rc<Device>,
    pub raw: vk::Image,
    pub view: vk::ImageView,
    memory: vk::DeviceMemory,
    pool: vk::DescriptorPool,
    pub descriptor: vk::DescriptorSet,
    nearest_pool: vk::DescriptorPool,
    nearest_descriptor: vk::DescriptorSet,
    pub width: u32,
    pub height: u32,
}
impl Image {
    pub fn new(device: &Rc<Device>, width: u32, height: u32, target: bool) -> Result<Self> {
        Self::with_format(device, width, height, target, IMAGE_FORMAT)
    }
    pub fn with_format(
        device: &Rc<Device>,
        width: u32,
        height: u32,
        target: bool,
        format: vk::Format,
    ) -> Result<Self> {
        unsafe {
            if width == 0
                || height == 0
                || width > device.properties.limits.max_image_dimension2_d
                || height > device.properties.limits.max_image_dimension2_d
            {
                return Err("image dimensions exceed GPU limits".into());
            }
            let mut image = Self {
                device: device.clone(),
                raw: vk::Image::null(),
                view: vk::ImageView::null(),
                memory: vk::DeviceMemory::null(),
                pool: vk::DescriptorPool::null(),
                descriptor: vk::DescriptorSet::null(),
                nearest_pool: vk::DescriptorPool::null(),
                nearest_descriptor: vk::DescriptorSet::null(),
                width,
                height,
            };
            let mut usage = vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST;
            if target {
                usage |= vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC;
            }
            image.raw = device
                .raw
                .create_image(
                    &vk::ImageCreateInfo::default()
                        .image_type(vk::ImageType::TYPE_2D)
                        .format(format)
                        .extent(vk::Extent3D {
                            width,
                            height,
                            depth: 1,
                        })
                        .mip_levels(1)
                        .array_layers(1)
                        .samples(vk::SampleCountFlags::TYPE_1)
                        .tiling(vk::ImageTiling::OPTIMAL)
                        .usage(usage)
                        .sharing_mode(vk::SharingMode::EXCLUSIVE),
                    None,
                )
                .map_err(error)?;
            let requirements = device.raw.get_image_memory_requirements(image.raw);
            let memory_type = device.memory_type(
                requirements.memory_type_bits,
                vk::MemoryPropertyFlags::DEVICE_LOCAL,
            )?;
            image.memory = device
                .raw
                .allocate_memory(
                    &vk::MemoryAllocateInfo::default()
                        .allocation_size(requirements.size)
                        .memory_type_index(memory_type),
                    None,
                )
                .map_err(error)?;
            device
                .raw
                .bind_image_memory(image.raw, image.memory, 0)
                .map_err(error)?;
            image.view = device
                .raw
                .create_image_view(
                    &vk::ImageViewCreateInfo::default()
                        .image(image.raw)
                        .view_type(vk::ImageViewType::TYPE_2D)
                        .format(format)
                        .subresource_range(color_range()),
                    None,
                )
                .map_err(error)?;
            (image.pool, image.descriptor) = device.descriptor(image.view, device.sampler)?;
            (image.nearest_pool, image.nearest_descriptor) =
                device.descriptor(image.view, device.nearest_sampler)?;
            Ok(image)
        }
    }
    pub fn filtered_descriptor(&self, filter: crate::TextureFilter) -> vk::DescriptorSet {
        match filter {
            crate::TextureFilter::Linear => self.descriptor,
            crate::TextureFilter::Nearest => self.nearest_descriptor,
        }
    }
    pub fn upload(&self, bytes: &[u8]) -> Result<()> {
        let mut staging = Buffer::new(
            &self.device,
            bytes.len(),
            vk::BufferUsageFlags::TRANSFER_SRC,
        )?;
        staging.write(bytes);
        self.device.immediate(|cmd| unsafe {
            barrier(
                &self.device.raw,
                cmd,
                self.raw,
                vk::ImageLayout::UNDEFINED,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            );
            let region = vk::BufferImageCopy::default()
                .image_subresource(color_layers())
                .image_extent(vk::Extent3D {
                    width: self.width,
                    height: self.height,
                    depth: 1,
                });
            self.device.raw.cmd_copy_buffer_to_image(
                cmd,
                staging.raw,
                self.raw,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[region],
            );
            barrier(
                &self.device.raw,
                cmd,
                self.raw,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            );
        })
    }
    pub fn read(&self) -> Result<Vec<u8>> {
        let staging = Buffer::new(
            &self.device,
            self.width as usize * self.height as usize * 4,
            vk::BufferUsageFlags::TRANSFER_DST,
        )?;
        self.device.immediate(|cmd| unsafe {
            barrier(
                &self.device.raw,
                cmd,
                self.raw,
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            );
            let region = vk::BufferImageCopy::default()
                .image_subresource(color_layers())
                .image_extent(vk::Extent3D {
                    width: self.width,
                    height: self.height,
                    depth: 1,
                });
            self.device.raw.cmd_copy_image_to_buffer(
                cmd,
                self.raw,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                staging.raw,
                &[region],
            );
            barrier(
                &self.device.raw,
                cmd,
                self.raw,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            );
        })?;
        Ok(staging.read())
    }
}
impl Drop for Image {
    fn drop(&mut self) {
        unsafe {
            if self.nearest_descriptor != vk::DescriptorSet::null() {
                let _ = self
                    .device
                    .raw
                    .free_descriptor_sets(self.nearest_pool, &[self.nearest_descriptor]);
            }
            if self.descriptor != vk::DescriptorSet::null() {
                let _ = self
                    .device
                    .raw
                    .free_descriptor_sets(self.pool, &[self.descriptor]);
            }
            self.device.raw.destroy_image_view(self.view, None);
            self.device.raw.destroy_image(self.raw, None);
            self.device.raw.free_memory(self.memory, None);
        }
    }
}

pub(crate) fn color_range() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange::default()
        .aspect_mask(vk::ImageAspectFlags::COLOR)
        .level_count(1)
        .layer_count(1)
}
pub(crate) fn color_layers() -> vk::ImageSubresourceLayers {
    vk::ImageSubresourceLayers::default()
        .aspect_mask(vk::ImageAspectFlags::COLOR)
        .layer_count(1)
}
pub(crate) unsafe fn barrier(
    device: &ash::Device,
    cmd: vk::CommandBuffer,
    image: vk::Image,
    old: vk::ImageLayout,
    new: vk::ImageLayout,
) {
    fn scope(layout: vk::ImageLayout) -> (vk::PipelineStageFlags, vk::AccessFlags) {
        match layout {
            vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL => (
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::AccessFlags::COLOR_ATTACHMENT_READ | vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
            ),
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL => (
                vk::PipelineStageFlags::FRAGMENT_SHADER,
                vk::AccessFlags::SHADER_READ,
            ),
            vk::ImageLayout::TRANSFER_DST_OPTIMAL => (
                vk::PipelineStageFlags::TRANSFER,
                vk::AccessFlags::TRANSFER_WRITE,
            ),
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL => (
                vk::PipelineStageFlags::TRANSFER,
                vk::AccessFlags::TRANSFER_READ,
            ),
            vk::ImageLayout::PRESENT_SRC_KHR => (
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                vk::AccessFlags::empty(),
            ),
            _ => (
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::AccessFlags::empty(),
            ),
        }
    }
    let (src_stage, src_access) = scope(old);
    let (dst_stage, dst_access) = scope(new);
    let dependency = vk::ImageMemoryBarrier::default()
        .old_layout(old)
        .new_layout(new)
        .src_access_mask(src_access)
        .dst_access_mask(dst_access)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(image)
        .subresource_range(color_range());
    unsafe {
        device.cmd_pipeline_barrier(
            cmd,
            src_stage,
            dst_stage,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[dependency],
        );
    }
}
