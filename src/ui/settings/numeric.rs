//! 数字字段的配置边界、编辑校验和原生 Edit 输入入口共用一份元数据。
use super::*;
use std::{cell::Cell, rc::Rc};
use windows_sys::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};

const SUBCLASS_ID: usize = 0x534e_554d;
const EM_REPLACESEL: u32 = 0x00c2;
const EM_SETCUEBANNER: u32 = 0x1501;
const EM_UNDO: u32 = 0x00c7;
type Proc = Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM, usize, usize) -> LRESULT>;
#[link(name = "comctl32")]
unsafe extern "system" {
    fn SetWindowSubclass(hwnd: HWND, proc: Proc, id: usize, data: usize) -> i32;
    fn RemoveWindowSubclass(hwnd: HWND, proc: Proc, id: usize) -> i32;
    fn DefSubclassProc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT;
}
#[link(name = "user32")]
unsafe extern "system" {
    fn OpenClipboard(owner: HWND) -> i32;
    fn GetClipboardData(format: u32) -> *mut std::ffi::c_void;
    fn CloseClipboard() -> i32;
    fn GetKeyState(key: i32) -> i16;
}

#[derive(Clone, Copy, Debug)]
pub(super) struct NumberField {
    pub section: &'static str,
    pub field: &'static str,
    pub caption: &'static str,
    pub min: u32,
    pub max: u32,
    pub hint: &'static str,
    pub error: &'static str,
}
impl NumberField {
    pub fn digits(self) -> usize {
        self.max.to_string().len()
    }
    pub fn parse(self, input: &str) -> Result<u32, &'static str> {
        if input.is_empty() {
            return Err("不能为空，请填写范围内的整数。");
        }
        if !input.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err("仅接受 ASCII 数字 0–9，不接受空格、符号或小数。");
        }
        if input.len() > self.digits() {
            return Err(self.error);
        }
        input
            .parse::<u32>()
            .ok()
            .filter(|value| (self.min..=self.max).contains(value))
            .ok_or(self.error)
    }
}

pub(super) const PORT: NumberField = NumberField {
    section: "listen",
    field: "port",
    caption: "监听端口",
    min: 1,
    max: u16::MAX as u32,
    hint: "1–65535",
    error: "端口范围 1–65535。",
};

// Config::validate_for_save deliberately permits the full u32 range. Only the
// two idle timeouts accept zero; do not substitute more restrictive UI limits.
pub(super) const NETWORK: [NumberField; 6] = [
    NumberField {
        section: "network",
        field: "connect_timeout_secs",
        caption: "上游建连超时（秒）",
        min: 1,
        max: u32::MAX,
        hint: "1–4294967295 秒",
        error: "范围为 1–4294967295 秒。",
    },
    NumberField {
        section: "network",
        field: "response_header_timeout_secs",
        caption: "响应头超时（秒）",
        min: 1,
        max: u32::MAX,
        hint: "1–4294967295 秒",
        error: "范围为 1–4294967295 秒。",
    },
    NumberField {
        section: "network",
        field: "response_body_idle_timeout_secs",
        caption: "响应体空闲超时（秒）",
        min: 0,
        max: u32::MAX,
        hint: "0–4294967295 秒；0 禁用",
        error: "范围为 0–4294967295 秒；0 禁用。",
    },
    NumberField {
        section: "network",
        field: "tunnel_idle_timeout_secs",
        caption: "CONNECT 空闲超时（秒）",
        min: 0,
        max: u32::MAX,
        hint: "0–4294967295 秒；0 禁用",
        error: "范围为 0–4294967295 秒；0 禁用。",
    },
    NumberField {
        section: "network",
        field: "shutdown_grace_secs",
        caption: "停止宽限期（秒）",
        min: 1,
        max: u32::MAX,
        hint: "1–4294967295 秒",
        error: "范围为 1–4294967295 秒。",
    },
    NumberField {
        section: "network",
        field: "max_active_requests",
        caption: "最大活动请求／隧道数",
        min: 1,
        max: u32::MAX,
        hint: "1–4294967295 个",
        error: "范围为 1–4294967295。",
    },
];

pub(super) fn find(section: &str, field: &str) -> Option<NumberField> {
    std::iter::once(PORT)
        .chain(NETWORK)
        .find(|spec| spec.section == section && spec.field == field)
}

#[derive(Clone, Copy)]
pub(super) struct NumberControl {
    pub control: HWND,
    pub label: HWND,
    pub hint: HWND,
    pub spec: NumberField,
}
#[derive(Clone, Copy)]
struct EditState {
    hint: HWND,
    spec: NumberField,
    attached: bool,
}
type EditOwner = Rc<Cell<EditState>>;

fn input_shape(spec: NumberField, input: &str) -> Result<(), &'static str> {
    if !input.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("未接受输入：仅限数字 0–9。");
    }
    if input.len() > spec.digits() {
        return Err(if spec.max == u16::MAX as u32 {
            "未接受输入：最多 5 位数字。"
        } else {
            "未接受输入：最多 10 位数字。"
        });
    }
    Ok(())
}

pub(super) fn replacement(
    spec: NumberField,
    current: &str,
    selection: (u32, u32),
    insert: &str,
) -> Result<String, &'static str> {
    input_shape(spec, current)?;
    input_shape(spec, insert)?;
    // Accepted text is ASCII, so native UTF-16 selection offsets are byte offsets.
    let start = selection.0.min(selection.1) as usize;
    let end = selection.0.max(selection.1) as usize;
    let start = start.min(current.len());
    let end = end.min(current.len());
    let candidate = format!("{}{}{}", &current[..start], insert, &current[end..]);
    input_shape(spec, &candidate)?;
    Ok(candidate)
}

fn attached_text(hwnd: HWND, state: &EditOwner) -> Option<String> {
    if !state.get().attached {
        return None;
    }
    // SAFETY: Owner-thread HWND and bounded writable buffer. Each text query
    // can invoke a subclass which destroys Edit; retaining Rc is not enough to
    // keep either HWND alive. Recheck our attachment between the native calls.
    unsafe {
        let length = GetWindowTextLengthW(hwnd).clamp(0, 16384) as usize;
        if !state.get().attached {
            return None;
        }
        let mut buffer = vec![0; length + 1];
        let count = GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32);
        if !state.get().attached {
            return None;
        }
        Some(String::from_utf16_lossy(
            &buffer[..count.clamp(0, length as i32) as usize],
        ))
    }
}

fn hint(state: &EditOwner, message: &str) {
    let hwnd = state.get().hint;
    let Some(current) = attached_text(hwnd, state) else {
        return;
    };
    if state.get().attached && current != message {
        // SetWindowText can destroy Edit too; do not continue with HWNDs here.
        set_text(hwnd, message);
    }
}
fn refresh(hwnd: HWND, state: &EditOwner) {
    let Some(input) = attached_text(hwnd, state) else {
        return;
    };
    let spec = state.get().spec;
    let message = if input.is_empty() {
        if spec.max == u16::MAX as u32 {
            "必填：1–65535"
        } else {
            "必填；请输入范围内整数"
        }
    } else {
        spec.parse(&input).err().unwrap_or(spec.hint)
    };
    hint(state, message);
}
fn accepts(hwnd: HWND, state: &EditOwner, insert: &str) -> bool {
    if !state.get().attached {
        return false;
    }
    let mut start = 0u32;
    let mut end = 0u32;
    // SAFETY: Live native EDIT and writable selection endpoints; no Cell borrow.
    unsafe {
        SendMessageW(
            hwnd,
            EM_GETSEL,
            (&mut start as *mut u32) as WPARAM,
            (&mut end as *mut u32) as LPARAM,
        );
    }
    if !state.get().attached {
        return false;
    }
    let Some(current) = attached_text(hwnd, state) else {
        return false;
    };
    match replacement(state.get().spec, &current, (start, end), insert) {
        Ok(_) => state.get().attached,
        Err(error) => {
            hint(state, error);
            false
        }
    }
}

// Read at most one field's capacity plus the terminator. A longer clipboard
// value is rejected whole, never truncated into a different valid number.
fn clipboard(owner: HWND, state: &EditOwner) -> Result<Option<String>, &'static str> {
    if !state.get().attached {
        return Ok(None);
    }
    let spec = state.get().spec;
    #[cfg(test)]
    {
        let input = TEST_CLIPBOARD.with(|source| source.borrow_mut().take());
        if let Some(input) = input {
            // Model a delayed clipboard provider's synchronous callback without
            // touching the desktop clipboard. Release the RefCell borrow first.
            if let Some(callback) = TEST_CLIPBOARD_REENTRY.with(|hook| hook.take()) {
                callback(owner);
            }
            return input
                .and_then(|units| clipboard_text(spec, &units))
                .map(Some);
        }
    }
    struct Clipboard;
    impl Drop for Clipboard {
        fn drop(&mut self) {
            // SAFETY: Owns one successful OpenClipboard on this thread.
            unsafe { CloseClipboard() };
        }
    }
    // SAFETY: Clipboard handles stay borrowed only until CloseClipboard; the
    // bounded locked allocation is copied and unlocked before any edit dispatch.
    unsafe {
        if OpenClipboard(owner) == 0 {
            return Err("未粘贴：剪贴板正忙，请重试。");
        }
        let _clipboard = Clipboard;
        if !state.get().attached {
            return Ok(None);
        }
        let memory = GetClipboardData(13); // CF_UNICODETEXT; may render delayed data.
        if !state.get().attached {
            return Ok(None);
        }
        if memory.is_null() {
            return Err("未粘贴：需要纯数字文本。");
        }
        let count = (GlobalSize(memory) / size_of::<u16>()).min(spec.digits() + 1);
        if count == 0 {
            return Err("未粘贴：文本缓冲区无效。");
        }
        let pointer = GlobalLock(memory).cast::<u16>();
        if pointer.is_null() {
            return Err("未粘贴：无法读取剪贴板。");
        }
        let buffer = std::slice::from_raw_parts(pointer, count);
        let value = clipboard_text(spec, buffer);
        GlobalUnlock(memory);
        value.map(Some)
    }
}

#[cfg(test)]
thread_local! {
    // One-shot source injection exercises native WM_PASTE without touching the
    // user's process-wide clipboard. The production reader is unchanged.
    static TEST_CLIPBOARD: std::cell::RefCell<Option<Result<Vec<u16>, &'static str>>> = const { std::cell::RefCell::new(None) };
    static TEST_CLIPBOARD_REENTRY: Cell<Option<fn(HWND)>> = const { Cell::new(None) };
}
#[cfg(test)]
pub(super) fn test_clipboard(input: Result<Vec<u16>, &'static str>) {
    TEST_CLIPBOARD.with(|source| *source.borrow_mut() = Some(input));
}
#[cfg(test)]
pub(super) fn test_clipboard_reentry(callback: fn(HWND)) {
    TEST_CLIPBOARD_REENTRY.with(|hook| hook.set(Some(callback)));
}

pub(super) fn clipboard_text(spec: NumberField, buffer: &[u16]) -> Result<String, &'static str> {
    let end = buffer
        .iter()
        .take(spec.digits() + 1)
        .position(|unit| *unit == 0)
        .ok_or("未粘贴：数字过长。")?;
    let input = String::from_utf16(&buffer[..end]).map_err(|_| "未粘贴：需要数字 0–9。")?;
    input_shape(spec, &input)?;
    Ok(input)
}

pub(super) fn shift_insert(key: WPARAM, shift: bool, control: bool) -> bool {
    key == 0x2d && shift && !control
}

unsafe fn argument(lparam: LPARAM, spec: NumberField) -> Result<String, &'static str> {
    if lparam == 0 {
        return Ok(String::new());
    }
    // SAFETY: WM_SETTEXT/EM_REPLACESEL guarantee a terminated UTF-16 input. Stop
    // after the field's small bound even when the supplied text is much longer.
    unsafe {
        let pointer = lparam as *const u16;
        let mut units = Vec::with_capacity(spec.digits());
        for index in 0..=spec.digits() {
            let unit = *pointer.add(index);
            if unit == 0 {
                return String::from_utf16(&units).map_err(|_| "未接受输入：仅限数字 0–9。");
            }
            units.push(unit);
        }
        Err("未接受输入：数字过长。")
    }
}

pub(super) fn attach(field: NumberControl) -> Result<(), String> {
    let state = Rc::new(Cell::new(EditState {
        hint: field.hint,
        spec: field.spec,
        attached: true,
    }));
    let data = Rc::into_raw(state.clone());
    // SAFETY: Owner-thread live EDIT; subclass owns one Rc until NCDESTROY.
    unsafe {
        if SetWindowSubclass(field.control, Some(procedure), SUBCLASS_ID, data as usize) == 0 {
            drop(Rc::from_raw(data));
            return Err(last_error("安装数字输入校验"));
        }
        SendMessageW(field.control, EM_SETLIMITTEXT, field.spec.digits(), 0);
        if !state.get().attached {
            return Err("数字输入框在初始化期间已销毁。".into());
        }
        let cue = wide(&format!("{}：{}", field.spec.caption, field.spec.hint));
        SendMessageW(field.control, EM_SETCUEBANNER, 0, cue.as_ptr() as LPARAM);
        if !state.get().attached {
            return Err("数字输入框在初始化期间已销毁。".into());
        }
    }
    refresh(field.control, &state);
    if !state.get().attached {
        return Err("数字输入框在初始化期间已销毁。".into());
    }
    Ok(())
}

unsafe extern "system" fn procedure(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    // SAFETY: This proc/id owns one Rc. Retain locally before native dispatch:
    // EDIT notifications can destroy the HWND and release the subclass's Rc.
    unsafe {
        let pointer = data as *const Cell<EditState>;
        Rc::increment_strong_count(pointer);
        let state = Rc::from_raw(pointer);
        let value = state.get();
        if message == WM_NCDESTROY {
            state.set(EditState {
                attached: false,
                ..value
            });
            if RemoveWindowSubclass(hwnd, Some(procedure), id) != 0 {
                drop(Rc::from_raw(pointer));
            }
            return DefSubclassProc(hwnd, message, w, l);
        }
        if !state.get().attached {
            return 0;
        }
        // Shift+Insert can paste inside native WM_KEYDOWN handling without a
        // WM_CHAR. Intercept it before dispatch, retaining native Copy/Cut/Undo.
        let keyboard_paste = message == WM_KEYDOWN
            && w == 0x2d
            && shift_insert(w, GetKeyState(0x10) < 0, GetKeyState(0x11) < 0);
        if message == WM_PASTE || (message == WM_CHAR && w == 0x16) || keyboard_paste {
            let input = clipboard(hwnd, &state);
            // OpenClipboard/GetClipboardData/CloseClipboard may dispatch delayed
            // provider work. Never continue into an HWND recycled after NCDESTROY.
            if !state.get().attached {
                return 0;
            }
            match input {
                Ok(Some(input)) => {
                    let input = wide(&input);
                    SendMessageW(hwnd, EM_REPLACESEL, 1, input.as_ptr() as LPARAM);
                }
                Ok(None) => {}
                Err(error) => hint(&state, error),
            }
            return 0;
        }
        if matches!(message, WM_SETTEXT | EM_REPLACESEL) {
            let input = argument(l, state.get().spec);
            if !state.get().attached {
                return 0;
            }
            let input = match input {
                Ok(input) => input,
                Err(error) => {
                    hint(&state, error);
                    return 0;
                }
            };
            if message == WM_SETTEXT {
                if let Err(error) = input_shape(state.get().spec, &input) {
                    hint(&state, error);
                    return 0;
                }
            } else if !accepts(hwnd, &state, &input) {
                return 0;
            }
        } else if (message == WM_CHAR && w >= 32 && w != 127) || message == WM_IME_CHAR {
            let input = char::from_u32(w as u32).map(|c| c.to_string());
            if !input.is_some_and(|input| accepts(hwnd, &state, &input)) {
                return 0;
            }
        }
        // accepts/hint can synchronously destroy Edit during a query or text
        // update. This is the final gate before continuing the native message.
        if !state.get().attached {
            return 0;
        }
        let result = DefSubclassProc(hwnd, message, w, l);
        if matches!(
            message,
            WM_SETTEXT
                | EM_REPLACESEL
                | WM_CHAR
                | WM_IME_CHAR
                | WM_KEYDOWN
                | WM_CUT
                | WM_CLEAR
                | WM_UNDO
                | EM_UNDO
        ) {
            refresh(hwnd, &state);
        }
        result
    }
}
