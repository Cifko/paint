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
/// drive the GPU (e.g. with NVIDIA's own driver). With a Vulkan GPU present,
/// use only Vulkan; otherwise keep OpenGL as the fallback. `WGPU_BACKEND`
/// still overrides this.
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
        // A software (CPU) Vulkan driver is slower than OpenGL on a real GPU.
        let has_gpu = pollster::block_on(vulkan.enumerate_adapters(wgpu::Backends::VULKAN))
            .iter()
            .any(|a| a.get_info().device_type != wgpu::DeviceType::Cpu);
        if has_gpu {
            setup.instance_descriptor.backends = wgpu::Backends::VULKAN;
        }
    }
    WgpuConfiguration {
        wgpu_setup: WgpuSetup::CreateNew(setup),
        ..Default::default()
    }
}
