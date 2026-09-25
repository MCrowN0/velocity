struct Push { viewport: vec2<f32>, premultiplied: u32, padding: u32 }
var<push_constant> push: Push;
@group(0) @binding(0) var image: texture_2d<f32>;
@group(0) @binding(1) var image_sampler: sampler;
struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
}
@vertex fn vs_main(@builtin(vertex_index) vertex: u32,
    @location(0) position: vec2<f32>, @location(1) size: vec2<f32>,
    @location(2) uv_rect: vec4<f32>, @location(3) color: vec4<f32>) -> VertexOut {
    let corner = vec2<f32>(f32(vertex & 1u), f32(vertex >> 1u));
    var out: VertexOut;
    out.position = vec4((position + corner * size) / push.viewport * 2. - 1., 0., 1.);
    out.uv = uv_rect.xy + corner * uv_rect.zw;
    out.color = color;
    return out;
}
@fragment fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let color = textureSample(image, image_sampler, in.uv) * in.color;
    if push.premultiplied != 0u { return color; }
    return vec4(color.rgb * color.a, color.a);
}
