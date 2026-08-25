//! Color and scalar helpers shared by the two map renderers.

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

pub fn smooth_curve(value: f32) -> f32 {
    value * value * (3.0 - 2.0 * value)
}

pub fn smoothstep(edge0: f32, edge1: f32, value: f32) -> f32 {
    smooth_curve(((value - edge0) / (edge1 - edge0)).clamp(0.0, 1.0))
}

pub fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        lerp(a[0], b[0], t),
        lerp(a[1], b[1], t),
        lerp(a[2], b[2], t),
    ]
}

pub fn scale(color: [f32; 3], factor: f32) -> [f32; 3] {
    [color[0] * factor, color[1] * factor, color[2] * factor]
}

pub fn to_rgb(color: [f32; 3]) -> [u8; 3] {
    [
        color[0].round().clamp(0.0, 255.0) as u8,
        color[1].round().clamp(0.0, 255.0) as u8,
        color[2].round().clamp(0.0, 255.0) as u8,
    ]
}

pub fn unit_to_u8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}
