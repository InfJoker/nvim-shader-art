use super::Encoder;

/// Kitty encoder for piped mode: outputs base64-encoded PNG frames to stdout.
///
/// Each frame is one line of base64 on stdout. The Lua plugin wraps it in
/// Kitty APC escapes and writes to the terminal from Neovim's event loop,
/// preventing write interleaving between sidecar and Neovim TUI.
pub struct KittyEncoder {
    pub grid_cols: u32,
    pub grid_rows: u32,
}

impl KittyEncoder {
    pub fn new(_row: u32, _col: u32, grid_cols: u32, grid_rows: u32) -> Self {
        Self {
            grid_cols,
            grid_rows,
        }
    }
}

impl Encoder for KittyEncoder {
    fn encode(&mut self, pixels: &[u8], width: u32, height: u32) -> Vec<u8> {
        let png_data = encode_png(pixels, width, height);
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&png_data);

        // Output: width,height,cols,rows;base64\n
        // The Lua plugin parses this and builds the Kitty APC escape.
        let header = format!("{},{},{},{};", width, height, self.grid_cols, self.grid_rows);
        let mut output = Vec::with_capacity(header.len() + b64.len() + 1);
        output.extend_from_slice(header.as_bytes());
        output.extend_from_slice(b64.as_bytes());
        output.push(b'\n');
        output
    }
}

/// Generate the delete escape for cleanup.
pub fn delete_escape(image_id: u32) -> Vec<u8> {
    format!("\x1b_Ga=d,d=I,i={image_id},q=2\x1b\\").into_bytes()
}

fn encode_png(pixels: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut buf, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(pixels).unwrap();
    }
    buf
}
