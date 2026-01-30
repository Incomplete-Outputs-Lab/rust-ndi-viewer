// ガウシアンブラー用のコンピュートシェーダー (5x5カーネル)
@group(0) @binding(0) var<storage, read> input: array<u32>;
@group(0) @binding(1) var<storage, read_write> output: array<u32>;
@group(0) @binding(2) var<uniform> dimensions: vec2<u32>; // width, height

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let x = id.x;
    let y = id.y;
    let width = dimensions.x;
    let height = dimensions.y;

    // 範囲チェック
    if (x >= width || y >= height) {
        return;
    }

    // ガウシアンカーネル 5x5
    // 1   4   6   4   1
    // 4  16  24  16   4
    // 6  24  36  24   6
    // 4  16  24  16   4
    // 1   4   6   4   1
    let kernel = array<u32, 25>(
        1u, 4u, 6u, 4u, 1u,
        4u, 16u, 24u, 16u, 4u,
        6u, 24u, 36u, 24u, 6u,
        4u, 16u, 24u, 16u, 4u,
        1u, 4u, 6u, 4u, 1u
    );
    let kernel_sum = 256u;

    var r_sum = 0u;
    var g_sum = 0u;
    var b_sum = 0u;
    var a_sum = 0u;

    // 5x5カーネルを適用
    for (var ky = 0; ky < 5; ky++) {
        for (var kx = 0; kx < 5; kx++) {
            // 境界処理: クランプ
            let px = clamp(i32(x) + kx - 2, 0, i32(width) - 1);
            let py = clamp(i32(y) + ky - 2, 0, i32(height) - 1);
            let idx = u32(py) * width + u32(px);

            // RGBA を u32 から読み取る（リトルエンディアン: ABGR）
            let pixel = input[idx];
            let r = pixel & 0xFFu;
            let g = (pixel >> 8u) & 0xFFu;
            let b = (pixel >> 16u) & 0xFFu;
            let a = (pixel >> 24u) & 0xFFu;

            let weight = kernel[u32(ky) * 5u + u32(kx)];
            r_sum += r * weight;
            g_sum += g * weight;
            b_sum += b * weight;
            a_sum += a * weight;
        }
    }

    // 正規化
    let r_out = r_sum / kernel_sum;
    let g_out = g_sum / kernel_sum;
    let b_out = b_sum / kernel_sum;
    let a_out = a_sum / kernel_sum;

    // 出力（リトルエンディアン: ABGR）
    let out_idx = y * width + x;
    output[out_idx] = r_out | (g_out << 8u) | (b_out << 16u) | (a_out << 24u);
}
