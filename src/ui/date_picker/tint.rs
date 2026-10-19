//! Recolor the native date/calendar raster, not its geometry or input model.
//! Keeping the themed native renderer avoids the classic-calendar DPI clipping
//! caused by disabling visual styles at dropdown time.
use super::{Palette, Resolved, high_contrast};
use std::{cell::RefCell, mem::size_of, ptr::null_mut, rc::Rc};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::*,
    UI::WindowsAndMessaging::*,
};

const ID: usize = 0x4454_434c;
const MAX_PIXELS: usize = 4 * 1024 * 1024;

#[cfg(test)]
mod tests;
type SubclassProc =
    Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM, usize, usize) -> LRESULT>;
#[link(name = "comctl32")]
unsafe extern "system" {
    fn SetWindowSubclass(hwnd: HWND, proc: SubclassProc, id: usize, data: usize) -> i32;
    fn GetWindowSubclass(hwnd: HWND, proc: SubclassProc, id: usize, data: *mut usize) -> i32;
    fn RemoveWindowSubclass(hwnd: HWND, proc: SubclassProc, id: usize) -> i32;
    fn DefSubclassProc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT;
}

type State = Rc<RefCell<Presentation>>;
struct Presentation {
    colors: Rc<Colors>,
    painting: bool,
    redraw: bool,
    attached: bool,
}
struct Colors {
    neutral: [u32; 256],
    accent: [u32; 256],
    border: u32,
}
fn dib(color: u32) -> u32 {
    0xff00_0000 | ((color & 255) << 16) | (color & 0xff00) | ((color >> 16) & 255)
}
impl Colors {
    fn new(p: Palette) -> Self {
        let tones = |ink, power| {
            std::array::from_fn(|luma| {
                // Native light surfaces become dark; the nonlinear ramp keeps
                // native gray disabled text and white-on-blue selection readable.
                let weight = ((1.0 - luma as f64 / 255.0).powf(power) * 100.0).round() as u32;
                dib(super::super::theme::blend(p.surface, ink, weight))
            })
        };
        Self {
            neutral: tones(p.text, 0.65),
            accent: tones(p.accent, 0.5),
            border: p.border,
        }
    }
    fn pixel(&self, pixel: u32) -> u32 {
        let [b, g, r, _] = pixel.to_le_bytes();
        let luma = (u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114) / 1000;
        let chroma = r.max(g).max(b) - r.min(g).min(b);
        if chroma > 24 {
            self.accent[luma as usize]
        } else {
            self.neutral[luma as usize]
        }
    }
}

unsafe fn state(hwnd: HWND) -> Option<State> {
    // SAFETY: Only this proc/id owns the retained Rc. Clone before native calls,
    // so detach or destruction during synchronous notifications cannot free it.
    unsafe {
        let mut data = 0;
        if GetWindowSubclass(hwnd, Some(procedure), ID, &mut data) == 0 || data == 0 {
            return None;
        }
        let pointer = data as *const RefCell<Presentation>;
        Rc::increment_strong_count(pointer);
        Some(Rc::from_raw(pointer))
    }
}

pub(super) unsafe fn active(hwnd: HWND) -> bool {
    // SAFETY: Owner-thread lookup only; retains and releases a module-owned Rc.
    unsafe { state(hwnd).is_some() }
}

/// Only the date component calls this, on its own picker/calendar/child HWNDs.
pub(super) unsafe fn apply(hwnd: HWND, kind: Resolved, palette: Palette) {
    // SAFETY: Live owner-thread HWND. No RefCell borrow crosses native dispatch;
    // the subclass owns one Rc until successful removal or NCDESTROY.
    unsafe {
        let existing = state(hwnd);
        if kind != Resolved::Dark || high_contrast() {
            if let Some(state) = existing
                && RemoveWindowSubclass(hwnd, Some(procedure), ID) != 0
            {
                state.borrow_mut().attached = false;
                drop(Rc::from_raw(Rc::as_ptr(&state)));
                RedrawWindow(
                    hwnd,
                    std::ptr::null(),
                    null_mut(),
                    RDW_INVALIDATE | RDW_ERASE | RDW_FRAME,
                );
            }
            return;
        }
        if let Some(state) = existing {
            state.borrow_mut().colors = Rc::new(Colors::new(palette));
        } else {
            let state = Rc::new(RefCell::new(Presentation {
                colors: Rc::new(Colors::new(palette)),
                painting: false,
                redraw: true,
                attached: true,
            }));
            let data = Rc::into_raw(state);
            if SetWindowSubclass(hwnd, Some(procedure), ID, data as usize) == 0 {
                drop(Rc::from_raw(data));
                return;
            }
        }
        InvalidateRect(hwnd, std::ptr::null(), 0);
    }
}

struct Frame {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    bits: *mut u32,
    width: i32,
    height: i32,
}
impl Frame {
    unsafe fn new(width: i32, height: i32) -> Option<Self> {
        let length = usize::try_from(width)
            .ok()?
            .checked_mul(usize::try_from(height).ok()?)?;
        if length == 0 || length > MAX_PIXELS {
            return None;
        }
        // SAFETY: Checked dimensions and sized top-down 32-bit DIB. The frame
        // owns both GDI objects, restores selection before deleting either one.
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            if dc.is_null() {
                return None;
            }
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = null_mut();
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            if bitmap.is_null() || bits.is_null() {
                if !bitmap.is_null() {
                    DeleteObject(bitmap);
                }
                DeleteDC(dc);
                return None;
            }
            let previous = SelectObject(dc, bitmap);
            if previous.is_null() || previous as isize == -1 {
                DeleteObject(bitmap);
                DeleteDC(dc);
                return None;
            }
            let frame = Self {
                dc,
                bitmap,
                previous,
                bits: bits.cast(),
                width,
                height,
            };
            std::slice::from_raw_parts_mut(frame.bits, length).fill(dib(GetSysColor(COLOR_WINDOW)));
            Some(frame)
        }
    }
    unsafe fn recolor(&self, colors: &Colors) {
        // SAFETY: Frame owns this sized DIB; flush queued GDI writes before CPU
        // access. The bitmap stays selected only in this private memory DC.
        unsafe {
            GdiFlush();
            let pixels =
                std::slice::from_raw_parts_mut(self.bits, (self.width * self.height) as usize);
            for pixel in pixels {
                *pixel = colors.pixel(*pixel);
            }
        }
    }
}
impl Drop for Frame {
    fn drop(&mut self) {
        // SAFETY: Unique frame-owned DC/bitmap, detached before deletion.
        unsafe {
            SelectObject(self.dc, self.previous);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}
struct Painting(State);
impl Painting {
    fn begin(state: &State) -> Option<Self> {
        let mut value = state.borrow_mut();
        if value.painting || !value.attached {
            return None;
        }
        value.painting = true;
        Some(Self(Rc::clone(state)))
    }
}
impl Drop for Painting {
    fn drop(&mut self) {
        self.0.borrow_mut().painting = false;
    }
}
struct SavedDc(HDC, i32);
impl SavedDc {
    unsafe fn new(dc: HDC) -> Option<Self> {
        if dc.is_null() {
            return None;
        }
        // SAFETY: Caller supplies a live borrowed DC, which outlives this guard.
        let index = unsafe { SaveDC(dc) };
        (index != 0).then_some(Self(dc, index))
    }
}
impl Drop for SavedDc {
    fn drop(&mut self) {
        // SAFETY: Successful SaveDC on a still-live borrowed DC.
        unsafe { RestoreDC(self.0, self.1) };
    }
}

unsafe fn render(
    hwnd: HWND,
    message: u32,
    flags: LPARAM,
    colors: &Colors,
    whole: bool,
) -> Option<Frame> {
    // SAFETY: Live owner-thread HWND. painting guards nested WM_PRINTCLIENT/
    // erase callbacks, forwarding them untouched to the native renderer. No
    // application data, screen pixels, geometry or input state is replaced.
    unsafe {
        let mut rect = RECT::default();
        let ok = if whole {
            GetWindowRect(hwnd, &mut rect)
        } else {
            GetClientRect(hwnd, &mut rect)
        };
        if ok == 0 {
            return None;
        }
        let frame = Frame::new(rect.right - rect.left, rect.bottom - rect.top)?;
        {
            let _saved = SavedDc::new(frame.dc)?;
            DefSubclassProc(hwnd, message, frame.dc as WPARAM, flags);
        }
        frame.recolor(colors);
        Some(frame)
    }
}
unsafe fn print_frame(hwnd: HWND, dc: HDC, flags: LPARAM, frame: &Frame) -> bool {
    // SAFETY: Live owner-thread HWND and borrowed print DC. Window-relative
    // rectangles remain in the caller's logical mapping; only our temporary
    // clip changes, and it is restored before any native fallback.
    unsafe {
        let mut outer = RECT::default();
        let mut client = RECT::default();
        let mut origin = POINT::default();
        if GetWindowRect(hwnd, &mut outer) == 0
            || GetClientRect(hwnd, &mut client) == 0
            || ClientToScreen(hwnd, &mut origin) == 0
            || outer.right - outer.left != frame.width
            || outer.bottom - outer.top != frame.height
        {
            return false;
        }
        let client = RECT {
            left: origin.x - outer.left + client.left,
            top: origin.y - outer.top + client.top,
            right: origin.x - outer.left + client.right,
            bottom: origin.y - outer.top + client.bottom,
        };
        let Some(_saved) = SavedDc::new(dc) else {
            return false;
        };
        let mut clip = IntersectClipRect(dc, 0, 0, frame.width, frame.height);
        if clip == ERROR {
            return false;
        }
        let print_client = flags & (PRF_CLIENT | PRF_ERASEBKGND) as LPARAM != 0;
        let print_nonclient = flags & PRF_NONCLIENT as LPARAM != 0;
        if print_client && !print_nonclient {
            clip = IntersectClipRect(dc, client.left, client.top, client.right, client.bottom);
        } else if print_nonclient && !print_client {
            clip = ExcludeClipRect(dc, client.left, client.top, client.right, client.bottom);
        }
        clip != ERROR
            && (clip == NULLREGION
                || BitBlt(dc, 0, 0, frame.width, frame.height, frame.dc, 0, 0, SRCCOPY) != 0)
    }
}

unsafe fn border(hwnd: HWND, color: u32) {
    // SAFETY: Borrowed window DC; compute actual native nonclient bands rather
    // than inventing border widths or changing styles. Never paint the client.
    unsafe {
        let mut outer = RECT::default();
        let mut client = RECT::default();
        let mut origin = POINT::default();
        if GetWindowRect(hwnd, &mut outer) == 0
            || GetClientRect(hwnd, &mut client) == 0
            || ClientToScreen(hwnd, &mut origin) == 0
        {
            return;
        }
        let dc = GetWindowDC(hwnd);
        if dc.is_null() {
            return;
        }
        if let Some(_saved) = SavedDc::new(dc) {
            ExcludeClipRect(
                dc,
                origin.x - outer.left,
                origin.y - outer.top,
                origin.x - outer.left + client.right,
                origin.y - outer.top + client.bottom,
            );
            SetDCBrushColor(dc, color);
            FillRect(
                dc,
                &RECT {
                    left: 0,
                    top: 0,
                    right: outer.right - outer.left,
                    bottom: outer.bottom - outer.top,
                },
                GetStockObject(DC_BRUSH),
            );
        }
        ReleaseDC(hwnd, dc);
    }
}
unsafe extern "system" fn procedure(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    _: usize,
    data: usize,
) -> LRESULT {
    // SAFETY: Owner-thread retained subclass Rc. All borrows end before calling
    // native code, which may synchronously detach/destroy this HWND.
    unsafe {
        let pointer = data as *const RefCell<Presentation>;
        Rc::increment_strong_count(pointer);
        let state = Rc::from_raw(pointer);
        if message == WM_NCDESTROY {
            state.borrow_mut().attached = false;
            RemoveWindowSubclass(hwnd, Some(procedure), ID);
            drop(Rc::from_raw(pointer));
            return DefSubclassProc(hwnd, message, w, l);
        }
        if message == WM_SETREDRAW {
            state.borrow_mut().redraw = w != 0;
            return DefSubclassProc(hwnd, message, w, l);
        }
        if message == WM_PRINT
            && (l == 0 || (l & PRF_CHECKVISIBLE as LPARAM != 0 && IsWindowVisible(hwnd) == 0))
        {
            return 0;
        }
        let (busy, redraw, colors) = {
            let value = state.borrow();
            (value.painting, value.redraw, Rc::clone(&value.colors))
        };
        if busy
            || !redraw
            || !matches!(
                message,
                WM_PAINT | WM_PRINT | WM_PRINTCLIENT | WM_ERASEBKGND | WM_NCPAINT
            )
            || high_contrast()
        {
            return DefSubclassProc(hwnd, message, w, l);
        }
        match message {
            WM_ERASEBKGND => 1,
            WM_PAINT => {
                let Some(_painting) = Painting::begin(&state) else {
                    return DefSubclassProc(hwnd, message, w, l);
                };
                let Some(frame) = render(
                    hwnd,
                    WM_PRINTCLIENT,
                    (PRF_CLIENT | PRF_ERASEBKGND | PRF_CHILDREN) as LPARAM,
                    &colors,
                    false,
                ) else {
                    return DefSubclassProc(hwnd, message, w, l);
                };
                if !state.borrow().attached {
                    return 0;
                }
                if w != 0 {
                    if let Some(_saved) = SavedDc::new(w as HDC) {
                        BitBlt(
                            w as HDC,
                            0,
                            0,
                            frame.width,
                            frame.height,
                            frame.dc,
                            0,
                            0,
                            SRCCOPY,
                        );
                    }
                } else {
                    let mut paint = PAINTSTRUCT::default();
                    let dc = BeginPaint(hwnd, &mut paint);
                    if !dc.is_null() {
                        BitBlt(dc, 0, 0, frame.width, frame.height, frame.dc, 0, 0, SRCCOPY);
                    }
                    EndPaint(hwnd, &paint);
                }
                0
            }
            WM_PRINT | WM_PRINTCLIENT if w != 0 => {
                let Some(_saved) = SavedDc::new(w as HDC) else {
                    return 0;
                };
                // Child/owned-only printing needs its native traversal, not a
                // filled parent raster. PRINTCLIENT retains its native flags.
                if message == WM_PRINT
                    && l & (PRF_CLIENT | PRF_ERASEBKGND | PRF_NONCLIENT) as LPARAM == 0
                {
                    return DefSubclassProc(hwnd, message, w, l);
                }
                let Some(_painting) = Painting::begin(&state) else {
                    return DefSubclassProc(hwnd, message, w, l);
                };
                if let Some(frame) = render(hwnd, message, l, &colors, message == WM_PRINT) {
                    if !state.borrow().attached {
                        return 0;
                    }
                    if message == WM_PRINT {
                        if !print_frame(hwnd, w as HDC, l, &frame) {
                            return DefSubclassProc(hwnd, message, w, l);
                        }
                    } else {
                        BitBlt(
                            w as HDC,
                            0,
                            0,
                            frame.width,
                            frame.height,
                            frame.dc,
                            0,
                            0,
                            SRCCOPY,
                        );
                    }
                    0
                } else {
                    DefSubclassProc(hwnd, message, w, l)
                }
            }
            WM_NCPAINT => {
                let result = DefSubclassProc(hwnd, message, w, l);
                if state.borrow().attached {
                    border(hwnd, colors.border);
                }
                result
            }
            _ => DefSubclassProc(hwnd, message, w, l),
        }
    }
}
