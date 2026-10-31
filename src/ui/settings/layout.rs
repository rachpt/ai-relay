//! 字段宽度与网络页响应式排布；只移动 HWND，不重建或提交草稿。
use super::*;

pub(super) const NETWORK_TWO_COLUMNS_DIP: i32 = 680;
const NETWORK_ROW_DIP: i32 = 88;
const NETWORK_COLUMN_MAX_DIP: i32 = 400;

pub(super) fn logical_width(width: i32, dpi: u32) -> i32 {
    ((i64::from(width) * 96 + i64::from(dpi.max(1)) / 2) / i64::from(dpi.max(1))) as i32
}

pub(super) fn network_columns(width_dip: i32) -> usize {
    if width_dip >= NETWORK_TWO_COLUMNS_DIP {
        2
    } else {
        1
    }
}
pub(super) fn network_actions_y(width_dip: i32) -> i32 {
    let rows = numeric::NETWORK.len().div_ceil(network_columns(width_dip));
    160 + rows as i32 * NETWORK_ROW_DIP - 4 + 24
}

impl Settings {
    pub(super) fn row_geometry(
        &self,
        dpi: u32,
        width: i32,
        row: (HWND, i32, i32, i32, i32),
    ) -> (i32, i32, i32, i32) {
        let (control, x, y, w, h) = row;
        let px = |dip| scale(dip, dpi);
        let available = |left: i32| (width - (left - px(24))).max(1);
        let fixed = |left: i32, top: i32, cap: i32, height: i32| {
            (left, top, px(cap).min(available(left)), px(height))
        };
        let left = px(180);
        let ip_width = px(256).min((available(left) - px(172)).max(1));
        if control == self.ip {
            return (left, px(y - 48), ip_width, px(h));
        }
        if let Some(number) = self
            .numbers
            .iter()
            .find(|number| number.control == self.port)
        {
            let port_label_x = left + ip_width + px(16);
            if control == number.label {
                return fixed(port_label_x, px(y - 48), 40, h);
            }
            if control == number.control {
                return fixed(port_label_x + px(48), px(y - 48), 108, h);
            }
            if control == number.hint
                && let Some(&(_, _, input_y, _, input_height)) =
                    self.controls.iter().find(|row| row.0 == self.port)
            {
                // Scale the actual Edit top and height separately, just as the
                // HWND is arranged. Scaling their DIP sum rounds one pixel up
                // into the Edit at 125%/175%; keep a real four-DIP gap instead.
                let hint_top = px(input_y - 48) + px(input_height) + px(4);
                return fixed(left, hint_top, 550, 20);
            }
        }
        let key_button_width = px(148).min(available(left) / 2);
        let key_button_x = left + available(left) - key_button_width;
        if control == self.key {
            return (
                left,
                px(y - 48),
                (key_button_x - left - px(8)).max(1),
                px(h),
            );
        }
        if control == self.key_visibility {
            return (key_button_x, px(y - 48), key_button_width.max(1), px(h));
        }
        let cap = if control == self.injection || control == self.storage {
            Some(320)
        } else if control == self.secrets {
            Some(440)
        } else if control == self.theme {
            Some(456)
        } else if control == self.category {
            Some(864)
        } else {
            None
        };
        if let Some(cap) = cap {
            return fixed(px(x), px(y - 48), cap, h);
        }
        for (index, number) in self
            .numbers
            .iter()
            .filter(|n| n.spec.section == "network")
            .enumerate()
        {
            let columns = network_columns(self.layout_width.get());
            let column_width = ((width - px(24) * (columns as i32 - 1)) / columns as i32)
                .min(px(NETWORK_COLUMN_MAX_DIP))
                .max(1);
            let column = index % columns;
            let row = index / columns;
            let left = px(24) + column as i32 * (column_width + px(24));
            let top = px(160 - 48 + row as i32 * NETWORK_ROW_DIP);
            if control == number.label {
                return (left, top, column_width, px(24));
            }
            if control == number.control {
                return (left, top + px(28), px(148).min(column_width), px(30));
            }
            if control == number.hint {
                return (left, top + px(62), column_width, px(22));
            }
        }
        // Compact CONNECT's short port array; URL/Origin/CIDR lists remain elastic.
        if self
            .fields
            .iter()
            .any(|(handle, _, field)| *handle == control && *field == "connect_allowed_ports")
        {
            return fixed(px(280), px(y - 48), 220, h);
        }
        // Text lists and credentials use the space the user actually has. Native
        // checkbox captions carry their own accessible name instead of an empty
        // checkbox paired with a separate static label.
        if let Some(&(label, edit)) = self
            .field_rows
            .iter()
            .find(|(label, edit)| *label == control || *edit == control)
        {
            // SAFETY: Same-thread live child style query; no native dispatch.
            let checkbox = unsafe {
                GetWindowLongPtrW(edit, GWL_STYLE) as u32 & BS_TYPEMASK as u32
                    == BS_AUTOCHECKBOX as u32
            };
            let left = if checkbox { px(24) } else { px(280) };
            if control == edit {
                return (left, px(y - 48), available(left), px(h));
            }
            if control == label {
                return fixed(px(24), px(y - 48), 244, h);
            }
        }
        // File actions retain their original IDs but wrap on narrow work areas.
        // SAFETY: Same-thread live child ID query; no native dispatch.
        let id = unsafe { GetDlgCtrlID(control) as u32 };
        if self.layout_width.get() < 688 {
            if let Some(index) = [ROLLBACK, EXPORT, IMPORT, RESTORE]
                .iter()
                .position(|candidate| *candidate == id)
            {
                let left = px(24 + (index % 2) as i32 * 176);
                return fixed(left, px(474 - 48 + (index / 2) as i32 * 42), 164, h);
            }
            if id == LOGIN_QUERY {
                return fixed(px(24), px(398 - 48), 144, h);
            }
            if control == self.login {
                return (px(24), px(y - 48), width.max(1), px(h));
            }
            if control == self.login_status {
                return (px(24), px(436 - 48), width.max(1), px(h));
            }
        }
        let width = if w == 550 || w == 706 {
            available(px(x))
        } else {
            px(w).min(available(px(x)))
        };
        (px(x), px(y - 48), width, px(h))
    }

    pub(super) fn action_geometry(
        &self,
        dpi: u32,
        width: i32,
        id: u32,
        y: i32,
    ) -> Option<(i32, i32, i32, i32)> {
        let index = [SAVE, START, STOP, LOAD]
            .iter()
            .position(|candidate| *candidate == id)?;
        let narrow = self.layout_width.get() < 688;
        let (x, y, w) = if narrow {
            (
                24 + (index % 2) as i32 * 212,
                y + (index / 2) as i32 * 44,
                196,
            )
        } else {
            let x = [24, 188, 400, 564][index];
            (x, y, if id == START { 196 } else { 148 })
        };
        Some((
            scale(x, dpi),
            scale(y - 48, dpi),
            scale(w, dpi).min((width - scale(x - 24, dpi)).max(1)),
            scale(32, dpi),
        ))
    }

    pub(super) fn reorder_navigation(&self) {
        // The category is created last and storage controls precede network
        // controls in creation order. Explicit ordering keeps native Tab/Shift+Tab
        // aligned with the visible form, including the reveal button after Key.
        let mut order: Vec<_> = self.controls.iter().map(|&(handle, ..)| handle).collect();
        order.sort_by_key(|handle| {
            if *handle == self.category {
                return (0, 0, 0);
            }
            // SAFETY: Owner-thread live child ID query only.
            let id = unsafe { GetDlgCtrlID(*handle) as u32 };

            if matches!(id, SAVE | START | STOP | LOAD) {
                return (
                    2,
                    0,
                    match id {
                        SAVE => 0,
                        START => 1,
                        STOP => 2,
                        _ => 3,
                    },
                );
            }
            let labelled = self
                .numbers
                .iter()
                .find(|number| number.label == *handle)
                .map(|number| number.control)
                .or_else(|| {
                    self.field_rows
                        .iter()
                        .find(|(label, _)| *label == *handle)
                        .map(|(_, control)| *control)
                })
                .or_else(|| {
                    self.base_labels
                        .iter()
                        .position(|label| *label == *handle)
                        .map(|index| {
                            [
                                self.url,
                                self.ip,
                                self.key,
                                self.injection,
                                self.storage,
                                self.secrets,
                            ][index]
                        })
                });
            let anchor = if *handle == self.key_visibility {
                self.key
            } else {
                labelled.unwrap_or(*handle)
            };
            let row = self.controls.iter().find(|row| row.0 == anchor);
            let (x, y) = row.map_or((0, 0), |row| (row.1, row.2));
            (
                1,
                y,
                if *handle == self.key_visibility {
                    x + 551
                } else {
                    x - i32::from(labelled.is_some())
                },
            )
        });
        // SAFETY: Owned sibling controls; reorder only, no activation or geometry.
        unsafe {
            for handle in order {
                SetWindowPos(
                    handle,
                    HWND_BOTTOM,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOREDRAW,
                );
            }
        }
    }
}
