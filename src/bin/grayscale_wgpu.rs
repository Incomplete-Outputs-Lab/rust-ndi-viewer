use anyhow::Result;
use arc_swap::ArcSwap;
use eframe::egui;
use rust_ndi_viewer::{create_native_options, NdiReceiver, TARGET_SOURCE_NAME};
use std::sync::Arc;
use std::thread;
use wgpu::util::DeviceExt;

// コンピュートシェーダーのワークグループサイズ
const WORKGROUP_SIZE: u32 = 256;

// 生のRGBAフレームデータ（wgpuで処理する前）
#[derive(Clone)]
struct RawFrame {
    width: i32,
    height: i32,
    data: Vec<u8>,
}

struct NdiApp {
    // スレッド間で共有する生フレームバッファ（ArcSwapでロックフリー）
    raw_frame_buffer: Arc<ArcSwap<Option<RawFrame>>>,

    // wgpuリソース
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,

    // egui用のテクスチャハンドル
    texture: Option<egui::TextureHandle>,
}

impl NdiApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let raw_frame_buffer = Arc::new(ArcSwap::from_pointee(None));
        let raw_frame_buffer_clone = raw_frame_buffer.clone();

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
                // copy frame
                let data = frame.data.to_vec();
                let raw = RawFrame {
                    width: frame.width,
                    height: frame.height,
                    data,
                };

                // Store using ArcSwap (lock-free)
                raw_frame_buffer_clone.store(Arc::new(Some(raw)));

                // これをしないとマウスカーソルを動かさないと再描画されない
                ctx.request_repaint();

                println!(
                    "Frame received (for wgpu): {}x{}, timecode={}",
                    frame.width, frame.height, frame.timecode
                );
            });
        });

        // wgpuインスタンスを手動で作成
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .expect("Failed to find an appropriate adapter");

        // Raspi4 (Mobile/Downlevel向け) の制限設定
        let mut limits = wgpu::Limits::downlevel_defaults();
        limits.max_color_attachments = 4;

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("Device"),
                required_features: wgpu::Features::empty(),
                required_limits: limits,
                memory_hints: Default::default(),
            },
            None,
        ))
        .expect("Failed to create device");

        let device = Arc::new(device);
        let queue = Arc::new(queue);

        // コンピュートシェーダーのコンパイル
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Grayscale Compute Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("grayscale.wgsl").into()),
        });

        // バインドグループレイアウト
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Grayscale Bind Group Layout"),
            entries: &[
                // Input buffer (read-only)
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // Output buffer (read-write)
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Grayscale Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Grayscale Pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        Self {
            raw_frame_buffer,
            device,
            queue,
            pipeline,
            bind_group_layout,
            texture: None,
        }
    }

    fn process_frame_with_wgpu(&self, raw: &RawFrame) -> Vec<u8> {
        let pixel_count = (raw.width as u32) * (raw.height as u32);
        let byte_size = (pixel_count * 4) as usize;

        // 入力バッファを作成してデータをアップロード
        let input_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Input Buffer"),
                contents: &raw.data,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            });

        // 出力バッファを作成
        let output_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Output Buffer"),
            size: byte_size as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        // CPUに読み戻すためのステージングバッファ
        let staging_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Staging Buffer"),
            size: byte_size as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // バインドグループを作成
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Grayscale Bind Group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: input_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: output_buffer.as_entire_binding(),
                },
            ],
        });

        // コマンドエンコーダーを作成
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Grayscale Encoder"),
            });

        {
            let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Grayscale Pass"),
                timestamp_writes: None,
            });
            compute_pass.set_pipeline(&self.pipeline);
            compute_pass.set_bind_group(0, &bind_group, &[]);

            // ワークグループ数を計算（切り上げ除算）
            let workgroup_count = (pixel_count + WORKGROUP_SIZE - 1) / WORKGROUP_SIZE;
            compute_pass.dispatch_workgroups(workgroup_count, 1, 1);
        }

        // 出力バッファからステージングバッファにコピー
        encoder.copy_buffer_to_buffer(&output_buffer, 0, &staging_buffer, 0, byte_size as u64);

        // コマンドを送信
        self.queue.submit(Some(encoder.finish()));

        // ステージングバッファをマップして結果を読み取る
        let buffer_slice = staging_buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            sender.send(result).unwrap();
        });

        // デバイスをポーリングしてマップ完了を待つ
        self.device.poll(wgpu::Maintain::Wait);
        receiver.recv().unwrap().unwrap();

        // データを取得
        let data = buffer_slice.get_mapped_range();
        let result = data.to_vec();
        drop(data);
        staging_buffer.unmap();

        result
    }
}

impl eframe::App for NdiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let panel_frame = egui::Frame::central_panel(&ctx.style()).fill(egui::Color32::BLACK);

        egui::CentralPanel::default()
            .frame(panel_frame)
            .show(ctx, |ui| {
                // 新しいフレームがあるかチェック（ArcSwapでロックフリー読み取り）
                let new_raw_frame = self.raw_frame_buffer.swap(Arc::new(None));
                let new_raw_frame =
                    Arc::try_unwrap(new_raw_frame).unwrap_or_else(|arc| (*arc).clone());

                // 新しいフレームが来ていればwgpuで処理
                let has_new_frame = new_raw_frame.is_some();
                if let Some(raw) = new_raw_frame {
                    let grayscale_data = self.process_frame_with_wgpu(&raw);

                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [raw.width as usize, raw.height as usize],
                        &grayscale_data,
                    );

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
        "NDI Grayscale Viewer (WGPU)",
        options,
        Box::new(|cc| Ok(Box::new(NdiApp::new(cc)))),
    )
    .map_err(|e| anyhow::anyhow!("GUI error: {}", e))
}
