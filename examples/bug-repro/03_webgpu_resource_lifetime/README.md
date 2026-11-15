# WebGPU backend: dropped buffers and textures are not destroyed

On the WebGPU backend, dropping a `wgpu::Buffer` or a `wgpu::Texture` does not
release its GPU memory: that only happens when the browser garbage collects
the JavaScript object. This repro counts the calls wgpu makes to
`createBuffer()`, `createTexture()` and `destroy()` while it:

1. creates, uses and drops 1000 buffers of 256 KiB, one per queue submission;
2. does the same with 200 RGBA8 textures of 512x512;
3. uses a bind group for 5 submissions after every handle to the buffers and
   the texture it binds was dropped, inside a validation error scope, then
   drops the bind group;
4. times `Queue::write_buffer` (16 MiB) and `Queue::write_texture`
   (2048x2048 RGBA8), and counts how many of those calls gave the browser a
   copy of the data rather than a view of the wasm memory.

Run it with:

```sh
CHROME=/path/to/chrome node examples/bug-repro/03_webgpu_resource_lifetime/run.mjs
```

It needs the `wasm32-unknown-unknown` target and the `wasm-bindgen` CLI of the
version in `Cargo.lock`. By default it uses Chrome's software adapter, so no
GPU is required; the counts do not depend on the adapter.
