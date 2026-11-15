struct Params {
    value: vec4<f32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var<storage, read_write> output: array<vec4<f32>, 1>;

@compute @workgroup_size(1)
fn main() {
    output[0] = params.value + textureLoad(source, vec2<i32>(0, 0), 0);
}
