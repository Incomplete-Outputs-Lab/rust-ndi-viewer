# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

A cross-platform NDI (Network Device Interface) video stream viewer built in Rust. The application uses `eframe` (egui) for the GUI and `grafton-ndi` for NDI SDK bindings. It discovers NDI sources on the network, connects to a specified source (or the first available one), and displays the video stream in real-time.

## Prerequisites

- **NDI 6 SDK** must be installed on the system (required by grafton-ndi)
- **Linux only**: `clang` is required for building (used by bindgen to generate NDI bindings)
  ```bash
  sudo apt-get install clang  # Debian/Ubuntu
  sudo dnf install clang      # Fedora
  ```

## Build Commands

```bash
# Build the project
cargo build

# Build with optimizations
cargo build --release

# Run the application
cargo run

# Run with additional NDI discovery IPs/subnets
cargo run -- 192.168.1.0/24 10.0.0.5
```

## Architecture

### Threading Model

The application uses a multi-threaded architecture to keep the GUI responsive:

1. **Main Thread (GUI)**: Runs the eframe/egui event loop, rendering frames from the shared buffer
2. **NDI Receiver Thread**: Spawned at startup in `NdiApp::new()`, handles all NDI operations:
   - NDI initialization and source discovery
   - Connection to the target source
   - Frame capture loop
   - Frame conversion to `egui::ColorImage`
   - Updates shared buffer with latest frame

### Frame Synchronization

- `Arc<Mutex<Option<egui::ColorImage>>>` serves as a lock-free-ish exchange buffer between threads
- NDI thread writes latest frame using `.lock().unwrap()` and replaces the `Option`
- GUI thread calls `.take()` on each render cycle to consume the latest frame
- If no new frame is available, the GUI continues displaying the previous texture

### Source Selection

The `TARGET_SOURCE_NAME` constant in `src/main.rs:9` determines which NDI source to connect to:
- Empty string `""`: connects to the first discovered source
- Non-empty: searches for an exact match by name

### Pixel Format Handling

The receiver is configured for `ReceiverColorFormat::RGBX_RGBA`. The frame capture loop:
- Only accepts `PixelFormat::RGBA` or `PixelFormat::RGBX`
- Validates line stride matches `width * 4`
- Skips compressed frames (data size too small for uncompressed)
- Converts accepted frames to `egui::ColorImage::from_rgba_unmultiplied()`

### Command-line Arguments

Optional IP addresses/subnets can be passed as arguments for NDI discovery on specific networks. The NDI thread parses these from `env::args()` and passes them to `FinderOptions::extra_ips()`.
