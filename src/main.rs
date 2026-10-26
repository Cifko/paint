#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod canvas;
mod color_picker;
#[cfg(target_os = "linux")]
mod desktop;
mod file;
mod floating;
mod history;
mod icons;
mod settings;
mod shortcuts;
mod text;
mod titlebar;

/// Names the window for the desktop: Wayland's app id and X11's WM_CLASS.
const APP_ID: &str = "paint";

fn main() -> eframe::Result {
    #[cfg(target_os = "linux")]
    desktop::install();

    let options = eframe::NativeOptions {
        viewport: titlebar::configure(eframe::egui::ViewportBuilder::default())
            .with_title("Paint")
            .with_app_id(APP_ID)
            .with_inner_size([1280.0, 800.0])
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../assets/icons/256.png"))
                    .expect("bundled icon is a valid PNG"),
            ),
        #[cfg(target_os = "linux")]
        wgpu_options: linux_wgpu_options(),
        ..Default::default()
    };
    eframe::run_native(
        "Paint",
        options,
        Box::new(|cc| {
            let mut app = app::PaintApp::new(cc);
            // Opening an image from the command line / "Open with".
            if let Some(path) = std::env::args_os().nth(1) {
                app.open_path(path.into());
            }
            Ok(Box::new(app))
        }),
    )
}

/// By default wgpu tries Vulkan and OpenGL side by side, and merely probing
/// OpenGL loads Mesa's EGL, which prints warnings on systems where it can't
/// drive a GPU (no GPU driver, a VM, NVIDIA's own driver). Use only Vulkan
/// when OpenGL can't do better; otherwise keep OpenGL as the fallback.
/// `WGPU_BACKEND` still overrides this.
#[cfg(target_os = "linux")]
fn linux_wgpu_options() -> eframe::egui_wgpu::WgpuConfiguration {
    use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup, WgpuSetupCreateNew};
    use eframe::wgpu;

    let mut setup = WgpuSetupCreateNew::without_display_handle();
    if wgpu::Backends::from_env().is_none() {
        let vulkan = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapters = pollster::block_on(vulkan.enumerate_adapters(wgpu::Backends::VULKAN));
        let vulkan_gpu = adapters
            .iter()
            .any(|a| a.get_info().device_type != wgpu::DeviceType::Cpu);
        // Only a software Vulkan driver: OpenGL can still be faster if there
        // is a GPU without Vulkan support (old ones), which needs a render
        // node. Without one OpenGL would be software too.
        let vulkan_software_only = !adapters.is_empty() && !has_render_node();
        if vulkan_gpu || vulkan_software_only {
            setup.instance_descriptor.backends = wgpu::Backends::VULKAN;
        }
    }
    WgpuConfiguration {
        wgpu_setup: WgpuSetup::CreateNew(setup),
        ..Default::default()
    }
}

/// Whether the kernel exposes a GPU for rendering (`/dev/dri/renderD*`).
#[cfg(target_os = "linux")]
fn has_render_node() -> bool {
    std::fs::read_dir("/dev/dri").is_ok_and(|dir| {
        dir.flatten()
            .any(|entry| entry.file_name().to_string_lossy().starts_with("renderD"))
    })
}
