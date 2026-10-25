//! CPU decoding on the shared background executor. No URI or image lookup occurs
//! inside GPUI presentation. Accepted decoders receive limits before allocation.
use super::accounts::content::ImageJob;
use image::ImageDecoder;
use std::sync::Arc;

pub(super) fn decode(job: &ImageJob) -> Option<(Arc<gpui::RenderImage>, usize)> {
    let bytes = job.bytes.as_slice();
    if bytes.is_empty() || bytes.len() > talk_core::protocol::content::MAX_IMAGE_BYTES {
        return None;
    }
    let format = image::guess_format(bytes).ok()?;
    if !matches!(
        format,
        image::ImageFormat::Png
            | image::ImageFormat::Jpeg
            | image::ImageFormat::Gif
            | image::ImageFormat::WebP
    ) {
        return None;
    }
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    let original = matches!(job.target, talk_core::ui::content::AssetTarget::Image(_));
    limits.max_image_width = Some(if original { 8192 } else { 4096 });
    limits.max_image_height = Some(if original { 8192 } else { 4096 });
    limits.max_alloc = Some(if original {
        96 * 1024 * 1024
    } else {
        32 * 1024 * 1024
    });
    reader.limits(limits);
    let decoder = reader.into_decoder().ok()?;
    let (width, height) = decoder.dimensions();
    if width == 0
        || height == 0
        || u64::from(width) * u64::from(height)
            > if original {
                16 * 1024 * 1024
            } else {
                4 * 1024 * 1024
            }
    {
        return None;
    }
    let image = image::DynamicImage::from_decoder(decoder).ok()?;
    use talk_core::ui::content::AssetTarget;
    // Match the avatar's square Cover viewport before upload. GPUI rounds the
    // image bounds, not the rectangular parent overflow mask; a non-square
    // source would otherwise put its rounded corners outside the visible crop.
    // Center cropping preserves aspect ratio and the existing Cover framing.
    let image = if !matches!(
        &job.target,
        AssetTarget::Image(_) | AssetTarget::Thumbnail(_)
    ) && width != height
    {
        let side = width.min(height);
        image.crop_imm((width - side) / 2, (height - side) / 2, side, side)
    } else {
        image
    };
    let (max_width, max_height) = match &job.target {
        AssetTarget::Image(_) => (8192, 8192),
        AssetTarget::Thumbnail(_) => (320, 240),
        _ => (128, 128),
    };
    let image = if image.width() > max_width || image.height() > max_height {
        image.thumbnail(max_width, max_height)
    } else {
        image
    };
    let mut pixels = image.into_rgba8();
    let bytes = pixels.len();
    for pixel in pixels.as_chunks_mut::<4>().0 {
        // GPUI's raster contract is straight-alpha BGRA, matching its own
        // image loader. The renderer performs the required alpha treatment.
        pixel.swap(0, 2);
    }
    Some((
        Arc::new(gpui::RenderImage::new([image::Frame::new(pixels)])),
        bytes,
    ))
}

pub(super) fn video_frame(frame: super::content_video::Frame) -> Option<Arc<gpui::RenderImage>> {
    if frame.width == 0
        || frame.height == 0
        || frame.width > 1920
        || frame.height > 1080
        || frame.bgra.len() != frame.width as usize * frame.height as usize * 4
    {
        return None;
    }
    let pixels = image::RgbaImage::from_raw(frame.width, frame.height, frame.bgra)?;
    Some(Arc::new(gpui::RenderImage::new([image::Frame::new(
        pixels,
    )])))
}

/// Persist only a small reencoded raster, never an authenticated original or metadata.
pub(super) fn thumbnail_bytes(image: &gpui::RenderImage) -> Option<Vec<u8>> {
    use image::ImageEncoder;
    struct Bounded(Vec<u8>);
    impl std::io::Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len()
                > talk_adapters::identity::CACHE_THUMBNAIL_BYTES.saturating_sub(self.0.len())
            {
                return Err(std::io::ErrorKind::FileTooLarge.into());
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let size = image.size(0);
    let width = u32::try_from(size.width.0).ok()?;
    let height = u32::try_from(size.height.0).ok()?;
    if width == 0 || height == 0 || width > 320 || height > 240 {
        return None;
    }
    let mut rgba = image.as_bytes(0)?.to_vec();
    if rgba.len() != width as usize * height as usize * 4 {
        return None;
    }
    for pixel in rgba.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    let mut png = Bounded(Vec::new());
    if image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&rgba, width, height, image::ExtendedColorType::Rgba8)
        .is_ok()
    {
        return Some(png.0);
    }
    let rgb: Vec<u8> = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| {
            [0, 1, 2].map(|i| {
                ((u16::from(p[i]) * u16::from(p[3]) + 255 * (255 - u16::from(p[3]))) / 255) as u8
            })
        })
        .collect();
    let mut jpeg = Bounded(Vec::new());
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 85)
        .write_image(&rgb, width, height, image::ExtendedColorType::Rgb8)
        .ok()?;
    Some(jpeg.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder;
    use talk_core::ids::{AccountId, AccountScope, RuntimeGeneration};
    use talk_core::protocol::content::ImageBytes;
    use talk_core::ui::content::AssetTarget;

    fn job(bytes: Vec<u8>) -> ImageJob {
        ImageJob {
            cached: false,
            attachment: None,
            preview_generation: None,
            account: AccountScope {
                account: AccountId::from_local(1),
                runtime: RuntimeGeneration::from_local(1),
            },
            target: AssetTarget::SelfAvatar,
            key: 1,
            bytes: ImageBytes::new(bytes).unwrap(),
        }
    }

    fn png(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(pixels, width, height, image::ExtendedColorType::Rgba8)
            .unwrap();
        bytes
    }

    #[test]
    fn raster_keeps_native_bgra_alpha_and_small_dimensions() {
        let (image, count) = decode(&job(png(1, 1, &[20, 40, 80, 128]))).unwrap();
        assert_eq!(image.as_bytes(0), Some([80, 40, 20, 128].as_slice()));
        assert_eq!(count, 4);
        assert_eq!(image.frame_count(), 1);
    }

    #[test]
    fn avatar_crop_preserves_square_pixels_and_centers_portrait_and_landscape_sources() {
        for (width, height, expected) in [(4, 2, vec![1u8, 2, 5, 6]), (2, 4, vec![2u8, 3, 4, 5])] {
            let pixels: Vec<_> = (0..8u8).flat_map(|value| [value, 40, 80, 255]).collect();
            let (avatar, count) = decode(&job(png(width, height, &pixels))).unwrap();
            assert_eq!(avatar.size(0).width.0, 2);
            assert_eq!(avatar.size(0).height.0, 2);
            assert_eq!(count, 16);
            let actual: Vec<_> = avatar
                .as_bytes(0)
                .unwrap()
                .as_chunks::<4>()
                .0
                .iter()
                .map(|p| p[2])
                .collect();
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn untrusted_formats_and_decode_dimensions_are_bounded() {
        assert!(
            decode(&job(
                b"<svg><image href='https://example.invalid'/></svg>".to_vec()
            ))
            .is_none()
        );
        assert!(decode(&job(vec![0, 1, 2])).is_none());
        let wide = png(4097, 1, &vec![255; 4097 * 4]);
        assert!(decode(&job(wide)).is_none());
        let too_many_pixels = png(2049, 2048, &vec![255; 2049 * 2048 * 4]);
        assert!(decode(&job(too_many_pixels)).is_none());
    }

    #[test]
    fn gallery_original_keeps_resolution_while_its_tile_is_bounded() {
        let context = super::super::fixtures::snapshot(Some("chat"))
            .selected
            .unwrap();
        let reference = talk_core::ui::content::AttachmentRef {
            context,
            message: 7,
            placeholder: talk_core::ui::Text::new("image").unwrap(),
        };
        let mut source = job(png(2048, 1024, &vec![255; 2048 * 1024 * 4]));
        source.target = AssetTarget::Image(reference.clone());
        let (original, bytes) = decode(&source).unwrap();
        assert_eq!(
            (original.size(0).width.0, original.size(0).height.0),
            (2048, 1024)
        );
        assert_eq!(bytes, 2048 * 1024 * 4);
        assert!(
            thumbnail_bytes(&original).is_none(),
            "originals cannot enter the disk thumbnail cache"
        );
        source.target = AssetTarget::Thumbnail(reference);
        let (tile, _) = decode(&source).unwrap();
        assert_eq!((tile.size(0).width.0, tile.size(0).height.0), (320, 160));
        let encoded = thumbnail_bytes(&tile).unwrap();
        let restored = image::load_from_memory(&encoded).unwrap();
        assert_eq!((restored.width(), restored.height()), (320, 160));
    }

    #[test]
    fn noisy_transparent_thumbnail_falls_back_to_a_bounded_metadata_free_jpeg() {
        let mut state = 31u32;
        let pixels: Vec<u8> = (0..320 * 240 * 4)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                state as u8
            })
            .collect();
        let render = gpui::RenderImage::new([image::Frame::new(
            image::RgbaImage::from_raw(320, 240, pixels).unwrap(),
        )]);
        let encoded = thumbnail_bytes(&render).unwrap();
        assert_eq!(
            image::guess_format(&encoded).unwrap(),
            image::ImageFormat::Jpeg
        );
        assert!(encoded.len() <= talk_adapters::identity::CACHE_THUMBNAIL_BYTES);
        let restored = image::load_from_memory(&encoded).unwrap();
        assert_eq!((restored.width(), restored.height()), (320, 240));
        assert!(!encoded.windows(6).any(|bytes| bytes == b"Exif\0\0"));
    }
}
