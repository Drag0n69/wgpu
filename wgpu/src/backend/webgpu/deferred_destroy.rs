//! Destroys the buffers and textures of the WebGPU backend once nothing uses
//! them any more.
//!
//! Dropping the last handle to a `GPUBuffer` or `GPUTexture` does not free its
//! memory: the browser only does that when it garbage collects the JavaScript
//! object, and the garbage collector is driven by the size of the JavaScript
//! heap, not by GPU memory. An application that creates and drops resources
//! every frame can therefore fill GPU memory long before a collection happens.
//!
//! To match what `wgpu-core` does on native, a resource is kept alive as long
//! as something refers to it:
//!
//! - a buffer, by its [`WebBuffer`] and by the bind groups that bind it;
//! - a texture created by wgpu, by its [`WebTexture`], by its views and by the
//!   bind groups that bind one of those views.
//!
//! All of them share a [`BufferLife`] or a [`TextureLife`]. Once the last
//! reference is gone, `destroy()` is called on the resource after two more
//! calls to `GPUQueue.submit()`: commands that use the resource may have been
//! recorded before it was dropped and submitted afterwards. WebGPU itself keeps
//! the memory of a destroyed resource until the work that was already submitted
//! has completed.
//!
//! Surface textures and external textures are not ours to destroy and have no
//! [`TextureLife`].
//!
//! [`WebBuffer`]: super::WebBuffer
//! [`WebTexture`]: super::WebTexture

use alloc::{rc::Rc, vec::Vec};

use super::webgpu_sys;

/// Keeps a `GPUBuffer` alive. Calls `destroy()` on it, after a delay, when
/// dropped.
#[derive(Debug)]
pub(crate) struct BufferLife(webgpu_sys::GpuBuffer);

impl BufferLife {
    pub(crate) fn new(buffer: &webgpu_sys::GpuBuffer) -> Rc<Self> {
        Rc::new(Self(buffer.clone()))
    }
}

impl Drop for BufferLife {
    fn drop(&mut self) {
        graveyard::dropped(Dead::Buffer(self.0.clone()));
    }
}

/// Keeps a `GPUTexture` created by `GPUDevice.createTexture()` alive. Calls
/// `destroy()` on it, after a delay, when dropped.
#[derive(Debug)]
pub(crate) struct TextureLife(webgpu_sys::GpuTexture);

impl TextureLife {
    pub(crate) fn new(texture: &webgpu_sys::GpuTexture) -> Rc<Self> {
        Rc::new(Self(texture.clone()))
    }
}

impl Drop for TextureLife {
    fn drop(&mut self) {
        graveyard::dropped(Dead::Texture(self.0.clone()));
    }
}

/// A resource that a bind group keeps alive.
#[derive(Debug, Clone)]
pub(crate) enum Held {
    Buffer(#[allow(dead_code)] Rc<BufferLife>),
    Texture(#[allow(dead_code)] Rc<TextureLife>),
}

/// Returns what a bind group created from `desc` must keep alive.
pub(crate) fn held_by_bind_group(desc: &crate::BindGroupDescriptor<'_>) -> Vec<Held> {
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

/// A resource that nothing refers to any more.
enum Dead {
    Buffer(webgpu_sys::GpuBuffer),
    Texture(webgpu_sys::GpuTexture),
}

/// Must be called after each `GPUQueue.submit()`.
///
/// Destroys the resources that were dropped before the previous submission.
pub(crate) fn submitted() {
    graveyard::submitted();
}

#[cfg(std)]
mod graveyard {
    use alloc::vec::Vec;
    use core::cell::RefCell;

    use super::Dead;

    #[derive(Default)]
    struct Graveyard {
        /// Resources dropped since the last submission.
        dropped: Vec<Dead>,
        /// Resources dropped before the last submission. These are destroyed
        /// by the next one.
        waiting: Vec<Dead>,
    }

    std::thread_local! {
        static GRAVEYARD: RefCell<Graveyard> = RefCell::new(Graveyard::default());
    }

    pub(super) fn dropped(dead: Dead) {
        GRAVEYARD.with(|graveyard| graveyard.borrow_mut().dropped.push(dead));
    }

    pub(super) fn submitted() {
        let ready = GRAVEYARD.with(|graveyard| {
            let mut graveyard = graveyard.borrow_mut();
            let dropped = core::mem::take(&mut graveyard.dropped);
            core::mem::replace(&mut graveyard.waiting, dropped)
        });
        for dead in ready {
            match dead {
                Dead::Buffer(buffer) => buffer.destroy(),
                Dead::Texture(texture) => texture.destroy(),
            }
        }
    }
}

#[cfg(not(std))]
mod graveyard {
    // Without thread-local storage, leave the resources to the garbage
    // collector, as before.
    pub(super) fn dropped(_: super::Dead) {}
    pub(super) fn submitted() {}
}
