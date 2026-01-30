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
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let frame_buffer = Arc::new(ArcSwap::from_pointee(None));
        let frame_buffer_clone = frame_buffer.clone();

        // egui::Contextをクローンしてスレッドで使用
        let ctx = cc.egui_ctx.clone();

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
                // CPUでガウシアンブラー処理（5x5カーネル）
                let width = frame.width as usize;
                let height = frame.height as usize;
                let mut blurred_data = vec![0u8; frame.data.len()];

                // ガウシアンカーネル 5x5 (正規化済み)
                // 1   4   6   4   1
                // 4  16  24  16   4
                // 6  24  36  24   6
                // 4  16  24  16   4
                // 1   4   6   4   1
                // 合計 = 256
                let kernel = [
                    1, 4, 6, 4, 1,
                    4, 16, 24, 16, 4,
                    6, 24, 36, 24, 6,
                    4, 16, 24, 16, 4,
                    1, 4, 6, 4, 1,
                ];
                let kernel_sum = 256;

                for y in 0..height {
                    for x in 0..width {
                        let mut r_sum = 0u32;
                        let mut g_sum = 0u32;
                        let mut b_sum = 0u32;
                        let mut a_sum = 0u32;

                        // 5x5カーネルを適用
                        for ky in 0..5 {
                            for kx in 0..5 {
                                // 境界処理: クランプ
                                let py = (y as i32 + ky - 2).clamp(0, height as i32 - 1) as usize;
                                let px = (x as i32 + kx - 2).clamp(0, width as i32 - 1) as usize;
                                let idx = (py * width + px) * 4;

                                let weight = kernel[ky as usize * 5 + kx as usize];
                                r_sum += frame.data[idx] as u32 * weight;
                                g_sum += frame.data[idx + 1] as u32 * weight;
                                b_sum += frame.data[idx + 2] as u32 * weight;
                                a_sum += frame.data[idx + 3] as u32 * weight;
                            }
                        }

                        let out_idx = (y * width + x) * 4;
                        blurred_data[out_idx] = (r_sum / kernel_sum) as u8;
                        blurred_data[out_idx + 1] = (g_sum / kernel_sum) as u8;
                        blurred_data[out_idx + 2] = (b_sum / kernel_sum) as u8;
                        blurred_data[out_idx + 3] = (a_sum / kernel_sum) as u8;
                    }
                }

                // Convert to egui::ColorImage
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [frame.width as usize, frame.height as usize],
                    &blurred_data,
                );

                // Store using ArcSwap (lock-free)
                frame_buffer_clone.store(Arc::new(Some(image)));

                // これをしないとマウスカーソルを動かさないと再描画されない
                ctx.request_repaint();

                println!(
                    "Frame received (blur CPU): {}x{}, timecode={}",
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

                    // Note: NDIスレッドがrequest_repaintを呼ぶため、
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

fn main() -> Result<()> {
    let options = create_native_options();

    eframe::run_native(
        "NDI Blur Viewer (CPU)",
        options,
        Box::new(|cc| Ok(Box::new(NdiApp::new(cc)))),
    )
    .map_err(|e| anyhow::anyhow!("GUI error: {}", e))
}
