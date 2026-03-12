pub mod ascii;
pub mod kitty;
pub mod sixel;

/// Trait for frame encoders that produce terminal output from RGBA pixels.
pub trait Encoder {
    /// Encode an RGBA pixel buffer into terminal output bytes.
    /// `width` and `height` are in pixels.
    fn encode(&mut self, pixels: &[u8], width: u32, height: u32) -> Vec<u8>;
}
