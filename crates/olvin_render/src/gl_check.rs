//! Startup check for a known dead end: wgpu's GL backend does not present on
//! the proprietary NVIDIA driver under Wayland (the window stays blank or the
//! app hangs). The decision is a pure function so it can be tested with fake
//! adapters.

use raw_window_handle::{HasDisplayHandle, RawDisplayHandle};

/// NVIDIA's PCI vendor id.
const NVIDIA: u32 = 0x10de;

/// What we know about the adapter and the session.
#[derive(Clone, Debug)]
pub struct GlCheckInput<'a> {
    pub backend: wgpu::Backend,
    pub vendor: u32,
    pub name: &'a str,
    pub driver: &'a str,
    pub driver_info: &'a str,
    /// The window system is Wayland.
    pub wayland: bool,
    /// `WGPU_BACKEND` is set: the user chose the backend on purpose.
    pub backend_forced: bool,
}

/// What to do about it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlCheck {
    /// Nothing wrong.
    Fine,
    /// Re-create the instance with Vulkan only.
    UseVulkan,
    /// Keep going, but tell the user what is wrong and how to fix it.
    Warn,
}

/// The proprietary driver, not Mesa's nouveau (whose GL works on Wayland).
fn proprietary_nvidia(i: &GlCheckInput) -> bool {
    let mesa = [i.name, i.driver, i.driver_info].iter().any(|s| {
        let s = s.to_lowercase();
        s.contains("mesa") || s.contains("nouveau") || s.contains("nvk")
    });
    i.vendor == NVIDIA && !mesa
}

/// GL on proprietary NVIDIA under Wayland is broken. If the backend was not
/// forced, retry on Vulkan; if it was, only warn.
pub fn check_gl(i: &GlCheckInput) -> GlCheck {
    if i.backend != wgpu::Backend::Gl || !i.wayland || !proprietary_nvidia(i) {
        return GlCheck::Fine;
    }
    if i.backend_forced {
        GlCheck::Warn
    } else {
        GlCheck::UseVulkan
    }
}

/// The one-time message for [`GlCheck::Warn`] (and a failed fallback).
pub fn warning(adapter: &str) -> String {
    format!(
        "OpenGL on the proprietary NVIDIA driver ({adapter}) under Wayland does not present \
         (wgpu limitation): expect a blank window or a hang. Fix: unset WGPU_BACKEND (or set \
         WGPU_BACKEND=vulkan) to use Vulkan, or run on Mesa's software GL with \
         __EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json WGPU_BACKEND=gl"
    )
}

/// Whether the window runs on Wayland: the display handle if it says so,
/// else `WAYLAND_DISPLAY`.
pub fn is_wayland(handle: &impl HasDisplayHandle) -> bool {
    match handle.display_handle().map(|h| h.as_raw()) {
        Ok(RawDisplayHandle::Wayland(_)) => true,
        Ok(_) => false,
        Err(_) => std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty()),
    }
}

/// `WGPU_BACKEND` is set to something.
pub fn backend_forced() -> bool {
    std::env::var_os("WGPU_BACKEND").is_some_and(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(backend: wgpu::Backend, vendor: u32, name: &str) -> GlCheckInput<'_> {
        GlCheckInput {
            backend,
            vendor,
            name,
            driver: "",
            driver_info: "",
            wayland: true,
            backend_forced: false,
        }
    }

    #[test]
    fn nvidia_gl_on_wayland_falls_back_to_vulkan() {
        let i = input(
            wgpu::Backend::Gl,
            NVIDIA,
            "NVIDIA GeForce RTX 3070/PCIe/SSE2",
        );
        assert_eq!(check_gl(&i), GlCheck::UseVulkan);
    }

    #[test]
    fn forced_backend_only_warns() {
        let mut i = input(
            wgpu::Backend::Gl,
            NVIDIA,
            "NVIDIA GeForce RTX 3070/PCIe/SSE2",
        );
        i.backend_forced = true;
        assert_eq!(check_gl(&i), GlCheck::Warn);
        assert!(warning(i.name).contains("WGPU_BACKEND"));
    }

    #[test]
    fn other_cases_are_fine() {
        let nv = "NVIDIA GeForce RTX 3070";
        // Vulkan on NVIDIA.
        assert_eq!(
            check_gl(&input(wgpu::Backend::Vulkan, NVIDIA, nv)),
            GlCheck::Fine
        );
        // X11 session.
        let mut x11 = input(wgpu::Backend::Gl, NVIDIA, nv);
        x11.wayland = false;
        assert_eq!(check_gl(&x11), GlCheck::Fine);
        // llvmpipe (the test path) and nouveau are Mesa.
        assert_eq!(
            check_gl(&input(
                wgpu::Backend::Gl,
                0x10005,
                "llvmpipe (LLVM 19.1.7, 256 bits)"
            )),
            GlCheck::Fine
        );
        assert_eq!(
            check_gl(&input(wgpu::Backend::Gl, NVIDIA, "Mesa NVE7 (nouveau)")),
            GlCheck::Fine
        );
        let mut mesa_driver = input(wgpu::Backend::Gl, NVIDIA, "NV167");
        mesa_driver.driver_info = "Mesa 25.1";
        assert_eq!(check_gl(&mesa_driver), GlCheck::Fine);
        // AMD GL on Wayland.
        assert_eq!(
            check_gl(&input(wgpu::Backend::Gl, 0x1002, "AMD Radeon")),
            GlCheck::Fine
        );
    }
}
