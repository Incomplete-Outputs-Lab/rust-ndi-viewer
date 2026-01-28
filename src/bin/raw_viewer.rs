use anyhow::Result;
use arc_swap::ArcSwap;
use eframe::egui;
use rust_ndi_viewer::{create_native_options, NdiReceiver, TARGET_SOURCE_NAME};
use std::sync::Arc;
use std::thread;

struct NdiApp {
    // スレッド間で共有する画像バッファ（ArcSwapでロックフリー）
    // NDIスレッドが書き込み、GUIスレッドが読み込む
    frame_buffer: Arc<ArcSwap<Option<egui::ColorImage>>>,

    // egui用のテクスチャハンドル
    texture: Option<egui::TextureHandle>,
}

impl NdiApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let frame_buffer = Arc::new(ArcSwap::from_pointee(None));
        let frame_buffer_clone = frame_buffer.clone();

        // NDI receiver thread - finds a source and pushes the latest frame to the shared buffer
        thread::spawn(move || {
            let receiver = match NdiReceiver::connect() {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("Failed to connect to NDI: {e}");
                    return;
                }
            };

            let _ = receiver.run_loop(|frame| {
                // Convert NDI frame into egui::ColorImage
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [frame.width as usize, frame.height as usize],
                    &frame.data,
                );

                // Store it using ArcSwap (lock-free)
                frame_buffer_clone.store(Arc::new(Some(image)));

                println!(
                    "Frame received: {}x{}, timecode={}",
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
        // 背景を黒にする
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

                    // 新しいフレームが来たときだけ再描画をリクエスト（パフォーマンス向上）
                    if has_new_frame {
                        ctx.request_repaint();
                    }
                } else {
                    // まだ映像が来ていない時の表示
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
        "NDI Raw Viewer",
        options,
        Box::new(|cc| Ok(Box::new(NdiApp::new(cc)))),
    )
    .map_err(|e| anyhow::anyhow!("GUI error: {}", e))
}
