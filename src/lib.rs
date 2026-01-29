use anyhow::Result;
use grafton_ndi::{
    Error, Finder, FinderOptions, LineStrideOrSize, PixelFormat, Receiver, ReceiverColorFormat,
    ReceiverOptions, NDI,
};
use std::env;
use std::time::Duration;

// ここに探したいNDIソース名を入れてください
// ※ 空文字 "" にすると、最初に見つかったソースに接続します
pub const TARGET_SOURCE_NAME: &str = "";

/// バリデーション済みフレームデータ
pub struct ValidatedFrame<'a> {
    pub width: i32,
    pub height: i32,
    pub data: &'a [u8],
    pub timecode: i64,
}

/// NDI受信機の初期化と接続を管理
pub struct NdiReceiver {
    receiver: Receiver,
}

impl NdiReceiver {
    /// NDIを初期化し、ソースを探索して接続する
    pub fn connect() -> Result<Self> {
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
        let ndi = NDI::new()?;
        println!("NDI initialized successfully\n");

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

        let finder = Finder::new(&ndi, &builder.build())?;

        println!("Looking for sources ...");
        let sources = loop {
            finder.wait_for_sources(Duration::from_secs(1))?;
            let sources = finder.sources(Duration::ZERO)?;
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
                    anyhow::bail!("No NDI source named \"{TARGET_SOURCE_NAME}\" available");
                }
            }
        };

        println!("\nCreating receiver for: {}", source);
        let recv_opts = ReceiverOptions::builder(source.clone())
            .color(ReceiverColorFormat::RGBX_RGBA)
            .build();

        let receiver = Receiver::new(&ndi, &recv_opts)?;
        println!("Receiver created. Waiting for video frame...\n");

        Ok(Self { receiver })
    }

    /// フレーム受信ループ。バリデーション済みのRGBAフレームをコールバックに渡す
    pub fn run_loop<F>(&self, mut callback: F) -> Result<()>
    where
        F: FnMut(ValidatedFrame),
    {
        loop {
            // Use capture_video_ref for zero-copy
            let video_frame_ref_opt = match self.receiver.capture_video_ref(Duration::from_secs(2)) {
                Ok(frame_opt) => frame_opt,
                Err(e) if matches!(e, Error::Timeout { .. }) => {
                    // No frame received in time, keep waiting
                    continue;
                }
                Err(e) => {
                    anyhow::bail!("Receiver error: {e}");
                }
            };

            // VideoFrameRefがNoneの場合はスキップ
            let video_frame_ref = match video_frame_ref_opt {
                Some(frame) => frame,
                None => continue,
            };

            let width = video_frame_ref.width();
            let height = video_frame_ref.height();
            let fourcc = video_frame_ref.pixel_format();
            let timecode = video_frame_ref.timecode();
            let line_stride = match video_frame_ref.line_stride_or_size() {
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
                    actual_stride = line_stride
                );
                continue;
            }

            // Validate length for uncompressed
            let expected_uncompressed_size = (width * height * 4) as usize;
            if video_frame_ref.data().len() < expected_uncompressed_size / 2 {
                eprintln!(
                    "Warning: Compressed video frame, data too small: {} bytes (expected {})",
                    video_frame_ref.data().len(),
                    expected_uncompressed_size
                );
                continue;
            }

            // Clone data only once for the callback (zero-copy until this point)
            let data = video_frame_ref.data();

            // Call callback with validated frame
            callback(ValidatedFrame {
                width,
                height,
                data,
                timecode,
            });
        }
    }
}

/// eframeウィンドウ作成の共通オプション（1920x1080、非フルスクリーン）
pub fn create_native_options() -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_fullscreen(false)
            .with_inner_size([1920.0, 1080.0]),
        ..Default::default()
    }
}
