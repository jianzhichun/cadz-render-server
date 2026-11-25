//! The headless graphics device: an adapter picked with no surface, its device, and egui-wgpu's
//! `RenderState`. With no surface the target format is `Rgba8Unorm` (egui-wgpu's
//! `RenderState::create`), so read-back bytes are RGBA in gamma space, as a window would show them.
//! The UI's render pass has an 8-bit stencil, as the desktop window asks (`NativeOptions::
//! stencil_buffer`): the drawing area clips layout viewports with it.

use std::sync::Arc;

use egui_wgpu::wgpu;

use crate::domain::memory::GpuMemory;

/// Stencil bits of the UI's render pass (`apps/cadcraft/src/main.rs`, `STENCIL_BITS`).
pub const STENCIL_BITS: u8 = 8;

/// Which adapters may be used (`--adapter`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    /// The best there is: a GPU before a software rasterizer.
    Auto,
    /// A GPU only.
    Hardware,
    /// A software rasterizer only (WARP, llvmpipe, lavapipe): what a machine with no GPU gets.
    Software,
}

impl Policy {
    pub fn parse(s: &str) -> Option<Policy> {
        match s.trim() {
            "auto" => Some(Policy::Auto),
            "hardware" => Some(Policy::Hardware),
            "software" => Some(Policy::Software),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Policy::Auto => "auto",
            Policy::Hardware => "hardware",
            Policy::Software => "software",
        }
    }

    fn admits(self, t: wgpu::DeviceType) -> bool {
        match self {
            Policy::Auto => true,
            Policy::Hardware => t != wgpu::DeviceType::Cpu,
            Policy::Software => t == wgpu::DeviceType::Cpu,
        }
    }
}

/// The device and what was chosen, for `hello`, `ready` and the UI's own setup.
pub struct Gpu {
    pub rs: egui_wgpu::RenderState,
    /// `hello.backend`.
    pub backend: Option<&'static str>,
    pub adapter: String,
    pub software: bool,
    /// The UI pass's stencil format, for the drawing area's pipelines.
    pub stencil: Option<wgpu::TextureFormat>,
}

/// The backends the instance is made with: Metal on macOS, DirectX 12 on Windows (the desktop app
/// leaves OpenGL out there, `apps/cadcraft/src/graphics.rs`), Vulkan and OpenGL elsewhere;
/// `WGPU_BACKEND` chooses others.
fn backends() -> wgpu::Backends {
    wgpu::Backends::from_env().unwrap_or(if cfg!(target_os = "windows") {
        wgpu::Backends::DX12
    } else if cfg!(target_os = "macos") {
        wgpu::Backends::METAL
    } else {
        wgpu::Backends::VULKAN | wgpu::Backends::GL
    })
}

/// Open the device on the adapter `policy` admits: hardware before software, then the platform's
/// preferred backend.
pub fn open(policy: Policy) -> Result<Gpu, String> {
    let mut setup = egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = backends();
    // FXC, the compiler Windows ships, unless `WGPU_DX12_COMPILER` says otherwise: another
    // program's dxcompiler.dll without its dxil.dll loses the device (`graphics.rs`).
    if cfg!(target_os = "windows") {
        setup.instance_descriptor.backend_options.dx12.shader_compiler = wgpu::Dx12Compiler::from_env().unwrap_or(wgpu::Dx12Compiler::Fxc);
    }
    setup.native_adapter_selector =
        Some(Arc::new(move |adapters: &[wgpu::Adapter], _surface: Option<&wgpu::Surface<'_>>| choose(adapters, policy)));
    let base = setup.device_descriptor.clone();
    setup.device_descriptor = Arc::new(move |adapter: &wgpu::Adapter| {
        let mut d = base(adapter);
        // Command previews of large selections outgrow wgpu's default buffer cap (`main.rs`).
        d.required_limits.max_buffer_size = d.required_limits.max_buffer_size.max(adapter.limits().max_buffer_size);
        // Small memory blocks: one frame's targets are a few blocks, not a whole idle one.
        d.memory_hints = wgpu::MemoryHints::MemoryUsage;
        d
    });
    let config = egui_wgpu::WgpuConfiguration { wgpu_setup: egui_wgpu::WgpuSetup::CreateNew(setup), ..Default::default() };
    let instance = pollster::block_on(config.wgpu_setup.new_instance());
    let stencil = egui_wgpu::depth_format_from_bits(0, STENCIL_BITS);
    let options = egui_wgpu::RendererOptions { msaa_samples: 1, depth_stencil_format: stencil, ..Default::default() };
    let rs = pollster::block_on(egui_wgpu::RenderState::create(&config, &instance, None, options)).map_err(|e| format!("graphics device: {e}"))?;
    if rs.target_format != wgpu::TextureFormat::Rgba8Unorm {
        return Err(format!("graphics device: unexpected target format {:?}", rs.target_format));
    }
    let info = rs.adapter.get_info();
    log::info!("graphics: {:?} {} ({:?}), stencil {stencil:?}", info.backend, info.name, info.device_type);
    Ok(Gpu {
        backend: backend_name(info.backend),
        adapter: info.name.clone(),
        software: info.device_type == wgpu::DeviceType::Cpu,
        stencil,
        rs,
    })
}

/// The adapter `policy` admits, best first: a discrete GPU, an integrated one, a virtual one, an
/// unknown kind, a software rasterizer; among equals the platform's preferred backend.
fn choose(adapters: &[wgpu::Adapter], policy: Policy) -> Result<wgpu::Adapter, String> {
    adapters
        .iter()
        .filter(|a| policy.admits(a.get_info().device_type))
        .max_by_key(|a| {
            let info = a.get_info();
            (rank(info.device_type), backend_preference(info.backend))
        })
        .cloned()
        .ok_or_else(|| {
            let found: Vec<String> =
                adapters.iter().map(|a| format!("{} ({:?}, {:?})", a.get_info().name, a.get_info().backend, a.get_info().device_type)).collect();
            format!(
                "no graphics adapter qualifies under `{}` (found: {})",
                policy.name(),
                if found.is_empty() { "none".into() } else { found.join(", ") }
            )
        })
}

fn rank(t: wgpu::DeviceType) -> u8 {
    match t {
        wgpu::DeviceType::DiscreteGpu => 4,
        wgpu::DeviceType::IntegratedGpu => 3,
        wgpu::DeviceType::VirtualGpu => 2,
        wgpu::DeviceType::Other => 1,
        wgpu::DeviceType::Cpu => 0,
    }
}

fn backend_preference(b: wgpu::Backend) -> u8 {
    match b {
        wgpu::Backend::Dx12 | wgpu::Backend::Metal => 3,
        wgpu::Backend::Vulkan => 2,
        wgpu::Backend::Gl => 1,
        _ => 0,
    }
}

fn backend_name(b: wgpu::Backend) -> Option<&'static str> {
    match b {
        wgpu::Backend::Vulkan => Some("vulkan"),
        wgpu::Backend::Dx12 => Some("dx12"),
        wgpu::Backend::Gl => Some("gl"),
        wgpu::Backend::Metal => Some("metal"),
        _ => None,
    }
}

/// What the device holds by label, where the backend's allocator can say (Vulkan and D3D12;
/// Metal and GL report nothing).
pub fn memory(device: &wgpu::Device) -> Option<GpuMemory> {
    let r = device.generate_allocator_report()?;
    Some(GpuMemory::new(r.total_allocated_bytes, r.total_reserved_bytes, r.allocations.into_iter().map(|a| (a.name, a.size))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policies_admit_their_kind_of_adapter() {
        assert_eq!(Policy::parse("software"), Some(Policy::Software));
        assert_eq!(Policy::parse("cpu"), None);
        assert!(Policy::Software.admits(wgpu::DeviceType::Cpu) && !Policy::Software.admits(wgpu::DeviceType::DiscreteGpu));
        assert!(!Policy::Hardware.admits(wgpu::DeviceType::Cpu) && Policy::Auto.admits(wgpu::DeviceType::Cpu));
    }
}
