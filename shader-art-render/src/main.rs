mod encoder;
mod renderer;
mod shader;

use clap::Parser;
use encoder::Encoder;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Parser)]
#[command(name = "shader-art-render", about = "Headless GPU shader renderer for terminal display")]
struct Cli {
    /// Path to .art shader file
    shader: PathBuf,

    /// Output mode
    #[arg(long, default_value = "ascii")]
    mode: String,

    /// Render width in pixels
    #[arg(long, default_value = "138")]
    width: u32,

    /// Render height in pixels
    #[arg(long, default_value = "32")]
    height: u32,

    /// Target FPS
    #[arg(long, default_value = "15")]
    fps: u32,

    /// Screen row for cursor positioning (kitty/sixel modes)
    #[arg(long, default_value = "1")]
    row: u32,

    /// Screen col for cursor positioning (kitty/sixel modes)
    #[arg(long, default_value = "1")]
    col: u32,

    /// Character grid columns for placeholder grid (kitty mode)
    #[arg(long, default_value = "69")]
    cols: u32,

    /// Character grid rows for placeholder grid (kitty mode)
    #[arg(long, default_value = "16")]
    rows: u32,

    /// TTY device for direct write (kitty/sixel modes)
    #[arg(long, default_value = "/dev/tty")]
    tty: PathBuf,
}

/// Query terminal size via ioctl. Returns (cols, rows) or None.
fn term_size() -> Option<(u16, u16)> {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) == 0
            && ws.ws_col > 0
            && ws.ws_row > 0
        {
            Some((ws.ws_col, ws.ws_row))
        } else {
            None
        }
    }
}

fn main() {
    let cli = Cli::parse();

    // Translate shader
    let wgsl = match shader::translate_shader(&cli.shader) {
        Ok(wgsl) => wgsl,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    // Create renderer
    let mut renderer = renderer::Renderer::new(cli.width, cli.height, &wgsl);
    let mut current_width = cli.width;
    let mut current_height = cli.height;

    // Create encoder
    let mut enc: Box<dyn Encoder> = match cli.mode.as_str() {
        "kitty" => Box::new(encoder::kitty::KittyEncoder::new(
            cli.row,
            cli.col,
            cli.cols,
            cli.rows,
        )),
        "sixel" => Box::new(encoder::sixel::SixelEncoder {
            row: cli.row,
            col: cli.col,
        }),
        _ => Box::new(encoder::ascii::AsciiEncoder::new()),
    };

    // Signal handling
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    signal_hook::flag::register(signal_hook::consts::SIGTERM, r.clone()).ok();
    signal_hook::flag::register(signal_hook::consts::SIGINT, r.clone()).ok();
    signal_hook::flag::register(signal_hook::consts::SIGHUP, r.clone()).ok();

    // SIGWINCH resize handling
    let resized = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGWINCH, resized.clone()).ok();

    // Open output
    // Kitty mode: frames go to stdout (Lua plugin wraps in APC and writes to tty)
    // Sixel mode: writes directly to tty
    // ASCII mode: writes to stdout (terminal element)
    let is_sixel = cli.mode == "sixel";
    let mut tty_file: Option<std::fs::File> = if is_sixel {
        std::fs::OpenOptions::new()
            .write(true)
            .open(&cli.tty)
            .ok()
    } else {
        None
    };

    let frame_duration = Duration::from_secs_f64(1.0 / cli.fps as f64);
    let start = Instant::now();
    let mut frame_count = 0u32;

    while running.load(Ordering::Relaxed) {
        // Handle SIGWINCH: resize renderer to match new terminal dimensions
        if resized.swap(false, Ordering::Relaxed) && cli.mode == "ascii" {
            if let Some((cols, rows)) = term_size() {
                let new_width = (cols as u32) * 2;
                let new_height = (rows as u32) * 2;
                if new_width != current_width || new_height != current_height {
                    renderer.resize(new_width, new_height);
                    current_width = new_width;
                    current_height = new_height;
                }
            }
        }

        let w = current_width;
        let h = current_height;

        let frame_start = Instant::now();
        let elapsed = start.elapsed().as_secs_f32();

        let uniforms = renderer::Uniforms {
            resolution: [w as f32, h as f32, 1.0],
            time: elapsed,
            mouse: [0.0; 4],
            frame: frame_count as i32,
            _pad: [0; 3],
        };

        let pixels = renderer.render_frame(&uniforms);
        let encoded = enc.encode(&pixels, w, h);

        let write_result = if let Some(ref mut tty) = tty_file {
            tty.write_all(&encoded).and_then(|_| tty.flush())
        } else {
            let stdout = std::io::stdout();
            let mut handle = stdout.lock();
            handle.write_all(&encoded).and_then(|_| handle.flush())
        };

        if write_result.is_err() {
            break; // broken pipe — exit cleanly
        }

        frame_count = frame_count.wrapping_add(1);

        // Sleep to maintain target FPS
        let elapsed_frame = frame_start.elapsed();
        if elapsed_frame < frame_duration {
            std::thread::sleep(frame_duration - elapsed_frame);
        }
    }

    // Kitty/ASCII cleanup is handled by the Lua plugin
}
