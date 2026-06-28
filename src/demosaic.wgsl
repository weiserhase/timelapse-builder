// Bilinear Bayer demosaic + white balance + camera->sRGB matrix + sRGB gamma.
// The colour math mirrors rawler's CPU developer (clip_euclidean_norm_avg and
// the sRGB transfer constants) so output matches; only the demosaic is bilinear.

struct Params {
    dims0: vec4<u32>, // full_w, full_h, active_x, active_y
    dims1: vec4<u32>, // out_w, out_h, _, _
    cfa: vec4<u32>,   // colour index (R=0,G=1,B=2) per 2x2 phase [00,01,10,11]
    wb: vec4<f32>,
    m0: vec4<f32>,
    m1: vec4<f32>,
    m2: vec4<f32>,
};

@group(0) @binding(0) var<uniform> P: Params;
@group(0) @binding(1) var<storage, read> mosaic: array<f32>;
@group(0) @binding(2) var<storage, read_write> outp: array<u32>;

fn samp(ix: i32, iy: i32) -> f32 {
    let w = i32(P.dims0.x);
    let h = i32(P.dims0.y);
    let cx = clamp(ix, 0, w - 1);
    let cy = clamp(iy, 0, h - 1);
    return mosaic[u32(cy) * P.dims0.x + u32(cx)];
}

fn col_at(ar: i32, ac: i32) -> u32 {
    let r = u32(ar & 1);
    let c = u32(ac & 1);
    return P.cfa[r * 2u + c];
}

fn srgb(v: f32) -> f32 {
    if (v <= 0.00304) {
        return v * 12.92;
    }
    return pow(v, 1.0 / 2.4) * 1.055 - 0.055;
}

fn clipnorm(p_in: vec3<f32>) -> vec3<f32> {
    let p = max(p_in, vec3<f32>(0.0));
    let mx = max(p.x, max(p.y, p.z));
    if (mx > 1.0) {
        let color = p / mx;
        let eucl = length(p) / sqrt(3.0);
        return (color + vec3<f32>(eucl)) / 2.0;
    }
    return p;
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let ow = P.dims1.x;
    let oh = P.dims1.y;
    if (gid.x >= ow || gid.y >= oh) {
        return;
    }
    let ax = i32(P.dims0.z + gid.x);
    let ay = i32(P.dims0.w + gid.y);

    let owncol = col_at(ay, ax);
    let center = samp(ax, ay);

    // Average each colour's nearest neighbours in the 3x3 (excluding centre).
    var sum = array<f32, 3>(0.0, 0.0, 0.0);
    var cnt = array<f32, 3>(0.0, 0.0, 0.0);
    for (var dy: i32 = -1; dy <= 1; dy = dy + 1) {
        for (var dx: i32 = -1; dx <= 1; dx = dx + 1) {
            if (dx == 0 && dy == 0) {
                continue;
            }
            let c = col_at(ay + dy, ax + dx);
            sum[c] = sum[c] + samp(ax + dx, ay + dy);
            cnt[c] = cnt[c] + 1.0;
        }
    }

    var rgb3 = array<f32, 3>(0.0, 0.0, 0.0);
    for (var i: u32 = 0u; i < 3u; i = i + 1u) {
        if (i == owncol) {
            rgb3[i] = center;
        } else if (cnt[i] > 0.0) {
            rgb3[i] = sum[i] / cnt[i];
        }
    }

    var rgb = vec3<f32>(rgb3[0], rgb3[1], rgb3[2]);
    rgb = rgb * P.wb.xyz;

    let lin = vec3<f32>(dot(P.m0.xyz, rgb), dot(P.m1.xyz, rgb), dot(P.m2.xyz, rgb));
    let cl = clipnorm(lin);
    let g = vec3<f32>(srgb(cl.x), srgb(cl.y), srgb(cl.z));

    let r8 = u32(clamp(g.x, 0.0, 1.0) * 255.0 + 0.5);
    let g8 = u32(clamp(g.y, 0.0, 1.0) * 255.0 + 0.5);
    let b8 = u32(clamp(g.z, 0.0, 1.0) * 255.0 + 0.5);
    outp[gid.y * ow + gid.x] = r8 | (g8 << 8u) | (b8 << 16u) | (255u << 24u);
}
