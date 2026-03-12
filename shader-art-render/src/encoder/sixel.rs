use super::Encoder;

/// Self-contained Sixel encoder with median-cut color quantization.
/// No external dependency on libsixel.
pub struct SixelEncoder {
    pub row: u32,
    pub col: u32,
}

/// Maximum colors in Sixel palette
const MAX_COLORS: usize = 256;

struct ColorEntry {
    r: u8,
    g: u8,
    b: u8,
}

impl Encoder for SixelEncoder {
    fn encode(&mut self, pixels: &[u8], width: u32, height: u32) -> Vec<u8> {
        let (palette, indexed) = quantize(pixels, width, height);

        let mut output = Vec::new();

        // Cursor position
        output.extend_from_slice(format!("\x1b[{};{}H", self.row, self.col).as_bytes());

        // Sixel start: DCS q
        output.extend_from_slice(b"\x1bPq");

        // Raster attributes: Pan;Pad;Ph;Pv (aspect 1:1, width, height)
        output.extend_from_slice(format!("\"1;1;{width};{height}").as_bytes());

        // Define palette
        for (i, color) in palette.iter().enumerate() {
            // Sixel uses 0-100 range for RGB
            let r = (color.r as u32 * 100) / 255;
            let g = (color.g as u32 * 100) / 255;
            let b = (color.b as u32 * 100) / 255;
            output.extend_from_slice(format!("#{i};2;{r};{g};{b}").as_bytes());
        }

        // Encode sixel rows (each 6 pixels high)
        let w = width as usize;
        let h = height as usize;

        for band_y in (0..h).step_by(6) {
            for color_idx in 0..palette.len() {
                let mut has_pixels = false;
                let mut row_data = Vec::with_capacity(w);

                for x in 0..w {
                    let mut sixel_bits: u8 = 0;
                    for dy in 0..6 {
                        let y = band_y + dy;
                        if y < h && indexed[y * w + x] == color_idx as u8 {
                            sixel_bits |= 1 << dy;
                        }
                    }
                    if sixel_bits != 0 {
                        has_pixels = true;
                    }
                    row_data.push(sixel_bits + 0x3f); // Sixel char = bits + 63
                }

                if has_pixels {
                    // Color selector
                    output.extend_from_slice(format!("#{color_idx}").as_bytes());

                    // RLE compress the row
                    rle_encode(&row_data, &mut output);

                    // Carriage return ($) to go back to start of this band
                    output.push(b'$');
                }
            }
            // Next band (-)
            output.push(b'-');
        }

        // Sixel end: ST
        output.extend_from_slice(b"\x1b\\");

        output
    }
}

/// Simple RLE compression for sixel data
fn rle_encode(data: &[u8], output: &mut Vec<u8>) {
    let mut i = 0;
    while i < data.len() {
        let ch = data[i];
        let mut count = 1;
        while i + count < data.len() && data[i + count] == ch && count < 255 {
            count += 1;
        }
        if count >= 3 {
            output.extend_from_slice(format!("!{count}").as_bytes());
            output.push(ch);
        } else {
            for _ in 0..count {
                output.push(ch);
            }
        }
        i += count;
    }
}

/// Median-cut color quantization: reduce to MAX_COLORS palette.
fn quantize(pixels: &[u8], width: u32, height: u32) -> (Vec<ColorEntry>, Vec<u8>) {
    let pixel_count = (width * height) as usize;
    let mut colors: Vec<[u8; 3]> = Vec::with_capacity(pixel_count);

    for i in 0..pixel_count {
        let idx = i * 4;
        colors.push([pixels[idx], pixels[idx + 1], pixels[idx + 2]]);
    }

    // Build initial box
    let indices: Vec<usize> = (0..pixel_count).collect();
    let mut boxes = vec![indices];

    // Split boxes until we have enough colors
    while boxes.len() < MAX_COLORS {
        // Find the box with the largest range to split
        let mut best_box = 0;
        let mut best_range = 0u32;

        for (bi, b) in boxes.iter().enumerate() {
            if b.len() < 2 {
                continue;
            }
            let range = box_range(b, &colors);
            if range > best_range {
                best_range = range;
                best_box = bi;
            }
        }

        if best_range == 0 {
            break;
        }

        let to_split = boxes.remove(best_box);
        let (a, b) = split_box(&to_split, &colors);
        if !a.is_empty() {
            boxes.push(a);
        }
        if !b.is_empty() {
            boxes.push(b);
        }
    }

    // Compute palette: average color per box
    let mut palette = Vec::with_capacity(boxes.len());
    for b in &boxes {
        let (mut sr, mut sg, mut sb) = (0u64, 0u64, 0u64);
        for &idx in b {
            sr += colors[idx][0] as u64;
            sg += colors[idx][1] as u64;
            sb += colors[idx][2] as u64;
        }
        let n = b.len() as u64;
        palette.push(ColorEntry {
            r: (sr / n.max(1)) as u8,
            g: (sg / n.max(1)) as u8,
            b: (sb / n.max(1)) as u8,
        });
    }

    // Map each pixel to nearest palette color
    let mut indexed = vec![0u8; pixel_count];
    for i in 0..pixel_count {
        let c = colors[i];
        let mut best = 0;
        let mut best_dist = u32::MAX;
        for (pi, p) in palette.iter().enumerate() {
            let dr = c[0] as i32 - p.r as i32;
            let dg = c[1] as i32 - p.g as i32;
            let db = c[2] as i32 - p.b as i32;
            let dist = (dr * dr + dg * dg + db * db) as u32;
            if dist < best_dist {
                best_dist = dist;
                best = pi;
            }
        }
        indexed[i] = best as u8;
    }

    (palette, indexed)
}

fn box_range(indices: &[usize], colors: &[[u8; 3]]) -> u32 {
    let mut min = [255u8; 3];
    let mut max = [0u8; 3];
    for &idx in indices {
        for c in 0..3 {
            min[c] = min[c].min(colors[idx][c]);
            max[c] = max[c].max(colors[idx][c]);
        }
    }
    let mut range = 0u32;
    for c in 0..3 {
        range = range.max((max[c] - min[c]) as u32);
    }
    range
}

fn split_box(indices: &[usize], colors: &[[u8; 3]]) -> (Vec<usize>, Vec<usize>) {
    // Find channel with largest range
    let mut min = [255u8; 3];
    let mut max = [0u8; 3];
    for &idx in indices {
        for c in 0..3 {
            min[c] = min[c].min(colors[idx][c]);
            max[c] = max[c].max(colors[idx][c]);
        }
    }

    let mut split_channel = 0;
    let mut max_range = 0;
    for c in 0..3 {
        let range = (max[c] - min[c]) as u32;
        if range > max_range {
            max_range = range;
            split_channel = c;
        }
    }

    // Sort by the split channel and split at median
    let mut sorted = indices.to_vec();
    sorted.sort_by_key(|&idx| colors[idx][split_channel]);

    let mid = sorted.len() / 2;
    let a = sorted[..mid].to_vec();
    let b = sorted[mid..].to_vec();
    (a, b)
}
