@group(0) @binding(0) var<storage, read> input: array<u32>;
@group(0) @binding(1) var<storage, read_write> output: array<u32>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let pixel_index = id.x;

    // バッファの範囲チェック
    if (pixel_index >= arrayLength(&input)) {
        return;
    }

    // RGBA を u32 から読み取る（リトルエンディアン: ABGR）
    let pixel = input[pixel_index];
    let r = pixel & 0xFFu;
    let g = (pixel >> 8u) & 0xFFu;
    let b = (pixel >> 16u) & 0xFFu;

    // ITU-R BT.601 輝度計算（整数演算で高速化）
    // 固定小数点: (77*R + 150*G + 29*B) >> 8
    let gray = (77u * r + 150u * g + 29u * b) >> 8u;

    // グレースケール値を R, G, B に設定し、Alpha は 255
    output[pixel_index] = gray | (gray << 8u) | (gray << 16u) | (255u << 24u);
}
