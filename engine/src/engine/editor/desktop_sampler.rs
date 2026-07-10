//! Desktop-wide pixel sampling for the crusty eyedropper (Win32 GDI).
//!
//! winit only delivers input while the pointer is over our window, but the
//! eyedropper must pick colors anywhere on the desktop — so while armed we
//! BitBlt the pixels around the global cursor every frame and edge-detect
//! pick/cancel clicks with `GetAsyncKeyState`.

use crusty_gui::widgets::EyedropperState;

/// Per-frame driver for an armed [`EyedropperState`].
#[derive(Default)]
pub struct DesktopSampler {
    lbutton_down: bool,
    rbutton_down: bool,
    escape_down: bool,
}

impl DesktopSampler {
    /// Call once per frame. No-op (state reset) unless `eye.active`.
    pub fn update(&mut self, eye: &mut EyedropperState) {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            GetAsyncKeyState, VK_ESCAPE, VK_LBUTTON, VK_RBUTTON,
        };

        let down = |vk: u16| unsafe { (GetAsyncKeyState(vk as i32) as u16) & 0x8000 != 0 };
        let (l, r, esc) = (down(VK_LBUTTON), down(VK_RBUTTON), down(VK_ESCAPE));

        if !eye.active {
            // Keep tracking so the click that *arms* the dropper (or a held
            // button) doesn't count as a pick on the first armed frame.
            (self.lbutton_down, self.rbutton_down, self.escape_down) = (l, r, esc);
            return;
        }

        if l && !self.lbutton_down {
            eye.picked = true;
        }
        if (r && !self.rbutton_down) || (esc && !self.escape_down) {
            eye.cancelled = true;
        }
        (self.lbutton_down, self.rbutton_down, self.escape_down) = (l, r, esc);

        eye.patch.clear();
        sample_patch(eye);
    }
}

/// Fill `eye.patch` with the desktop pixels centered on the cursor.
fn sample_patch(eye: &mut EyedropperState) {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GdiFlush, GetDC,
        ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, SRCCOPY,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;

    let side = eye.patch_side as i32;
    if side <= 0 {
        return;
    }

    unsafe {
        let mut pt = POINT { x: 0, y: 0 };
        if GetCursorPos(&mut pt) == 0 {
            return;
        }
        let screen = GetDC(0);
        if screen == 0 {
            return;
        }
        let mem = CreateCompatibleDC(screen);
        if mem == 0 {
            ReleaseDC(0, screen);
            return;
        }

        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: side,
            biHeight: -side, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: 0, // BI_RGB
            ..std::mem::zeroed()
        };
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let dib = CreateDIBSection(mem, &info, DIB_RGB_COLORS, &mut bits, 0, 0);
        if dib != 0 && !bits.is_null() {
            let old = SelectObject(mem, dib);
            let half = side / 2;
            let ok = BitBlt(mem, 0, 0, side, side, screen, pt.x - half, pt.y - half, SRCCOPY);
            GdiFlush();
            if ok != 0 {
                // 32bpp DIB pixels are laid out B,G,R,X.
                let px = std::slice::from_raw_parts(bits as *const u8, (side * side * 4) as usize);
                eye.patch.reserve((side * side) as usize);
                for p in px.chunks_exact(4) {
                    eye.patch.push([p[2], p[1], p[0]]);
                }
            }
            SelectObject(mem, old);
            DeleteObject(dib);
        }
        DeleteDC(mem);
        ReleaseDC(0, screen);
    }
}
