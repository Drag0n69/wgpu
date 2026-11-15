//! Reproduction: on the WebGPU backend, buffers and textures that are dropped
//! stay in GPU memory until the browser garbage collects them.
//!
//! This crate only does something in a browser. `web/index.html` counts the
//! calls to `createBuffer`, `createTexture` and `destroy` that wgpu makes while
//! the functions below run, and `run.mjs` builds everything and runs the page
//! in headless Chrome. See `README.md`.

#![cfg(target_arch = "wasm32")]

use std::{cell::RefCell, rc::Rc};

use wasm_bindgen::prelude::*;

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

thread_local! {
    static GPU: RefCell<Option<Rc<Gpu>>> = const { RefCell::new(None) };
    static KEPT_BIND_GROUP: RefCell<Option<wgpu::BindGroup>> = const { RefCell::new(None) };
}

fn gpu() -> Rc<Gpu> {
    GPU.with(|gpu| gpu.borrow().clone().expect("call `init` first"))
}

/// Requests a device. Returns a description of the adapter.
#[wasm_bindgen]
pub async fn init() -> Result<String, JsValue> {
    console_error_panic_hook::set_once();

    let instance = wgpu::Instance::default();
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions::default())
        .await
        .map_err(|error| JsValue::from_str(&format!("no adapter: {error}")))?;
    let info = adapter.get_info();
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor::default())
        .await
        .map_err(|error| JsValue::from_str(&format!("no device: {error}")))?;
    GPU.with(|gpu| *gpu.borrow_mut() = Some(Rc::new(Gpu { device, queue })));

    Ok(format!("{} ({:?})", info.name, info.backend))
}

/// Submits `count` times without any work, the way a frame ends.
#[wasm_bindgen]
pub fn submit_empty(count: u32) {
    let gpu = gpu();
    for _ in 0..count {
        gpu.queue.submit(None);
    }
}

/// What a renderer does when it allocates a scratch buffer each frame:
/// creates a buffer, writes to it, submits, and drops it. `count` times.
#[wasm_bindgen]
pub fn churn_buffers(count: u32, size: u32) {
    let gpu = gpu();
    for frame in 0..count {
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("repro scratch buffer"),
            size: size as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        gpu.queue.write_buffer(&buffer, 0, &[frame as u8; 16]);
        gpu.queue.submit(None);
        drop(buffer);
    }
}

/// Same as [`churn_buffers`], with square RGBA8 textures and a view of each.
#[wasm_bindgen]
pub fn churn_textures(count: u32, size: u32) {
    let gpu = gpu();
    for _ in 0..count {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("repro scratch texture"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
            texture_binding_view_dimension: None,
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        gpu.queue.submit(None);
        drop(view);
        drop(texture);
    }
}

/// Creates two buffers, a texture and a bind group that binds them, drops
/// every handle but the bind group, then uses the bind group in a compute pass
/// for `frames` submissions.
///
/// The bind group must keep its resources alive, as it does on native.
/// Returns the validation error the browser reported, or an empty string.
#[wasm_bindgen]
pub async fn use_bind_group_after_dropping_its_resources(frames: u32) -> String {
    let gpu = gpu();
    let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);

    let module = gpu
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
    let pipeline = gpu
        .device
        .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

    let params = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("repro bound uniform buffer"),
        size: 16,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let output = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("repro bound storage buffer"),
        size: 16,
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    });
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("repro bound texture"),
        size: wgpu::Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
        texture_binding_view_dimension: None,
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: params.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: output.as_entire_binding(),
            },
        ],
    });

    // From here on, only the bind group refers to the resources.
    drop(params);
    drop(output);
    drop(view);
    drop(texture);

    for _ in 0..frames {
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        gpu.queue.submit([encoder.finish()]);
    }

    KEPT_BIND_GROUP.with(|kept| *kept.borrow_mut() = Some(bind_group));

    match scope.pop().await {
        Some(error) => format!("{error}"),
        None => String::new(),
    }
}

/// Drops the bind group that [`use_bind_group_after_dropping_its_resources`]
/// kept.
#[wasm_bindgen]
pub fn drop_bind_group() {
    KEPT_BIND_GROUP.with(|kept| *kept.borrow_mut() = None);
}

/// Calls `Queue::write_buffer` `count` times with `size` bytes.
#[wasm_bindgen]
pub fn write_buffer_repeatedly(count: u32, size: u32) {
    let gpu = gpu();
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("repro write_buffer destination"),
        size: size as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let data = vec![0x5a_u8; size as usize];
    for _ in 0..count {
        gpu.queue.write_buffer(&buffer, 0, &data);
        gpu.queue.submit(None);
    }
    buffer.destroy();
}

/// Calls `Queue::write_texture` `count` times with a square RGBA8 image.
#[wasm_bindgen]
pub fn write_texture_repeatedly(count: u32, size: u32) {
    let gpu = gpu();
    let extent = wgpu::Extent3d {
        width: size,
        height: size,
        depth_or_array_layers: 1,
    };
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("repro write_texture destination"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
        texture_binding_view_dimension: None,
    });
    let data = vec![0x5a_u8; (size * size * 4) as usize];
    for _ in 0..count {
        gpu.queue.write_texture(
            texture.as_image_copy(),
            &data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size * 4),
                rows_per_image: Some(size),
            },
            extent,
        );
        gpu.queue.submit(None);
    }
    texture.destroy();
}
