# Rust NDI Viewer

A simple tool for viewing NDI (Network Device Interface) streams, written in Rust using [eframe](https://github.com/emilk/egui/tree/master/crates/eframe) and [grafton-ndi](https://github.com/GrantSparks/grafton-ndi).  

## Setup

**Requires [NDI 6 SDK](https://ndi.video/for-developers/ndi-sdk/) installed locally.**

### Note on Linux Requirements 

**On Linux, [clang](https://clang.llvm.org/) is required for building this project.**  

This is because `grafton-ndi` uses `bindgen`, which depends on `clang` to generate Rust bindings for the NDI SDK during build time.  


Make sure `clang` is installed and available in your environment:
```bash
sudo apt-get install clang
# or on Fedora
sudo dnf install clang
```

If you encounter build errors related to `bindgen` or inability to find `clang`, double-check your installation.

For more details, see the [bindgen requirements](https://rust-lang.github.io/rust-bindgen/requirements.html).


### Dependencies

- [eframe (egui)](https://github.com/emilk/egui/tree/master/crates/eframe) — For the GUI
- [grafton-ndi](https://github.com/GrantSparks/grafton-ndi) — NDI bindings for Rust
- [NDI SDK](https://ndi.video/for-developers/ndi-sdk/) — Native library required by grafton-ndi

Please make sure the NDI 6 SDK is installed and available on your system before running the application.