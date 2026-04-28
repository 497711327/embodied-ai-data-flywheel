//! `list-cameras` — probe /dev/video0..N and print capabilities.
//!
//! Usage: list-cameras [--max-index 8]

use clap::Parser;
use tracing::warn;
use v4l::{video::Capture, Device};

#[derive(Parser)]
#[command(about = "List available V4L2 camera devices")]
struct Args {
    /// Highest /dev/videoN index to probe.
    #[arg(long, default_value_t = 8)]
    max_index: usize,
}

fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .init();

    let args = Args::parse();

    println!("{:<8} {:<20} {:<20} {}", "Index", "Device", "Formats", "Resolution");
    println!("{}", "-".repeat(72));

    let mut found = 0;

    for idx in 0..=args.max_index {
        let path = format!("/dev/video{}", idx);
        if !std::path::Path::new(&path).exists() {
            continue;
        }

        match Device::new(idx) {
            Ok(dev) => {
                let caps = match dev.query_caps() {
                    Ok(c) => c,
                    Err(e) => {
                        warn!("Cannot query caps for /dev/video{}: {}", idx, e);
                        continue;
                    }
                };

                // Only list capture devices (not metadata/codec nodes).
                if !caps.capabilities.contains(v4l::capability::Flags::VIDEO_CAPTURE) {
                    continue;
                }

                // Collect supported formats.
                let formats: Vec<String> = dev
                    .enum_formats()
                    .unwrap_or_default()
                    .iter()
                    .map(|f| f.fourcc.str().unwrap_or("????").trim().to_string())
                    .collect();

                // Try to get current resolution.
                let res = dev
                    .format()
                    .map(|f| format!("{}×{}", f.width, f.height))
                    .unwrap_or_else(|_| "?×?".to_string());

                println!(
                    "{:<8} {:<20} {:<20} {}",
                    idx,
                    path,
                    formats.join(" "),
                    res
                );
                found += 1;
            }
            Err(_) => {}
        }
    }

    if found == 0 {
        println!("No V4L2 capture devices found (0..{}).", args.max_index);
        println!();
        println!("On WSL: attach a camera with  usbipd bind --busid <ID>  and  usbipd attach --wsl --busid <ID>");
    } else {
        println!("\n{} device(s) found.", found);
    }
}
