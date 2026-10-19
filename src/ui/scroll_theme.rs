//! Non-client scrollbar presentation and dark-mode mouse tracking. Text, tables,
//! scroll ranges, selection, clipboard, keyboard/wheel input and MSAA stay native.
//!
//! Call `apply` on the owning UI thread after native/control theme selection and
//! again on theme/DPI changes. Only multiline Edit, report ListView and the known
//! viewport have mouse adapters; no styles, regions or client pixels are changed.
//! Light/high contrast retain native tracking. PrintWindow implementations which
//! omit WM_PRINT/PRF_NONCLIENT cannot include the non-client overlay.
#![cfg(windows)]

use super::theme::{Palette, Resolved};
use crate::windows::scale;
use std::{cell::Cell, mem::size_of, ptr::null, rc::Rc};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::*,
    UI::{Controls::*, WindowsAndMessaging::*},
};

const SUBCLASS_ID: usize = 0x5343_5448;
const TRACK_TIMER: usize = SUBCLASS_ID;
const REPEAT_DELAY: u32 = 400;
const REPEAT_INTERVAL: u32 = 50;
type SubclassProc =
    Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM, usize, usize) -> LRESULT>;
#[link(name = "comctl32")]
unsafe extern "system" {
    fn SetWindowSubclass(hwnd: HWND, proc: SubclassProc, id: usize, data: usize) -> i32;
    fn GetWindowSubclass(hwnd: HWND, proc: SubclassProc, id: usize, data: *mut usize) -> i32;
    fn RemoveWindowSubclass(hwnd: HWND, proc: SubclassProc, id: usize) -> i32;
    fn DefSubclassProc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT;
}

// Documented public user32 APIs behind otherwise unused windows-sys features.
#[repr(C)]
struct MouseTracking {
    size: u32,
    flags: u32,
    hwnd: HWND,
    hover_time: u32,
}
const TRACK_LEAVE: u32 = 2;
const TRACK_NONCLIENT: u32 = 0x10;
const TRACK_CANCEL: u32 = 0x8000_0000;
#[link(name = "user32")]
unsafe extern "system" {
    fn SetCapture(hwnd: HWND) -> HWND;
    fn GetCapture() -> HWND;
    fn ReleaseCapture() -> i32;
    fn IsWindowEnabled(hwnd: HWND) -> i32;

    fn TrackMouseEvent(event: *mut MouseTracking) -> i32;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    LineBefore,
    PageBefore,
    Thumb,
    PageAfter,
    LineAfter,
}
impl Part {
    fn index(self) -> usize {
        match self {
            Self::LineBefore => 1,
            Self::PageBefore => 2,
            Self::Thumb => 3,
            Self::PageAfter => 4,
            Self::LineAfter => 5,
        }
    }
    fn command(self) -> Option<i32> {
        match self {
            Self::LineBefore => Some(SB_LINEUP),
            Self::PageBefore => Some(SB_PAGEUP),
            Self::Thumb => None,
            Self::PageAfter => Some(SB_PAGEDOWN),
            Self::LineAfter => Some(SB_LINEDOWN),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Hit {
    vertical: bool,
    part: Part,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Driver {
    Edit,
    Report,
    Viewport,
}
#[derive(Clone, Copy)]
struct Tracking {
    generation: u64,
    hit: Hit,
    driver: Driver,
    point: POINT,
    grip: i32,
    start_axis: i32,
    start_position: i32,
    was_visible: bool,
    repeating: bool,
}

#[derive(Clone, Copy)]
struct Appearance {
    palette: Palette,
    dpi: u32,
    attached: bool,
    enabled: bool,
    painting: bool,
    redraw: bool,
    defer_paint: u32,
    hover: Option<Hit>,
    tracking_leave: bool,
    tracking: Option<Tracking>,
    tracking_generation: u64,
    timer: usize,
    keyboard_hook: HHOOK,
}
type State = Rc<Cell<Appearance>>;

struct PaintDeferral(State);
impl PaintDeferral {
    fn new(state: &State) -> Self {
        let mut value = state.get();
        value.defer_paint = value.defer_paint.saturating_add(1);
        state.set(value);
        Self(Rc::clone(state))
    }
}
impl Drop for PaintDeferral {
    fn drop(&mut self) {
        let mut value = self.0.get();
        value.defer_paint = value.defer_paint.saturating_sub(1);
        self.0.set(value);
    }
}

fn supported(class: &str, style: u32) -> bool {
    class.eq_ignore_ascii_case("SysListView32")
        || (class.eq_ignore_ascii_case("Static") && style & WS_VSCROLL != 0)
        || (class.eq_ignore_ascii_case("Edit")
            && style & ES_MULTILINE as u32 != 0
            && style & (WS_VSCROLL | WS_HSCROLL) != 0)
}

unsafe fn supported_window(hwnd: HWND) -> bool {
    // SAFETY: public queries tolerate an invalid HWND; class writes are bounded.
    unsafe {
        if IsWindow(hwnd) == 0 || GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & WS_CHILD == 0 {
            return false;
        }
        let mut name = [0u16; 64];
        let count = GetClassNameW(hwnd, name.as_mut_ptr(), name.len() as i32);
        if count <= 0 {
            return false;
        }
        let class = String::from_utf16_lossy(&name[..count as usize]);
        supported(&class, GetWindowLongPtrW(hwnd, GWL_STYLE) as u32)
            // SetScrollInfo removes WS_VSCROLL while short settings pages fit.
            // Keep this known host themed so a later page can restore its bar.
            || (class.eq_ignore_ascii_case("Static")
                && GetDlgCtrlID(hwnd) == 2400
                && GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_CONTROLPARENT != 0)
    }
}

unsafe fn state(hwnd: HWND) -> Option<State> {
    // SAFETY: only this proc/id installs data, owning one Rc until removal. The
    // lookup cannot call application code; retain before any native forwarding.
    unsafe {
        let mut data = 0;
        if GetWindowSubclass(hwnd, Some(procedure), SUBCLASS_ID, &mut data) == 0 || data == 0 {
            return None;
        }
        let pointer = data as *const Cell<Appearance>;
        Rc::increment_strong_count(pointer);
        Some(Rc::from_raw(pointer))
    }
}

/// End only this subclass's interaction before the shared host changes content.
///
/// # Safety
/// Call on the HWND's owning UI thread. Native selection capture and other
/// windows' capture are left alone; ReleaseCapture may synchronously reenter.
pub(super) unsafe fn cancel(hwnd: HWND) {
    // SAFETY: Retain our presentation state before releasing our own capture.
    unsafe {
        if let Some(state) = state(hwnd) {
            stop_tracking(hwnd, &state);
        }
    }
}

unsafe fn detach(hwnd: HWND, state: &State) {
    // SAFETY: owner thread and retained Rc. Disable even if removal fails; in
    // that case the installed owner remains alive until a later removal/destroy.
    unsafe {
        let mut value = state.get();
        value.enabled = false;
        state.set(value);
        stop_tracking(hwnd, state);
        value = state.get();
        if value.attached && RemoveWindowSubclass(hwnd, Some(procedure), SUBCLASS_ID) != 0 {
            value.attached = false;
            state.set(value);
            drop(Rc::from_raw(Rc::as_ptr(state)));
        }
    }
}

/// Theme native non-client scrollbars without changing their styles/range/page.
/// Light/high-contrast modes remove the subclass and request native frame paint.
/// Failed HWND/API/attachment queries leave native rendering in place.
///
/// # Safety
/// `hwnd` must belong to the calling UI thread. Call after native theme changes;
/// the caller must resolve high contrast to `Resolved::Contrast`. No application
/// state or borrowed palette is retained. Unsupported/invalid HWNDs are ignored.
pub(super) unsafe fn apply(hwnd: HWND, kind: Resolved, palette: Palette, dpi: u32) {
    // SAFETY: owner-thread contract; we only query native state and install/remove
    // our own subclass. RedrawWindow may reenter/destroy, so nothing is borrowed.
    unsafe {
        if IsWindow(hwnd) == 0 {
            return;
        }
        let previous = state(hwnd);
        if kind != Resolved::Dark || !supported_window(hwnd) {
            if let Some(previous) = previous {
                detach(hwnd, &previous);
                let value = previous.get();
                if IsWindow(hwnd) != 0 {
                    RedrawWindow(
                        hwnd,
                        null(),
                        std::ptr::null_mut(),
                        RDW_INVALIDATE
                            | RDW_FRAME
                            | RDW_NOCHILDREN
                            | if value.redraw && value.defer_paint == 0 {
                                RDW_UPDATENOW
                            } else {
                                0
                            },
                    );
                }
            }
            return;
        }
        let state = if let Some(state) = previous {
            stop_tracking(hwnd, &state);
            let mut value = state.get();
            if !value.attached || IsWindow(hwnd) == 0 {
                return;
            }
            value.palette = palette;
            value.dpi = dpi.clamp(96, 768);
            value.enabled = true;
            state.set(value);
            state
        } else {
            let state = Rc::new(Cell::new(Appearance {
                palette,
                dpi: dpi.clamp(96, 768),
                attached: true,
                enabled: true,
                painting: false,
                redraw: true,
                defer_paint: 0,
                hover: None,
                tracking_leave: false,
                tracking: None,
                tracking_generation: 0,
                timer: 0,
                keyboard_hook: std::ptr::null_mut(),
            }));
            let owner = Rc::into_raw(Rc::clone(&state));
            if SetWindowSubclass(hwnd, Some(procedure), SUBCLASS_ID, owner as usize) == 0 {
                drop(Rc::from_raw(owner));
                return;
            }
            state
        };
        if state.get().redraw && state.get().defer_paint == 0 {
            RedrawWindow(
                hwnd,
                null(),
                std::ptr::null_mut(),
                RDW_INVALIDATE | RDW_FRAME | RDW_UPDATENOW | RDW_NOCHILDREN,
            );
        } else {
            invalidate_frame(hwnd);
        }
    }
}

struct SavedDc(HDC, i32);
impl SavedDc {
    unsafe fn new(dc: HDC) -> Option<Self> {
        // SAFETY: borrowed live DC; a failed save means we must not modify it.
        let index = unsafe { SaveDC(dc) };
        (index != 0).then_some(Self(dc, index))
    }
}
impl Drop for SavedDc {
    fn drop(&mut self) {
        // SAFETY: matching successful SaveDC; DC outlives the guard.
        unsafe {
            RestoreDC(self.0, self.1);
        }
    }
}
struct WindowDc(HWND, HDC);
impl Drop for WindowDc {
    fn drop(&mut self) {
        // SAFETY: paired with GetWindowDC, never DeleteDC on a window DC.
        unsafe {
            ReleaseDC(self.0, self.1);
        }
    }
}

fn valid(rect: RECT) -> bool {
    rect.left < rect.right && rect.top < rect.bottom
}
fn intersection(a: RECT, b: RECT) -> RECT {
    RECT {
        left: a.left.max(b.left),
        top: a.top.max(b.top),
        right: a.right.min(b.right),
        bottom: a.bottom.min(b.bottom),
    }
}
fn offset(rect: RECT, origin: POINT) -> RECT {
    RECT {
        left: rect.left - origin.x,
        top: rect.top - origin.y,
        right: rect.right - origin.x,
        bottom: rect.bottom - origin.y,
    }
}

#[derive(Clone, Copy)]
struct Bar {
    rect: RECT,
    vertical: bool,
    button: i32,
    thumb_top: i32,
    thumb_bottom: i32,
    states: [u32; 6],
    scrollable: bool,
}
impl Bar {
    fn length(self) -> i32 {
        if self.vertical {
            self.rect.bottom - self.rect.top
        } else {
            self.rect.right - self.rect.left
        }
    }
    fn segment(self, start: i32, end: i32) -> RECT {
        let mut rect = self.rect;
        if self.vertical {
            rect.top += start;
            rect.bottom = self.rect.top + end;
        } else {
            rect.left += start;
            rect.right = self.rect.left + end;
        }
        rect
    }
}

unsafe fn bar(hwnd: HWND, vertical: bool, origin: POINT, bounds: RECT) -> Option<Bar> {
    // SAFETY: only standard non-client object IDs (not OBJID_CLIENT, which can
    // dispatch SBM_GETSCROLLBARINFO). Both structures are correctly sized.
    unsafe {
        let mut info = SCROLLBARINFO {
            cbSize: size_of::<SCROLLBARINFO>() as u32,
            ..Default::default()
        };
        if GetScrollBarInfo(
            hwnd,
            if vertical {
                OBJID_VSCROLL
            } else {
                OBJID_HSCROLL
            },
            &mut info,
        ) == 0
            || info.rgstate[0] & (STATE_SYSTEM_INVISIBLE | STATE_SYSTEM_OFFSCREEN) != 0
        {
            return None;
        }
        // rcScrollBar is in screen coordinates. Thumb offsets are relative to
        // the bar's top/left, NOT screen or window coordinates. dxyLineButton is
        // the native end-button extent, not the thumb length. The Learn member
        // descriptions are terse; native top/bottom tests below check both axes
        // at nonzero window origins rather than treating offsets as absolute.
        // https://learn.microsoft.com/windows/win32/api/winuser/ns-winuser-scrollbarinfo
        let rect = offset(info.rcScrollBar, origin);
        let clipped = intersection(rect, bounds);
        if !valid(rect)
            || clipped.left != rect.left
            || clipped.top != rect.top
            || clipped.right != rect.right
            || clipped.bottom != rect.bottom
        {
            return None;
        }
        let mut scroll = SCROLLINFO {
            cbSize: size_of::<SCROLLINFO>() as u32,
            fMask: SIF_RANGE | SIF_PAGE | SIF_POS | SIF_TRACKPOS,
            ..Default::default()
        };
        if GetScrollInfo(hwnd, if vertical { SB_VERT } else { SB_HORZ }, &mut scroll) == 0 {
            return None;
        }
        let range = i64::from(scroll.nMax) - i64::from(scroll.nMin) + 1;
        let mut result = Bar {
            rect,
            vertical,
            button: info.dxyLineButton,
            thumb_top: info.xyThumbTop,
            thumb_bottom: info.xyThumbBottom,
            states: info.rgstate,
            scrollable: range > i64::from(scroll.nPage.max(1)),
        };
        result.button = result.button.clamp(0, result.length() / 2);
        Some(result)
    }
}

#[derive(Clone, Copy)]
struct Range {
    min: i32,
    max: i32,
    page: u32,
    position: i32,
}
impl Range {
    fn end(self) -> i32 {
        (i64::from(self.max) - i64::from(self.page.saturating_sub(1)))
            .clamp(i64::from(self.min), i64::from(i32::MAX)) as i32
    }
    fn clamp(self, position: i64) -> i32 {
        position.clamp(i64::from(self.min), i64::from(self.end())) as i32
    }
}
unsafe fn range(hwnd: HWND, vertical: bool) -> Option<Range> {
    // SAFETY: sized output and public non-client scrollbar query only.
    unsafe {
        let mut info = SCROLLINFO {
            cbSize: size_of::<SCROLLINFO>() as u32,
            fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
            ..Default::default()
        };
        if GetScrollInfo(hwnd, if vertical { SB_VERT } else { SB_HORZ }, &mut info) == 0
            || info.nMax < info.nMin
        {
            return None;
        }
        Some(Range {
            min: info.nMin,
            max: info.nMax,
            page: info.nPage,
            position: info.nPos,
        })
    }
}
fn contains(rect: RECT, point: POINT) -> bool {
    point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
}
fn axis(bar: Bar, point: POINT) -> i32 {
    if bar.vertical {
        point.y.saturating_sub(bar.rect.top)
    } else {
        point.x.saturating_sub(bar.rect.left)
    }
}
fn hit(bar: Bar, point: POINT) -> Option<Hit> {
    if !contains(bar.rect, point) || !bar.scrollable {
        return None;
    }
    let position = axis(bar, point);
    let part = if position < bar.button {
        Part::LineBefore
    } else if position >= bar.length() - bar.button {
        Part::LineAfter
    } else if position < bar.thumb_top {
        Part::PageBefore
    } else if position < bar.thumb_bottom {
        Part::Thumb
    } else {
        Part::PageAfter
    };
    let unavailable = STATE_SYSTEM_UNAVAILABLE | STATE_SYSTEM_INVISIBLE | STATE_SYSTEM_OFFSCREEN;
    (bar.states[0] & unavailable == 0 && bar.states[part.index()] & unavailable == 0).then_some(
        Hit {
            vertical: bar.vertical,
            part,
        },
    )
}
fn thumb_position(bar: Bar, range: Range, point: POINT, grip: i32) -> i32 {
    let thumb = (bar.thumb_bottom - bar.thumb_top).max(0);
    let travel = (bar.length() - 2 * bar.button - thumb).max(0);
    if travel == 0 {
        return range.clamp(i64::from(range.position));
    }
    let pixel = (i64::from(axis(bar, point)) - i64::from(grip) - i64::from(bar.button))
        .clamp(0, i64::from(travel));
    let span = i64::from(range.end()) - i64::from(range.min);
    range.clamp(i64::from(range.min) + (pixel * span + i64::from(travel) / 2) / i64::from(travel))
}
// Edit's horizontal SIF_POS is pixels, whereas EM_LINESCROLL takes average-font
// characters. Round once in signed 64-bit arithmetic; never put a position in a
// WM_HSCROLL high word (which silently truncates at 65535).
fn horizontal_characters(target: i32, current: i32, width: i32) -> i32 {
    let delta = i64::from(target) - i64::from(current);
    let width = i64::from(width.max(1));
    // At exactly half a native character, keep the current lattice point.
    // Symmetric half-away rounding would alternate between adjacent positions
    // when the same mouse coordinate is delivered again (including button-up).
    let magnitude = (delta.abs() + (width - 1) / 2) / width;
    (magnitude * delta.signum()).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}
unsafe fn screen_bar(hwnd: HWND, vertical: bool) -> Option<Bar> {
    // SAFETY: public geometry query. Using screen bounds keeps hit testing and
    // capture coordinates in the same space even outside the client rectangle.
    unsafe {
        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect) == 0 {
            return None;
        }
        bar(hwnd, vertical, POINT::default(), rect)
    }
}
unsafe fn driver(hwnd: HWND) -> Option<Driver> {
    // SAFETY: bounded class/style queries; no user-owned State is accessed.
    unsafe {
        let mut name = [0u16; 64];
        let count = GetClassNameW(hwnd, name.as_mut_ptr(), name.len() as i32);
        if count <= 0 {
            return None;
        }
        let class = String::from_utf16_lossy(&name[..count as usize]);
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        if class.eq_ignore_ascii_case("Edit") && style & ES_MULTILINE as u32 != 0 {
            Some(Driver::Edit)
        } else if class.eq_ignore_ascii_case("SysListView32") && style & LVS_TYPEMASK == LVS_REPORT
        {
            Some(Driver::Report)
        } else if class.eq_ignore_ascii_case("Static")
            && GetDlgCtrlID(hwnd) == 2400
            && GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_CONTROLPARENT != 0
        {
            Some(Driver::Viewport)
        } else {
            None
        }
    }
}
unsafe fn edit_character_width(hwnd: HWND, interaction: Option<(&State, Tracking)>) -> Option<i32> {
    // SAFETY: borrowed native font, selected only into this acquired DC. SavedDc
    // restores the selected object before WindowDc releases the DC, on all exits.
    unsafe {
        let font = SendMessageW(hwnd, WM_GETFONT, 0, 0) as HGDIOBJ;
        if IsWindow(hwnd) == 0
            || interaction.is_some_and(|(state, tracking)| !same_tracking(hwnd, state, tracking))
        {
            return None;
        }
        let dc = GetDC(hwnd);
        if dc.is_null() {
            return None;
        }
        let _dc = WindowDc(hwnd, dc);
        let _saved = SavedDc::new(dc)?;
        // WM_GETFONT returns NULL for Edit's SYSTEM_FONT, not the font which
        // happens to be selected in an acquired/cached DC after native painting.
        let font = if font.is_null() {
            GetStockObject(SYSTEM_FONT)
        } else {
            font
        };
        if font.is_null() || SelectObject(dc, font).is_null() {
            return None;
        }
        let mut metrics = TEXTMETRICW::default();
        if GetTextMetricsW(dc, &mut metrics) == 0 || metrics.tmAveCharWidth <= 0 {
            return None;
        }
        if metrics.tmPitchAndFamily & TMPF_FIXED_PITCH == 0 {
            return Some(metrics.tmAveCharWidth);
        }
        // Native Edit's proportional-font unit is the dialog-style rounded
        // alphabet average, not TEXTMETRIC's (usually 'x') tmAveCharWidth.
        // Reproduce it with public GDI, without moving text to probe the unit.
        let alphabet: Vec<u16> = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz"
            .encode_utf16()
            .collect();
        let mut extent = windows_sys::Win32::Foundation::SIZE::default();
        if GetTextExtentPoint32W(dc, alphabet.as_ptr(), alphabet.len() as i32, &mut extent) == 0 {
            return None;
        }
        let width = ((i64::from(extent.cx) / 26 + 1) / 2) as i32;
        (width > 0).then_some(width)
    }
}
unsafe fn report_line_height(hwnd: HWND, state: &State, tracking: Tracking) -> Option<i32> {
    // SAFETY: LVM_GETITEMRECT writes a synchronous stack RECT. Report SIF_POS is
    // a row index, but LVM_SCROLL takes pixels, rounded to whole rows by native.
    // Revalidate between synchronous queries, before using their results.
    unsafe {
        let top = SendMessageW(hwnd, LVM_GETTOPINDEX, 0, 0);
        if !same_tracking(hwnd, state, tracking) {
            return None;
        }
        let mut rect = RECT {
            left: LVIR_BOUNDS as i32,
            ..Default::default()
        };
        if SendMessageW(
            hwnd,
            LVM_GETITEMRECT,
            top as WPARAM,
            (&mut rect as *mut RECT) as LPARAM,
        ) == 0
            || !same_tracking(hwnd, state, tracking)
        {
            return None;
        }
        (rect.bottom > rect.top).then_some(rect.bottom.saturating_sub(rect.top))
    }
}
unsafe fn drive(hwnd: HWND, state: &State, driver: Driver, vertical: bool, target: i32) {
    // SAFETY: owner-thread adapters send documented messages to live controls.
    // No synthetic SetScrollInfo/track position and no 16-bit thumb message is
    // used. Synchronous queries/notifications may cancel and even start another
    // capture on the same live HWND; only the original interaction may continue.
    unsafe {
        let Some((tracking, _)) = tracking_bar(hwnd, state) else {
            return;
        };
        if tracking.driver != driver || tracking.hit.vertical != vertical {
            return;
        }
        let Some(info) = range(hwnd, vertical) else {
            return;
        };
        let target = info.clamp(i64::from(target));
        if target == info.position || !same_tracking(hwnd, state, tracking) {
            return;
        }
        match driver {
            Driver::Viewport if vertical => super::viewport::scroll_to(hwnd, target),
            Driver::Edit if vertical => {
                let first = SendMessageW(hwnd, EM_GETFIRSTVISIBLELINE, 0, 0) as i32;
                if same_tracking(hwnd, state, tracking) {
                    let delta = (i64::from(target) - i64::from(first))
                        .clamp(i64::from(i32::MIN), i64::from(i32::MAX));
                    SendMessageW(hwnd, EM_LINESCROLL, 0, delta as LPARAM);
                }
            }
            Driver::Edit => {
                if target == info.min || target == info.end() {
                    SendMessageW(
                        hwnd,
                        WM_HSCROLL,
                        if target == info.min {
                            SB_LEFT
                        } else {
                            SB_RIGHT
                        } as WPARAM,
                        0,
                    );
                } else if let Some(width) = edit_character_width(hwnd, Some((state, tracking)))
                    && same_tracking(hwnd, state, tracking)
                {
                    let mut delta = horizontal_characters(target, info.position, width);
                    // EM_LINESCROLL can pan beyond the longest line. Keep the
                    // quantized character step inside the native scrollbar range.
                    let next = i64::from(info.position) + i64::from(delta) * i64::from(width);
                    if next > i64::from(info.end()) {
                        delta -= 1;
                    } else if next < i64::from(info.min) {
                        delta += 1;
                    }
                    SendMessageW(hwnd, EM_LINESCROLL, delta as WPARAM, 0);
                }
            }
            Driver::Report => {
                let mut current = info.position;
                let unit = if vertical {
                    report_line_height(hwnd, state, tracking)
                } else {
                    Some(1)
                };
                let Some(unit) = unit else { return };
                if !same_tracking(hwnd, state, tracking) {
                    return;
                }
                let mut pixels = (i64::from(target) - i64::from(current)) * i64::from(unit);
                if vertical && i32::try_from(pixels).is_err() {
                    // A huge owner-data list can exceed a signed pixel delta.
                    // Reveal the full-width row index first, then move at most
                    // one native page to make that row the top visible row.
                    SendMessageW(hwnd, LVM_ENSUREVISIBLE, target as WPARAM, 0);
                    if !same_tracking(hwnd, state, tracking) {
                        return;
                    }
                    let Some(info) = range(hwnd, true) else {
                        return;
                    };
                    current = info.position;
                    pixels = (i64::from(info.clamp(i64::from(target))) - i64::from(current))
                        * i64::from(unit);
                }
                if same_tracking(hwnd, state, tracking)
                    && let Ok(pixels) = i32::try_from(pixels)
                {
                    SendMessageW(
                        hwnd,
                        LVM_SCROLL,
                        if vertical { 0 } else { pixels as WPARAM },
                        if vertical { pixels as LPARAM } else { 0 },
                    );
                }
            }
            Driver::Viewport => {}
        }
    }
}
unsafe fn invalidate_frame(hwnd: HWND) {
    // SAFETY: queues damage only; never dispatches painting inside a no-redraw
    // layout/geometry notification. Native WM_PAINT/NCPAINT runs before overlay.
    unsafe {
        let mut window = RECT::default();
        let mut client = RECT::default();
        let mut origin = POINT::default();
        if GetWindowRect(hwnd, &mut window) == 0
            || GetClientRect(hwnd, &mut client) == 0
            || ClientToScreen(hwnd, &mut origin) == 0
        {
            return;
        }
        let window = offset(window, origin);
        // RedrawWindow rectangles use client coordinates, including negative NC
        // coordinates. Four strips exclude the client without allocating a region
        // or forcing a client paint for EM_SETRECTNP/no-redraw geometry changes.
        for rect in [
            RECT {
                bottom: client.top,
                ..window
            },
            RECT {
                top: client.bottom,
                ..window
            },
            RECT {
                right: client.left,
                top: client.top,
                bottom: client.bottom,
                ..window
            },
            RECT {
                left: client.right,
                top: client.top,
                bottom: client.bottom,
                ..window
            },
        ] {
            if valid(rect) {
                RedrawWindow(
                    hwnd,
                    &rect,
                    std::ptr::null_mut(),
                    RDW_INVALIDATE | RDW_FRAME | RDW_NOCHILDREN,
                );
            }
        }
    }
}
unsafe fn track_leave(hwnd: HWND, cancel: bool) -> bool {
    // SAFETY: documented TRACKMOUSEEVENT layout, owner-thread HWND, no callbacks.
    unsafe {
        let mut event = MouseTracking {
            size: size_of::<MouseTracking>() as u32,
            flags: TRACK_LEAVE | TRACK_NONCLIENT | if cancel { TRACK_CANCEL } else { 0 },
            hwnd,
            hover_time: 0,
        };
        TrackMouseEvent(&mut event) != 0
    }
}
unsafe fn stop_tracking(hwnd: HWND, state: &State) {
    // SAFETY: clear scalar state before ReleaseCapture's synchronous callback.
    // Only release capture/timer owned by this interaction, never Edit selection.
    unsafe {
        let mut value = state.get();
        let tracking = value.tracking.take().is_some();
        let timer = value.timer;
        let leave = value.tracking_leave;
        let hook = value.keyboard_hook;
        let changed = tracking || value.hover.is_some();
        value.timer = 0;
        value.keyboard_hook = std::ptr::null_mut();
        value.hover = None;
        value.tracking_leave = false;
        state.set(value);
        if timer != 0 {
            KillTimer(hwnd, timer);
        }
        if !hook.is_null() {
            UnhookWindowsHookEx(hook);
        }
        if leave {
            track_leave(hwnd, true);
        }
        if tracking && GetCapture() == hwnd {
            ReleaseCapture();
        }
        let value = state.get();
        if changed && value.attached && value.enabled && value.redraw && IsWindow(hwnd) != 0 {
            invalidate_frame(hwnd);
        }
    }
}
unsafe fn tracking_bar(hwnd: HWND, state: &State) -> Option<(Tracking, Bar)> {
    // SAFETY: scalar state is copied before public HWND/capture/geometry queries.
    unsafe {
        let value = state.get();
        let tracking = value.tracking?;
        if !value.attached
            || !value.enabled
            || !value.redraw
            || value.defer_paint != 0
            || GetCapture() != hwnd
            || IsWindowEnabled(hwnd) == 0
            || GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & WS_VISIBLE == 0
            || (tracking.was_visible && IsWindowVisible(hwnd) == 0)
            || driver(hwnd) != Some(tracking.driver)
        {
            return None;
        }
        let bar = screen_bar(hwnd, tracking.hit.vertical)?;
        let unavailable =
            STATE_SYSTEM_UNAVAILABLE | STATE_SYSTEM_INVISIBLE | STATE_SYSTEM_OFFSCREEN;
        (bar.scrollable
            && (bar.states[0] | bar.states[tracking.hit.part.index()]) & unavailable == 0)
            .then_some((tracking, bar))
    }
}
unsafe fn same_tracking(hwnd: HWND, state: &State, expected: Tracking) -> bool {
    // SAFETY: owner-thread scalar identity plus public capture/geometry queries.
    // A canceled/restarted capture can have the same HWND, axis and grip; its
    // generation must differ before any stale query result can drive scrolling.
    unsafe {
        tracking_bar(hwnd, state).is_some_and(|(current, _)| {
            current.generation == expected.generation
                && current.hit == expected.hit
                && current.driver == expected.driver
        })
    }
}
unsafe fn repeat_step(hwnd: HWND, state: &State) -> bool {
    // SAFETY: native line/page commands carry no (16-bit) thumb position. Keep
    // their control-specific increments; re-hit-test so paging stops at the mouse.
    unsafe {
        let Some((tracking, bar)) = tracking_bar(hwnd, state) else {
            stop_tracking(hwnd, state);
            return false;
        };
        if tracking.hit.part == Part::Thumb || hit(bar, tracking.point) != Some(tracking.hit) {
            return false;
        }
        let Some(range) = range(hwnd, tracking.hit.vertical) else {
            return false;
        };
        let Some(command) = tracking.hit.part.command() else {
            return false;
        };
        let before = matches!(tracking.hit.part, Part::LineBefore | Part::PageBefore);
        if !same_tracking(hwnd, state, tracking)
            || (before && range.position <= range.min)
            || (!before && range.position >= range.end())
        {
            return false;
        }
        SendMessageW(
            hwnd,
            if tracking.hit.vertical {
                WM_VSCROLL
            } else {
                WM_HSCROLL
            },
            command as WPARAM,
            0,
        );
        true
    }
}
unsafe extern "system" fn tracking_keyboard(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: thread-local (not global) hook, installed only during our capture.
    // NC input must not steal focus/caret just to receive Escape: the focused
    // HWND may be a sibling, and IsDialogMessage can consume it before dispatch.
    // The hook retains our Rc and owns no borrowed app state. A synchronous stop
    // unhooks itself safely; all non-Escape keys continue through the hook chain.
    unsafe {
        if code == HC_ACTION as i32 && w == 0x1b && l as usize & 0x8000_0000 == 0 {
            let hwnd = GetCapture();
            if let Some(state) = state(hwnd)
                && state.get().tracking.is_some()
            {
                stop_tracking(hwnd, &state);
                overlay(hwnd, &state, std::ptr::null_mut());
                return 1;
            }
        }
        CallNextHookEx(std::ptr::null_mut(), code, w, l)
    }
}
unsafe fn begin_tracking(hwnd: HWND, state: &State, vertical: bool, point: POINT) -> bool {
    // SAFETY: only supported dark non-client bars are intercepted. Capture is
    // explicit, not DefSubclassProc's modal scrollbar loop; timers exist only
    // until this interaction ends (repeat and capture/disabled/hidden checks).
    unsafe {
        let Some(driver) = driver(hwnd) else {
            return false;
        };
        if matches!(driver, Driver::Viewport) && !vertical {
            return false;
        }
        let Some(bar) = screen_bar(hwnd, vertical) else {
            return false;
        };
        if !contains(bar.rect, point) {
            return false;
        }
        stop_tracking(hwnd, state);
        let Some(hit) = hit(bar, point) else {
            return true;
        };
        let Some(range) = range(hwnd, vertical) else {
            return true;
        };
        let mut value = state.get();
        if !value.attached
            || !value.enabled
            || !value.redraw
            || value.defer_paint != 0
            || IsWindowEnabled(hwnd) == 0
        {
            return true;
        }
        value.tracking_generation = value.tracking_generation.wrapping_add(1);
        value.tracking = Some(Tracking {
            generation: value.tracking_generation,
            hit,
            driver,
            point,
            grip: axis(bar, point) - bar.thumb_top,
            start_axis: axis(bar, point),
            start_position: range.position,
            was_visible: IsWindowVisible(hwnd) != 0,
            repeating: false,
        });
        value.hover = Some(hit);
        state.set(value);
        let Some(tracking) = value.tracking else {
            return true;
        };
        SetCapture(hwnd);
        if !same_tracking(hwnd, state, tracking) {
            if state
                .get()
                .tracking
                .is_some_and(|current| current.generation == tracking.generation)
            {
                stop_tracking(hwnd, state);
            }
            return true;
        }
        let hook = SetWindowsHookExW(
            WH_KEYBOARD,
            Some(tracking_keyboard),
            std::ptr::null_mut(),
            windows_sys::Win32::System::Threading::GetCurrentThreadId(),
        );
        let mut value = state.get();
        value.keyboard_hook = hook;
        state.set(value);
        if hook.is_null() {
            stop_tracking(hwnd, state);
            return true;
        }
        let timer = SetTimer(
            hwnd,
            TRACK_TIMER,
            if hit.part == Part::Thumb {
                REPEAT_INTERVAL
            } else {
                REPEAT_DELAY
            },
            None,
        );
        let mut value = state.get();
        value.timer = timer;
        state.set(value);
        if timer == 0 {
            stop_tracking(hwnd, state);
            return true;
        }
        repeat_step(hwnd, state);
        true
    }
}
unsafe fn move_tracking(hwnd: HWND, state: &State, point: POINT) {
    // SAFETY: retain no Cell borrow across drive()/native notifications. A click
    // without motion keeps the exact native position despite pixel quantization.
    unsafe {
        let Some((mut tracking, bar)) = tracking_bar(hwnd, state) else {
            stop_tracking(hwnd, state);
            return;
        };
        tracking.point = point;
        let mut value = state.get();
        value.tracking = Some(tracking);
        value.hover = hit(bar, point);
        state.set(value);
        if tracking.hit.part == Part::Thumb
            && let Some(range) = range(hwnd, tracking.hit.vertical)
        {
            let target = if axis(bar, point) == tracking.start_axis {
                range.clamp(i64::from(tracking.start_position))
            } else {
                thumb_position(bar, range, point, tracking.grip)
            };
            drive(hwnd, state, tracking.driver, tracking.hit.vertical, target);
        }
    }
}
unsafe fn update_hover(hwnd: HWND, state: &State, point: POINT) -> bool {
    // SAFETY: non-client geometry queries only. Leave tracking has no timer and
    // does not steal native client capture/focus or selection/caret processing.
    unsafe {
        if driver(hwnd).is_none() {
            return false;
        }
        let mut hovered = None;
        let mut on_bar = false;
        for vertical in [true, false] {
            if let Some(bar) = screen_bar(hwnd, vertical)
                && contains(bar.rect, point)
            {
                on_bar = true;
                if IsWindowEnabled(hwnd) != 0 {
                    hovered = hit(bar, point);
                }
                break;
            }
        }
        let mut value = state.get();
        if !value.tracking_leave && on_bar {
            value.tracking_leave = track_leave(hwnd, false);
        }
        value.hover = hovered;
        state.set(value);
        on_bar
    }
}
fn mouse_point(l: LPARAM) -> POINT {
    POINT {
        x: (l as u16 as i16) as i32,
        y: ((l as usize >> 16) as u16 as i16) as i32,
    }
}
fn geometry_message(message: u32) -> bool {
    matches!(
        message,
        WM_SIZE
            | WM_NCCALCSIZE
            | WM_WINDOWPOSCHANGING
            | WM_WINDOWPOSCHANGED
            | WM_STYLECHANGING
            | WM_STYLECHANGED
            | WM_THEMECHANGED
            | WM_SYSCOLORCHANGE
            | WM_SETTINGCHANGE
            | WM_DPICHANGED
            | WM_DPICHANGED_AFTERPARENT
            | WM_SHOWWINDOW
            | WM_SETFONT
            | WM_SETREDRAW
            | EM_SETRECT
            | EM_SETRECTNP
    )
}

unsafe fn mouse_input(
    hwnd: HWND,
    state: &State,
    message: u32,
    w: WPARAM,
    l: LPARAM,
) -> Option<bool> {
    // SAFETY: only scrollbar NC input or our own capture/timer is consumed. All
    // client input outside that capture continues through the native control.
    unsafe {
        let value = state.get();
        if !value.attached || !value.enabled {
            return None;
        }
        match message {
            WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK
                if w == HTVSCROLL as WPARAM || w == HTHSCROLL as WPARAM =>
            {
                begin_tracking(hwnd, state, w == HTVSCROLL as WPARAM, mouse_point(l))
                    .then_some(true)
            }
            WM_NCMOUSEMOVE | WM_NCMOUSEHOVER if value.tracking.is_none() => {
                update_hover(hwnd, state, mouse_point(l)).then_some(true)
            }
            WM_NCMOUSELEAVE => {
                let mut value = state.get();
                value.tracking_leave = false;
                if value.tracking.is_none() {
                    value.hover = None;
                }
                state.set(value);
                None
            }
            WM_MOUSEMOVE | WM_NCMOUSEMOVE | WM_LBUTTONUP | WM_NCLBUTTONUP
                if value.tracking.is_some() =>
            {
                let mut point = mouse_point(l);
                let client = matches!(message, WM_MOUSEMOVE | WM_LBUTTONUP);
                if client && ClientToScreen(hwnd, &mut point) == 0 {
                    stop_tracking(hwnd, state);
                    return Some(true);
                }
                if message == WM_MOUSEMOVE && w & 1 == 0 {
                    stop_tracking(hwnd, state);
                } else {
                    move_tracking(hwnd, state, point);
                    if matches!(message, WM_LBUTTONUP | WM_NCLBUTTONUP)
                        && let (Some(previous), Some(current)) =
                            (value.tracking, state.get().tracking)
                        && current.generation == previous.generation
                    {
                        stop_tracking(hwnd, state);
                    }
                }
                Some(true)
            }
            WM_KEYDOWN | WM_SYSKEYDOWN if value.tracking.is_some() && w == 0x1b => {
                stop_tracking(hwnd, state);
                Some(true)
            }
            WM_TIMER if w == TRACK_TIMER || (value.timer != 0 && w == value.timer) => {
                let Some((mut tracking, _)) = tracking_bar(hwnd, state) else {
                    stop_tracking(hwnd, state);
                    return Some(false);
                };

                if tracking.hit.part == Part::Thumb {
                    return Some(false);
                }
                if !tracking.repeating {
                    if SetTimer(hwnd, value.timer, REPEAT_INTERVAL, None) == 0 {
                        stop_tracking(hwnd, state);
                        return Some(true);
                    }
                    tracking.repeating = true;
                    let mut value = state.get();
                    value.tracking = Some(tracking);
                    state.set(value);
                }
                Some(repeat_step(hwnd, state))
            }
            WM_NCLBUTTONUP if w == HTVSCROLL as WPARAM || w == HTHSCROLL as WPARAM => {
                update_hover(hwnd, state, mouse_point(l)).then_some(true)
            }
            _ => None,
        }
    }
}

unsafe fn fill(dc: HDC, rect: RECT, color: u32) {
    // SAFETY: valid DC/rectangle; stock brush is borrowed and restored by SavedDc.
    unsafe {
        if valid(rect) {
            SetDCBrushColor(dc, color);
            FillRect(dc, &rect, GetStockObject(DC_BRUSH) as HBRUSH);
        }
    }
}
fn component_color(states: u32, scrollable: bool, palette: Palette) -> u32 {
    if !scrollable || states & STATE_SYSTEM_UNAVAILABLE != 0 {
        palette.border
    } else if states & (STATE_SYSTEM_PRESSED | STATE_SYSTEM_HOTTRACKED) != 0 {
        palette.accent
    } else {
        palette.disabled
    }
}
unsafe fn capsule(dc: HDC, rect: RECT, color: u32) {
    // SAFETY: caller clips to the bar; stock brush/pen are borrowed and restored.
    unsafe {
        let brush = GetStockObject(DC_BRUSH);
        let pen = GetStockObject(NULL_PEN);
        if brush.is_null() || pen.is_null() || rect.right <= rect.left || rect.bottom <= rect.top {
            return;
        }
        let old_brush = SelectObject(dc, brush);
        let old_pen = SelectObject(dc, pen);
        SetDCBrushColor(dc, color);
        let radius = ((rect.right - rect.left).min(rect.bottom - rect.top) / 2).max(1);
        RoundRect(
            dc,
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            radius,
            radius,
        );
        SelectObject(dc, old_pen);
        SelectObject(dc, old_brush);
    }
}

fn slim_thumb(bar: Bar, dpi: u32) -> Option<RECT> {
    let hidden = STATE_SYSTEM_INVISIBLE | STATE_SYSTEM_OFFSCREEN;
    if bar.states[3] & hidden != 0 || !bar.scrollable {
        return None;
    }
    let top = bar.thumb_top.max(bar.button);
    let bottom = bar.thumb_bottom.min(bar.length() - bar.button);
    if top >= bottom {
        return None;
    }
    let mut thumb = bar.segment(top, bottom);
    let thickness = scale(6, dpi).max(3);
    if bar.vertical {
        let available = (thumb.right - thumb.left).max(1);
        let thickness = thickness.min((available - 2).max(1));
        let center = (thumb.left + thumb.right) / 2;
        thumb.left = center - thickness / 2;
        thumb.right = thumb.left + thickness;
    } else {
        let available = (thumb.bottom - thumb.top).max(1);
        let thickness = thickness.min((available - 2).max(1));
        let center = (thumb.top + thumb.bottom) / 2;
        thumb.top = center - thickness / 2;
        thumb.bottom = thumb.top + thickness;
    }
    Some(thumb)
}

unsafe fn draw_bar(dc: HDC, mut bar: Bar, appearance: Appearance) {
    // SAFETY: saved clip is narrowed to the exact native bar rectangle. No pixels
    // outside it (including the native client edge) are intentionally modified.
    unsafe {
        let Some(_saved) = SavedDc::new(dc) else {
            return;
        };
        if IntersectClipRect(
            dc,
            bar.rect.left,
            bar.rect.top,
            bar.rect.right,
            bar.rect.bottom,
        ) == ERROR
        {
            return;
        }
        let p = appearance.palette;
        // Only NC pixels are themed. Our capture uses the native full-width
        // thumb/end/track geometry, not the six-DIP painted capsule's hit box.
        if let Some(hover) = appearance.hover
            && hover.vertical == bar.vertical
        {
            bar.states[hover.part.index()] |= STATE_SYSTEM_HOTTRACKED;
        }
        if let Some(tracking) = appearance.tracking
            && tracking.hit.vertical == bar.vertical
            && (tracking.hit.part == Part::Thumb || appearance.hover == Some(tracking.hit))
        {
            bar.states[tracking.hit.part.index()] |= STATE_SYSTEM_PRESSED;
        }
        fill(dc, bar.rect, p.background);
        let hidden = STATE_SYSTEM_INVISIBLE | STATE_SYSTEM_OFFSCREEN;
        if bar.states[0] & hidden != 0 {
            return;
        }
        if let Some(thumb) = slim_thumb(bar, appearance.dpi) {
            capsule(
                dc,
                thumb,
                component_color(bar.states[0] | bar.states[3], bar.scrollable, p),
            );
        }
    }
}

unsafe fn draw(hwnd: HWND, dc: HDC, appearance: Appearance) {
    // SAFETY: live native child and DC in window coordinates. Query only; do not
    // synthesize scroll geometry or call SetScrollInfo/ShowScrollBar/style APIs.
    unsafe {
        let Some(_saved) = SavedDc::new(dc) else {
            return;
        };
        let mut window = RECT::default();
        let mut client = RECT::default();
        let mut client_origin = POINT::default();
        if GetWindowRect(hwnd, &mut window) == 0
            || GetClientRect(hwnd, &mut client) == 0
            || ClientToScreen(hwnd, &mut client_origin) == 0
        {
            return;
        }
        let origin = POINT {
            x: window.left,
            y: window.top,
        };
        let bounds = offset(window, origin);
        let client = RECT {
            left: client_origin.x - origin.x,
            top: client_origin.y - origin.y,
            right: client_origin.x - origin.x + client.right,
            bottom: client_origin.y - origin.y + client.bottom,
        };
        if ExcludeClipRect(dc, client.left, client.top, client.right, client.bottom) == ERROR {
            return;
        }
        let vertical = bar(hwnd, true, origin, bounds);
        let horizontal = bar(hwnd, false, origin, bounds);
        if let Some(bar) = vertical {
            draw_bar(dc, bar, appearance);
        }
        if let Some(bar) = horizontal {
            draw_bar(dc, bar, appearance);
        }
        if let (Some(v), Some(h)) = (vertical, horizontal) {
            // Intersection of the extended scrollbar strips, not of their
            // (normally disjoint) rectangles. Works with left-hand scrollbars.
            let corner = intersection(
                RECT {
                    left: v.rect.left,
                    right: v.rect.right,
                    top: h.rect.top,
                    bottom: h.rect.bottom,
                },
                bounds,
            );
            fill(dc, corner, appearance.palette.surface);
        }
    }
}

#[cfg(test)]
std::thread_local! {
    static SCREEN_PASSES: Cell<usize> = const { Cell::new(0) };
}

unsafe fn overlay(hwnd: HWND, state: &State, print_dc: HDC) {
    // SAFETY: retained Rc survives synchronous detach/destruction. No Cell borrow
    // spans FFI, and painting suppresses recursive overlays, not native messages.
    unsafe {
        let mut value = state.get();
        if !value.attached
            || !value.enabled
            || value.painting
            || (print_dc.is_null() && (!value.redraw || value.defer_paint != 0))
            || !supported_window(hwnd)
        {
            return;
        }
        value.painting = true;
        state.set(value);
        if print_dc.is_null() {
            #[cfg(test)]
            SCREEN_PASSES.with(|count| count.set(count.get() + 1));
            let dc = GetWindowDC(hwnd);
            if !dc.is_null() {
                let _dc = WindowDc(hwnd, dc);
                draw(hwnd, dc, value);
            }
        } else {
            draw(hwnd, print_dc, value);
        }
        // Do not overwrite colors/enabled/attached changed by a nested callback.
        value = state.get();
        value.painting = false;
        state.set(value);
    }
}

fn repaints_bar(message: u32) -> bool {
    matches!(
        message,
        WM_NCPAINT
            | WM_PAINT
            | WM_VSCROLL
            | WM_HSCROLL
            | WM_SIZE
            | WM_WINDOWPOSCHANGED
            | WM_MOUSEWHEEL
            | WM_MOUSEHWHEEL
            | WM_NCMOUSEMOVE
            | WM_NCMOUSEHOVER
            | WM_NCMOUSELEAVE
            | WM_NCLBUTTONDOWN
            | WM_NCLBUTTONUP
            | WM_NCLBUTTONDBLCLK
            | WM_MOUSEMOVE
            | WM_MOUSELEAVE
            | WM_LBUTTONUP
            | WM_CAPTURECHANGED
            | WM_CANCELMODE
            | WM_TIMER
            | WM_ENABLE
            | WM_SETFOCUS
            | WM_KILLFOCUS
            | WM_KEYDOWN
            | WM_KEYUP
            | WM_CHAR
            | WM_SETTEXT
            | WM_CUT
            | WM_PASTE
            | WM_CLEAR
            | WM_UNDO
            | WM_STYLECHANGED
            | WM_THEMECHANGED
            | WM_SYSCOLORCHANGE
            | WM_DPICHANGED
            | WM_DPICHANGED_AFTERPARENT
            | WM_SHOWWINDOW
            | EM_SETSEL
            | EM_REPLACESEL
            | EM_LINESCROLL
            | EM_SCROLL
            | EM_SCROLLCARET
            | LVM_SCROLL
            | LVM_ENSUREVISIBLE
            | LVM_INSERTITEMW
            | LVM_DELETEITEM
            | LVM_DELETEALLITEMS
            | LVM_SETITEMCOUNT
            | LVM_INSERTCOLUMNW
            | LVM_DELETECOLUMN
            | LVM_SETCOLUMNWIDTH
    )
}

unsafe extern "system" fn procedure(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    // SAFETY: subclass owns one Rc, retained BEFORE default processing; another
    // subclass/parent notification may synchronously destroy or untheme the HWND.
    unsafe {
        let pointer = data as *const Cell<Appearance>;
        Rc::increment_strong_count(pointer);
        let state = Rc::from_raw(pointer);
        if message == WM_NCDESTROY {
            let mut value = state.get();
            value.attached = false;
            value.enabled = false;
            state.set(value);
            stop_tracking(hwnd, &state);
            RemoveWindowSubclass(hwnd, Some(procedure), id);
            drop(Rc::from_raw(pointer));
            return DefSubclassProc(hwnd, message, w, l);
        }
        if message == WM_NCHITTEST && driver(hwnd).is_some() {
            let point = mouse_point(l);
            // STATIC hosts otherwise return a client/transparent hit even on
            // their native scrollbar. Restrict the correction to actual, visible
            // system bar rectangles; client text selection is never intercepted.
            for vertical in [true, false] {
                if let Some(bar) = screen_bar(hwnd, vertical)
                    && contains(bar.rect, point)
                {
                    return if vertical { HTVSCROLL } else { HTHSCROLL } as LRESULT;
                }
            }
        }
        let geometry = geometry_message(message);
        let deferred = geometry.then(|| PaintDeferral::new(&state));
        if message == WM_SETREDRAW {
            let mut value = state.get();
            value.redraw = w != 0;
            state.set(value);
        }
        // Native scroll-info updates may query NCCALCSIZE without a resize;
        // suppress intermediate painting, but do not cancel that very drag.
        if (geometry && message != WM_NCCALCSIZE)
            || matches!(
                message,
                WM_CANCELMODE | WM_CAPTURECHANGED | WM_KILLFOCUS | WM_DESTROY
            )
            || (message == WM_ENABLE && w == 0)
        {
            stop_tracking(hwnd, &state);
        }
        if let Some(paint) = mouse_input(hwnd, &state, message, w, l) {
            if state.get().attached && IsWindow(hwnd) != 0 {
                if state.get().defer_paint == 0
                    && state.get().tracking.is_some()
                    && tracking_bar(hwnd, &state).is_none()
                {
                    stop_tracking(hwnd, &state);
                }
                if paint {
                    overlay(hwnd, &state, std::ptr::null_mut());
                }
            }
            return 0;
        }
        // Native WM_PRINT can change its caller's DC. Restore its original
        // window-coordinate mapping/clip before our non-client-only pass, then
        // draw() separately saves/restores all overlay changes.
        let print = message == WM_PRINT && w != 0 && l & PRF_NONCLIENT as LPARAM != 0;
        let saved = if print { SavedDc::new(w as HDC) } else { None };
        let can_print = saved.is_some();
        let result = DefSubclassProc(hwnd, message, w, l);
        drop(saved);
        drop(deferred);
        if !state.get().attached || IsWindow(hwnd) == 0 {
            return result;
        }
        if state.get().defer_paint == 0
            && state.get().tracking.is_some()
            && tracking_bar(hwnd, &state).is_none()
        {
            stop_tracking(hwnd, &state);
        }
        if geometry {
            invalidate_frame(hwnd);
        } else if print {
            if can_print && (l & PRF_CHECKVISIBLE as LPARAM == 0 || IsWindowVisible(hwnd) != 0) {
                overlay(hwnd, &state, w as HDC);
            }
        } else if repaints_bar(message) {
            overlay(hwnd, &state, std::ptr::null_mut());
        }
        // WM_PRINTCLIENT is intentionally forwarded without any NC overlay.
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr::null_mut;
    use windows_sys::Win32::System::Threading::{
        GR_GDIOBJECTS, GetCurrentProcess, GetGuiResources,
    };

    // Public SDK APIs behind unused windows-sys feature gates.
    #[link(name = "user32")]
    unsafe extern "system" {
        fn PrintWindow(hwnd: HWND, dc: HDC, flags: u32) -> i32;
        fn EnableScrollBar(hwnd: HWND, flags: u32, arrows: u32) -> i32;
        fn GetFocus() -> HWND;
        fn SetFocus(hwnd: HWND) -> HWND;
        fn GetCaretPos(point: *mut POINT) -> i32;
        fn EnableWindow(hwnd: HWND, enabled: i32) -> i32;
    }

    fn palette() -> Palette {
        Palette {
            background: 0x24201e,
            surface: 0x322c29,
            text: 0xf5f2ee,
            disabled: 0xb2aaa2,
            accent: 0xffc078,
            border: 0x574e47,
            accent_surface: 0x543e29,
        }
    }
    struct Window(HWND);
    impl Drop for Window {
        fn drop(&mut self) {
            // SAFETY: test-owned HWND, children are destroyed with their parent.
            unsafe {
                DestroyWindow(self.0);
            }
        }
    }
    unsafe fn window(class: &str, style: u32, parent: HWND) -> HWND {
        // SAFETY: temporary native windows on this thread; no application state.
        unsafe {
            let class: Vec<u16> = class.encode_utf16().chain(Some(0)).collect();
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                null(),
                style,
                if parent.is_null() { 173 } else { 7 },
                if parent.is_null() { 127 } else { 11 },
                if parent.is_null() { 700 } else { 360 },
                if parent.is_null() { 500 } else { 240 },
                parent,
                null_mut(),
                null_mut(),
                null(),
            );
            assert!(!hwnd.is_null());
            hwnd
        }
    }
    unsafe fn edit(parent: HWND) -> HWND {
        // SAFETY: synchronous messages copy text; no clipboard is modified.
        unsafe {
            let hwnd = window(
                "EDIT",
                WS_CHILD
                    | WS_VISIBLE
                    | WS_BORDER
                    | WS_VSCROLL
                    | WS_HSCROLL
                    | (ES_MULTILINE | ES_AUTOVSCROLL | ES_AUTOHSCROLL | ES_READONLY) as u32,
                parent,
            );
            let text = (0..160)
                .map(|n| format!("Line {n:03} {}\r\n", "native selection ".repeat(18)))
                .collect::<String>();
            let text: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
            SendMessageW(hwnd, WM_SETTEXT, 0, text.as_ptr() as LPARAM);
            SendMessageW(hwnd, EM_SETSEL, 7, 29);
            hwnd
        }
    }
    unsafe fn scroll(hwnd: HWND, axis: i32) -> (i32, i32, u32, i32, i32) {
        // SAFETY: correctly sized output, standard non-client scrollbar.
        unsafe {
            let mut value = SCROLLINFO {
                cbSize: size_of::<SCROLLINFO>() as u32,
                fMask: SIF_ALL,
                ..Default::default()
            };
            assert_ne!(GetScrollInfo(hwnd, axis, &mut value), 0);
            (
                value.nMin,
                value.nMax,
                value.nPage,
                value.nPos,
                value.nTrackPos,
            )
        }
    }
    unsafe fn native_bar(hwnd: HWND, vertical: bool) -> SCROLLBARINFO {
        // SAFETY: correctly sized output, public standard bar object ID.
        unsafe {
            let mut value = SCROLLBARINFO {
                cbSize: size_of::<SCROLLBARINFO>() as u32,
                ..Default::default()
            };
            assert_ne!(
                GetScrollBarInfo(
                    hwnd,
                    if vertical {
                        OBJID_VSCROLL
                    } else {
                        OBJID_HSCROLL
                    },
                    &mut value
                ),
                0
            );
            value
        }
    }
    unsafe fn selection(hwnd: HWND) -> (u32, u32) {
        // SAFETY: stack outputs live for synchronous EM_GETSEL.
        unsafe {
            let (mut start, mut end) = (0u32, 0u32);
            SendMessageW(
                hwnd,
                EM_GETSEL,
                (&mut start as *mut u32) as WPARAM,
                (&mut end as *mut u32) as LPARAM,
            );
            (start, end)
        }
    }
    struct BitmapDc {
        dc: HDC,
        bitmap: HBITMAP,
        old: HGDIOBJ,
    }
    impl BitmapDc {
        unsafe fn new() -> Self {
            // SAFETY: scratch bitmap/DC, restored and released by Drop.
            unsafe {
                let screen = GetDC(null_mut());
                let dc = CreateCompatibleDC(screen);
                let bitmap = CreateCompatibleBitmap(screen, 800, 600);
                ReleaseDC(null_mut(), screen);
                assert!(!dc.is_null() && !bitmap.is_null());
                let old = SelectObject(dc, bitmap as HGDIOBJ);
                Self { dc, bitmap, old }
            }
        }
    }
    impl Drop for BitmapDc {
        fn drop(&mut self) {
            // SAFETY: unselect before deleting the owned bitmap/DC.
            unsafe {
                SelectObject(self.dc, self.old);
                DeleteObject(self.bitmap as HGDIOBJ);
                DeleteDC(self.dc);
            }
        }
    }

    fn packed(point: POINT) -> LPARAM {
        (u32::from(point.x as u16) | (u32::from(point.y as u16) << 16)) as LPARAM
    }
    unsafe fn press(hwnd: HWND, vertical: bool, part: Part) -> POINT {
        // SAFETY: test-owned hidden child; send actual NC input, not a direct
        // tracker call. The lower probe prevents a regression from hanging in
        // DefWindowProc's modal tracking loop and asserts it was never entered.
        unsafe {
            let bar = screen_bar(hwnd, vertical).unwrap();
            let along = match part {
                Part::LineBefore => bar.button / 2,
                Part::PageBefore => (bar.button + bar.thumb_top) / 2,
                Part::Thumb => (bar.thumb_top + bar.thumb_bottom) / 2,
                Part::PageAfter => (bar.thumb_bottom + bar.length() - bar.button) / 2,
                Part::LineAfter => bar.length() - bar.button / 2 - 1,
            };
            let point = if vertical {
                POINT {
                    x: (bar.rect.left + bar.rect.right) / 2,
                    y: bar.rect.top + along,
                }
            } else {
                POINT {
                    x: bar.rect.left + along,
                    y: (bar.rect.top + bar.rect.bottom) / 2,
                }
            };
            assert_eq!(hit(bar, point), Some(Hit { vertical, part }));
            assert_eq!(
                SendMessageW(hwnd, WM_NCHITTEST, 0, packed(point)),
                if vertical { HTVSCROLL } else { HTHSCROLL } as LRESULT,
                "a real pointer on the native bar must reach dark NC tracking"
            );
            SendMessageW(
                hwnd,
                WM_NCLBUTTONDOWN,
                if vertical { HTVSCROLL } else { HTHSCROLL } as WPARAM,
                packed(point),
            );
            assert_eq!(
                GetCapture(),
                hwnd,
                "NC input did not capture the real child"
            );
            assert!(state(hwnd).unwrap().get().tracking.is_some());
            point
        }
    }
    unsafe fn captured_mouse(hwnd: HWND, message: u32, screen: POINT) {
        // SAFETY: capture delivers client-coordinate messages even over NC bars.
        unsafe {
            let mut point = screen;
            assert_ne!(ScreenToClient(hwnd, &mut point), 0);
            SendMessageW(
                hwnd,
                message,
                usize::from(message == WM_MOUSEMOVE),
                packed(point),
            );
        }
    }
    unsafe extern "system" fn reject_native_tracking(
        hwnd: HWND,
        message: u32,
        w: WPARAM,
        l: LPARAM,
        id: usize,
        data: usize,
    ) -> LRESULT {
        // SAFETY: private probe lives until the test child is destroyed. Only a
        // regression reaching the native modal NC entry increments this counter.
        unsafe {
            if matches!(message, WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK) {
                let count = &*(data as *const Cell<usize>);
                count.set(count.get() + 1);
                return 0;
            }
            if message == WM_NCDESTROY {
                RemoveWindowSubclass(hwnd, Some(reject_native_tracking), id);
            }
            DefSubclassProc(hwnd, message, w, l)
        }
    }
    unsafe fn tracking_probe(hwnd: HWND, count: &Cell<usize>) {
        // SAFETY: tests destroy the child before dropping this stack counter.
        unsafe {
            assert_ne!(
                SetWindowSubclass(
                    hwnd,
                    Some(reject_native_tracking),
                    SUBCLASS_ID + 2,
                    (count as *const Cell<usize>) as usize
                ),
                0
            );
        }
    }
    unsafe fn printed_tracking(hwnd: HWND, bitmap: &BitmapDc) {
        // SAFETY: scratch DC. Query the live native thumb after scrolling, while
        // capture is STILL held; this cannot be satisfied by painting on release.
        unsafe {
            assert_eq!(GetCapture(), hwnd);
            let value = state(hwnd).unwrap().get();
            let tracking = value.tracking.unwrap();
            let mut window = RECT::default();
            assert_ne!(GetWindowRect(hwnd, &mut window), 0);
            let origin = POINT {
                x: window.left,
                y: window.top,
            };
            let bounds = offset(window, origin);
            let bar = bar(hwnd, tracking.hit.vertical, origin, bounds).unwrap();
            let thumb = slim_thumb(bar, value.dpi).unwrap();
            SendMessageW(
                hwnd,
                WM_PRINT,
                bitmap.dc as WPARAM,
                (PRF_CLIENT | PRF_NONCLIENT) as LPARAM,
            );
            let center = POINT {
                x: (thumb.left + thumb.right) / 2,
                y: (thumb.top + thumb.bottom) / 2,
            };
            assert_eq!(
                GetPixel(bitmap.dc, center.x, center.y),
                value.palette.accent
            );
            let edge = if bar.vertical {
                POINT {
                    x: bar.rect.left + 1,
                    y: center.y,
                }
            } else {
                POINT {
                    x: center.x,
                    y: bar.rect.top + 1,
                }
            };
            assert_eq!(
                GetPixel(bitmap.dc, edge.x, edge.y),
                value.palette.background
            );
            assert_eq!(GetCapture(), hwnd);
        }
    }

    #[test]
    fn class_filter_and_native_state_colors() {
        assert!(supported("EDIT", ES_MULTILINE as u32 | WS_VSCROLL));
        assert!(supported("Edit", ES_MULTILINE as u32 | WS_HSCROLL));
        assert!(supported("SysListView32", 0));
        for (class, style) in [
            ("Edit", WS_VSCROLL),
            ("Edit", ES_MULTILINE as u32),
            ("ComboLBox", WS_VSCROLL),
            ("SCROLLBAR", 0),
            ("AIRelay.PageViewport", WS_VSCROLL),
        ] {
            assert!(!supported(class, style));
        }
        let p = palette();
        assert_eq!(component_color(STATE_SYSTEM_UNAVAILABLE, true, p), p.border);
        assert_eq!(component_color(STATE_SYSTEM_PRESSED, true, p), p.accent);
        assert_eq!(component_color(0, false, p), p.border);
    }

    #[test]
    fn known_viewport_keeps_dark_scrollbars_across_empty_and_scrolling_pages() {
        // SAFETY: Hidden private HWND tree, scratch bitmap and native scroll
        // ranges only. No user settings, system theme, mouse or clipboard changes.
        unsafe {
            let parent = Window(window("STATIC", WS_POPUP, null_mut()));
            let mut viewport = super::super::viewport::Viewport::default();
            viewport.create(parent.0).unwrap();
            viewport.layout(96, 640, 360, 0);
            viewport.prepare(96, 360, 0);
            let hwnd = viewport.handle;
            assert_eq!(GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & WS_VSCROLL, 0);
            let p = palette();
            apply(hwnd, Resolved::Dark, p, 96);
            assert!(
                state(hwnd).is_some(),
                "fit page must retain the host subclass"
            );
            let bitmap = BitmapDc::new();
            for _ in 0..3 {
                viewport.prepare(96, 820, 1);
                viewport.arrange(96, 820, 1);
                assert_ne!(GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & WS_VSCROLL, 0);
                let info = native_bar(hwnd, true);
                let mut rect = RECT::default();
                GetWindowRect(hwnd, &mut rect);
                let x = info.rcScrollBar.left - rect.left + 1;
                let y = info.rcScrollBar.top - rect.top + info.dxyLineButton + 2;
                SendMessageW(
                    hwnd,
                    WM_PRINT,
                    bitmap.dc as WPARAM,
                    (PRF_CLIENT | PRF_NONCLIENT) as LPARAM,
                );
                assert_eq!(
                    GetPixel(bitmap.dc, x, y),
                    p.background,
                    "restored native scrollbar lost its dark track"
                );
                viewport.prepare(96, 360, 3);
                viewport.arrange(96, 360, 3);
                apply(hwnd, Resolved::Dark, p, 96);
                assert!(state(hwnd).is_some());
            }
            apply(hwnd, Resolved::Contrast, p, 96);
            assert!(state(hwnd).is_none());
            let other = window("STATIC", WS_CHILD, parent.0);
            apply(other, Resolved::Dark, p, 96);
            assert!(
                state(other).is_none(),
                "unmarked borderless static remains native"
            );
        }
    }

    #[test]
    fn native_edit_state_detach_print_and_600_gdi_passes() {
        // SAFETY: test owns all windows/DCs; no user data, network or clipboard.
        unsafe {
            let parent = Window(window("STATIC", WS_POPUP | WS_VISIBLE, null_mut()));
            let hwnd = edit(parent.0);
            let bitmap = BitmapDc::new();
            let p = palette();
            let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
            let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let text_len = SendMessageW(hwnd, WM_GETTEXTLENGTH, 0, 0);
            SetFocus(hwnd);
            let focus = GetFocus();
            let sel = selection(hwnd);
            let before = [scroll(hwnd, SB_VERT), scroll(hwnd, SB_HORZ)];
            for dpi in [96, 144, 192] {
                for kind in [
                    Resolved::Dark,
                    Resolved::Light,
                    Resolved::Dark,
                    Resolved::Contrast,
                ] {
                    apply(hwnd, kind, p, dpi);
                    assert_eq!(state(hwnd).is_some(), kind == Resolved::Dark);
                    assert_eq!(GetWindowLongPtrW(hwnd, GWL_STYLE), style);
                    assert_eq!(GetWindowLongPtrW(hwnd, GWL_EXSTYLE), ex_style);
                    assert_eq!([scroll(hwnd, SB_VERT), scroll(hwnd, SB_HORZ)], before);
                    assert_eq!(selection(hwnd), sel);
                    assert_eq!(GetFocus(), focus);
                    assert_eq!(SendMessageW(hwnd, WM_GETTEXTLENGTH, 0, 0), text_len);
                }
            }
            apply(hwnd, Resolved::Dark, p, 96);
            let info = native_bar(hwnd, true);
            assert_eq!(
                info.rgstate[0] & (STATE_SYSTEM_INVISIBLE | STATE_SYSTEM_OFFSCREEN),
                0
            );
            let mut wr = RECT::default();
            GetWindowRect(hwnd, &mut wr);
            let x = info.rcScrollBar.left - wr.left + 1;
            let y = info.rcScrollBar.top - wr.top + info.dxyLineButton + 2;
            let sentinel = 0x123456;
            for kind in [
                Resolved::Dark,
                Resolved::Light,
                Resolved::Contrast,
                Resolved::Dark,
            ] {
                apply(hwnd, kind, p, 96);
                SetPixel(bitmap.dc, x, y, sentinel);
                let printed = PrintWindow(hwnd, bitmap.dc, 0);
                if printed == 0 {
                    println!(
                        "PrintWindow unavailable for test child (error {}); testing explicit WM_PRINT/PRF_NONCLIENT instead",
                        windows_sys::Win32::Foundation::GetLastError()
                    );
                    SendMessageW(
                        hwnd,
                        WM_PRINT,
                        bitmap.dc as WPARAM,
                        (PRF_NONCLIENT | PRF_CLIENT) as LPARAM,
                    );
                }
                if kind == Resolved::Dark {
                    assert_eq!(
                        GetPixel(bitmap.dc, x, y),
                        p.background,
                        "PrintWindow missed NC overlay"
                    );
                } else {
                    assert_ne!(
                        GetPixel(bitmap.dc, x, y),
                        p.background,
                        "native frame was not restored"
                    );
                }
            }
            EnableScrollBar(hwnd, SB_BOTH as u32, ESB_DISABLE_BOTH);
            let disabled = native_bar(hwnd, true);
            assert_ne!(
                (disabled.rgstate[0] | disabled.rgstate[3]) & STATE_SYSTEM_UNAVAILABLE,
                0
            );
            SendMessageW(hwnd, WM_PRINT, bitmap.dc as WPARAM, PRF_NONCLIENT as LPARAM);
            let thumb_x = (info.rcScrollBar.left + info.rcScrollBar.right) / 2 - wr.left;
            let thumb_y =
                info.rcScrollBar.top - wr.top + (info.xyThumbTop + info.xyThumbBottom) / 2;
            assert_eq!(GetPixel(bitmap.dc, thumb_x, thumb_y), p.border);
            EnableScrollBar(hwnd, SB_BOTH as u32, ESB_ENABLE_BOTH);
            for message in [WM_PRINT, WM_PRINTCLIENT] {
                SetPixel(bitmap.dc, x, y, sentinel);
                let saved = SaveDC(bitmap.dc);
                // Isolate the NC part of this caller's bitmap for PRINTCLIENT.
                IntersectClipRect(bitmap.dc, x, y, x + 1, y + 1);
                SendMessageW(hwnd, message, bitmap.dc as WPARAM, PRF_NONCLIENT as LPARAM);
                RestoreDC(bitmap.dc, saved);
                assert_eq!(
                    GetPixel(bitmap.dc, x, y),
                    if message == WM_PRINT {
                        p.background
                    } else {
                        sentinel
                    }
                );
            }
            let h = native_bar(hwnd, false);
            let cx = info.rcScrollBar.left - wr.left + 1;
            let cy = h.rcScrollBar.top - wr.top + 1;
            SendMessageW(
                hwnd,
                WM_PRINT,
                bitmap.dc as WPARAM,
                (PRF_NONCLIENT | PRF_CLIENT) as LPARAM,
            );
            assert_eq!(
                GetPixel(bitmap.dc, cx, cy),
                p.surface,
                "corner remains white"
            );
            let old_color = SetTextColor(bitmap.dc, sentinel);
            let process = GetCurrentProcess();
            let before_gdi = GetGuiResources(process, GR_GDIOBJECTS);
            for n in 0..600 {
                SendMessageW(
                    hwnd,
                    WM_PRINT,
                    bitmap.dc as WPARAM,
                    (PRF_NONCLIENT | PRF_CLIENT) as LPARAM,
                );
                if n % 3 == 0 {
                    apply(hwnd, Resolved::Light, p, 96);
                    apply(hwnd, Resolved::Dark, p, 96);
                }
            }
            let after_gdi = GetGuiResources(process, GR_GDIOBJECTS);
            assert_eq!(
                before_gdi, after_gdi,
                "600 paint/theme passes leaked GDI resources"
            );
            assert_eq!(GetTextColor(bitmap.dc), sentinel);
            SetTextColor(bitmap.dc, old_color);
            assert_eq!([scroll(hwnd, SB_VERT), scroll(hwnd, SB_HORZ)], before);
            assert_eq!(selection(hwnd), sel);
            println!("600 native scrollbar print/theme passes: GDI {before_gdi} -> {after_gdi}");
            InvalidateRect(hwnd, null(), 0);
            SendMessageW(hwnd, WM_PAINT, 0, 0);
            assert_eq!(GetUpdateRect(hwnd, null_mut(), 0), 0);
            let dc = GetWindowDC(hwnd);
            assert_eq!(
                GetPixel(dc, x, y),
                p.background,
                "WM_PAINT overwrote the NC overlay"
            );
            ReleaseDC(hwnd, dc);
            let retained = state(hwnd).unwrap();
            DestroyWindow(hwnd);
            assert!(!retained.get().attached);
            apply(null_mut(), Resolved::Dark, p, 96);
            apply(hwnd, Resolved::Dark, p, 96);
        }
    }

    #[test]
    fn native_bar_offsets_follow_both_axes_and_native_scroll_input() {
        // SAFETY: test-owned Edit, standard native scroll messages only.
        unsafe {
            let parent = Window(window("STATIC", WS_POPUP | WS_VISIBLE, null_mut()));
            let hwnd = edit(parent.0);
            apply(hwnd, Resolved::Dark, palette(), 192);
            let sel = selection(hwnd);
            for (vertical, message) in [(true, WM_VSCROLL), (false, WM_HSCROLL)] {
                SendMessageW(hwnd, message, SB_TOP as WPARAM, 0);
                let top = native_bar(hwnd, vertical);
                let length = if vertical {
                    top.rcScrollBar.bottom - top.rcScrollBar.top
                } else {
                    top.rcScrollBar.right - top.rcScrollBar.left
                };
                assert!(top.rcScrollBar.top > 0 && top.rcScrollBar.left > 0);
                assert!(
                    (top.xyThumbTop - top.dxyLineButton).abs() <= 1,
                    "thumb start is bar-relative"
                );
                assert!(top.xyThumbBottom > top.xyThumbTop && top.xyThumbBottom <= length);
                SendMessageW(hwnd, message, SB_BOTTOM as WPARAM, 0);
                let bottom = native_bar(hwnd, vertical);
                assert!(bottom.xyThumbTop > top.xyThumbTop);
                assert!(
                    (bottom.xyThumbBottom - (length - bottom.dxyLineButton)).abs() <= 1,
                    "thumb end is bar-relative"
                );
            }
            assert_eq!(selection(hwnd), sel);
            let before = scroll(hwnd, SB_VERT).3;
            SendMessageW(hwnd, WM_MOUSEWHEEL, (120u32 << 16) as WPARAM, 0);
            assert!(scroll(hwnd, SB_VERT).3 < before);
            let current = scroll(hwnd, SB_VERT);
            apply(hwnd, Resolved::Light, palette(), 96);
            assert_eq!(scroll(hwnd, SB_VERT), current);
        }
    }

    unsafe extern "system" fn destroy_during_default(
        hwnd: HWND,
        message: u32,
        w: WPARAM,
        l: LPARAM,
        id: usize,
        data: usize,
    ) -> LRESULT {
        // SAFETY: test-only subclass; data selects a synchronous lifetime probe.
        unsafe {
            if matches!(message, WM_SETTEXT | EM_LINESCROLL) {
                if data == 1 {
                    DestroyWindow(hwnd);
                    return 0;
                }
                apply(hwnd, Resolved::Light, palette(), 96);
            }
            if message == WM_NCDESTROY {
                RemoveWindowSubclass(hwnd, Some(destroy_during_default), id);
            }
            DefSubclassProc(hwnd, message, w, l)
        }
    }

    #[test]
    fn reentrant_native_destroy_and_detach_keep_state_alive() {
        // SAFETY: private native children and test-only lifetime probes.
        unsafe {
            let parent = Window(window("STATIC", WS_POPUP, null_mut()));
            for action in [1, 2] {
                let hwnd = edit(parent.0);
                assert_ne!(
                    SetWindowSubclass(hwnd, Some(destroy_during_default), SUBCLASS_ID + 1, action),
                    0
                );
                apply(hwnd, Resolved::Dark, palette(), 96);
                let retained = state(hwnd).unwrap();
                let text = [b'N' as u16, 0];
                SendMessageW(hwnd, WM_SETTEXT, 0, text.as_ptr() as LPARAM);
                assert!(!retained.get().attached);
                if action == 2 {
                    assert!(state(hwnd).is_none());
                    assert_eq!(SendMessageW(hwnd, WM_GETTEXTLENGTH, 0, 0), 1);
                } else {
                    assert_eq!(IsWindow(hwnd), 0);
                }
            }
        }
    }

    #[test]
    fn native_listview_and_invisible_bars() {
        // SAFETY: test-owned common controls, no application/controller state.
        unsafe {
            let init = INITCOMMONCONTROLSEX {
                dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_LISTVIEW_CLASSES,
            };
            assert_ne!(InitCommonControlsEx(&init), 0);
            let parent = Window(window("STATIC", WS_POPUP | WS_VISIBLE, null_mut()));
            let hwnd = window(
                "SysListView32",
                WS_CHILD | WS_VISIBLE | LVS_REPORT,
                parent.0,
            );
            let mut column = LVCOLUMNW {
                mask: LVCF_WIDTH,
                cx: 1000,
                ..Default::default()
            };
            SendMessageW(
                hwnd,
                LVM_INSERTCOLUMNW,
                0,
                (&mut column as *mut LVCOLUMNW) as LPARAM,
            );
            for i in 0..100 {
                let mut item = LVITEMW {
                    iItem: i,
                    ..Default::default()
                };
                assert_eq!(
                    SendMessageW(
                        hwnd,
                        LVM_INSERTITEMW,
                        0,
                        (&mut item as *mut LVITEMW) as LPARAM
                    ),
                    i as LRESULT
                );
            }
            let before = [scroll(hwnd, SB_VERT), scroll(hwnd, SB_HORZ)];
            apply(hwnd, Resolved::Dark, palette(), 144);
            assert!(state(hwnd).is_some());
            assert_eq!([scroll(hwnd, SB_VERT), scroll(hwnd, SB_HORZ)], before);
            apply(hwnd, Resolved::Contrast, palette(), 144);
            assert!(state(hwnd).is_none());
            let edit = window(
                "EDIT",
                WS_CHILD | ES_MULTILINE as u32 | WS_VSCROLL,
                parent.0,
            );
            // A hidden HWND is not necessarily marked OFFSCREEN by MSAA. Hide
            // the actual bar explicitly in the fixture, not in production code.
            assert_ne!(ShowScrollBar(edit, SB_VERT, 0), 0);
            let info = native_bar(edit, true);
            assert_ne!(
                info.rgstate[0] & (STATE_SYSTEM_INVISIBLE | STATE_SYSTEM_OFFSCREEN),
                0
            );
            let origin = POINT::default();
            let bounds = RECT {
                left: 0,
                top: 0,
                right: 10000,
                bottom: 10000,
            };
            assert!(bar(edit, true, origin, bounds).is_none());
            apply(parent.0, Resolved::Dark, palette(), 96);
            assert!(state(parent.0).is_none());
        }
    }

    #[test]
    fn hit_geometry_full_native_thumb_tracks_and_32bit_mapping() {
        for vertical in [true, false] {
            let bar = Bar {
                rect: if vertical {
                    RECT {
                        left: -180,
                        top: 123,
                        right: -163,
                        bottom: 323,
                    }
                } else {
                    RECT {
                        left: -180,
                        top: 123,
                        right: 20,
                        bottom: 140,
                    }
                },
                vertical,
                button: 17,
                thumb_top: 50,
                thumb_bottom: 80,
                states: [0; 6],
                scrollable: true,
            };
            let point = |at| {
                if vertical {
                    POINT {
                        x: bar.rect.left + 1,
                        y: bar.rect.top + at,
                    }
                } else {
                    POINT {
                        x: bar.rect.left + at,
                        y: bar.rect.top + 1,
                    }
                }
            };
            for (at, part) in [
                (0, Part::LineBefore),
                (17, Part::PageBefore),
                (49, Part::PageBefore),
                (50, Part::Thumb),
                (79, Part::Thumb),
                (80, Part::PageAfter),
                (182, Part::PageAfter),
                (183, Part::LineAfter),
            ] {
                assert_eq!(hit(bar, point(at)), Some(Hit { vertical, part }));
            }
            assert!(hit(bar, point(-1)).is_none());
            assert!(hit(bar, point(200)).is_none());
            let thumb = slim_thumb(bar, 96).unwrap();
            assert!(
                !contains(thumb, point(65)),
                "native full-width thumb must remain clickable"
            );
            let mut disabled = bar;
            disabled.states[3] = STATE_SYSTEM_UNAVAILABLE;
            assert!(hit(disabled, point(65)).is_none());
            let range = Range {
                min: -1_000_000,
                max: 2_000_000_000,
                page: 701,
                position: 80_000,
            };
            assert_eq!(thumb_position(bar, range, point(-30000), 15), range.min);
            assert_eq!(thumb_position(bar, range, point(30000), 15), range.end());
            assert!(thumb_position(bar, range, point(115), 15) > 65535);
            let full = Range {
                min: i32::MIN,
                max: i32::MAX,
                page: 0,
                position: 0,
            };
            assert_eq!(thumb_position(bar, full, point(32), 15), i32::MIN);
            assert_eq!(thumb_position(bar, full, point(168), 15), i32::MAX);
            let fixed = Range {
                min: 7,
                max: 12,
                page: u32::MAX,
                position: 9,
            };
            assert_eq!(fixed.end(), 7);
            assert_eq!(fixed.clamp(i64::MAX), 7);
            let tiny = Bar {
                button: 90,
                thumb_top: 90,
                thumb_bottom: 110,
                ..bar
            };
            assert_eq!(thumb_position(tiny, range, point(40), 1), range.position);
        }
        for (target, current, width, expected) in [
            (99, 0, 10, 10),
            (0, 99, 10, -10),
            (94, 0, 10, 9),
            (4, 0, 8, 0),
            (0, 4, 8, 0),
            (5, 0, 8, 1),
            (0, 5, 8, -1),
            (i32::MAX, i32::MIN, 1, i32::MAX),
            (i32::MIN, i32::MAX, 1, i32::MIN),
            (655_390, 0, 10, 65_539),
        ] {
            assert_eq!(horizontal_characters(target, current, width), expected);
        }
        for message in [
            WM_SIZE,
            WM_WINDOWPOSCHANGED,
            WM_NCCALCSIZE,
            WM_DPICHANGED,
            WM_THEMECHANGED,
            WM_SETREDRAW,
            EM_SETRECTNP,
        ] {
            assert!(geometry_message(message));
        }
        for message in [WM_PAINT, WM_NCPAINT, EM_LINESCROLL, WM_MOUSEWHEEL] {
            assert!(!geometry_message(message));
        }
    }

    #[test]
    fn hidden_edit_mouse_drag_updates_native_position_before_release_and_prints_dark() {
        // SAFETY: hidden test HWNDs; no synthetic system input, real clipboard,
        // GUI launch, network or user configuration is involved.
        unsafe {
            let calls = Cell::new(0);
            let parent = Window(window("STATIC", WS_POPUP, null_mut()));
            let hwnd = edit(parent.0);
            tracking_probe(hwnd, &calls);
            apply(hwnd, Resolved::Dark, palette(), 96);
            let bitmap = BitmapDc::new();
            let initial = [scroll(hwnd, SB_VERT), scroll(hwnd, SB_HORZ)];
            let bar = screen_bar(hwnd, true).unwrap();
            let hover = POINT {
                x: (bar.rect.left + bar.rect.right) / 2,
                y: bar.rect.top + (bar.thumb_top + bar.thumb_bottom) / 2,
            };
            SendMessageW(hwnd, WM_NCMOUSEMOVE, HTVSCROLL as WPARAM, packed(hover));
            let retained = state(hwnd).unwrap();
            assert_eq!(
                retained.get().hover,
                Some(Hit {
                    vertical: true,
                    part: Part::Thumb
                })
            );
            assert!(retained.get().tracking.is_none());
            assert_eq!(retained.get().timer, 0, "hover must not start polling");
            SendMessageW(hwnd, WM_NCMOUSELEAVE, 0, 0);
            assert!(retained.get().hover.is_none());
            let selected = selection(hwnd);
            let focus = GetFocus();
            let mut caret = POINT::default();
            let has_caret = GetCaretPos(&mut caret) != 0;
            let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
            let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            for vertical in [true, false] {
                let point = press(hwnd, vertical, Part::Thumb);
                captured_mouse(hwnd, WM_MOUSEMOVE, point);
                assert_eq!(
                    scroll(hwnd, if vertical { SB_VERT } else { SB_HORZ }).3,
                    initial[usize::from(!vertical)].3,
                    "a stationary click must not quantize the position"
                );
                let mut target = point;
                if vertical {
                    target.y += 65;
                } else {
                    target.x += 85;
                }
                captured_mouse(hwnd, WM_MOUSEMOVE, target);
                let current = scroll(hwnd, if vertical { SB_VERT } else { SB_HORZ });
                assert!(
                    current.3 > 0,
                    "native scroll position must change before button release"
                );
                assert_eq!(current.0, initial[usize::from(!vertical)].0);
                assert_eq!(current.1, initial[usize::from(!vertical)].1);
                assert_eq!(current.2, initial[usize::from(!vertical)].2);
                if vertical {
                    assert_eq!(
                        SendMessageW(hwnd, EM_GETFIRSTVISIBLELINE, 0, 0) as i32,
                        current.3
                    );
                }
                captured_mouse(hwnd, WM_MOUSEMOVE, target);
                assert_eq!(
                    scroll(hwnd, if vertical { SB_VERT } else { SB_HORZ }).3,
                    current.3,
                    "repeated coordinates must not walk the quantized native position on axis {vertical}"
                );
                printed_tracking(hwnd, &bitmap);
                assert_eq!(
                    scroll(hwnd, if vertical { SB_VERT } else { SB_HORZ }).3,
                    current.3,
                    "printing must not alter the native position"
                );
                captured_mouse(hwnd, WM_LBUTTONUP, target);
                let retained = state(hwnd).unwrap();
                assert!(retained.get().tracking.is_none());
                assert_eq!(retained.get().timer, 0);
                assert_ne!(GetCapture(), hwnd);
                assert_eq!(
                    scroll(hwnd, if vertical { SB_VERT } else { SB_HORZ }).3,
                    current.3
                );
            }
            assert_eq!(selection(hwnd), selected);
            assert_eq!(GetFocus(), focus, "NC tracking must not steal focus");
            let mut after_caret = POINT::default();
            assert_eq!(GetCaretPos(&mut after_caret) != 0, has_caret);
            if has_caret {
                assert_eq!((after_caret.x, after_caret.y), (caret.x, caret.y));
            }
            assert_eq!(GetWindowLongPtrW(hwnd, GWL_STYLE), style);
            assert_eq!(GetWindowLongPtrW(hwnd, GWL_EXSTYLE), ex_style);
            let first = scroll(hwnd, SB_VERT).3;
            SendMessageW(hwnd, WM_MOUSEWHEEL, (120u32 << 16) as WPARAM, 0);
            assert!(
                scroll(hwnd, SB_VERT).3 < first,
                "wheel still uses native handling"
            );
            assert_eq!(
                calls.get(),
                0,
                "dark NC input entered native modal tracking"
            );
            SendMessageW(hwnd, EM_SETSEL, 7, 7);
            SendMessageW(hwnd, WM_KEYDOWN, 0x27, 0);
            assert_eq!(
                selection(hwnd),
                (8, 8),
                "native keyboard selection remains intact"
            );
        }
    }

    #[test]
    fn edit_horizontal_font_units_and_full_width_32bit_vertical_drag() {
        // SAFETY: hidden private native Edit controls and a borrowed stock font.
        unsafe {
            let calls = Cell::new(0);
            let parent = Window(window("STATIC", WS_POPUP, null_mut()));
            let hwnd = edit(parent.0);
            let selected = selection(hwnd);
            let mut default_width = None;
            for font in [
                std::ptr::null_mut(),
                GetStockObject(SYSTEM_FONT),
                GetStockObject(DEFAULT_GUI_FONT),
                GetStockObject(SYSTEM_FIXED_FONT),
            ] {
                SendMessageW(hwnd, WM_SETFONT, font as WPARAM, 0);
                let width = edit_character_width(hwnd, None).unwrap();
                if font.is_null() {
                    default_width = Some(width);
                } else if font == GetStockObject(SYSTEM_FONT) {
                    assert_eq!(
                        Some(width),
                        default_width,
                        "NULL WM_GETFONT must use SYSTEM_FONT"
                    );
                }
                SendMessageW(hwnd, WM_HSCROLL, SB_LEFT as WPARAM, 0);
                let initial = scroll(hwnd, SB_HORZ);
                let origin = SendMessageW(hwnd, EM_POSFROMCHAR, 0, 0) as u16 as i16 as i32;
                SendMessageW(hwnd, EM_LINESCROLL, 17, 0);
                let position = scroll(hwnd, SB_HORZ);
                let moved = SendMessageW(hwnd, EM_POSFROMCHAR, 0, 0) as u16 as i16 as i32;
                assert_eq!(
                    position.3 - initial.3,
                    17 * width,
                    "native Edit average-character unit for font {font:p}, measured width {width}"
                );
                assert_eq!(
                    origin - moved,
                    17 * width,
                    "native text displacement must match the full scrollbar value for font {font:p}"
                );
                assert_eq!(
                    (position.0, position.1, position.2),
                    (initial.0, initial.1, initial.2)
                );
                SendMessageW(hwnd, EM_LINESCROLL, (-9i32) as WPARAM, 0);
                assert_eq!(scroll(hwnd, SB_HORZ).3, initial.3 + 8 * width);
                assert_eq!(selection(hwnd), selected);
                println!(
                    "Native Edit font {font:p}: unit {width}px, 17 characters = {}px scroll/text displacement",
                    position.3 - initial.3
                );
            }
            let width = edit_character_width(hwnd, None).unwrap();
            SendMessageW(hwnd, WM_HSCROLL, SB_LEFT as WPARAM, 0);
            SendMessageW(hwnd, EM_LINESCROLL, 17, 0);
            assert_eq!(
                scroll(hwnd, SB_HORZ).3,
                17 * width,
                "native horizontal position is pixels, EM_LINESCROLL is average characters"
            );
            SendMessageW(hwnd, EM_LINESCROLL, (-9i32) as WPARAM, 0);
            assert_eq!(scroll(hwnd, SB_HORZ).3, 8 * width);
            tracking_probe(hwnd, &calls);
            apply(hwnd, Resolved::Dark, palette(), 96);
            press(hwnd, false, Part::Thumb);
            let retained = state(hwnd).unwrap();
            let native = scroll(hwnd, SB_HORZ);
            drive(hwnd, &retained, Driver::Edit, false, 31 * width);
            assert_eq!(scroll(hwnd, SB_HORZ).3, 31 * width);
            drive(hwnd, &retained, Driver::Edit, false, 6 * width);
            assert_eq!(scroll(hwnd, SB_HORZ).3, 6 * width);
            let halfway = 12 * width + width / 2;
            drive(hwnd, &retained, Driver::Edit, false, halfway);
            let quantized = scroll(hwnd, SB_HORZ).3;
            for _ in 0..4 {
                drive(hwnd, &retained, Driver::Edit, false, halfway);
                assert_eq!(
                    scroll(hwnd, SB_HORZ).3,
                    quantized,
                    "half-character targets must be idempotent"
                );
            }
            drive(hwnd, &retained, Driver::Edit, false, i32::MAX);
            assert_eq!(
                scroll(hwnd, SB_HORZ).3,
                (i64::from(native.1) - i64::from(native.2.saturating_sub(1))) as i32
            );
            apply(hwnd, Resolved::Light, palette(), 96);
            SendMessageW(hwnd, EM_SETLIMITTEXT, 2_000_000, 0);
            let text: Vec<u16> = "x\r\n"
                .repeat(90_000)
                .encode_utf16()
                .chain(Some(0))
                .collect();
            SendMessageW(hwnd, WM_SETTEXT, 0, text.as_ptr() as LPARAM);
            SendMessageW(hwnd, WM_VSCROLL, SB_TOP as WPARAM, 0);
            tracking_probe(hwnd, &calls);
            apply(hwnd, Resolved::Dark, palette(), 96);
            let initial = scroll(hwnd, SB_VERT);
            assert!(
                initial.1 > 80_000,
                "fixture must exceed the WM_VSCROLL high word"
            );
            let mut point = press(hwnd, true, Part::Thumb);
            point.y += 1000;
            captured_mouse(hwnd, WM_MOUSEMOVE, point);
            assert!(scroll(hwnd, SB_VERT).3 > 65535);
            assert_eq!(
                SendMessageW(hwnd, EM_GETFIRSTVISIBLELINE, 0, 0) as i32,
                scroll(hwnd, SB_VERT).3
            );
            let bitmap = BitmapDc::new();
            printed_tracking(hwnd, &bitmap);
            captured_mouse(hwnd, WM_LBUTTONUP, point);
            let final_info = scroll(hwnd, SB_VERT);
            assert_eq!(
                (final_info.0, final_info.1, final_info.2),
                (initial.0, initial.1, initial.2)
            );
            let wide: Vec<u16> = format!("{}\r\n{}", "wide".repeat(20_000), "wide".repeat(20_000))
                .encode_utf16()
                .chain(Some(0))
                .collect();
            SendMessageW(hwnd, WM_SETTEXT, 0, wide.as_ptr() as LPARAM);
            SendMessageW(hwnd, WM_HSCROLL, SB_LEFT as WPARAM, 0);
            let initial = scroll(hwnd, SB_HORZ);
            assert!(
                initial.1 > initial.2 as i32,
                "long native line must be scrollable"
            );
            println!(
                "Native Edit long-line horizontal range (not widened by presentation): {initial:?}"
            );
            let mut point = press(hwnd, false, Part::Thumb);
            point.x += 1000;
            captured_mouse(hwnd, WM_MOUSEMOVE, point);
            assert_eq!(
                scroll(hwnd, SB_HORZ).3,
                (i64::from(initial.1) - i64::from(initial.2.saturating_sub(1)))
                    .max(i64::from(initial.0)) as i32,
                "drag clamps to the actual native long-line horizontal range"
            );
            printed_tracking(hwnd, &bitmap);
            captured_mouse(hwnd, WM_LBUTTONUP, point);
            let final_info = scroll(hwnd, SB_HORZ);
            assert_eq!(
                (final_info.0, final_info.1, final_info.2),
                (initial.0, initial.1, initial.2)
            );
            assert_eq!(calls.get(), 0);
        }
    }

    #[test]
    fn held_line_page_repeat_stops_at_pointer_and_cancel_releases_ownership() {
        // SAFETY: only test children receive NC/capture/timer messages. Repeat
        // ticks are explicit; no sleeps or message-pump timing assumptions.
        unsafe {
            let calls = Cell::new(0);
            let parent = Window(window("STATIC", WS_POPUP, null_mut()));
            let hwnd = edit(parent.0);
            tracking_probe(hwnd, &calls);
            apply(hwnd, Resolved::Dark, palette(), 96);
            let point = press(hwnd, true, Part::LineAfter);
            let first = scroll(hwnd, SB_VERT).3;
            assert!(first > 0);
            SendMessageW(hwnd, WM_TIMER, TRACK_TIMER, 0);
            assert!(scroll(hwnd, SB_VERT).3 > first);
            let repeated = scroll(hwnd, SB_VERT).3;
            captured_mouse(
                hwnd,
                WM_MOUSEMOVE,
                POINT {
                    x: point.x - 500,
                    y: point.y,
                },
            );
            SendMessageW(hwnd, WM_TIMER, TRACK_TIMER, 0);
            assert_eq!(
                scroll(hwnd, SB_VERT).3,
                repeated,
                "holding outside the pressed button must pause repeat"
            );
            captured_mouse(hwnd, WM_MOUSEMOVE, point);
            SendMessageW(hwnd, WM_TIMER, TRACK_TIMER, 0);
            assert!(scroll(hwnd, SB_VERT).3 > repeated);
            SendMessageW(hwnd, WM_CANCELMODE, 0, 0);
            let retained = state(hwnd).unwrap();
            assert!(retained.get().tracking.is_none());
            assert_eq!(retained.get().timer, 0);
            assert_ne!(GetCapture(), hwnd);
            let canceled = scroll(hwnd, SB_VERT).3;
            SendMessageW(hwnd, WM_TIMER, TRACK_TIMER, 0);
            assert_eq!(scroll(hwnd, SB_VERT).3, canceled);
            SendMessageW(hwnd, WM_VSCROLL, SB_TOP as WPARAM, 0);
            let point = press(hwnd, true, Part::PageAfter);
            assert!(scroll(hwnd, SB_VERT).3 > 1);
            for _ in 0..30 {
                SendMessageW(hwnd, WM_TIMER, TRACK_TIMER, 0);
            }
            let stopped = scroll(hwnd, SB_VERT).3;
            assert!(
                stopped < scroll(hwnd, SB_VERT).1,
                "track paging should stop at the mouse, not the far end"
            );
            SendMessageW(hwnd, WM_TIMER, TRACK_TIMER, 0);
            assert_eq!(scroll(hwnd, SB_VERT).3, stopped);
            captured_mouse(hwnd, WM_LBUTTONUP, point);
            let before = scroll(hwnd, SB_VERT).3;
            let point = press(hwnd, true, Part::LineBefore);
            assert!(scroll(hwnd, SB_VERT).3 < before);
            SendMessageW(hwnd, WM_TIMER, TRACK_TIMER, 0);
            assert!(scroll(hwnd, SB_VERT).3 < before - 1);
            captured_mouse(hwnd, WM_LBUTTONUP, point);
            SendMessageW(hwnd, WM_VSCROLL, SB_BOTTOM as WPARAM, 0);
            let before = scroll(hwnd, SB_VERT).3;
            let point = press(hwnd, true, Part::PageBefore);
            assert!(scroll(hwnd, SB_VERT).3 < before - 1);
            for _ in 0..30 {
                SendMessageW(hwnd, WM_TIMER, TRACK_TIMER, 0);
            }
            let stopped = scroll(hwnd, SB_VERT).3;
            assert!(
                stopped > 0,
                "reverse track paging must also stop at the pointer"
            );
            SendMessageW(hwnd, WM_TIMER, TRACK_TIMER, 0);
            assert_eq!(scroll(hwnd, SB_VERT).3, stopped);
            captured_mouse(hwnd, WM_LBUTTONUP, point);
            assert_eq!(calls.get(), 0);
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum QueryAction {
        Continue,
        Cancel,
        TransferCapture,
        Disable,
        Hide,
        Resize,
        Theme,
        Font,
        Restart,
    }
    struct QueryProbe {
        query: u32,
        action: QueryAction,
        huge_height: bool,
        vertical: bool,
        armed: Cell<bool>,
        fired: Cell<usize>,
        returned: Cell<bool>,
        continuations: Cell<usize>,
        position_after: Cell<i32>,
    }
    unsafe extern "system" fn cancel_during_scroll_query(
        hwnd: HWND,
        message: u32,
        w: WPARAM,
        l: LPARAM,
        id: usize,
        data: usize,
    ) -> LRESULT {
        // SAFETY: test-owned subclass and stack probe; only Cell scalars cross
        // synchronous callbacks. Native windows outlive the probe. Inflate only
        // the queried row rectangle, not the native list or its item allocation.
        unsafe {
            let probe = &*(data as *const QueryProbe);
            if probe.returned.get() && matches!(message, LVM_SCROLL | EM_LINESCROLL) {
                probe.continuations.set(probe.continuations.get() + 1);
            }
            let trigger = message == probe.query && probe.armed.replace(false);
            let result = if message == LVM_GETITEMRECT && probe.huge_height {
                *(l as *mut RECT) = RECT {
                    left: 0,
                    top: 0,
                    right: 100,
                    bottom: 100_000,
                };
                1
            } else {
                if message == WM_NCDESTROY {
                    RemoveWindowSubclass(hwnd, Some(cancel_during_scroll_query), id);
                }
                DefSubclassProc(hwnd, message, w, l)
            };
            if trigger {
                probe.fired.set(probe.fired.get() + 1);
                match probe.action {
                    QueryAction::Continue => {}
                    QueryAction::Cancel => {
                        SendMessageW(hwnd, WM_CANCELMODE, 0, 0);
                    }
                    QueryAction::TransferCapture => {
                        SetCapture(GetParent(hwnd));
                    }
                    QueryAction::Disable => {
                        EnableWindow(hwnd, 0);
                    }
                    QueryAction::Hide => {
                        ShowWindow(hwnd, SW_HIDE);
                    }
                    QueryAction::Resize => {
                        MoveWindow(hwnd, 7, 11, 390, 250, 0);
                    }
                    QueryAction::Theme => {
                        SendMessageW(hwnd, WM_THEMECHANGED, 0, 0);
                    }
                    QueryAction::Font => {
                        SendMessageW(
                            hwnd,
                            WM_SETFONT,
                            GetStockObject(DEFAULT_GUI_FONT) as WPARAM,
                            0,
                        );
                    }
                    QueryAction::Restart => {
                        SendMessageW(hwnd, WM_CANCELMODE, 0, 0);
                        press(hwnd, probe.vertical, Part::Thumb);
                    }
                }
                probe
                    .position_after
                    .set(scroll(hwnd, if probe.vertical { SB_VERT } else { SB_HORZ }).3);
                probe.returned.set(true);
            }
            result
        }
    }

    #[test]
    fn native_query_cancel_keeps_attachment_but_never_continues_stale_scroll() {
        // SAFETY: hidden test-owned native controls. Owner-data rows and a test
        // rectangle probe reach the genuine >i32 pixel branch without allocating
        // billions of pixels/items or changing its production overflow threshold.
        unsafe {
            let init = INITCOMMONCONTROLSEX {
                dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_LISTVIEW_CLASSES,
            };
            assert_ne!(InitCommonControlsEx(&init), 0);
            let calls = Cell::new(0);
            for (driver, vertical, query, huge_height) in [
                (Driver::Report, true, LVM_ENSUREVISIBLE, true),
                (Driver::Report, true, LVM_GETTOPINDEX, false),
                (Driver::Report, true, LVM_GETITEMRECT, false),
                (Driver::Edit, true, EM_GETFIRSTVISIBLELINE, false),
                (Driver::Edit, false, WM_GETFONT, false),
            ] {
                for action in [
                    QueryAction::Continue,
                    QueryAction::Cancel,
                    QueryAction::TransferCapture,
                    QueryAction::Disable,
                    QueryAction::Hide,
                    QueryAction::Resize,
                    QueryAction::Theme,
                    QueryAction::Font,
                    QueryAction::Restart,
                ] {
                    let probe = QueryProbe {
                        query,
                        action,
                        huge_height,
                        vertical,
                        armed: Cell::new(false),
                        fired: Cell::new(0),
                        returned: Cell::new(false),
                        continuations: Cell::new(0),
                        position_after: Cell::new(0),
                    };
                    // Drop HWNDs before the stack probe on every assertion exit.
                    let parent = Window(window("STATIC", WS_POPUP, null_mut()));
                    let hwnd = if driver == Driver::Report {
                        let hwnd = window(
                            "SysListView32",
                            WS_CHILD | WS_VISIBLE | LVS_REPORT | LVS_OWNERDATA,
                            parent.0,
                        );
                        let mut column = LVCOLUMNW {
                            mask: LVCF_WIDTH,
                            cx: 1000,
                            ..Default::default()
                        };
                        SendMessageW(
                            hwnd,
                            LVM_INSERTCOLUMNW,
                            0,
                            (&mut column as *mut LVCOLUMNW) as LPARAM,
                        );
                        SendMessageW(hwnd, LVM_SETITEMCOUNT, 100_000, 0);
                        hwnd
                    } else {
                        edit(parent.0)
                    };
                    assert_ne!(
                        SetWindowSubclass(
                            hwnd,
                            Some(cancel_during_scroll_query),
                            SUBCLASS_ID + 3,
                            (&probe as *const QueryProbe) as usize
                        ),
                        0
                    );
                    tracking_probe(hwnd, &calls);
                    apply(hwnd, Resolved::Dark, palette(), 96);
                    press(hwnd, vertical, Part::Thumb);
                    let retained = state(hwnd).unwrap();
                    let tracking = retained.get().tracking.unwrap();
                    let info = range(hwnd, vertical).unwrap();
                    let target = if huge_height { 50_000 } else { info.end() / 2 };
                    assert!(target > info.min && target < info.end());
                    if huge_height {
                        assert!(
                            i32::try_from((i64::from(target) - i64::from(info.position)) * 100_000)
                                .is_err()
                        );
                    }
                    probe.armed.set(true);
                    drive(hwnd, &retained, driver, vertical, target);
                    assert_eq!(
                        probe.fired.get(),
                        1,
                        "query {query:#x}, action {action:?} did not run"
                    );
                    assert!(
                        retained.get().attached && IsWindow(hwnd) != 0,
                        "cancel must not depend on detach/destroy"
                    );
                    assert!(Rc::ptr_eq(&retained, &state(hwnd).unwrap()));
                    if huge_height {
                        // A stale continuation really would fit LVM_SCROLL after
                        // ENSUREVISIBLE: do not let a second overflow mask the bug.
                        let remainder =
                            (i64::from(target) - i64::from(probe.position_after.get())) * 100_000;
                        assert!(remainder > 0 && i32::try_from(remainder).is_ok());
                    }
                    if action == QueryAction::Continue {
                        assert_eq!(probe.continuations.get(), 1);
                        assert!(same_tracking(hwnd, &retained, tracking));
                    } else {
                        assert_eq!(
                            probe.continuations.get(),
                            0,
                            "stale scroll after query {query:#x}, action {action:?}"
                        );
                        assert_eq!(
                            scroll(hwnd, if vertical { SB_VERT } else { SB_HORZ }).3,
                            probe.position_after.get()
                        );
                        assert!(!same_tracking(hwnd, &retained, tracking));
                        if action == QueryAction::Restart {
                            let current = retained.get().tracking.unwrap();
                            assert_eq!(GetCapture(), hwnd);
                            assert_ne!(current.generation, tracking.generation);
                            assert!(same_tracking(hwnd, &retained, current));
                        } else {
                            assert!(retained.get().tracking.is_none());
                            assert_eq!(retained.get().timer, 0);
                            assert!(retained.get().keyboard_hook.is_null());
                        }
                    }
                    stop_tracking(hwnd, &retained);
                    if GetCapture() == parent.0 {
                        ReleaseCapture();
                    }
                }
            }
            assert_eq!(calls.get(), 0);
        }
    }

    #[test]
    fn native_scroll_notifications_can_destroy_or_detach_during_capture() {
        // SAFETY: hidden children and lower native-message probes. The retained
        // Rc must outlive synchronous native scroll notifications and their
        // capture/timer/hook cleanup, not just a standalone WM_SETTEXT callback.
        unsafe {
            let calls = Cell::new(0);
            let parent = Window(window("STATIC", WS_POPUP, null_mut()));
            for action in [1, 2] {
                let hwnd = edit(parent.0);
                SetWindowSubclass(hwnd, Some(destroy_during_default), SUBCLASS_ID + 1, action);
                tracking_probe(hwnd, &calls);
                apply(hwnd, Resolved::Dark, palette(), 96);
                let retained = state(hwnd).unwrap();
                let mut point = press(hwnd, true, Part::Thumb);
                point.y += 35;
                captured_mouse(hwnd, WM_MOUSEMOVE, point);
                assert!(!retained.get().attached);
                assert!(retained.get().tracking.is_none());
                assert_eq!(retained.get().timer, 0);
                assert!(retained.get().keyboard_hook.is_null());
                assert_ne!(GetCapture(), hwnd);
                if action == 1 {
                    assert_eq!(IsWindow(hwnd), 0);
                } else {
                    assert!(state(hwnd).is_none());
                    assert!(scroll(hwnd, SB_VERT).3 > 0);
                }
            }
            assert_eq!(calls.get(), 0);
        }
    }

    #[test]
    fn capture_escape_disable_hide_theme_dpi_resize_and_destroy_cancel() {
        // SAFETY: hidden native HWND tree and scalar messages; no global theme,
        // focus, cursor or input injection. Capture is always released by the SUT.
        unsafe {
            let parent = Window(window("STATIC", WS_POPUP, null_mut()));
            let bitmap = BitmapDc::new();
            for action in 0..14 {
                let hwnd = edit(parent.0);
                apply(hwnd, Resolved::Dark, palette(), 96);
                let retained = state(hwnd).unwrap();
                let mut point = press(hwnd, true, Part::Thumb);
                point.y += 30;
                captured_mouse(hwnd, WM_MOUSEMOVE, point);
                let before = scroll(hwnd, SB_VERT);
                match action {
                    0 => {
                        SendMessageW(hwnd, WM_KEYDOWN, 0x1b, 0);
                    }
                    1 => {
                        ReleaseCapture();
                    }
                    2 => {
                        EnableWindow(hwnd, 0);
                    }
                    3 => {
                        ShowWindow(hwnd, SW_HIDE);
                    }
                    4 => apply(hwnd, Resolved::Contrast, palette(), 96),
                    5 => apply(hwnd, Resolved::Light, palette(), 96),
                    6 => {
                        SendMessageW(hwnd, WM_THEMECHANGED, 0, 0);
                    }
                    7 => {
                        SendMessageW(hwnd, WM_DPICHANGED_AFTERPARENT, 0, 0);
                    }
                    8 => {
                        MoveWindow(hwnd, 9, 13, 390, 250, 0);
                    }
                    9 => {
                        DestroyWindow(hwnd);
                    }
                    10 => {
                        SetCapture(parent.0);
                    }
                    11 => {
                        SendMessageW(hwnd, WM_SETREDRAW, 0, 0);
                    }
                    12 => {
                        // This is the keyboard-hook path used when focus stays
                        // on a different control, not a sent Edit key message.
                        assert_eq!(tracking_keyboard(HC_ACTION as i32, 0x1b, 0), 1);
                    }
                    _ => {
                        EnableScrollBar(hwnd, SB_VERT as u32, ESB_DISABLE_BOTH);
                        SendMessageW(hwnd, WM_TIMER, TRACK_TIMER, 0);
                    }
                }
                assert!(retained.get().tracking.is_none(), "cancel action {action}");
                assert_eq!(retained.get().timer, 0);
                assert!(retained.get().keyboard_hook.is_null());
                assert!(retained.get().hover.is_none());
                assert_ne!(GetCapture(), hwnd);
                if action == 10 {
                    assert_eq!(GetCapture(), parent.0);
                    ReleaseCapture();
                }
                if matches!(action, 4 | 5 | 9) {
                    assert!(!retained.get().attached);
                }
                if !matches!(action, 8 | 9) {
                    assert_eq!(
                        scroll(hwnd, SB_VERT),
                        before,
                        "cancel must not invent scroll state"
                    );
                    SendMessageW(hwnd, WM_TIMER, TRACK_TIMER, 0);
                    assert_eq!(
                        scroll(hwnd, SB_VERT),
                        before,
                        "stale repeat continued after cancel"
                    );
                }
                if matches!(action, 4 | 5) {
                    let bar = native_bar(hwnd, true);
                    let mut window = RECT::default();
                    GetWindowRect(hwnd, &mut window);
                    SendMessageW(hwnd, WM_PRINT, bitmap.dc as WPARAM, PRF_NONCLIENT as LPARAM);
                    assert_ne!(
                        GetPixel(
                            bitmap.dc,
                            bar.rcScrollBar.left - window.left + 1,
                            bar.rcScrollBar.top - window.top + bar.dxyLineButton + 2
                        ),
                        palette().background,
                        "contrast/light must restore the native frame during an active drag"
                    );
                }
                DestroyWindow(hwnd);
            }
        }
    }

    #[test]
    fn hidden_report_and_viewport_use_native_ranges_during_capture() {
        // SAFETY: hidden common controls and the real known viewport; no data
        // files or external GUI. All native scroll state stays control-owned.
        unsafe {
            let init = INITCOMMONCONTROLSEX {
                dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_LISTVIEW_CLASSES,
            };
            assert_ne!(InitCommonControlsEx(&init), 0);
            let calls = Cell::new(0);
            let parent = Window(window("STATIC", WS_POPUP, null_mut()));
            let list = window(
                "SysListView32",
                WS_CHILD | WS_VISIBLE | LVS_REPORT | LVS_OWNERDATA,
                parent.0,
            );
            let mut column = LVCOLUMNW {
                mask: LVCF_WIDTH,
                cx: 50_000,
                ..Default::default()
            };
            // Multiple normal columns avoid any per-column SDK/version width
            // limit while the total native horizontal range still exceeds 16 bits.
            for index in 0..3 {
                SendMessageW(
                    list,
                    LVM_INSERTCOLUMNW,
                    index,
                    (&mut column as *mut LVCOLUMNW) as LPARAM,
                );
            }
            SendMessageW(list, LVM_SETITEMCOUNT, 100_000, 0);
            tracking_probe(list, &calls);
            apply(list, Resolved::Dark, palette(), 96);
            let bitmap = BitmapDc::new();
            let initial = [scroll(list, SB_VERT), scroll(list, SB_HORZ)];
            for vertical in [true, false] {
                let mut point = press(list, vertical, Part::Thumb);
                if vertical {
                    point.y += 1000;
                } else {
                    point.x += 1000;
                }
                captured_mouse(list, WM_MOUSEMOVE, point);
                let info = scroll(list, if vertical { SB_VERT } else { SB_HORZ });
                assert!(info.3 > 65535, "list thumb truncated on axis {vertical}");
                assert_eq!(
                    (info.0, info.1, info.2),
                    (
                        initial[usize::from(!vertical)].0,
                        initial[usize::from(!vertical)].1,
                        initial[usize::from(!vertical)].2
                    )
                );
                if vertical {
                    assert_eq!(SendMessageW(list, LVM_GETTOPINDEX, 0, 0) as i32, info.3);
                }
                printed_tracking(list, &bitmap);
                captured_mouse(list, WM_LBUTTONUP, point);
            }
            let mut viewport = super::super::viewport::Viewport::default();
            viewport.create(parent.0).unwrap();
            viewport.layout(96, 640, 360, 0);
            viewport.prepare(96, 150_000, 0);
            viewport.arrange(96, 150_000, 0);
            let hwnd = viewport.handle;
            tracking_probe(hwnd, &calls);
            apply(hwnd, Resolved::Dark, palette(), 96);
            let original = scroll(hwnd, SB_VERT);
            let mut point = press(hwnd, true, Part::Thumb);
            point.y += 1000;
            captured_mouse(hwnd, WM_MOUSEMOVE, point);
            assert!(
                scroll(hwnd, SB_VERT).3 > 65535,
                "viewport requires full-width scalar scroll_to"
            );
            let position = scroll(hwnd, SB_VERT).3;
            printed_tracking(hwnd, &bitmap);
            captured_mouse(hwnd, WM_LBUTTONUP, point);
            viewport.refresh(96, 150_000, 0);
            let final_info = scroll(hwnd, SB_VERT);
            assert_eq!(
                final_info.3, position,
                "viewport stored offset must match native position"
            );
            assert_eq!(
                (final_info.0, final_info.1, final_info.2),
                (original.0, original.1, original.2)
            );
            assert_eq!(calls.get(), 0);
        }
    }

    #[test]
    fn shared_viewport_page_change_cancels_tracking_without_overwriting_offsets() {
        use super::super::viewport;
        // SAFETY: Hidden native test host and real NC input; the lower probe
        // prevents a regression entering Windows' modal scrollbar loop.
        unsafe {
            let calls = Cell::new(0);
            let parent = Window(window("STATIC", WS_POPUP, null_mut()));
            let mut viewport = viewport::Viewport::default();
            viewport.create(parent.0).unwrap();
            viewport.layout(96, 800, 600, 0);
            let hwnd = viewport.handle;
            for (page, position) in [(0, 40), (1, 120), (0, 40)] {
                viewport.prepare(96, 820, page);
                viewport.refresh(96, 820, page);
                viewport::scroll_to(hwnd, position);
            }
            tracking_probe(hwnd, &calls);
            apply(hwnd, Resolved::Dark, palette(), 96);
            let retained = state(hwnd).unwrap();
            for part in [Part::Thumb, Part::LineAfter, Part::PageAfter] {
                // A native page step must not reach the end and legitimately
                // cancel capture before the active-page transition under test.
                let height = if part == Part::PageAfter { 4000 } else { 820 };
                for (from, to) in [(0, 1), (1, 0)] {
                    for (page, position) in [(0, 40), (1, 120)] {
                        viewport.prepare(96, height, page);
                        viewport.refresh(96, height, page);
                        viewport::scroll_to(hwnd, position);
                    }
                    viewport.prepare(96, height, from);
                    viewport.refresh(96, height, from);
                    let point = press(hwnd, true, part);
                    let tracking = retained.get().tracking.unwrap();
                    let source = scroll(hwnd, SB_VERT).3;
                    assert_ne!(retained.get().timer, 0);
                    assert!(!retained.get().keyboard_hook.is_null());

                    // A routine same-page cache refresh must not cancel a drag.
                    viewport.prepare(96, height, from);
                    viewport.refresh(96, height, from);
                    assert!(same_tracking(hwnd, &retained, tracking));
                    viewport.prepare(96, height, to);
                    assert!(retained.get().tracking.is_none());
                    assert_eq!(retained.get().timer, 0);
                    assert!(retained.get().keyboard_hook.is_null());
                    assert!(!retained.get().tracking_leave);
                    assert!(retained.get().hover.is_none());
                    assert_ne!(GetCapture(), hwnd);
                    viewport.refresh(96, height, to);
                    let target = scroll(hwnd, SB_VERT).3;

                    // Already queued timer/mouse messages belong to the old page.
                    SendMessageW(hwnd, WM_TIMER, TRACK_TIMER, 0);
                    captured_mouse(hwnd, WM_MOUSEMOVE, point);
                    captured_mouse(hwnd, WM_LBUTTONUP, point);
                    assert_eq!(scroll(hwnd, SB_VERT).3, target);
                    viewport.refresh(96, height, to);
                    assert_eq!(scroll(hwnd, SB_VERT).3, target);
                    viewport.prepare(96, height, from);
                    viewport.refresh(96, height, from);
                    assert_eq!(scroll(hwnd, SB_VERT).3, source);
                }
            }
            assert_eq!(calls.get(), 0);
        }
    }

    #[test]
    fn shared_viewport_page_change_can_destroy_host_during_capture_release() {
        use super::super::viewport;

        #[derive(Default)]
        struct CaptureDestroyProbe {
            capture_changes: Cell<usize>,
            clean_before_destroy: Cell<bool>,
            destroyed: Cell<bool>,
            nc_destroys: Cell<usize>,
        }
        unsafe extern "system" fn destroy_on_capture_changed(
            hwnd: HWND,
            message: u32,
            w: WPARAM,
            l: LPARAM,
            id: usize,
            data: usize,
        ) -> LRESULT {
            // SAFETY: Probe data outlives the HWND owner. Count before destroying
            // so nested capture notifications cannot trigger a second destroy.
            unsafe {
                let probe = &*(data as *const CaptureDestroyProbe);
                if message == WM_CAPTURECHANGED {
                    let previous = probe.capture_changes.get();
                    probe.capture_changes.set(previous + 1);
                    if previous == 0 {
                        let clean = state(hwnd).is_some_and(|state| {
                            let value = state.get();
                            value.attached
                                && value.tracking.is_none()
                                && value.timer == 0
                                && value.keyboard_hook.is_null()
                                && value.hover.is_none()
                                && !value.tracking_leave
                        });
                        probe
                            .clean_before_destroy
                            .set(clean && GetCapture().is_null());
                        probe.destroyed.set(DestroyWindow(hwnd) != 0);
                        return 0;
                    }
                }
                if message == WM_NCDESTROY {
                    probe.nc_destroys.set(probe.nc_destroys.get() + 1);
                    RemoveWindowSubclass(hwnd, Some(destroy_on_capture_changed), id);
                }
                DefSubclassProc(hwnd, message, w, l)
            }
        }

        // SAFETY: Hidden real viewport and NC input on this thread. Install the
        // destroy probe below the dark tracker; real NCDESTROY frees its Page
        // while the active-page message is still inside cancel/ReleaseCapture.
        unsafe {
            for refresh in [false, true] {
                let probe = CaptureDestroyProbe::default();
                let calls = Cell::new(0);
                let parent = Window(window("STATIC", WS_POPUP, null_mut()));
                let mut viewport = viewport::Viewport::default();
                viewport.create(parent.0).unwrap();
                viewport.layout(96, 800, 600, 0);
                viewport.prepare(96, 820, 0);
                viewport.refresh(96, 820, 0);
                let hwnd = viewport.handle;
                viewport::scroll_to(hwnd, 40);
                assert_ne!(
                    SetWindowSubclass(
                        hwnd,
                        Some(destroy_on_capture_changed),
                        SUBCLASS_ID + 3,
                        (&probe as *const CaptureDestroyProbe) as usize,
                    ),
                    0
                );
                tracking_probe(hwnd, &calls);
                apply(hwnd, Resolved::Dark, palette(), 96);
                let retained = state(hwnd).unwrap();
                press(hwnd, true, Part::Thumb);
                assert_ne!(retained.get().timer, 0);
                assert!(!retained.get().keyboard_hook.is_null());
                assert_eq!(probe.capture_changes.get(), 0);

                if refresh {
                    viewport.refresh(96, 820, 1);
                } else {
                    assert_eq!(viewport.prepare(96, 820, 1), 1);
                }

                assert_eq!(probe.capture_changes.get(), 1);
                assert!(probe.clean_before_destroy.get());
                assert!(probe.destroyed.get());
                assert_eq!(probe.nc_destroys.get(), 1);
                assert_eq!(IsWindow(hwnd), 0);
                assert!(state(hwnd).is_none());
                let value = retained.get();
                assert!(!value.attached);
                assert!(!value.enabled);
                assert!(value.tracking.is_none());
                assert_eq!(value.timer, 0);
                assert!(value.keyboard_hook.is_null());
                assert!(value.hover.is_none());
                assert!(!value.tracking_leave);
                assert!(GetCapture().is_null());
                assert_eq!(calls.get(), 0);
            }
        }
    }

    #[test]
    fn cancellation_leaves_native_selection_and_other_window_capture_untouched() {
        // SAFETY: Private HWNDs on this thread; cancel owns neither native Edit
        // selection capture nor a sibling's capture and must not release either.
        unsafe {
            let parent = Window(window("STATIC", WS_POPUP, null_mut()));
            let hwnd = edit(parent.0);
            let other = edit(parent.0);
            apply(hwnd, Resolved::Dark, palette(), 96);
            let selected = selection(hwnd);
            for captured in [hwnd, other] {
                SetCapture(captured);
                cancel(hwnd);
                assert_eq!(GetCapture(), captured);
                assert_eq!(selection(hwnd), selected);
                ReleaseCapture();
            }
        }
    }

    #[test]
    fn geometry_and_no_redraw_queue_damage_without_synchronous_overlay() {
        // SAFETY: hidden HWND/DC; the test hook observes only this thread's NC
        // screen passes, not printing. This checks the intermediate layout, not
        // merely a screenshot after the transaction has already completed.
        unsafe {
            let parent = Window(window("STATIC", WS_POPUP, null_mut()));
            let hwnd = edit(parent.0);
            apply(hwnd, Resolved::Dark, palette(), 96);
            let before = SCREEN_PASSES.with(Cell::get);
            SendMessageW(hwnd, WM_SETREDRAW, 0, 0);
            MoveWindow(hwnd, 31, 29, 390, 250, 0);
            SendMessageW(hwnd, WM_DPICHANGED_AFTERPARENT, 0, 0);
            SendMessageW(hwnd, WM_THEMECHANGED, 0, 0);
            SendMessageW(hwnd, WM_NCPAINT, 1, 0);
            assert_eq!(
                SCREEN_PASSES.with(Cell::get),
                before,
                "geometry/no-redraw dispatched an intermediate overlay"
            );
            assert!(state(hwnd).unwrap().get().tracking.is_none());
            SendMessageW(hwnd, WM_SETREDRAW, 1, 0);
            assert_eq!(SCREEN_PASSES.with(Cell::get), before);
            SendMessageW(hwnd, WM_NCPAINT, 1, 0);
            assert!(SCREEN_PASSES.with(Cell::get) > before);
            ValidateRect(hwnd, null());
            invalidate_frame(hwnd);
            assert_eq!(
                GetUpdateRect(hwnd, std::ptr::null_mut(), 0),
                0,
                "NC-only damage must not create a client update region"
            );
        }
    }

    #[test]
    fn mouse_tracking_print_and_cancellation_do_not_leak_gdi() {
        // SAFETY: hidden scratch windows and bitmap; repeated real capture is
        // balanced before resource counts, and no timer stays active afterwards.
        unsafe {
            let calls = Cell::new(0);
            let parent = Window(window("STATIC", WS_POPUP, null_mut()));
            let hwnd = edit(parent.0);
            tracking_probe(hwnd, &calls);
            apply(hwnd, Resolved::Dark, palette(), 96);
            let bitmap = BitmapDc::new();
            let once = || {
                for vertical in [true, false] {
                    let message = if vertical { WM_VSCROLL } else { WM_HSCROLL };
                    SendMessageW(hwnd, message, SB_TOP as WPARAM, 0);
                    SendMessageW(
                        hwnd,
                        WM_PRINT,
                        bitmap.dc as WPARAM,
                        (PRF_CLIENT | PRF_NONCLIENT) as LPARAM,
                    );
                    for release in [true, false] {
                        SendMessageW(hwnd, message, SB_TOP as WPARAM, 0);
                        let mut point = press(hwnd, vertical, Part::Thumb);
                        if vertical {
                            point.y += 45;
                        } else {
                            point.x += 45;
                        }
                        captured_mouse(hwnd, WM_MOUSEMOVE, point);
                        printed_tracking(hwnd, &bitmap);
                        if release {
                            captured_mouse(hwnd, WM_LBUTTONUP, point);
                        } else {
                            SendMessageW(hwnd, WM_CANCELMODE, 0, 0);
                        }
                    }
                    for part in [Part::LineAfter, Part::PageAfter] {
                        SendMessageW(hwnd, message, SB_TOP as WPARAM, 0);
                        press(hwnd, vertical, part);
                        SendMessageW(hwnd, WM_TIMER, TRACK_TIMER, 0);
                        SendMessageW(hwnd, WM_KEYDOWN, 0x1b, 0);
                    }
                    SendMessageW(hwnd, WM_NCPAINT, 1, 0);
                    assert_eq!(state(hwnd).unwrap().get().timer, 0);
                    assert!(state(hwnd).unwrap().get().keyboard_hook.is_null());
                    assert_ne!(GetCapture(), hwnd);
                }
                GdiFlush();
                GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS)
            };
            // Warm the complete cycle, including returning from a scrolled
            // native print to the top, horizontal font queries and both exits.
            // One vertical drag omits those lazy native/GDI initialization paths.
            once();
            let before = once();
            assert_eq!(once(), before, "full-path GDI warmup did not stabilize");
            for iteration in 0..100 {
                assert_eq!(
                    once(),
                    before,
                    "GDI grew in complete capture/print cycle {iteration}"
                );
            }
            println!(
                "100 complete mouse/print/cancel cycles: GDI {before} -> {}",
                GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS)
            );
            assert_eq!(state(hwnd).unwrap().get().timer, 0);
            assert_ne!(GetCapture(), hwnd);
            assert_eq!(calls.get(), 0);
        }
    }

    #[test]
    fn slim_thumb_is_narrow_centered_and_keeps_native_track() {
        let bar = |vertical| Bar {
            rect: RECT {
                left: 0,
                top: 0,
                right: 200,
                bottom: 200,
            },
            vertical,
            button: 17,
            thumb_top: 30,
            thumb_bottom: 90,
            states: [0; 6],
            scrollable: true,
        };
        let vertical = slim_thumb(bar(true), 96).unwrap();
        assert_eq!(vertical.right - vertical.left, scale(6, 96));
        assert_eq!((vertical.left + vertical.right) / 2, 100);
        assert_eq!(vertical.top, 30);
        let horizontal = slim_thumb(
            Bar {
                thumb_top: 80,
                thumb_bottom: 120,
                ..bar(false)
            },
            192,
        )
        .unwrap();
        assert_eq!(horizontal.bottom - horizontal.top, scale(6, 192));
        assert_eq!((horizontal.top + horizontal.bottom) / 2, 100);
        assert_eq!(horizontal.left, 80);
        assert_eq!(horizontal.right, 120);
        let mut hidden = bar(true);
        hidden.states[3] = STATE_SYSTEM_INVISIBLE;
        assert!(slim_thumb(hidden, 96).is_none());
        let mut fixed = bar(true);
        fixed.scrollable = false;
        assert!(slim_thumb(fixed, 96).is_none());
    }
}
