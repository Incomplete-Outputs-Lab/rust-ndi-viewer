use anyhow::Result;
use eframe::egui;
use rust_ndi_viewer::{create_native_options, NdiReceiver, TARGET_SOURCE_NAME};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::thread;

// バッファに保持する最大フレーム数（オーバーフロー防止）
const MAX_BUFFER_SIZE: usize = 180;

// 表示遅延フレーム数（この数だけフレームがバッファに溜まってから表示開始）
const BUFFER_DELAY_FRAMES: usize = 60;

struct NdiApp {
    // スレッド間で共有するフレームバッファ（ColorImage + timecode）
    // Note: VecDequeは頻繁にpush/popするため、ArcSwapよりMutexが適切
    frame_buffer: Arc<Mutex<VecDeque<(egui::ColorImage, i64)>>>,

    // egui用のテクスチャハンドル
    texture: Option<egui::TextureHandle>,
}

impl NdiApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let frame_buffer = Arc::new(Mutex::new(VecDeque::new()));
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
                // Convert NDI frame into egui::ColorImage
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [frame.width as usize, frame.height as usize],
                    &frame.data,
                );

                // try_lockでブロッキングを回避（ロックが取れなければフレームをドロップ）
                if let Ok(mut buf) = frame_buffer_clone.try_lock() {
                    buf.push_back((image, frame.timecode));

                    // バッファが上限を超えたら古いフレームを破棄
                    while buf.len() > MAX_BUFFER_SIZE {
                        buf.pop_front();
                        eprintln!("Warning: Frame buffer overflow, dropping oldest frame");
                    }

                    // これをしないとマウスカーソルを動かさないと再描画されない
                    ctx.request_repaint();

                    println!(
                        "Frame received: {}x{}, timecode={}",
                        frame.width, frame.height, frame.timecode
                    );
                } else {
                    // ロックが取れなかった場合はフレームをドロップ（パフォーマンス優先）
                    eprintln!("Frame dropped: lock contention");
                }
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
                // バッファから固定遅延でフレームを取得
                // try_lockでブロッキングを回避してパフォーマンス向上
                let mut display_image = None;
                if let Ok(mut buf) = self.frame_buffer.try_lock() {
                    // バッファに BUFFER_DELAY_FRAMES + 1 以上のフレームが溜まったら表示開始
                    if buf.len() > BUFFER_DELAY_FRAMES {
                        if let Some((image, timecode)) = buf.pop_front() {
                            display_image = Some(image);
                            println!(
                                "Displaying frame: timecode={}, buffer_size={}",
                                timecode,
                                buf.len()
                            );
                        }
                    }
                }
                // ロックが取れなかった場合はスキップ（次のフレームで再試行）

                // 新しい画像が来ていればテクスチャを更新
                if let Some(image) = display_image {
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
        "NDI Buffered Viewer",
        options,
        Box::new(|cc| Ok(Box::new(NdiApp::new(cc)))),
    )
    .map_err(|e| anyhow::anyhow!("GUI error: {}", e))
}
