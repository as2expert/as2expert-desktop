//! Remote-session detection and software-OpenGL fallback.
//!
//! Remote desktop protocols (Windows RDP, and most VNC/VDI setups) only provide
//! OpenGL 1.1, so the modern GPU context egui needs cannot be created. When we
//! detect such a session — or the user forces it with `AS2EXPERT_SOFTWARE_GL=1`
//! — we switch to a software renderer instead. Local sessions are untouched and
//! keep hardware acceleration.

/// True if the app appears to be running inside a remote session.
pub fn is_remote_session() -> bool {
    #[cfg(windows)]
    {
        // SM_REMOTESESSION is set for RDP / Terminal Services sessions.
        unsafe { win::GetSystemMetrics(win::SM_REMOTESESSION) != 0 }
    }
    #[cfg(not(windows))]
    {
        // On Linux/macOS the reliable trigger is the retry-on-failure path in
        // main(); this pre-enables software GL for the common SSH-forwarded case.
        std::env::var_os("SSH_CONNECTION").is_some() || std::env::var_os("SSH_CLIENT").is_some()
    }
}

/// Force software OpenGL for the rest of the process. Idempotent.
pub fn enable_software_gl() {
    // Mesa (Linux, and the bundled Windows llvmpipe build) honor these.
    std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1");
    std::env::set_var("GALLIUM_DRIVER", "llvmpipe");

    #[cfg(windows)]
    win::load_bundled_software_gl();
}

#[cfg(windows)]
mod win {
    use std::os::windows::ffi::OsStrExt;

    pub const SM_REMOTESESSION: i32 = 0x1000;
    const LOAD_WITH_ALTERED_SEARCH_PATH: u32 = 0x0000_0008;

    #[link(name = "user32")]
    extern "system" {
        pub fn GetSystemMetrics(index: i32) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryExW(
            name: *const u16,
            file: *mut core::ffi::c_void,
            flags: u32,
        ) -> *mut core::ffi::c_void;
    }

    /// Preload the bundled Mesa llvmpipe `opengl32.dll` (shipped in a `mesa`
    /// folder next to the executable). Once loaded under the name `opengl32.dll`,
    /// the windowing layer reuses it instead of the system's 1.1 driver.
    ///
    /// `LOAD_WITH_ALTERED_SEARCH_PATH` makes the DLL's own directory the search
    /// root for its dependencies (libgallium_wgl.dll, etc.).
    pub fn load_bundled_software_gl() {
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        let Some(dir) = exe.parent() else {
            return;
        };
        let dll = dir.join("mesa").join("opengl32.dll");
        if !dll.is_file() {
            return;
        }
        let wide: Vec<u16> = dll
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        unsafe {
            LoadLibraryExW(
                wide.as_ptr(),
                core::ptr::null_mut(),
                LOAD_WITH_ALTERED_SEARCH_PATH,
            );
        }
    }
}
