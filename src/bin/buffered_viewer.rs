use anyhow::Result;
use eframe::egui;
use rust_ndi_viewer::{create_native_options, NdiReceiver, TARGET_SOURCE_NAME};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

const MAX_BUFFER_SIZE: usize = 30;

struct NdiApp {
    // スレッド間で共有するフレームバッファ（ColorImage + timecode）
    // Note: VecDequeは頻繁にpush/popするため、ArcSwapよりMutexが適切
    frame_buffer: Arc<Mutex<VecDeque<(egui::ColorImage, i64)>>>,

    // ベースライン timecode と Instant（初回フレーム受信時に記録）
    base_timecode: Option<i64>,
    base_instant: Option<Instant>,

    // egui用のテクスチャハンドル
    texture: Option<egui::TextureHandle>,
}

impl NdiApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let frame_buffer = Arc::new(Mutex::new(VecDeque::new()));
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
            base_timecode: None,
            base_instant: None,
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
                let now = Instant::now();

                // バッファから表示タイミングに達したフレームを取得
                // try_lockでブロッキングを回避してパフォーマンス向上
                let mut display_image = None;
                if let Ok(mut buf) = self.frame_buffer.try_lock() {
                    // ベースラインの初期化（初回フレーム受信時）
                    if self.base_timecode.is_none() && !buf.is_empty() {
                        let (_, first_timecode) = &buf[0];
                        self.base_timecode = Some(*first_timecode);
                        self.base_instant = Some(now);
                        println!("Baseline set: timecode={}, instant={:?}", first_timecode, now);
                    }

                    // ベースラインが設定されている場合、表示タイミングを計算
                    if let (Some(base_tc), Some(base_inst)) =
                        (self.base_timecode, self.base_instant)
                    {
                        // バッファの先頭から順にチェック
                        while let Some((_img, tc)) = buf.front() {
                            // このフレームの表示タイミングを計算（100ns単位をnsに変換）
                            let frame_offset_ns = (tc - base_tc) * 100; // 100ns -> ns
                            let display_time = base_inst
                                + std::time::Duration::from_nanos(frame_offset_ns as u64);

                            if now >= display_time {
                                // 表示タイミングに達したのでフレームを取り出す
                                if let Some((image, timecode)) = buf.pop_front() {
                                    display_image = Some(image);
                                    println!("Displaying frame: timecode={}, delay={:?}", timecode, now - display_time);
                                }
                            } else {
                                // まだ表示タイミングに達していないので待つ
                                break;
                            }
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

                    ctx.request_repaint();
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

                // 常に再描画をリクエスト（タイミングチェックのため）
                ctx.request_repaint();
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
