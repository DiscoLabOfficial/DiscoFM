struct Settings { frames: u32, target_offset: u32, score_offset: u32, largest_offset: u32, output_stride: u32, floor: f32, padding0: u32, padding1: u32 }
@group(0) @binding(0) var<storage, read> signal: array<f32>;
@group(0) @binding(1) var<storage, read> centers: array<u32>;
@group(0) @binding(2) var<storage, read> window: array<f32>;
@group(0) @binding(3) var<storage, read_write> output: array<f32>;
@group(0) @binding(4) var<uniform> settings: Settings;
@group(0) @binding(5) var<storage, read> reference_spectrum: array<f32>;
@group(0) @binding(6) var<storage, read> target_logs: array<f32>;
var<workgroup> values: array<vec2<f32>, FFT_SIZE>;

@compute @workgroup_size(256)
fn main(@builtin(local_invocation_index) thread: u32, @builtin(workgroup_id) group: vec3<u32>) {
    for (var i = thread; i < FFT_SIZE; i += 256u) {
        let source = i32(centers[group.x]) + i32(i) - i32(FFT_SIZE / 2u);
        var sample = 0.0;
        if (source >= 0 && source < i32(settings.frames)) { sample = signal[group.y * settings.frames + u32(source)]; }
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
    var sums = vec2<f32>(0.0);
    let bins = FFT_SIZE / 2u + 1u;
    let base = group.y * settings.output_stride;
    for (var i = thread; i < bins; i += 256u) {
        let magnitude = length(values[i]) * (4.0 / f32(FFT_SIZE));
        let index = settings.target_offset + group.x * bins + i;
        let delta = magnitude - reference_spectrum[index];
        sums += vec2<f32>(abs(log(max(magnitude, settings.floor)) - target_logs[index]), delta * delta);
        if (FFT_SIZE == 4096u) { output[base + settings.largest_offset + group.x*bins + i] = magnitude; }
    }
    // Reuse FFT storage: 4096 complex values already occupy the full 32 KiB.
    workgroupBarrier();
    values[thread] = sums;
    workgroupBarrier();
    for (var step = 128u; step > 0u; step /= 2u) {
        if (thread < step) { values[thread] += values[thread+step]; }
        workgroupBarrier();
    }
    if (thread == 0u) {
        output[base + settings.score_offset + group.x*2u] = values[0].x;
        output[base + settings.score_offset + group.x*2u+1u] = values[0].y;
    }
}
