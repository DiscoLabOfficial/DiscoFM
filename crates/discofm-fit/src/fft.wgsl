struct Settings { frames: u32, offset: u32, padding0: u32, padding1: u32 }
@group(0) @binding(0) var<storage, read> signal: array<f32>;
@group(0) @binding(1) var<storage, read> centers: array<u32>;
@group(0) @binding(2) var<storage, read> window: array<f32>;
@group(0) @binding(3) var<storage, read_write> output: array<f32>;
@group(0) @binding(4) var<uniform> settings: Settings;
var<workgroup> values: array<vec2<f32>, FFT_SIZE>;

@compute @workgroup_size(256)
fn main(@builtin(local_invocation_index) thread: u32, @builtin(workgroup_id) group: vec3<u32>) {
    for (var i = thread; i < FFT_SIZE; i += 256u) {
        let source = i32(centers[group.x]) + i32(i) - i32(FFT_SIZE / 2u);
        var sample = 0.0;
        if (source >= 0 && source < i32(settings.frames)) { sample = signal[u32(source)]; }
        values[reverseBits(i) >> (32u - FFT_BITS)] = vec2<f32>(sample * window[i], 0.0);
    }
    workgroupBarrier();
    for (var width = 2u; width <= FFT_SIZE; width *= 2u) {
        let half = width / 2u;
        for (var pair = thread; pair < FFT_SIZE / 2u; pair += 256u) {
            let j = pair % half;
            let a = (pair / half) * width + j;
            let b = a + half;
            let angle = -6.283185307179586 * f32(j) / f32(width);
            let twiddle = vec2<f32>(cos(angle), sin(angle));
            let v = values[b];
            let product = vec2<f32>(v.x * twiddle.x - v.y * twiddle.y, v.x * twiddle.y + v.y * twiddle.x);
            let first = values[a];
            values[a] = first + product;
            values[b] = first - product;
        }
        workgroupBarrier();
    }
    for (var i = thread; i <= FFT_SIZE / 2u; i += 256u) {
        output[settings.offset + group.x * (FFT_SIZE / 2u + 1u) + i] = length(values[i]) * (4.0 / f32(FFT_SIZE));
    }
}
