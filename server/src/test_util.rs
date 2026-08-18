use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A solid-colour PNG, for the paths that take an uploaded image.
pub fn test_png(size: u32, pixel: [u8; 4]) -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(size, size, image::Rgba(pixel));
    let mut out = Vec::new();
    image::ImageEncoder::write_image(
        image::codecs::png::PngEncoder::new(&mut out),
        image.as_raw(),
        size,
        size,
        image::ExtendedColorType::Rgba8,
    )
    .expect("encode a test PNG");
    out
}

pub fn unique_temp_dir(name: &str) -> PathBuf {
    let counter = TEMP_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "_onlinerpg_{name}_{}_{counter}",
        std::process::id()
    ))
}
