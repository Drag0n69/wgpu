//! GripForge (MOT-133, written here, not in wgpu 30.0.1): the WebGPU backend destroys a buffer or a texture once
//! nothing uses it.
//!
//! Upstream, dropping a [`super::WebBuffer`] or a [`super::WebTexture`] does nothing (`// no-op`): the `GPUBuffer` or
//! `GPUTexture` stays in video memory until the JavaScript garbage collector takes its wrapper, and that collector
//! runs on the pressure of the JavaScript heap, not of the graphics card. A Bevy frame on WebGPU makes buffers it
//! drops a frame later (the clustering of the lights: five per view and per frame; a material changed every frame:
//! two), and textures come and go too, so the card filled: 5 GB in 40 s on an 8 GB card (the GripForge showcase), on
//! every demo of the site.
//!
//! Here, as wgpu-core does natively, a resource lives as long as a Rust handle holds it, or a bind group that binds it
//! (a texture: through any of its views). The resource, its views and those bind groups share a life ([`BufferLife`],
//! [`TextureLife`]). When the last one goes, the resource waits for two queue submissions, then is destroyed. Two,
//! because commands recorded with it may still be waiting for the next submit (WebGPU itself keeps a destroyed
//! resource until the work already submitted is done). The textures of the canvas and the external ones have no life:
//! they are not ours to destroy.
use super::webgpu_sys;
use alloc::{rc::Rc, vec::Vec};

/// What keeps one `GPUBuffer` alive: held by its `WebBuffer` (and its clones) and by the bind groups that bind it.
#[derive(Debug)]
pub(crate) struct BufferLife(webgpu_sys::GpuBuffer);

impl BufferLife {
    pub(crate) fn new(buffer: &webgpu_sys::GpuBuffer) -> Rc<Self> {
        Rc::new(Self(buffer.clone()))
    }
}

/// What keeps one `GPUTexture` made by `createTexture` alive: held by its `WebTexture`, its views and the bind groups
/// that bind one of them.
#[derive(Debug)]
pub(crate) struct TextureLife(webgpu_sys::GpuTexture);

impl TextureLife {
    pub(crate) fn new(texture: &webgpu_sys::GpuTexture) -> Rc<Self> {
        Rc::new(Self(texture.clone()))
    }
}

/// A life a bind group holds (only to keep it).
#[derive(Debug, Clone)]
pub(crate) enum Held {
    Buffer(#[allow(dead_code)] Rc<BufferLife>),
    Texture(#[allow(dead_code)] Rc<TextureLife>),
}

/// The lives of the buffers and textures a bind group binds.
pub(crate) fn of(desc: &crate::BindGroupDescriptor<'_>) -> Vec<Held> {
    desc.entries
        .iter()
        .filter_map(|entry| match &entry.resource {
            crate::BindingResource::Buffer(binding) => {
                Some(Held::Buffer(binding.buffer.inner.as_webgpu().life.clone()))
            }
            crate::BindingResource::TextureView(view) => {
                view.inner.as_webgpu().life.clone().map(Held::Texture)
            }
            _ => None,
        })
        .collect()
}

/// A resource whose last holder went.
enum Dead {
    Buffer(webgpu_sys::GpuBuffer),
    Texture(webgpu_sys::GpuTexture),
}

#[cfg(std)]
mod graveyard {
    use super::Dead;
    use alloc::vec::Vec;
    use core::cell::RefCell;

    /// The resources dropped since the last submit, and those dropped before it (destroyed after the next one).
    #[derive(Default)]
    struct Graveyard {
        dropped: Vec<Dead>,
        waiting: Vec<Dead>,
    }

    std::thread_local! {
        static GRAVEYARD: RefCell<Graveyard> = RefCell::new(Graveyard::default());
    }

    pub(super) fn dropped(dead: Dead) {
        GRAVEYARD.with(|g| g.borrow_mut().dropped.push(dead));
    }

    pub(super) fn submitted() {
        let ready = GRAVEYARD.with(|g| {
            let mut g = g.borrow_mut();
            let dropped = core::mem::take(&mut g.dropped);
            core::mem::replace(&mut g.waiting, dropped)
        });
        for dead in ready {
            match dead {
                Dead::Buffer(b) => b.destroy(),
                Dead::Texture(t) => t.destroy(),
            }
        }
    }
}

#[cfg(not(std))]
mod graveyard {
    // Without std (no thread-local storage): as upstream, the garbage collector frees them.
    pub(super) fn dropped(_: super::Dead) {}
    pub(super) fn submitted() {}
}

impl Drop for BufferLife {
    fn drop(&mut self) {
        graveyard::dropped(Dead::Buffer(self.0.clone()));
    }
}

impl Drop for TextureLife {
    fn drop(&mut self) {
        graveyard::dropped(Dead::Texture(self.0.clone()));
    }
}

/// After each `GPUQueue.submit`: destroys the resources dropped before the previous one.
pub(crate) fn submitted() {
    graveyard::submitted();
}
