//! Renders drawn as Hex's windows draw them, for the sandbox's snapshots:
//! sRGB blending into a non-sRGB target. Theme colours resolve for the blend
//! mode when the view is built, so a runtime turns it on before its first
//! frame.

use rune::render::{Gpu, OFFSCREEN_FORMAT, Offscreen, Renderer};
use rune::text::TextSystem;
use rune::{Cx, El, Env, Runtime};

pub(crate) fn format() -> wgpu::TextureFormat {
    OFFSCREEN_FORMAT.remove_srgb_suffix()
}

pub(crate) fn runtime<S: 'static>(
    state: S,
    view: fn(&S, &mut Cx<S>) -> El,
    size: (f32, f32),
    env: Env,
    text: TextSystem,
) -> Runtime<S> {
    let mut rt = Runtime::with_text(state, view, size, env, text);
    rt.set_srgb_blend(true);
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    rt.set_system_scroll_bars(rune::ScrollBars::Always);
    rt
}

pub(crate) fn renderer(device: &wgpu::Device) -> Renderer {
    let mut renderer = Renderer::new(device, format());
    renderer.set_srgb_blend(true);
    renderer
}

pub(crate) fn target(device: &wgpu::Device, width: u32, height: u32) -> Offscreen {
    Offscreen::with_format(device, width, height, format())
}

/// `rt` settled and drawn into `width` × `height` pixels: their RGBA.
pub(crate) fn draw<S: 'static>(gpu: &Gpu, rt: &mut Runtime<S>, width: u32, height: u32) -> Vec<u8> {
    rt.settle_all();
    rt.frame(0.0);
    let mut renderer = renderer(&gpu.device);
    let target = target(&gpu.device, width, height);
    renderer.upload_images(&gpu.device, &gpu.queue, &mut rt.images);
    renderer.prepare(
        &gpu.device,
        &gpu.queue,
        &mut rt.text.atlas,
        &rt.instances,
        (target.width, target.height),
    );
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    renderer.draw(&mut encoder, &target.view, rt.clear_color());
    target.copy(&mut encoder);
    gpu.queue.submit([encoder.finish()]);
    target.read(&gpu.device)
}
