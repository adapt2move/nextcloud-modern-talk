//! Bounded native avatar intake. No local path or image bytes reach GPUI.
use image::{ImageDecoder, ImageFormat, ImageReader, imageops::FilterType};
use std::{
    fs::File,
    io::{Cursor, Read},
    path::Path,
};
use talk_core::ports::ErrorKind;

const MAX_FILE: u64 = 8 * 1024 * 1024;
const MAX_PIXELS: u64 = 16 * 1024 * 1024;

pub(super) fn prepare(path: &Path) -> Result<Vec<u8>, ErrorKind> {
    let file = File::open(path).map_err(|_| ErrorKind::NotFound)?;
    let metadata = file.metadata().map_err(|_| ErrorKind::Transport)?;
    if !metadata.is_file() {
        return Err(ErrorKind::Malformed);
    }
    if metadata.len() > MAX_FILE {
        return Err(ErrorKind::TooLarge);
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ErrorKind::Transport)?;
    if bytes.len() as u64 > MAX_FILE {
        return Err(ErrorKind::TooLarge);
    }
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| ErrorKind::Malformed)?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|_| ErrorKind::Malformed)?;
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(ErrorKind::TooLarge);
    }
    let orientation = decoder.orientation().map_err(|_| ErrorKind::Malformed)?;
    let mut image = image::DynamicImage::from_decoder(decoder).map_err(|_| ErrorKind::Malformed)?;
    image.apply_orientation(orientation);
    let side = image.width().min(image.height());
    let square = image
        .crop_imm(
            (image.width() - side) / 2,
            (image.height() - side) / 2,
            side,
            side,
        )
        .resize_exact(512, 512, FilterType::Lanczos3);
    let mut output = Cursor::new(Vec::new());
    square
        .write_to(&mut output, ImageFormat::Png)
        .map_err(|_| ErrorKind::Malformed)?;
    let output = output.into_inner();
    if output.len() > nextcloud_talk_api::MAX_AVATAR_BYTES {
        return Err(ErrorKind::TooLarge);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, Rgba, RgbaImage};
    #[test]
    fn center_crop_is_square_and_rejects_non_images() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("wide.png");
        let mut picture = RgbaImage::from_pixel(6, 2, Rgba([0, 0, 255, 255]));
        for x in 2..4 {
            for y in 0..2 {
                picture.put_pixel(x, y, Rgba([255, 0, 0, 255]));
            }
        }
        DynamicImage::ImageRgba8(picture).save(&path).unwrap();
        let result = prepare(&path).unwrap();
        let image = image::load_from_memory(&result).unwrap().to_rgba8();
        assert_eq!(image.dimensions(), (512, 512));
        assert_eq!(image.get_pixel(256, 256), &Rgba([255, 0, 0, 255]));
        std::fs::write(&path, b"not an image").unwrap();
        assert_eq!(prepare(&path), Err(ErrorKind::Malformed));
    }
}
