//! NDI FrameSync Viewer — clock-corrected video capture with egui display.
//!
//! Uses the FrameSync API for time-base corrected capture. Ideal for smooth
//! playback synced to display refresh (e.g. GPU v-sync). FrameSync captures
//! return immediately and handle clock drift between sender and receiver.

use anyhow::Result;
use arc_swap::ArcSwap;
use eframe::egui;
use grafton_ndi::{
    Finder, FinderOptions, FrameSync, LineStrideOrSize, PixelFormat, Receiver,
    ReceiverColorFormat, ReceiverOptions, ScanType, NDI,
};
use rust_ndi_viewer::{create_native_options, TARGET_SOURCE_NAME};
use std::env;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

const CAPTURE_INTERVAL_MS: u64 = 33; // ~30 fps display rate

struct NdiApp {
    frame_buffer: Arc<ArcSwap<Option<egui::ColorImage>>>,
    texture: Option<egui::TextureHandle>,
}

impl NdiApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let frame_buffer = Arc::new(ArcSwap::from_pointee(None));
        let frame_buffer_clone = frame_buffer.clone();
        let ctx = cc.egui_ctx.clone();

        thread::spawn(move || {
            let args: Vec<String> = env::args().collect();
            let mut extra_ips = Vec::new();
            let mut i = 1;
            while i < args.len() {
                if !args[i].starts_with("--") {
                    extra_ips.push(args[i].as_str());
                }
                i += 1;
            }

            println!("NDI FrameSync Viewer");
            println!("====================\n");

            let ndi = match NDI::new() {
                Ok(n) => n,
                Err(e) => {
                    eprintln!("NDI init failed: {e}");
                    return;
                }
            };

            let mut builder = FinderOptions::builder().show_local_sources(true);
            if !extra_ips.is_empty() {
                println!("Searching additional IPs/subnets:");
                for ip in &extra_ips {
                    println!("  - {ip}");
                    builder = builder.extra_ips(*ip);
                }
                println!();
            }

            let finder = match Finder::new(&ndi, &builder.build()) {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("Finder failed: {e}");
                    return;
                }
            };

            println!("Looking for sources ...");
            let sources = loop {
                if let Err(e) = finder.wait_for_sources(Duration::from_secs(1)) {
                    eprintln!("wait_for_sources: {e}");
                    return;
                }
                match finder.sources(Duration::ZERO) {
                    Ok(s) if !s.is_empty() => {
                        println!("Found {} source(s):", s.len());
                        for (i, source) in s.iter().enumerate() {
                            println!("  {}. {source}", i + 1);
                        }
                        break s;
                    }
                    Ok(_) => {}
                    Err(e) => {
                        eprintln!("sources: {e}");
                        return;
                    }
                }
            };

            let source = if TARGET_SOURCE_NAME.is_empty() {
                &sources[0]
            } else {
                match sources.iter().find(|s| s.name == TARGET_SOURCE_NAME) {
                    Some(s) => s,
                    None => {
                        eprintln!("No NDI source named \"{TARGET_SOURCE_NAME}\" available");
                        return;
                    }
                }
            };

            println!("\nCreating receiver for: {source}");
            let recv_opts = ReceiverOptions::builder(source.clone())
                .color(ReceiverColorFormat::RGBX_RGBA)
                .build();
            let receiver = match Receiver::new(&ndi, &recv_opts) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("Receiver failed: {e}");
                    return;
                }
            };

            println!("Creating FrameSync for clock-corrected capture...");
            let framesync = match FrameSync::new(&receiver) {
                Ok(fs) => fs,
                Err(e) => {
                    eprintln!("FrameSync failed: {e}");
                    return;
                }
            };
            println!("FrameSync created. Starting capture loop...\n");

            loop {
                if let Some(video) = framesync.capture_video(ScanType::Progressive) {
                    if let Some(image) = validate_and_convert(&video) {
                        frame_buffer_clone.store(Arc::new(Some(image)));
                        ctx.request_repaint();
                    }
                }
                thread::sleep(Duration::from_millis(CAPTURE_INTERVAL_MS));
            }
        });

        Self {
            frame_buffer,
            texture: None,
        }
    }
}

fn validate_and_convert(
    video: &grafton_ndi::FrameSyncVideoRef<'_>,
) -> Option<egui::ColorImage> {
    match video.pixel_format() {
        PixelFormat::RGBA | PixelFormat::RGBX => {}
        _ => return None,
    }

    let line_stride = match video.line_stride_or_size() {
        LineStrideOrSize::LineStrideBytes(s) => s,
        LineStrideOrSize::DataSizeBytes(_) => return None,
    };
    let width = video.width();
    let height = video.height();
    if line_stride != width * 4 {
        return None;
    }

    let expected_size = (width * height * 4) as usize;
    let data = video.data();
    if data.len() < expected_size {
        return None; // truncated or compressed
    }

    let image = egui::ColorImage::from_rgba_unmultiplied(
        [width as usize, height as usize],
        &data[..expected_size],
    );
    Some(image)
}

impl eframe::App for NdiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let panel_frame = egui::Frame::central_panel(&ctx.style()).fill(egui::Color32::BLACK);

        egui::CentralPanel::default()
            .frame(panel_frame)
            .show(ctx, |ui| {
                let new_image = self.frame_buffer.swap(Arc::new(None));
                let new_image = Arc::try_unwrap(new_image).unwrap_or_else(|arc| (*arc).clone());

                if let Some(image) = new_image {
                    self.texture = Some(ctx.load_texture(
                        "ndi-frame",
                        image,
                        egui::TextureOptions::LINEAR,
                    ));
                }

                if let Some(texture) = &self.texture {
                    let size = ui.available_size();
                    ui.centered_and_justified(|ui| {
                        ui.image((texture.id(), size));
                    });
                } else {
                    ui.centered_and_justified(|ui| {
                        ui.label(
                            egui::RichText::new(format!(
                                "Waiting for NDI Source: {}...",
                                TARGET_SOURCE_NAME
                            ))
                            .color(egui::Color32::WHITE)
                            .size(32.0),
                        );
                    });
                }
            });
    }
}

fn main() -> Result<()> {
    let options = create_native_options();

    eframe::run_native(
        "NDI FrameSync Viewer",
        options,
        Box::new(|cc| Ok(Box::new(NdiApp::new(cc)))),
    )
    .map_err(|e| anyhow::anyhow!("GUI error: {}", e))
}
