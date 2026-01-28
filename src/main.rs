use anyhow::Result;
use eframe::egui;
use grafton_ndi::{Finder, FinderOptions, NDI, Receiver, ReceiverColorFormat, ReceiverOptions};
use std::sync::{Arc, Mutex};
use std::thread;

// ここに探したいNDIソース名を入れてください
// ※ 空文字 "" にすると、最初に見つかったソースに接続します
const TARGET_SOURCE_NAME: &str = "";

struct NdiApp {
    // スレッド間で共有する画像バッファ
    // NDIスレッドが書き込み、GUIスレッドが読み込む
    frame_buffer: Arc<Mutex<Option<egui::ColorImage>>>,
    
    // egui用のテクスチャハンドル
    texture: Option<egui::TextureHandle>,
}

impl NdiApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let frame_buffer = Arc::new(Mutex::new(None));
        let frame_buffer_clone = frame_buffer.clone();

        // NDI receiver thread - finds a source and pushes the latest frame to the shared buffer
        thread::spawn(move || {
            use grafton_ndi::{
                Error, Finder, FinderOptions, PixelFormat, Receiver, ReceiverColorFormat,
                ReceiverOptions, NDI, LineStrideOrSize,
            };
            use std::{
                env,
                time::{Duration, Instant},
            };

            // Parse command line: allow picking extra discovery IPs if provided
            let args: Vec<String> = env::args().collect();
            let mut extra_ips = Vec::new();
            let mut i = 1;
            while i < args.len() {
                if !args[i].starts_with("--") {
                    extra_ips.push(args[i].as_str());
                }
                i += 1;
            }

            println!("NDI Video Receiver - GUI Frame Injector Example");
            println!("==============================================\n");

            // Initialize NDI
            let ndi = match NDI::new() {
                Ok(n) => {
                    println!("NDI initialized successfully\n");
                    n
                }
                Err(e) => {
                    eprintln!("Failed to initialize NDI: {e}");
                    return;
                }
            };

            // Discover sources
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
                    eprintln!("Failed to create NDI Finder: {e}");
                    return;
                }
            };

            println!("Looking for sources ...");
            let sources = loop {
                if let Err(e) = finder.wait_for_sources(Duration::from_secs(1)) {
                    eprintln!("Source wait error: {e}");
                    return;
                }
                let sources = match finder.sources(Duration::ZERO) {
                    Ok(list) => list,
                    Err(e) => {
                        eprintln!("Error getting sources: {e}");
                        return;
                    }
                };
                if !sources.is_empty() {
                    let count = sources.len();
                    println!("Found {count} source(s):");
                    for (i, source) in sources.iter().enumerate() {
                        let num = i + 1;
                        println!("  {num}. {source}");
                    }
                    break sources;
                }
            };

            // Pick source according to const (empty = first available)
            let source = if TARGET_SOURCE_NAME.is_empty() {
                &sources[0]
            } else {
                match sources.iter().find(|s| s.name == TARGET_SOURCE_NAME) {
                    Some(src) => src,
                    None => {
                        eprintln!("No NDI source named \"{TARGET_SOURCE_NAME}\" available");
                        return;
                    }
                }
            };

            println!("\nCreating receiver for: {}", source);
            let recv_opts = ReceiverOptions::builder(source.clone())
                .color(ReceiverColorFormat::RGBX_RGBA)
                .build();

            let receiver = match Receiver::new(&ndi, &recv_opts) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("Failed to create receiver: {e}");
                    return;
                }
            };

            println!("Receiver created. Waiting for video frame...\n");

            // Receive frames in a loop, push latest to frame_buffer
            loop {
                let start_time = Instant::now();
                let video_frame = match receiver.capture_video(Duration::from_secs(2)) {
                    Ok(frame) => frame,
                    Err(e) if matches!(e, Error::Timeout { .. }) => {
                        // No frame received in time, keep waiting
                        continue;
                    }
                    Err(e) => {
                        eprintln!("Receiver error: {e}");
                        break;
                    }
                };

                let width = video_frame.width;
                let height = video_frame.height;
                let fourcc = video_frame.pixel_format;
                let line_stride = match video_frame.line_stride_or_size {
                    LineStrideOrSize::LineStrideBytes(stride) => stride,
                    LineStrideOrSize::DataSizeBytes(_) => {
                        eprintln!("Unexpected data size instead of stride -- skipping frame.");
                        continue;
                    }
                };

                // Only accept RGBA, RGBX
                match fourcc {
                    PixelFormat::RGBA | PixelFormat::RGBX => {}
                    other => {
                        eprintln!("Warning: Got unexpected format {other:?}, skipping frame.");
                        continue;
                    }
                }

                let expected_stride = width * 4;
                if line_stride != expected_stride {
                    eprintln!(
                        "Line stride ({actual_stride}) doesn't match width*4 ({expected_stride}); skipping frame.",
                        actual_stride=line_stride
                    );
                    continue;
                }

                // Validate length for uncompressed
                let expected_uncompressed_size = (width * height * 4) as usize;
                if video_frame.data.len() < expected_uncompressed_size / 2 {
                    eprintln!(
                        "Warning: Compressed video frame, data too small: {} bytes (expected {})",
                        video_frame.data.len(),
                        expected_uncompressed_size
                    );
                    continue;
                }

                // Convert NDI frame into egui::ColorImage
                // SAFETY: video_frame.data is RGBA (or RGBX, which is sufficiently RGBA for display)
                // We copy as u8s to egui::ColorImage (which expects sRGBA)
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [width as usize, height as usize],
                    &video_frame.data,
                );

                {
                    // Swap it into the Arc<Mutex<...>>, the GUI thread will take() and display
                    let mut buf = frame_buffer_clone.lock().unwrap();
                    *buf = Some(image);
                }
                // (Optional) Print stats
                let t = start_time.elapsed();
                println!(
                    "Frame received: {width}x{height}, {fourcc:?}, stride={line_stride} bytes, delay={:?}",
                    t
                );
            }
        });

        Self {
            frame_buffer,
            texture: None,
        }
    }
}

impl eframe::App for NdiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 背景を黒にする
        let panel_frame = egui::Frame::central_panel(&ctx.style()).fill(egui::Color32::BLACK);

        egui::CentralPanel::default().frame(panel_frame).show(ctx, |ui| {
            // 最新フレームがあるかチェック
            let new_image = {
                let mut buffer = self.frame_buffer.lock().unwrap();
                buffer.take() // バッファからmoveして取得（OptionをNoneに戻す）
            };

            // 新しい画像が来ていればテクスチャを更新
            if let Some(image) = new_image {
                self.texture = Some(ctx.load_texture(
                    "ndi-frame",
                    image,
                    egui::TextureOptions::LINEAR, // 拡大縮小時のフィルタ
                ));
            }

            // テクスチャがあれば描画
            if let Some(texture) = &self.texture {
                // アスペクト比を維持しつつ画面最大に表示
                // 利用可能なサイズを取得
                let size = ui.available_size();
                
                // 画像ウィジェットを表示
                // shrink_to_fit() でアスペクト比維持
                ui.centered_and_justified(|ui| {
                    ui.image((texture.id(), size));
                });
            } else {
                // まだ映像が来ていない時の表示
                ui.centered_and_justified(|ui| {
                    ui.label(
                        egui::RichText::new(format!("Waiting for NDI Source: {}...", TARGET_SOURCE_NAME))
                            .color(egui::Color32::WHITE)
                            .size(32.0)
                    );
                });
            }
        });
    }
}


fn main() -> Result<()> {
    // フルスクリーン設定
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_fullscreen(false) // 最大解像度（フルスクリーン）で起動
            .with_inner_size([1920.0, 1080.0]),
        ..Default::default()
    };

    eframe::run_native(
        "NDI Viewer",
        options,
        Box::new(|cc| Ok(Box::new(NdiApp::new(cc)))),
    )
    .map_err(|e| anyhow::anyhow!("GUI error: {}", e))
}
