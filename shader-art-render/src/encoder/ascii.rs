use super::Encoder;

/// ASCII encoder using half-block characters with truecolor ANSI.
/// Each character cell represents 2 vertical pixels:
/// - Top pixel → foreground color
/// - Bottom pixel → background color
/// Uses U+2580 UPPER HALF BLOCK (▀)
///
/// Optimizations to keep frame size within pty buffer limits (~16KB on macOS):
/// - Skip SGR when both fg and bg match previous cell
/// - Only emit changed color component (fg or bg) when one matches
/// - Use `\x1b[0m` reset when transitioning to default/black
pub struct AsciiEncoder {
    prev_fg: Option<(u8, u8, u8)>,
    prev_bg: Option<(u8, u8, u8)>,
}

impl AsciiEncoder {
    pub fn new() -> Self {
        Self {
            prev_fg: None,
            prev_bg: None,
        }
    }
}

impl Encoder for AsciiEncoder {
    fn encode(&mut self, pixels: &[u8], width: u32, height: u32) -> Vec<u8> {
        let mut output = Vec::with_capacity(16_384);

        // No cursor home — Lua handles positioning when writing to tty.

        let row_bytes = width as usize * 4;

        // Reset color tracking at start of each frame
        self.prev_fg = None;
        self.prev_bg = None;

        let mut y = 0u32;
        while y < height {
            let top_row_start = y as usize * row_bytes;
            let bot_row_start = if y + 1 < height {
                (y + 1) as usize * row_bytes
            } else {
                top_row_start
            };

            for x in 0..width as usize {
                let ti = top_row_start + x * 4;
                let bi = bot_row_start + x * 4;

                let fg = (pixels[ti], pixels[ti + 1], pixels[ti + 2]);
                let bg = (pixels[bi], pixels[bi + 1], pixels[bi + 2]);

                let fg_same = self.prev_fg == Some(fg);
                let bg_same = self.prev_bg == Some(bg);

                if !fg_same && !bg_same {
                    // Both changed — emit combined SGR
                    write_sgr_both(&mut output, fg, bg);
                } else if !fg_same {
                    // Only fg changed
                    write_sgr_fg(&mut output, fg);
                } else if !bg_same {
                    // Only bg changed
                    write_sgr_bg(&mut output, bg);
                }
                // else: both same — skip SGR entirely

                output.extend_from_slice("\u{2580}".as_bytes());

                self.prev_fg = Some(fg);
                self.prev_bg = Some(bg);
            }

            // Reset + newline: each row is one on_stdout line.
            // Lua handles cursor positioning per row.
            output.extend_from_slice(b"\x1b[0m\n");
            self.prev_fg = None;
            self.prev_bg = None;
            y += 2;
        }

        output
    }
}

#[inline]
fn write_sgr_both(out: &mut Vec<u8>, fg: (u8, u8, u8), bg: (u8, u8, u8)) {
    // Use itoa for fast integer formatting
    out.extend_from_slice(b"\x1b[38;2;");
    write_u8_trio(out, fg.0, fg.1, fg.2);
    out.extend_from_slice(b";48;2;");
    write_u8_trio(out, bg.0, bg.1, bg.2);
    out.push(b'm');
}

#[inline]
fn write_sgr_fg(out: &mut Vec<u8>, fg: (u8, u8, u8)) {
    out.extend_from_slice(b"\x1b[38;2;");
    write_u8_trio(out, fg.0, fg.1, fg.2);
    out.push(b'm');
}

#[inline]
fn write_sgr_bg(out: &mut Vec<u8>, bg: (u8, u8, u8)) {
    out.extend_from_slice(b"\x1b[48;2;");
    write_u8_trio(out, bg.0, bg.1, bg.2);
    out.push(b'm');
}

/// Write three u8 values separated by semicolons, without using format!().
#[inline]
fn write_u8_trio(out: &mut Vec<u8>, a: u8, b: u8, c: u8) {
    write_u8(out, a);
    out.push(b';');
    write_u8(out, b);
    out.push(b';');
    write_u8(out, c);
}

/// Fast u8 to ASCII without allocation.
#[inline]
fn write_u8(out: &mut Vec<u8>, v: u8) {
    if v >= 100 {
        out.push(b'0' + v / 100);
        out.push(b'0' + (v / 10) % 10);
        out.push(b'0' + v % 10);
    } else if v >= 10 {
        out.push(b'0' + v / 10);
        out.push(b'0' + v % 10);
    } else {
        out.push(b'0' + v);
    }
}
