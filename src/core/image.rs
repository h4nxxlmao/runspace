use crate::core::types::RawImage;
use crate::{Error, Result};

pub fn encode_png(image: &RawImage) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, image.width, image.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|error| Error::Invalid(format!("could not encode screenshot: {error}")))?;
        writer
            .write_image_data(&image.rgba)
            .map_err(|error| Error::Invalid(format!("could not encode screenshot: {error}")))?;
    }
    Ok(bytes)
}
