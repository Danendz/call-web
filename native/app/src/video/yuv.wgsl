// YUV planes to RGBA, with the BT.601 limited-range matrix libyuv's I420ToABGR uses on the CPU path, so colors
// match. Chroma is read without filtering, one sample per 2×2 pixels, as libyuv does.

@group(0) @binding(0) var y_plane: texture_2d<f32>;
// The U plane, or for NV12 U and V interleaved
@group(0) @binding(1) var u_plane: texture_2d<f32>;
@group(0) @binding(2) var v_plane: texture_2d<f32>;

// One triangle that covers the whole target
@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32(i >> 1u) * 4 - 1);
    return vec4<f32>(x, y, 0.0, 1.0);
}

fn rgba(y: f32, u: f32, v: f32) -> vec4<f32> {
    let c = 1.164 * (y - 16.0 / 255.0);
    let d = u - 0.5;
    let e = v - 0.5;
    let rgb = vec3<f32>(c + 1.596 * e, c - 0.391 * d - 0.813 * e, c + 2.018 * d);
    return vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}

@fragment
fn fs_i420(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(at.xy);
    let chroma = pixel / 2;
    return rgba(
        textureLoad(y_plane, pixel, 0).r,
        textureLoad(u_plane, chroma, 0).r,
        textureLoad(v_plane, chroma, 0).r,
    );
}

@fragment
fn fs_nv12(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(at.xy);
    let uv = textureLoad(u_plane, pixel / 2, 0).rg;
    return rgba(textureLoad(y_plane, pixel, 0).r, uv.x, uv.y);
}
