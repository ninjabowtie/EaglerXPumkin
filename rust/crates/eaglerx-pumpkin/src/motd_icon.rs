//! Eagler MOTD icons are a binary frame containing 64x64 RGBA pixels.
//! JSON carries a boolean announcing that frame, never a PNG URL.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::io::Cursor;

pub fn decode_data_uri(uri: &str) -> Result<Vec<u8>, String> {
    if uri.len() > 2_097_152 {
        return Err("motd_icon must be at most 2097152 bytes".into());
    }
    let encoded = uri.strip_prefix("data:image/png;base64,").ok_or(
        "motd_icon must be a 64x64 PNG data URI (data:image/png;base64,...); image URLs are unsupported",
    )?;
    let png = STANDARD
        .decode(encoded)
        .map_err(|e| format!("motd_icon has invalid base64: {e}"))?;
    decode_png(&png).map_err(|e| format!("motd_icon: {e}"))
}

pub fn decode_png(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_limits(png::Limits { bytes: 2_097_152 });
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let info = reader.info();
    if info.width != 64 || info.height != 64 || info.animation_control.is_some() {
        return Err("server icon must be a non-animated 64x64 PNG".into());
    }
    let mut data = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or("PNG output is too large")?
    ];
    let output = reader.next_frame(&mut data).map_err(|e| e.to_string())?;
    reader.finish().map_err(|e| e.to_string())?;
    let data = &data[..output.buffer_size()];
    let mut rgba = Vec::with_capacity(64 * 64 * 4);
    match output.color_type {
        png::ColorType::Rgba => rgba.extend_from_slice(data),
        png::ColorType::Rgb => {
            for rgb in data.as_chunks::<3>().0 {
                rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            }
        }
        png::ColorType::Grayscale => {
            for &gray in data {
                rgba.extend_from_slice(&[gray, gray, gray, 255]);
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for pixel in data.as_chunks::<2>().0 {
                rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]);
            }
        }
        png::ColorType::Indexed => return Err("PNG palette was not expanded".into()),
    }
    Ok(rgba)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn png(width: u32, height: u32, color: png::ColorType, pixels: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, width, height);
            encoder.set_color(color);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(pixels).unwrap();
        }
        bytes
    }

    pub fn test_icon_uri() -> String {
        format!(
            "data:image/png;base64,{}",
            STANDARD.encode(png(
                64,
                64,
                png::ColorType::Rgba,
                &[17, 34, 51, 68].repeat(4096)
            ))
        )
    }

    #[test]
    fn converts_png_channels_to_wire_rgba() {
        for (color, input, expected) in [
            (png::ColorType::Rgba, vec![17, 34, 51, 68], [17, 34, 51, 68]),
            (png::ColorType::Rgb, vec![17, 34, 51], [17, 34, 51, 255]),
            (png::ColorType::Grayscale, vec![17], [17, 17, 17, 255]),
            (
                png::ColorType::GrayscaleAlpha,
                vec![17, 68],
                [17, 17, 17, 68],
            ),
        ] {
            assert_eq!(
                decode_png(&png(64, 64, color, &input.repeat(4096))).unwrap(),
                expected.repeat(4096)
            );
        }
    }

    #[test]
    fn expands_palette_transparency_and_strips_sixteen_bit_channels() {
        let mut indexed = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut indexed, 64, 64);
            encoder.set_color(png::ColorType::Indexed);
            encoder.set_depth(png::BitDepth::One);
            encoder.set_palette(vec![17, 34, 51, 70, 80, 90]);
            encoder.set_trns(vec![68, 255]);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[0; 512]).unwrap();
        }
        assert_eq!(decode_png(&indexed).unwrap(), [17, 34, 51, 68].repeat(4096));
        let mut sixteen = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut sixteen, 64, 64);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Sixteen);
            let mut writer = encoder.write_header().unwrap();
            writer
                .write_image_data(&[17, 1, 34, 2, 51, 3, 68, 4].repeat(4096))
                .unwrap();
        }
        assert_eq!(decode_png(&sixteen).unwrap(), [17, 34, 51, 68].repeat(4096));
    }

    #[test]
    fn rejects_animated_png() {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 64, 64);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_animated(1, 0).unwrap();
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[0; 64 * 64 * 4]).unwrap();
        }
        assert!(decode_png(&bytes).is_err());
    }

    #[test]
    fn rejects_bad_images_before_serving_queries() {
        for uri in [
            "",
            "https://example.com/icon.png",
            "data:image/png;base64,???",
            "data:image/png;base64,YmFk",
        ] {
            assert!(decode_data_uri(uri).is_err());
        }
        let oversized = png(65, 64, png::ColorType::Rgba, &[0; 65 * 64 * 4]);
        assert!(decode_png(&oversized).is_err());
        let valid = png(64, 64, png::ColorType::Rgba, &[0; 64 * 64 * 4]);
        assert!(decode_png(&valid[..valid.len() / 2]).is_err());
        let mut corrupt = valid;
        corrupt[29] ^= 1; // IHDR CRC
        assert!(decode_png(&corrupt).is_err());
    }
}
