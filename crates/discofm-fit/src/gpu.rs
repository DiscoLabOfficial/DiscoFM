//! GPU computes the same windowed FFT magnitudes as the CPU path (realfft). Chip clocks stay
//! in Nuked-OPN2. Buffers/pipelines persist through every candidate of a run.
use std::{
    sync::{Arc, Mutex, OnceLock, mpsc},
    time::Duration,
};
use wgpu::util::DeviceExt;

struct Context {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipelines: Vec<wgpu::ComputePipeline>,
    score_pipelines: Vec<wgpu::ComputePipeline>,
    name: String,
    error: Arc<Mutex<Option<String>>>,
}
impl Context {
    fn check(&self) -> Result<(), String> {
        match self
            .error
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
        {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }
}
static CONTEXT: OnceLock<Result<Arc<Context>, String>> = OnceLock::new();
const SIZES: [usize; 3] = [512, 1024, 4096];

fn context() -> Result<Arc<Context>, String> {
    CONTEXT
        .get_or_init(|| {
            // wgpu enables the backend validation layers in every build with
            // debug assertions. The Khronos Vulkan layer from SDK 1.4.357
            // crashes (null read in its bundled mimalloc) on the first call
            // from a fresh calibration thread, killing debug GUIs on machines
            // with the SDK installed. wgpu-core still validates API use;
            // WGPU_VALIDATION=1 / WGPU_DEBUG=1 bring the layers back.
            let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
            descriptor.flags = descriptor
                .flags
                .difference(wgpu::InstanceFlags::DEBUG | wgpu::InstanceFlags::VALIDATION)
                .with_env();
            let instance = wgpu::Instance::new(descriptor);
            let mut adapters: Vec<_> =
                pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()))
                    .into_iter()
                    .filter_map(|adapter| {
                        let info = adapter.get_info();
                        adapter_rank(info.device_type, info.backend).map(|rank| (rank, adapter))
                    })
                    .collect();
            adapters.sort_by_key(|(rank, _)| *rank);
            let limits = wgpu::Limits {
                max_compute_workgroup_storage_size: 32768,
                ..Default::default()
            };
            let mut rejected = Vec::new();
            let mut opened = None;
            for (_, adapter) in adapters {
                let info = adapter.get_info();
                if adapter.limits().max_compute_workgroup_storage_size < 32768 {
                    rejected.push(format!(
                        "{} ({:?}): needs 32 KiB workgroup memory for the 4096-point FFT",
                        info.name, info.backend
                    ));
                    continue;
                }
                match pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                    label: Some("DiscoFM fitter compute"),
                    required_limits: limits.clone(),
                    ..Default::default()
                })) {
                    Ok((device, queue)) => {
                        opened = Some((info, device, queue));
                        break;
                    }
                    Err(error) => {
                        rejected.push(format!("{} ({:?}): {error}", info.name, info.backend))
                    }
                }
            }
            let Some((info, device, queue)) = opened else {
                return Err(if rejected.is_empty() {
                    "no hardware GPU adapter found".into()
                } else {
                    rejected.join("; ")
                });
            };
            let error = Arc::new(Mutex::new(None));
            let reported = Arc::clone(&error);
            device.on_uncaptured_error(Arc::new(move |failure: wgpu::Error| {
                *reported.lock().unwrap_or_else(|p| p.into_inner()) = Some(failure.to_string());
            }));
            let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let pipelines = SIZES
                .into_iter()
                .map(|size| {
                    let source = format!(
                        "const FFT_SIZE: u32 = {size}u;\nconst FFT_BITS: u32 = {}u;\n{}",
                        size.ilog2(),
                        include_str!("fft.wgsl")
                    );
                    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                        label: Some("DiscoFM radix-2 STFT"),
                        source: wgpu::ShaderSource::Wgsl(source.into()),
                    });
                    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                        label: Some("DiscoFM STFT"),
                        layout: None,
                        module: &module,
                        entry_point: Some("main"),
                        compilation_options: Default::default(),
                        cache: None,
                    })
                })
                .collect();
            let score_pipelines = SIZES
                .into_iter()
                .map(|size| {
                    let source = format!(
                        "const FFT_SIZE: u32 = {size}u;\nconst FFT_BITS: u32 = {}u;\n{}",
                        size.ilog2(),
                        include_str!("score.wgsl")
                    );
                    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                        label: Some("DiscoFM batched spectral loss"),
                        source: wgpu::ShaderSource::Wgsl(source.into()),
                    });
                    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                        label: Some("DiscoFM FFT and reduction"),
                        layout: None,
                        module: &module,
                        entry_point: Some("main"),
                        compilation_options: Default::default(),
                        cache: None,
                    })
                })
                .collect();
            if let Some(error) = pollster::block_on(scope.pop()) {
                return Err(error.to_string());
            }
            let context = Arc::new(Context {
                device,
                queue,
                pipelines,
                score_pipelines,
                name: format!("{} ({:?})", info.name, info.backend),
                error,
            });
            context.check()?;
            Ok(context)
        })
        .clone()
}
/// Discrete cards first. `PowerPreference::HighPerformance` alone could hand a
/// hybrid laptop's integrated GPU to the fitter; it is now used only when every
/// discrete adapter fails to open a device. Within a type, Vulkan first: the
/// numerical GPU tests were validated on it. Software adapters never qualify.
fn adapter_rank(kind: wgpu::DeviceType, backend: wgpu::Backend) -> Option<(u8, u8)> {
    let kind = match kind {
        wgpu::DeviceType::DiscreteGpu => 0,
        wgpu::DeviceType::IntegratedGpu => 1,
        wgpu::DeviceType::VirtualGpu => 2,
        wgpu::DeviceType::Other => 3,
        wgpu::DeviceType::Cpu => return None,
    };
    let backend = match backend {
        wgpu::Backend::Vulkan | wgpu::Backend::Metal => 0,
        wgpu::Backend::Dx12 => 1,
        _ => 2,
    };
    Some((kind, backend))
}
pub(crate) struct GpuFft {
    context: Arc<Context>,
    signal: wgpu::Buffer,
    output: wgpu::Buffer,
    readback: wgpu::Buffer,
    groups: Vec<wgpu::BindGroup>,
    counts: Vec<u32>,
    lengths: Vec<usize>,
    bytes: u64,
    frames: usize,
}
impl GpuFft {
    pub fn new(frames: usize, layouts: &[(Vec<usize>, Vec<f32>)]) -> Result<Self, String> {
        let context = context()?;
        context.check()?;
        let device = &context.device;
        let signal = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("candidate signal"),
            size: (frames * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let lengths = layouts
            .iter()
            .zip(SIZES)
            .map(|((centers, _), size)| centers.len() * (size / 2 + 1))
            .collect::<Vec<_>>();
        let bytes = (lengths.iter().sum::<usize>() * 4) as u64;
        if bytes > device.limits().max_storage_buffer_binding_size {
            return Err("STFT exceeds GPU storage limit".into());
        }
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("STFT magnitudes"),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("STFT readback"),
            size: bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut offset = 0;
        let mut groups = Vec::new();
        for (index, (centers, window)) in layouts.iter().enumerate() {
            let centers = centers.iter().map(|n| *n as u32).collect::<Vec<_>>();
            let centers = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("STFT centers"),
                contents: bytemuck::cast_slice(&centers),
                usage: wgpu::BufferUsages::STORAGE,
            });
            let window = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Hann window"),
                contents: bytemuck::cast_slice(window),
                usage: wgpu::BufferUsages::STORAGE,
            });
            let settings = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("STFT settings"),
                contents: bytemuck::cast_slice(&[frames as u32, offset as u32, 0, 0]),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            groups.push(
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("STFT buffers"),
                    layout: &context.pipelines[index].get_bind_group_layout(0),
                    entries: &[&signal, &centers, &window, &output, &settings]
                        .into_iter()
                        .enumerate()
                        .map(|(binding, buffer)| wgpu::BindGroupEntry {
                            binding: binding as u32,
                            resource: buffer.as_entire_binding(),
                        })
                        .collect::<Vec<_>>(),
                }),
            );
            offset += lengths[index];
        }
        context.check()?;
        Ok(Self {
            context,
            signal,
            output,
            readback,
            groups,
            counts: layouts.iter().map(|(c, _)| c.len() as u32).collect(),
            lengths,
            bytes,
            frames,
        })
    }
    pub fn name(&self) -> &str {
        &self.context.name
    }
    pub fn extract(
        &self,
        signal: &[f32],
        cancel: &impl Fn() -> bool,
    ) -> Result<Vec<Vec<f32>>, String> {
        if signal.len() != self.frames {
            return Err("GPU signal length mismatch".into());
        }
        self.context.check()?;
        self.context
            .queue
            .write_buffer(&self.signal, 0, bytemuck::cast_slice(signal));
        let mut encoder = self
            .context
            .device
            .create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("STFT"),
                timestamp_writes: None,
            });
            for (index, group) in self.groups.iter().enumerate() {
                pass.set_pipeline(&self.context.pipelines[index]);
                pass.set_bind_group(0, group, &[]);
                pass.dispatch_workgroups(self.counts[index], 1, 1);
            }
        }
        encoder.copy_buffer_to_buffer(&self.output, 0, &self.readback, 0, self.bytes);
        self.context.queue.submit([encoder.finish()]);
        self.context.check()?;
        let (sender, receiver) = mpsc::sync_channel(1);
        struct Unmap<'a>(&'a wgpu::Buffer);
        impl Drop for Unmap<'_> {
            fn drop(&mut self) {
                self.0.unmap();
            }
        }
        let _unmap = Unmap(&self.readback);
        self.readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        let started = std::time::Instant::now();
        loop {
            if cancel() {
                return Err("cancelled".into());
            }
            self.context
                .device
                .poll(wgpu::PollType::Poll)
                .map_err(|e| e.to_string())?;
            self.context.check()?;
            match receiver.try_recv() {
                Ok(result) => {
                    result.map_err(|e| e.to_string())?;
                    break;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err("GPU readback disconnected".into());
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
            if started.elapsed() > Duration::from_secs(5) {
                return Err("GPU readback timed out".into());
            }
            std::thread::sleep(Duration::from_micros(100));
        }
        let mapped = self
            .readback
            .slice(..)
            .get_mapped_range()
            .map_err(|e| e.to_string())?;
        let values: &[f32] = bytemuck::cast_slice(&mapped);
        if values.iter().any(|v| !v.is_finite() || *v < 0.0) {
            return Err("GPU returned invalid FFT magnitudes".into());
        }
        let mut offset = 0;
        let result = self
            .lengths
            .iter()
            .map(|length| {
                let part = values[offset..offset + length].to_vec();
                offset += length;
                part
            })
            .collect();
        drop(mapped);
        self.context.check()?;
        Ok(result)
    }
}
/// Dispatcher-owned buffers: CPU workers never submit to a shared job slot.
/// Each worker owns a slot; reference spectra and layouts are shared.
pub(crate) struct GpuScore {
    context: Arc<Context>,
    resident: Arc<Resident>,
    signal: wgpu::Buffer,
    output: wgpu::Buffer,
    readback: wgpu::Buffer,
    groups: Vec<wgpu::BindGroup>,
    counts: Vec<usize>,
    frames: usize,
    stride: usize,
    largest: usize,
    capacity: usize,
}
/// Read-only buffers uploaded once per run and bound by every slot.
struct Resident {
    reference: wgpu::Buffer,
    logs: wgpu::Buffer,
    /// Centers, Hann window and settings for each resolution.
    layouts: Vec<[wgpu::Buffer; 3]>,
}
pub(crate) struct SpectralScore {
    pub log_sum: [f64; 3],
    pub squared_error: [f64; 3],
    pub largest: Vec<f32>,
}
impl GpuScore {
    pub fn new(
        frames: usize,
        layouts: &[(Vec<usize>, Vec<f32>)],
        target: &[Vec<f32>],
        floor: f64,
        capacity: usize,
    ) -> Result<Self, String> {
        let context = context()?;
        context.check()?;
        let device = &context.device;
        let counts: Vec<_> = layouts.iter().map(|(c, _)| c.len()).collect();
        let largest = counts.iter().sum::<usize>() * 2;
        let stride = largest + target[2].len();
        let flat: Vec<_> = target.iter().flatten().copied().collect();
        let logs: Vec<_> = flat
            .iter()
            .map(|v| f64::from(*v).max(floor).ln() as f32)
            .collect();
        let upload = |label, values: &[u8], usage| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: values,
                usage,
            })
        };
        let reference = upload(
            "resident reference spectra",
            bytemuck::cast_slice(&flat),
            wgpu::BufferUsages::STORAGE,
        );
        let logs = upload(
            "resident reference log spectra",
            bytemuck::cast_slice(&logs),
            wgpu::BufferUsages::STORAGE,
        );
        let mut resident_layouts = Vec::new();
        let mut target_offset = 0;
        let mut score_offset = 0;
        for (index, (centers, window)) in layouts.iter().enumerate() {
            let centers: Vec<_> = centers.iter().map(|v| *v as u32).collect();
            let settings = [
                frames as u32,
                target_offset as u32,
                score_offset as u32,
                largest as u32,
                stride as u32,
                (floor as f32).to_bits(),
                0,
                0,
            ];
            resident_layouts.push([
                upload(
                    "window centers",
                    bytemuck::cast_slice(&centers),
                    wgpu::BufferUsages::STORAGE,
                ),
                upload(
                    "Hann",
                    bytemuck::cast_slice(window),
                    wgpu::BufferUsages::STORAGE,
                ),
                upload(
                    "batch settings",
                    bytemuck::cast_slice(&settings),
                    wgpu::BufferUsages::UNIFORM,
                ),
            ]);
            target_offset += target[index].len();
            score_offset += counts[index] * 2;
        }
        let resident = Arc::new(Resident {
            reference,
            logs,
            layouts: resident_layouts,
        });
        Self::with_resident(context, resident, counts, frames, stride, largest, capacity)
    }
    /// Another independent slot over the same resident reference data.
    pub fn slot(&self) -> Result<Self, String> {
        Self::with_resident(
            Arc::clone(&self.context),
            Arc::clone(&self.resident),
            self.counts.clone(),
            self.frames,
            self.stride,
            self.largest,
            self.capacity,
        )
    }
    fn with_resident(
        context: Arc<Context>,
        resident: Arc<Resident>,
        counts: Vec<usize>,
        frames: usize,
        stride: usize,
        largest: usize,
        capacity: usize,
    ) -> Result<Self, String> {
        context.check()?;
        let device = &context.device;
        let capacity = capacity.clamp(1, 4);
        let create = |label, size, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let signal = create(
            "batch signals",
            (frames * capacity * 4) as u64,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        );
        let output = create(
            "spectral reductions and partial trajectories",
            (stride * capacity * 4) as u64,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        );
        let readback = create(
            "compact batch readback",
            (stride * capacity * 4) as u64,
            wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        );
        let groups = resident
            .layouts
            .iter()
            .enumerate()
            .map(|(index, [centers, window, settings])| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("spectral batch"),
                    layout: &context.score_pipelines[index].get_bind_group_layout(0),
                    entries: &[
                        &signal,
                        centers,
                        window,
                        &output,
                        settings,
                        &resident.reference,
                        &resident.logs,
                    ]
                    .into_iter()
                    .enumerate()
                    .map(|(binding, buffer)| wgpu::BindGroupEntry {
                        binding: binding as u32,
                        resource: buffer.as_entire_binding(),
                    })
                    .collect::<Vec<_>>(),
                })
            })
            .collect();
        context.check()?;
        Ok(Self {
            context,
            resident,
            signal,
            output,
            readback,
            groups,
            counts,
            frames,
            stride,
            largest,
            capacity,
        })
    }
    pub fn score(
        &self,
        signals: &[&[f32]],
        cancel: &impl Fn() -> bool,
    ) -> Result<Vec<SpectralScore>, String> {
        if signals.is_empty()
            || signals.len() > self.capacity
            || signals.iter().any(|s| s.len() != self.frames)
        {
            return Err("GPU batch shape mismatch".into());
        }
        self.context.check()?;
        for (i, signal) in signals.iter().enumerate() {
            self.context.queue.write_buffer(
                &self.signal,
                (i * self.frames * 4) as u64,
                bytemuck::cast_slice(signal),
            );
        }
        let mut encoder = self
            .context
            .device
            .create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("batched STFT and distance reductions"),
                timestamp_writes: None,
            });
            for (i, group) in self.groups.iter().enumerate() {
                pass.set_pipeline(&self.context.score_pipelines[i]);
                pass.set_bind_group(0, group, &[]);
                pass.dispatch_workgroups(self.counts[i] as u32, signals.len() as u32, 1);
            }
        }
        let bytes = (signals.len() * self.stride * 4) as u64;
        encoder.copy_buffer_to_buffer(&self.output, 0, &self.readback, 0, bytes);
        self.context.queue.submit([encoder.finish()]);
        let (tx, rx) = mpsc::sync_channel(1);
        struct Unmap<'a>(&'a wgpu::Buffer);
        impl Drop for Unmap<'_> {
            fn drop(&mut self) {
                self.0.unmap();
            }
        }
        let _guard = Unmap(&self.readback);
        self.readback
            .slice(..bytes)
            .map_async(wgpu::MapMode::Read, move |r| {
                let _ = tx.send(r);
            });
        let started = std::time::Instant::now();
        loop {
            if cancel() {
                return Err("cancelled".into());
            }
            self.context
                .device
                .poll(wgpu::PollType::Poll)
                .map_err(|e| e.to_string())?;
            self.context.check()?;
            match rx.try_recv() {
                Ok(r) => {
                    r.map_err(|e| e.to_string())?;
                    break;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err("GPU readback disconnected".into());
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
            if started.elapsed() > Duration::from_secs(5) {
                return Err("GPU score readback timed out".into());
            }
            std::thread::sleep(Duration::from_micros(100));
        }
        let mapped = self
            .readback
            .slice(..bytes)
            .get_mapped_range()
            .map_err(|e| e.to_string())?;
        let values: &[f32] = bytemuck::cast_slice(&mapped);
        if values.iter().any(|v| !v.is_finite() || *v < 0.0) {
            return Err("GPU returned invalid score".into());
        }
        let mut results = Vec::new();
        for candidate in values.chunks_exact(self.stride) {
            let mut result = SpectralScore {
                log_sum: [0.0; 3],
                squared_error: [0.0; 3],
                largest: candidate[self.largest..].to_vec(),
            };
            let mut offset = 0;
            for (i, count) in self.counts.iter().enumerate() {
                for pair in candidate[offset..offset + count * 2].as_chunks::<2>().0 {
                    result.log_sum[i] += f64::from(pair[0]);
                    result.squared_error[i] += f64::from(pair[1]);
                }
                offset += count * 2;
            }
            results.push(result);
        }
        drop(mapped);
        self.context.check()?;
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn discrete_gpus_outrank_integrated_and_software_adapters_never_qualify() {
        use wgpu::{Backend, DeviceType};
        let rank = super::adapter_rank;
        let mut ranked = [
            (DeviceType::IntegratedGpu, Backend::Vulkan),
            (DeviceType::DiscreteGpu, Backend::Dx12),
            (DeviceType::IntegratedGpu, Backend::Dx12),
            (DeviceType::DiscreteGpu, Backend::Vulkan),
        ];
        ranked.sort_by_key(|&(kind, backend)| rank(kind, backend));
        assert_eq!(
            ranked,
            [
                (DeviceType::DiscreteGpu, Backend::Vulkan),
                (DeviceType::DiscreteGpu, Backend::Dx12),
                (DeviceType::IntegratedGpu, Backend::Vulkan),
                (DeviceType::IntegratedGpu, Backend::Dx12),
            ]
        );
        assert_eq!(rank(DeviceType::Cpu, Backend::Vulkan), None);
    }
    #[test]
    fn every_fft_and_loss_shader_parses_and_validates_without_a_gpu() {
        for body in [include_str!("fft.wgsl"), include_str!("score.wgsl")] {
            for size in super::SIZES {
                let source = format!(
                    "const FFT_SIZE: u32 = {size}u;\nconst FFT_BITS: u32 = {}u;\n{}",
                    size.ilog2(),
                    body
                );
                let module = wgpu::naga::front::wgsl::parse_str(&source)
                    .unwrap_or_else(|error| panic!("{}", error.emit_to_string(&source)));
                wgpu::naga::valid::Validator::new(
                    wgpu::naga::valid::ValidationFlags::all(),
                    wgpu::naga::valid::Capabilities::all(),
                )
                .validate(&module)
                .expect("valid WGSL compute kernel");
            }
        }
    }
}
