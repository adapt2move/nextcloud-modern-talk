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
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let decoder = reader.into_decoder().ok()?;
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 4 * 1024 * 1024 {
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
        AssetTarget::Image(_) => (1600, 1200),
        AssetTarget::Thumbnail(_) => (400, 300),
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

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder;
    use talk_core::ids::{AccountId, AccountScope, RuntimeGeneration};
    use talk_core::protocol::content::ImageBytes;
    use talk_core::ui::content::AssetTarget;

    fn job(bytes: Vec<u8>) -> ImageJob {
        ImageJob {
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
}
