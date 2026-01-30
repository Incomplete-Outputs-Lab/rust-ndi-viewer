use anyhow::Result;
use arc_swap::ArcSwap;
use eframe::egui;
use rust_ndi_viewer::{create_native_options, NdiReceiver, TARGET_SOURCE_NAME};
use std::sync::Arc;

struct NdiApp {
    // スレッド間で共有する画像バッファ（ArcSwapでロックフリー）
    frame_buffer: Arc<ArcSwap<Option<egui::ColorImage>>>,

    // egui用のテクスチャハンドル
    texture: Option<egui::TextureHandle>,
}

impl NdiApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let frame_buffer = Arc::new(ArcSwap::from_pointee(None));
        let frame_buffer_clone = frame_buffer.clone();

        // egui::Contextをクローンして非同期タスクで使用
        let ctx = cc.egui_ctx.clone();

        // Tokio runtime for NDI receiver
        tokio::spawn(async move {
            let receiver = match NdiReceiver::connect() {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("Failed to connect to NDI: {e}");
                    return;
                }
            };

            // Note: run_loopは同期的なので、spawn_blockingで実行
            tokio::task::spawn_blocking(move || {
                let _ = receiver.run_loop(|frame| {
                    // Convert NDI frame into egui::ColorImage
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [frame.width as usize, frame.height as usize],
                        &frame.data,
                    );

                    // Store using ArcSwap (lock-free)
                    frame_buffer_clone.store(Arc::new(Some(image)));

                    // 受信時に再描画をリクエスト
                    ctx.request_repaint();

                    println!(
                        "Frame received (tokio): {}x{}, timecode={}",
                        frame.width, frame.height, frame.timecode
                    );
                });
            })
            .await
            .ok();
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

                    // Note: tokioタスクがrequest_repaintを呼ぶため、
                    // ここでは明示的に呼ばなくても新しいフレームが来たら自動的に再描画される
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

#[tokio::main]
async fn main() -> Result<()> {
    let options = create_native_options();

    eframe::run_native(
        "NDI Tokio Viewer",
        options,
        Box::new(|cc| Ok(Box::new(NdiApp::new(cc)))),
    )
    .map_err(|e| anyhow::anyhow!("GUI error: {}", e))
}
