use anyhow::Result;
use arc_swap::ArcSwap;
use eframe::egui;
use rust_ndi_viewer::{create_native_options, NdiReceiver, TARGET_SOURCE_NAME};
use std::sync::Arc;
use std::thread;

struct NdiApp {
    // スレッド間で共有する画像バッファ（ArcSwapでロックフリー）
    frame_buffer: Arc<ArcSwap<Option<egui::ColorImage>>>,

    // egui用のテクスチャハンドル
    texture: Option<egui::TextureHandle>,
}

impl NdiApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let frame_buffer = Arc::new(ArcSwap::from_pointee(None));
        let frame_buffer_clone = frame_buffer.clone();

        // NDI receiver thread
        thread::spawn(move || {
            let receiver = match NdiReceiver::connect() {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("Failed to connect to NDI: {e}");
                    return;
                }
            };

            let _ = receiver.run_loop(|frame| {
                // CPUでグレースケール変換（ITU-R BT.601）
                // 整数演算で高速化（固定小数点: 256倍スケール）
                let mut grayscale_data = frame.data.clone();

                for chunk in grayscale_data.chunks_exact_mut(4) {
                    let r = chunk[0] as u32;
                    let g = chunk[1] as u32;
                    let b = chunk[2] as u32;

                    // 輝度計算: Y = 0.299*R + 0.587*G + 0.114*B
                    // 固定小数点: 77*R + 150*G + 29*B >> 8
                    let gray = ((77 * r + 150 * g + 29 * b) >> 8) as u8;

                    // R, G, B を gray 値に置換
                    chunk[0] = gray;
                    chunk[1] = gray;
                    chunk[2] = gray;
                    // Alpha は元のまま（chunk[3]）
                }

                // Convert to egui::ColorImage
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [frame.width as usize, frame.height as usize],
                    &grayscale_data,
                );

                // Store using ArcSwap (lock-free)
                frame_buffer_clone.store(Arc::new(Some(image)));

                println!(
                    "Frame received (grayscale CPU): {}x{}, timecode={}",
                    frame.width, frame.height, frame.timecode
                );
            });
        });

        Self {
            frame_buffer,
            texture: None,
        }
    }
}

impl eframe::App for NdiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let panel_frame = egui::Frame::central_panel(&ctx.style()).fill(egui::Color32::BLACK);

        egui::CentralPanel::default()
            .frame(panel_frame)
            .show(ctx, |ui| {
                // 最新フレームがあるかチェック（ArcSwapでロックフリー読み取り）
                let new_image = self.frame_buffer.swap(Arc::new(None));
                let new_image = Arc::try_unwrap(new_image).unwrap_or_else(|arc| (*arc).clone());

                // 新しい画像が来ていればテクスチャを更新
                let has_new_frame = new_image.is_some();
                if let Some(image) = new_image {
                    self.texture = Some(ctx.load_texture(
                        "ndi-frame",
                        image,
                        egui::TextureOptions::LINEAR,
                    ));
                }

                // テクスチャがあれば描画
                if let Some(texture) = &self.texture {
                    let size = ui.available_size();
                    ui.centered_and_justified(|ui| {
                        ui.image((texture.id(), size));
                    });

                    // 新しいフレームが来たときだけ再描画をリクエスト
                    if has_new_frame {
                        ctx.request_repaint();
                    }
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
        "NDI Grayscale Viewer (CPU)",
        options,
        Box::new(|cc| Ok(Box::new(NdiApp::new(cc)))),
    )
    .map_err(|e| anyhow::anyhow!("GUI error: {}", e))
}
