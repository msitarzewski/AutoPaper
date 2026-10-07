//! Images from providers are untrusted input: sniffed (never trusting the claimed type), size-checked
//! from the header before decoding, then stored, thumbnailed, hashed and rendered per display.
//!
//! Everything here is synchronous and CPU-bound (a 4K Lanczos render takes a noticeable fraction of a
//! second); async callers run it on a blocking thread.

use std::borrow::Cow;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Cursor, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use image::codecs::jpeg::JpegEncoder;
use image::imageops::{self, FilterType};
use image::metadata::Orientation;
use image::{DynamicImage, GenericImageView, ImageDecoder, ImageError, ImageReader, RgbImage};
use image_hasher::{BitOrder, HashAlg, HasherConfig};

use crate::error::{AutoPaperError, InvalidInputReason, Result};
use crate::providers::{FreeSize, ImageCapabilities};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Webp,
}

impl ImageFormat {
    pub fn extension(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpg",
            ImageFormat::Webp => "webp",
        }
    }

    fn codec(self) -> image::ImageFormat {
        match self {
            ImageFormat::Png => image::ImageFormat::Png,
            ImageFormat::Jpeg => image::ImageFormat::Jpeg,
            ImageFormat::Webp => image::ImageFormat::WebP,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DecodeLimits {
    pub max_bytes: usize,
    pub max_pixels: u64,
    pub max_side: u32,
}

impl Default for DecodeLimits {
    fn default() -> Self {
        Self { max_bytes: 50 * 1024 * 1024, max_pixels: 64_000_000, max_side: 16_384 }
    }
}

/// The widest pixel any accepted format decodes to (16-bit RGBA PNG), for image's allocation limit.
const MAX_BYTES_PER_PIXEL: u64 = 8;
/// Decoders' working memory beyond the pixels (row buffers, metadata chunks).
const DECODER_HEADROOM: u64 = 16 * 1024 * 1024;
const THUMBNAIL_WIDTH: u32 = 640;
const THUMBNAIL_QUALITY: u8 = 85;
/// Thumbnails of very tall images are centre-cropped to at most 1:4 so they stay small.
const THUMBNAIL_MAX_HEIGHT: u32 = THUMBNAIL_WIDTH * 4;
const RENDER_QUALITY: u8 = 90;
/// Renders upscale by more than this get the light unsharp mask.
const UNSHARP_ABOVE_SCALE: f64 = 1.25;
const UNSHARP_SIGMA: f32 = 0.8;
const UNSHARP_AMOUNT: f32 = 0.5;
const UNSHARP_THRESHOLD: i16 = 3;
/// Listed sizes whose aspect is within ~1% of the closest one count as a tie (decided by pixels).
const ASPECT_TIE: f64 = 0.01;
/// Free sizes: candidates keep at least this share of the pixels the caps allow at the exact aspect.
const FREE_SIZE_MIN_SHARE: f64 = 0.95;
/// Free sizes whose aspect is within 0.1% (log-ratio) of the closest one count as a tie, decided by pixels: no
/// pixels are given up for a difference nobody could see.
const FREE_ASPECT_TIE: f64 = 0.001;

/// PNG / JPEG / WebP by magic bytes; anything else is `None`.
pub fn sniff(bytes: &[u8]) -> Option<ImageFormat> {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";
    const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF];
    if bytes.starts_with(PNG) {
        Some(ImageFormat::Png)
    } else if bytes.starts_with(JPEG) {
        Some(ImageFormat::Jpeg)
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(&b"WEBP"[..]) {
        Some(ImageFormat::Webp)
    } else {
        None
    }
}

/// Sniffs, checks byte size and header dimensions against `limits`, then decodes (with image's own
/// limits set too, and any EXIF orientation applied). `InvalidResponse` on any violation (with a short
/// reason, never the bytes).
pub fn decode(bytes: &[u8], limits: &DecodeLimits) -> Result<(ImageFormat, DynamicImage)> {
    if bytes.len() > limits.max_bytes {
        return Err(invalid(format!(
            "the image file is too large ({} bytes; the limit is {})",
            bytes.len(),
            limits.max_bytes
        )));
    }
    let format = sniff(bytes).ok_or_else(|| invalid("not a PNG, JPEG or WebP image"))?;
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| invalid("the image couldn't be read"))?;
    if reader.format() != Some(format.codec()) {
        return Err(invalid("the image's contents don't match its type"));
    }
    reader.limits(image_limits(limits));
    // Builds the decoder from the header alone; nothing is allocated for pixels yet.
    let mut decoder = reader.into_decoder().map_err(decode_error)?;
    let (width, height) = decoder.dimensions();
    check_dimensions(width, height, limits)?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut image = DynamicImage::from_decoder(decoder).map_err(decode_error)?;
    image.apply_orientation(orientation);
    Ok((format, image))
}

/// Writes `bytes` as-is (already validated by `decode`) to `path`, creating parent directories; writes to
/// a temporary sibling and renames, so a crash never leaves a half file.
pub fn write_original(bytes: &[u8], path: &Path) -> Result<()> {
    write_atomic(path, |writer| writer.write_all(bytes).map_err(AutoPaperError::from))
}

/// 640 px wide (height by aspect, at least 1 px; images taller than 1:4 are centre-cropped to 1:4 first)
/// JPEG, quality 85, atomic write.
pub fn write_thumbnail(image: &DynamicImage, path: &Path) -> Result<()> {
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        return Err(AutoPaperError::invalid_input(InvalidInputReason::Other, "the image has no pixels"));
    }
    let scaled = scale_round(height, THUMBNAIL_WIDTH, width).max(1);
    let thumbnail = if scaled > THUMBNAIL_MAX_HEIGHT {
        let (x, y, crop_w, crop_h) = cover_crop(width, height, THUMBNAIL_WIDTH, THUMBNAIL_MAX_HEIGHT);
        image.crop_imm(x, y, crop_w, crop_h).resize_exact(THUMBNAIL_WIDTH, THUMBNAIL_MAX_HEIGHT, FilterType::Triangle)
    } else {
        image.resize_exact(THUMBNAIL_WIDTH, scaled, FilterType::Triangle)
    };
    write_jpeg(&thumbnail.into_rgb8(), THUMBNAIL_QUALITY, path)
}

/// 64-bit perceptual hash (image_hasher, 8×8, `HashAlg::Gradient`, Lanczos3 resize, LSB-first bits)
/// packed big-endian into a u64; stable across platforms for the same pixels.
pub fn phash(image: &DynamicImage) -> u64 {
    let hasher = HasherConfig::new()
        .hash_size(8, 8)
        .hash_alg(HashAlg::Gradient)
        .resize_filter(FilterType::Lanczos3)
        .bit_order(BitOrder::LsbFirst)
        .to_hasher();
    let hash = hasher.hash_image(image);
    hash.as_bytes().iter().take(8).fold(0u64, |packed, &byte| (packed << 8) | u64::from(byte))
}

pub fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// Renders for a display: centre-crop to `width:height`, Lanczos3 resize to exactly width × height, a
/// light unsharp mask only when upscaling by more than 1.25×, then JPEG quality 90 (8-bit RGB, sRGB,
/// no metadata), atomic write. `InvalidInput` when either side is 0 or the size exceeds
/// `DecodeLimits::default()` (16 384 px a side, 64 MP).
pub fn render_cover(image: &DynamicImage, width: u32, height: u32, path: &Path) -> Result<()> {
    let caps = DecodeLimits::default();
    if width == 0
        || height == 0
        || width > caps.max_side
        || height > caps.max_side
        || u64::from(width) * u64::from(height) > caps.max_pixels
    {
        return Err(AutoPaperError::invalid_input(
            InvalidInputReason::DisplaySizeInvalid,
            format!("can't render for a {width}×{height} display"),
        ));
    }
    let (source_w, source_h) = image.dimensions();
    if source_w == 0 || source_h == 0 {
        return Err(AutoPaperError::invalid_input(InvalidInputReason::Other, "the image has no pixels"));
    }
    let (x, y, crop_w, crop_h) = cover_crop(source_w, source_h, width, height);
    let rgb: Cow<'_, RgbImage> = match image {
        DynamicImage::ImageRgb8(buffer) => Cow::Borrowed(buffer),
        other => Cow::Owned(other.to_rgb8()),
    };
    let view = imageops::crop_imm(rgb.as_ref(), x, y, crop_w, crop_h);
    let mut rendered = imageops::resize(&*view, width, height, FilterType::Lanczos3);
    if f64::from(width) / f64::from(crop_w) > UNSHARP_ABOVE_SCALE {
        light_unsharp(&mut rendered);
    }
    write_jpeg(&rendered, RENDER_QUALITY, path)
}

/// The size to request: among `caps.sizes`, the one whose aspect is closest to `target_w:target_h`
/// (log-ratio distance; sizes within ~1% of the closest aspect count as a tie), ties broken by more
/// pixels. With `caps.free_size` (which takes precedence over `sizes`), a size on its `step` grid within
/// `max_side`/`max_pixels`, as large as the caps allow at the target aspect, and of those within 5% of that
/// many pixels the one closest to the target aspect. Landscape targets never get a portrait size when a
/// landscape one exists. A zero target side counts as 1; with no usable sizes at all, the target itself is
/// returned.
pub fn choose_size(caps: &ImageCapabilities, target_w: u32, target_h: u32) -> (u32, u32) {
    let (target_w, target_h) = (target_w.max(1), target_h.max(1));
    if let Some(size) = caps.free_size.and_then(|free| free_size(free, target_w, target_h)) {
        return size;
    }
    closest_listed(&caps.sizes, target_w, target_h).unwrap_or((target_w, target_h))
}

fn invalid(detail: impl Into<String>) -> AutoPaperError {
    AutoPaperError::InvalidResponse { detail: detail.into() }
}

fn image_limits(limits: &DecodeLimits) -> image::Limits {
    let mut image_limits = image::Limits::default();
    image_limits.max_image_width = Some(limits.max_side);
    image_limits.max_image_height = Some(limits.max_side);
    image_limits.max_alloc =
        Some(limits.max_pixels.saturating_mul(MAX_BYTES_PER_PIXEL).saturating_add(DECODER_HEADROOM));
    image_limits
}

fn check_dimensions(width: u32, height: u32, limits: &DecodeLimits) -> Result<()> {
    if width == 0 || height == 0 {
        return Err(invalid("the image has no pixels"));
    }
    if width > limits.max_side || height > limits.max_side || u64::from(width) * u64::from(height) > limits.max_pixels {
        return Err(invalid(format!("the image is too large ({width}×{height})")));
    }
    Ok(())
}

fn decode_error(error: ImageError) -> AutoPaperError {
    invalid(match error {
        ImageError::Limits(_) => "the image is too large",
        ImageError::Unsupported(_) => "the image uses an encoding that isn't supported",
        ImageError::IoError(_) => "the image data is truncated",
        _ => "the image data is corrupt",
    })
}

fn encode_error(error: ImageError) -> AutoPaperError {
    match error {
        ImageError::IoError(io) => AutoPaperError::from(io),
        other => AutoPaperError::Internal { detail: format!("couldn't encode the image: {other}") },
    }
}

/// `value × numerator / denominator`, rounded to nearest, saturating at `u32::MAX`.
fn scale_round(value: u32, numerator: u32, denominator: u32) -> u32 {
    let denominator = u64::from(denominator.max(1));
    let scaled = (u64::from(value) * u64::from(numerator) + denominator / 2) / denominator;
    u32::try_from(scaled).unwrap_or(u32::MAX)
}

/// The centred `(x, y, width, height)` of the largest region of a `source_w × source_h` image with the
/// aspect `target_w:target_h` (all arguments non-zero).
fn cover_crop(source_w: u32, source_h: u32, target_w: u32, target_h: u32) -> (u32, u32, u32, u32) {
    let source_wider = u64::from(source_w) * u64::from(target_h) > u64::from(target_w) * u64::from(source_h);
    if source_wider {
        let crop_w = scale_round(source_h, target_w, target_h).clamp(1, source_w);
        ((source_w - crop_w) / 2, 0, crop_w, source_h)
    } else {
        let crop_h = scale_round(source_w, target_h, target_w).clamp(1, source_h);
        (0, (source_h - crop_h) / 2, source_w, crop_h)
    }
}

/// Unsharp mask at half strength: sharpens what Lanczos softened when upscaling, without halos on
/// smooth gradients (differences at or below the threshold are left alone).
fn light_unsharp(image: &mut RgbImage) {
    let blurred = imageops::blur(&*image, UNSHARP_SIGMA);
    for (value, soft) in image.iter_mut().zip(blurred.iter()) {
        let difference = i16::from(*value) - i16::from(*soft);
        if difference.abs() > UNSHARP_THRESHOLD {
            let sharpened = f32::from(*value) + UNSHARP_AMOUNT * f32::from(difference);
            *value = sharpened.round().clamp(0.0, 255.0) as u8;
        }
    }
}

fn write_jpeg(image: &RgbImage, quality: u8, path: &Path) -> Result<()> {
    write_atomic(path, |writer| {
        JpegEncoder::new_with_quality(writer, quality).encode_image(image).map_err(encode_error)
    })
}

/// Writes through `write` into a new temporary sibling of `path` (created exclusively, so an existing
/// file or link there is never followed), syncs it, then renames it over `path`. The temporary file is
/// removed on any failure.
fn write_atomic(path: &Path, write: impl FnOnce(&mut BufWriter<File>) -> Result<()>) -> Result<()> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let file_name = path
        .file_name()
        .ok_or_else(|| AutoPaperError::invalid_input(InvalidInputReason::Other, "the image path has no file name"))?;
    let parent = path.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temp_name = OsString::from(".");
    temp_name.push(file_name);
    temp_name.push(format!(".{}-{}.tmp", std::process::id(), SEQUENCE.fetch_add(1, Ordering::Relaxed)));
    let temp = parent.join(temp_name);
    let file = OpenOptions::new().write(true).create_new(true).open(&temp)?;
    let result = (|| {
        let mut writer = BufWriter::new(file);
        write(&mut writer)?;
        let file = writer.into_inner().map_err(|error| AutoPaperError::from(error.into_error()))?;
        file.sync_all()?;
        // Closed before the rename: Windows can't rename a file that's still open.
        drop(file);
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn closest_listed(sizes: &[(u32, u32)], target_w: u32, target_h: u32) -> Option<(u32, u32)> {
    let usable = || sizes.iter().copied().filter(|&(w, h)| w > 0 && h > 0);
    let skip_portrait = target_w > target_h && usable().any(|(w, h)| w > h);
    let target = log_aspect(target_w, target_h);
    let scored: Vec<((u32, u32), f64)> = usable()
        .filter(|&(w, h)| !(skip_portrait && h > w))
        .map(|(w, h)| ((w, h), (log_aspect(w, h) - target).abs()))
        .collect();
    let closest = scored.iter().map(|&(_, distance)| distance).fold(f64::INFINITY, f64::min);
    scored
        .into_iter()
        .filter(|&(_, distance)| distance <= closest + ASPECT_TIE)
        .max_by(|(a, a_distance), (b, b_distance)| {
            pixels(*a).cmp(&pixels(*b)).then_with(|| b_distance.total_cmp(a_distance))
        })
        .map(|(size, _)| size)
}

/// Among sizes on the `step` grid within the caps, only those with at least `FREE_SIZE_MIN_SHARE` of the
/// pixels the caps allow at the exact target aspect count; of those, the closest aspect wins, aspects within
/// `FREE_ASPECT_TIE` of it being decided by more pixels. So the aspect is as close as the grid allows without
/// giving up more than 5% of the pixels (16:9 at 2,088,960 px: 1904 × 1072, 0.09% off, rather than
/// 1920 × 1072, 0.7% off).
fn free_size(free: FreeSize, target_w: u32, target_h: u32) -> Option<(u32, u32)> {
    let step = free.step.max(1);
    if free.max_side < step || free.max_pixels < u64::from(step) * u64::from(step) {
        return None;
    }
    let aspect = f64::from(target_w) / f64::from(target_h);
    let max_steps = free.max_side / step;
    let mut candidates: Vec<(u32, u32)> = Vec::new();
    for width in (1..=max_steps).map(|steps| steps * step) {
        // The tallest height this width allows; it only falls as the width grows.
        let tallest = (free.max_pixels / u64::from(width) / u64::from(step)).min(u64::from(max_steps));
        let Ok(tallest @ 1..) = u32::try_from(tallest) else { break };
        let ideal = f64::from(width) / aspect / f64::from(step);
        for steps in [ideal.floor(), ideal.ceil()] {
            let steps = if steps.is_finite() { steps.clamp(1.0, f64::from(tallest)) as u32 } else { 1 };
            candidates.push((width, steps * step));
        }
    }
    // The pixels at the exact aspect, as large as the caps allow (off the grid). A coarse grid may have nothing
    // that close: then the share is of its largest size.
    let max_side = f64::from(free.max_side);
    let exact = if aspect >= 1.0 { max_side * (max_side / aspect) } else { (max_side * aspect) * max_side };
    let most = candidates.iter().map(|&size| pixels(size)).max()?;
    let enough = exact.min(free.max_pixels as f64).min(most as f64) * FREE_SIZE_MIN_SHARE;
    let target = aspect.ln();
    let error = |(w, h): (u32, u32)| (log_aspect(w, h) - target).abs();
    let eligible: Vec<(u32, u32)> = candidates.into_iter().filter(|&size| pixels(size) as f64 >= enough).collect();
    let closest = eligible.iter().map(|&size| error(size)).fold(f64::INFINITY, f64::min);
    eligible
        .into_iter()
        .filter(|&size| error(size) <= closest + FREE_ASPECT_TIE)
        .max_by(|&a, &b| pixels(a).cmp(&pixels(b)).then_with(|| error(b).total_cmp(&error(a))))
}

fn log_aspect(width: u32, height: u32) -> f64 {
    (f64::from(width) / f64::from(height)).ln()
}

fn pixels((width, height): (u32, u32)) -> u64 {
    u64::from(width) * u64::from(height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::codecs::png::PngEncoder;
    use image::codecs::webp::WebPEncoder;
    use image::{ExtendedColorType, ImageEncoder, Rgb};

    /// A deterministic test card: diagonal gradient with a bright disc off-centre.
    fn card(width: u32, height: u32) -> DynamicImage {
        let buffer = RgbImage::from_fn(width, height, |x, y| {
            let (fx, fy) = (x as f32 / width as f32, y as f32 / height as f32);
            let (dx, dy) = (fx - 0.3, fy - 0.4);
            if dx * dx + dy * dy < 0.04 {
                Rgb([250, 240, 200])
            } else {
                Rgb([(fx * 200.0) as u8, (fy * 180.0) as u8, ((1.0 - fx) * 160.0) as u8])
            }
        });
        DynamicImage::ImageRgb8(buffer)
    }

    fn encode(image: &DynamicImage, format: ImageFormat) -> Vec<u8> {
        let rgb = image.to_rgb8();
        let mut out = Vec::new();
        match format {
            ImageFormat::Png => {
                PngEncoder::new(&mut out).write_image(&rgb, rgb.width(), rgb.height(), ExtendedColorType::Rgb8).unwrap()
            }
            ImageFormat::Jpeg => JpegEncoder::new_with_quality(&mut out, 90).encode_image(&rgb).unwrap(),
            ImageFormat::Webp => WebPEncoder::new_lossless(&mut out)
                .write_image(&rgb, rgb.width(), rgb.height(), ExtendedColorType::Rgb8)
                .unwrap(),
        }
        out
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for &byte in bytes {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
            }
        }
        !crc
    }

    fn push_chunk(png: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        png.extend((data.len() as u32).to_be_bytes());
        let start = png.len();
        png.extend(kind);
        png.extend(data);
        let crc = crc32(&png[start..]);
        png.extend(crc.to_be_bytes());
    }

    /// A well-formed PNG whose header claims `width × height` RGBA but carries no pixel data.
    fn png_claiming(width: u32, height: u32) -> Vec<u8> {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = Vec::new();
        ihdr.extend(width.to_be_bytes());
        ihdr.extend(height.to_be_bytes());
        ihdr.extend([8, 6, 0, 0, 0]);
        push_chunk(&mut png, b"IHDR", &ihdr);
        push_chunk(&mut png, b"IDAT", &[0x78, 0x9C, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01]);
        push_chunk(&mut png, b"IEND", &[]);
        png
    }

    fn invalid_detail(result: Result<(ImageFormat, DynamicImage)>) -> String {
        match result {
            Err(AutoPaperError::InvalidResponse { detail }) => detail,
            Err(other) => panic!("expected InvalidResponse, got {other:?}"),
            Ok((format, image)) => panic!("expected an error, decoded {format:?} {}x{}", image.width(), image.height()),
        }
    }

    fn mean_rgb(image: &RgbImage) -> [f64; 3] {
        let mut sum = [0f64; 3];
        for pixel in image.pixels() {
            for (total, channel) in sum.iter_mut().zip(pixel.0) {
                *total += f64::from(channel);
            }
        }
        let count = f64::from(image.width() * image.height());
        sum.map(|total| total / count)
    }

    #[test]
    fn sniffs_each_format_and_decodes_it() {
        let source = card(48, 27);
        for format in [ImageFormat::Png, ImageFormat::Jpeg, ImageFormat::Webp] {
            let bytes = encode(&source, format);
            assert_eq!(sniff(&bytes), Some(format));
            let (decoded_format, image) = decode(&bytes, &DecodeLimits::default()).unwrap();
            assert_eq!(decoded_format, format);
            assert_eq!(image.dimensions(), (48, 27), "{format:?}");
        }
        assert_eq!(ImageFormat::Jpeg.extension(), "jpg");
    }

    #[test]
    fn rejects_non_images_and_disguised_bytes() {
        for bytes in [
            &b""[..],
            b"GIF89a\x01\x00\x01\x00\x00\x00\x00",
            b"BM\x00\x00\x00\x00",
            b"<html>not an image</html>",
            b"{\"error\":{\"message\":\"no\"}}",
            b"RIFF\x00\x00\x00\x00WAVEfmt ",
            b"\x89PNG\r\n",
        ] {
            assert_eq!(sniff(bytes), None);
            assert!(invalid_detail(decode(bytes, &DecodeLimits::default())).contains("not a PNG"));
        }
        // Right magic, garbage after it.
        let mut fake = b"\x89PNG\r\n\x1a\n".to_vec();
        fake.extend([0x42; 64]);
        assert_eq!(sniff(&fake), Some(ImageFormat::Png));
        let detail = invalid_detail(decode(&fake, &DecodeLimits::default()));
        assert!(!detail.contains("BBBB"), "{detail}");
        // A truncated real image.
        let png = encode(&card(32, 32), ImageFormat::Png);
        assert!(decode(&png[..png.len() / 2], &DecodeLimits::default()).is_err());
    }

    #[test]
    fn rejects_a_huge_claimed_size_from_the_header_alone() {
        // 100000 × 100000 RGBA would need 40 GB; the header check stops it before any pixel buffer.
        let bytes = png_claiming(100_000, 100_000);
        assert!(bytes.len() < 100);
        assert!(invalid_detail(decode(&bytes, &DecodeLimits::default())).contains("too large"));
        // Within max_side but over max_pixels (81 MP).
        assert!(invalid_detail(decode(&png_claiming(9_000, 9_000), &DecodeLimits::default())).contains("too large"));
        // A sane header with no pixel data is caught by the decoder instead.
        assert!(!invalid_detail(decode(&png_claiming(64, 64), &DecodeLimits::default())).contains("too large"));
    }

    #[test]
    fn enforces_byte_pixel_and_side_limits() {
        let png = encode(&card(40, 30), ImageFormat::Png);
        let bytes_cap = DecodeLimits { max_bytes: png.len() - 1, ..DecodeLimits::default() };
        assert!(invalid_detail(decode(&png, &bytes_cap)).contains("file is too large"));
        let pixel_cap = DecodeLimits { max_pixels: 1_199, ..DecodeLimits::default() };
        assert!(invalid_detail(decode(&png, &pixel_cap)).contains("too large"));
        let side_cap = DecodeLimits { max_side: 39, ..DecodeLimits::default() };
        assert!(invalid_detail(decode(&png, &side_cap)).contains("too large"));
        let exact = DecodeLimits { max_bytes: png.len(), max_pixels: 1_200, max_side: 40 };
        assert!(decode(&png, &exact).is_ok());
    }

    #[test]
    fn writes_originals_atomically_into_new_directories() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("images/2026/abc.png");
        write_original(b"first", &path).unwrap();
        write_original(b"second", &path).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second");
        let leftovers: Vec<_> = fs::read_dir(path.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(leftovers, vec![OsString::from("abc.png")]);
        assert!(matches!(write_original(b"x", Path::new("/")), Err(AutoPaperError::InvalidInput { .. })));
    }

    #[test]
    fn thumbnails_are_640_wide_jpegs() {
        let dir = tempfile::tempdir().unwrap();
        for ((w, h), expected_h) in [((800, 450), 360), ((300, 200), 427), ((1000, 10), 6), ((100, 1000), 2560)] {
            let path = dir.path().join(format!("thumbs/{w}x{h}.jpg"));
            write_thumbnail(&card(w, h), &path).unwrap();
            let bytes = fs::read(&path).unwrap();
            assert_eq!(sniff(&bytes), Some(ImageFormat::Jpeg));
            let (_, thumb) = decode(&bytes, &DecodeLimits::default()).unwrap();
            assert_eq!(thumb.dimensions(), (640, expected_h), "{w}x{h}");
        }
    }

    #[test]
    fn cover_crop_is_centred() {
        assert_eq!(cover_crop(1536, 1024, 16, 9), (0, 80, 1536, 864));
        assert_eq!(cover_crop(3840, 2160, 2560, 1080), (0, 270, 3840, 1620));
        assert_eq!(cover_crop(3840, 2160, 1080, 1920), (1312, 0, 1215, 2160));
        assert_eq!(cover_crop(1920, 1080, 3840, 2160), (0, 0, 1920, 1080));
        assert_eq!(cover_crop(5, 5_000, 1, 1), (0, 2_497, 5, 5));
    }

    #[test]
    fn renders_exact_display_sizes() {
        let dir = tempfile::tempdir().unwrap();
        let source = card(320, 180);
        // 16:9, 16:10, 21:9, portrait, odd, and a >1.25x upscale (sharpened).
        for (w, h) in [(160, 90), (160, 100), (210, 90), (90, 160), (101, 37), (641, 361)] {
            let path = dir.path().join(format!("renders/x-{w}x{h}.jpg"));
            render_cover(&source, w, h, &path).unwrap();
            let bytes = fs::read(&path).unwrap();
            assert_eq!(sniff(&bytes), Some(ImageFormat::Jpeg));
            assert!(!bytes.windows(4).any(|window| window == b"Exif"), "no EXIF");
            assert!(!bytes.windows(11).any(|window| window == b"ICC_PROFILE"), "no ICC profile");
            let (_, rendered) = decode(&bytes, &DecodeLimits::default()).unwrap();
            assert_eq!(rendered.dimensions(), (w, h));
            assert!(matches!(rendered, DynamicImage::ImageRgb8(_)));
        }
        // RGBA sources are flattened to RGB.
        let rgba = DynamicImage::ImageRgba8(card(64, 36).to_rgba8());
        render_cover(&rgba, 32, 18, &dir.path().join("rgba.jpg")).unwrap();
    }

    #[test]
    fn render_keeps_the_centre() {
        // Red | green | blue thirds; a square render must show only the green middle.
        let source = DynamicImage::ImageRgb8(RgbImage::from_fn(300, 100, |x, _| match x / 100 {
            0 => Rgb([255, 0, 0]),
            1 => Rgb([0, 255, 0]),
            _ => Rgb([0, 0, 255]),
        }));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("square.jpg");
        render_cover(&source, 50, 50, &path).unwrap();
        let (_, rendered) = decode(&fs::read(&path).unwrap(), &DecodeLimits::default()).unwrap();
        let [r, g, b] = mean_rgb(&rendered.to_rgb8());
        assert!(g > 230.0 && r < 25.0 && b < 25.0, "{r} {g} {b}");
    }

    #[test]
    fn render_rejects_impossible_sizes() {
        let dir = tempfile::tempdir().unwrap();
        let source = card(32, 18);
        for (w, h) in [(0, 100), (100, 0), (20_000, 100), (10_000, 10_000)] {
            let result = render_cover(&source, w, h, &dir.path().join("x.jpg"));
            assert!(
                matches!(result, Err(AutoPaperError::InvalidInput { reason: InvalidInputReason::DisplaySizeInvalid, .. })),
                "{w}x{h}"
            );
        }
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn light_unsharp_leaves_flat_areas_alone_and_lifts_edges() {
        let mut flat = RgbImage::from_pixel(16, 16, Rgb([120, 130, 140]));
        light_unsharp(&mut flat);
        assert!(flat.pixels().all(|p| p.0 == [120, 130, 140]));
        let mut edge = RgbImage::from_fn(16, 16, |x, _| if x < 8 { Rgb([60, 60, 60]) } else { Rgb([200, 200, 200]) });
        light_unsharp(&mut edge);
        assert!(edge.get_pixel(8, 8).0[0] > 200);
        assert!(edge.get_pixel(7, 8).0[0] < 60);
    }

    fn sizes(list: &[(u32, u32)]) -> ImageCapabilities {
        ImageCapabilities { sizes: list.to_vec(), free_size: None }
    }

    #[test]
    fn chooses_from_an_openai_like_fixed_list() {
        let caps = sizes(&[(1024, 1024), (1536, 1024), (1024, 1536), (2560, 1440), (3840, 2160), (3840, 1648)]);
        assert_eq!(choose_size(&caps, 2560, 1440), (3840, 2160));
        assert_eq!(choose_size(&caps, 1920, 1080), (3840, 2160));
        assert_eq!(choose_size(&caps, 3440, 1440), (3840, 1648));
        assert_eq!(choose_size(&caps, 5120, 2160), (3840, 1648));
        // 16:10 is closer to 3:2 than to 16:9.
        assert_eq!(choose_size(&caps, 2560, 1600), (1536, 1024));
        assert_eq!(choose_size(&caps, 1080, 1920), (1024, 1536));
        assert_eq!(choose_size(&caps, 1000, 1000), (1024, 1024));
        // With a real 16:10 size, both 16:10 entries tie on aspect and the larger wins.
        let with_16_10 = sizes(&[(3840, 2160), (2560, 1600), (3632, 2272), (1536, 1024)]);
        assert_eq!(choose_size(&with_16_10, 2880, 1800), (3632, 2272));
    }

    #[test]
    fn chooses_from_gemini_aspect_lists() {
        let caps =
            sizes(&[(2752, 1536), (5504, 3072), (6336, 2688), (5056, 3392), (4096, 4096), (3072, 5504), (4800, 3584)]);
        assert_eq!(choose_size(&caps, 5120, 2880), (5504, 3072));
        assert_eq!(choose_size(&caps, 3440, 1440), (6336, 2688));
        assert_eq!(choose_size(&caps, 2560, 1600), (5056, 3392));
        assert_eq!(choose_size(&caps, 1600, 1200), (4800, 3584));
        assert_eq!(choose_size(&caps, 2160, 3840), (3072, 5504));
    }

    #[test]
    fn landscape_targets_never_get_portrait_sizes_when_landscape_exists() {
        // By aspect alone the portrait size is closer to 1.1:1.
        let caps = sizes(&[(1024, 1536), (1920, 1024)]);
        assert_eq!(choose_size(&caps, 1100, 1000), (1920, 1024));
        // With only portrait sizes there's no choice.
        assert_eq!(choose_size(&sizes(&[(1024, 1536)]), 1920, 1080), (1024, 1536));
    }

    #[test]
    fn free_sizes_follow_the_target_aspect_within_caps() {
        let comfy = ImageCapabilities {
            sizes: vec![(1024, 1024)],
            free_size: Some(FreeSize { step: 16, max_side: 2560, max_pixels: 4_200_000 }),
        };
        assert_eq!(choose_size(&comfy, 3840, 2160), (2560, 1440));
        assert_eq!(choose_size(&comfy, 2560, 1600), (2560, 1600));
        assert_eq!(choose_size(&comfy, 3440, 1440), (2560, 1072), "2.3881 for 2.3889 (2560 × 1056 would be 2.4242)");
        assert_eq!(choose_size(&comfy, 1440, 2560), (1440, 2560));
        assert_eq!(choose_size(&comfy, 1000, 1000), (2048, 2048));
        for (w, h) in [(3840, 2160), (2560, 1600), (3440, 1440), (1000, 1000), (7, 3), (1, 100_000), (0, 0)] {
            let (rw, rh) = choose_size(&comfy, w, h);
            assert!(rw % 16 == 0 && rh % 16 == 0 && rw >= 16 && rh >= 16, "{w}x{h} -> {rw}x{rh}");
            assert!(rw <= 2560 && rh <= 2560 && u64::from(rw) * u64::from(rh) <= 4_200_000, "{w}x{h} -> {rw}x{rh}");
        }
        let unit_step = ImageCapabilities {
            sizes: vec![],
            free_size: Some(FreeSize { step: 0, max_side: 1000, max_pixels: 1_000_000 }),
        };
        assert_eq!(choose_size(&unit_step, 16, 9), (1000, 563), "562 and 563 are equally close: more pixels");
        // An unusable free size falls back to the list, then to the target itself.
        let broken = ImageCapabilities {
            sizes: vec![(1536, 1024)],
            free_size: Some(FreeSize { step: 64, max_side: 32, max_pixels: 1 }),
        };
        assert_eq!(choose_size(&broken, 1920, 1080), (1536, 1024));
        assert_eq!(choose_size(&sizes(&[]), 1920, 1080), (1920, 1080));
        assert_eq!(choose_size(&sizes(&[(0, 0)]), 0, 1080), (1, 1080));
    }

    #[test]
    fn phash_is_stable_and_tolerates_resizing() {
        let image = card(256, 144);
        let hash = phash(&image);
        assert_eq!(hash, phash(&image));
        assert_eq!(hash, phash(&card(256, 144)));
        let resized = image.resize_exact(240, 135, FilterType::Triangle);
        assert!(hamming(hash, phash(&resized)) <= 4, "{}", hamming(hash, phash(&resized)));
        let different = DynamicImage::ImageRgb8(imageops::rotate180(&image.to_rgb8()));
        assert!(hamming(hash, phash(&different)) >= 16, "{}", hamming(hash, phash(&different)));
        assert_eq!(hamming(0, u64::MAX), 64);
    }
}
