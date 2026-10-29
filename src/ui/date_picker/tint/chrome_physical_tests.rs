// Explicit, screen-only baseline/adapted evidence; never part of headless tests.
use super::*;
use std::{fmt::Write as _, io::Write as _, path::Path};
use windows_sys::Win32::{Graphics::Dwm::DwmFlush, UI::HiDpi::GetDpiForWindow};

const LIGHT: u32 = 150;

struct Capture {
    frame: Frame,
    outer: RECT,
    client: RECT,
    children: Vec<(HWND, RECT)>,
    elapsed: Duration,
}
unsafe fn capture(hwnd: HWND) -> Capture {
    // SAFETY: Capture only this fixture's complete window. Validate the whole
    // bounded outer rectangle against WindowFromPoint, before AND after copying;
    // a foreign/occluding window fails rather than leaking into saved evidence.
    unsafe {
        let (outer, client) = outer_client(hwnd);
        let work = crate::ui::overview::rect_work_area(&outer).unwrap();
        assert!(
            includes(work, outer),
            "physical popup is outside the work area"
        );
        assert_ne!(IsWindowVisible(hwnd), 0);
        let visible = || {
            for y in outer.top..outer.bottom {
                for x in outer.left..outer.right {
                    let hit = WindowFromPoint(POINT { x, y });
                    assert!(
                        hit == hwnd || IsChild(hwnd, hit) != 0,
                        "physical popup is occluded at {x},{y}"
                    );
                }
            }
        };
        visible();
        let frame = Frame::new(outer.right - outer.left, outer.bottom - outer.top).unwrap();
        let screen = GetDC(null_mut());
        assert!(!screen.is_null());
        let start = Instant::now();
        let copied = BitBlt(
            frame.dc,
            0,
            0,
            frame.width,
            frame.height,
            screen,
            outer.left,
            outer.top,
            SRCCOPY,
        );
        ReleaseDC(null_mut(), screen);
        assert_ne!(copied, 0);
        GdiFlush();
        let elapsed = start.elapsed();
        visible();
        Capture {
            frame,
            outer,
            client,
            children: children(hwnd),
            elapsed,
        }
    }
}
fn rect(rect: RECT) -> (i32, i32, i32, i32) {
    (rect.left, rect.top, rect.right, rect.bottom)
}
fn screen_rect(sample: &Capture) -> RECT {
    RECT {
        left: sample.outer.left + sample.client.left,
        top: sample.outer.top + sample.client.top,
        right: sample.outer.left + sample.client.right,
        bottom: sample.outer.top + sample.client.bottom,
    }
}
unsafe fn pixels(sample: &Capture) -> &[u32] {
    // SAFETY: Capture owns this flushed, bounded top-down DIB for the borrow.
    unsafe {
        std::slice::from_raw_parts(
            sample.frame.bits,
            (sample.frame.width * sample.frame.height) as usize,
        )
    }
}
fn color(pixel: u32) -> u32 {
    let [b, g, r, _] = pixel.to_le_bytes();
    u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16)
}

#[derive(Default)]
struct Band {
    total: usize,
    light: usize,
    bounds: Option<RECT>,
    rows: std::collections::BTreeMap<i32, usize>,
    columns: std::collections::BTreeMap<i32, usize>,
    colors: std::collections::BTreeMap<u32, usize>,
}
impl Band {
    fn add(&mut self, x: i32, y: i32, pixel: u32) {
        self.total += 1;
        if luma(color(pixel)) <= LIGHT {
            return;
        }
        self.light += 1;
        self.bounds = Some(self.bounds.map_or(
            RECT {
                left: x,
                top: y,
                right: x + 1,
                bottom: y + 1,
            },
            |r| RECT {
                left: r.left.min(x),
                top: r.top.min(y),
                right: r.right.max(x + 1),
                bottom: r.bottom.max(y + 1),
            },
        ));
        *self.rows.entry(y).or_default() += 1;
        *self.columns.entry(x).or_default() += 1;
        *self.colors.entry(color(pixel)).or_default() += 1;
    }
    fn report(&self, label: &str, output: &mut String) {
        let top = |counts: &std::collections::BTreeMap<i32, usize>| {
            let mut values: Vec<_> = counts.iter().map(|(axis, count)| (*axis, *count)).collect();
            values.sort_by_key(|(axis, count)| (std::cmp::Reverse(*count), *axis));
            values.truncate(8);
            values
        };
        writeln!(
            output,
            "  {label}: light={}/{} bbox={:?} top_rows={:?} top_columns={:?}",
            self.light,
            self.total,
            self.bounds.map(rect),
            top(&self.rows),
            top(&self.columns)
        )
        .unwrap();
        let mut colors: Vec<_> = self
            .colors
            .iter()
            .map(|(color, count)| (*color, *count))
            .collect();
        colors.sort_by_key(|(color, count)| (std::cmp::Reverse(*count), *color));
        colors.truncate(6);
        writeln!(output, "    light_COLORREF={colors:06x?}").unwrap();
    }
}

unsafe fn measure(
    label: &str,
    sample: &Capture,
    calendar: HWND,
    calendar_client: RECT,
    output: &mut String,
) -> usize {
    // SAFETY: Read only saved screen pixels. Public hit queries locate any bright
    // calendar client edge; never infer all light pixels are erroneous borders.
    unsafe {
        writeln!(
            output,
            "{label}: outer_screen={:?} client_window={:?} capture_us={} children={:?}",
            rect(sample.outer),
            rect(sample.client),
            sample.elapsed.as_micros(),
            sample
                .children
                .iter()
                .map(|(_, r)| rect(*r))
                .collect::<Vec<_>>()
        )
        .unwrap();
        let mut nc = Band::default();
        let mut client = Band::default();
        let mut gutters = Band::default();
        let mut child = Band::default();
        let mut edge = Band::default();
        let mut no_hit = Band::default();
        let mut navigation = Band::default();
        let mut edge_hits = std::collections::BTreeMap::<u32, usize>::new();
        let width = scale(4, GetDpiForWindow(calendar));
        for (index, pixel) in pixels(sample).iter().copied().enumerate() {
            let x = index as i32 % sample.frame.width;
            let y = index as i32 / sample.frame.width;
            if !inside(sample.client, x, y) {
                nc.add(x, y, pixel);
                continue;
            }
            client.add(x, y, pixel);
            if sample.children.iter().any(|(_, r)| inside(*r, x, y)) {
                child.add(x, y, pixel);
            } else if sample.children.iter().any(|(hwnd, _)| *hwnd == calendar) {
                gutters.add(x, y, pixel);
            }
            let cx = x + sample.outer.left - calendar_client.left;
            let cy = y + sample.outer.top - calendar_client.top;
            let w = calendar_client.right - calendar_client.left;
            let h = calendar_client.bottom - calendar_client.top;
            if cx >= 0
                && cy >= 0
                && cx < w
                && cy < h
                && (cx < width || cy < width || cx >= w - width || cy >= h - width)
            {
                edge.add(x, y, pixel);
                if luma(color(pixel)) > LIGHT {
                    let mut hit = MCHITTESTINFO {
                        cbSize: size_of::<MCHITTESTINFO>() as u32,
                        pt: POINT { x: cx, y: cy },
                        ..Default::default()
                    };
                    let part = SendMessageW(
                        calendar,
                        MCM_HITTEST,
                        0,
                        (&mut hit as *mut MCHITTESTINFO) as LPARAM,
                    ) as u32;
                    *edge_hits.entry(part).or_default() += 1;
                    if part == MCHT_NOWHERE {
                        no_hit.add(x, y, pixel);
                    }
                    if part == MCHT_TITLEBTNPREV || part == MCHT_TITLEBTNNEXT {
                        navigation.add(x, y, pixel);
                    }
                }
            }
        }
        for (name, band) in [
            ("NC", &nc),
            ("client", &client),
            ("host_gutters", &gutters),
            ("child_outer", &child),
            ("calendar_client_edge_4DIP", &edge),
            ("bright_edge_no_native_hit", &no_hit),
            ("edge_native_navigation", &navigation),
        ] {
            band.report(name, output);
        }
        writeln!(output, "  bright_client_edge_MCM_HITTEST={edge_hits:06x?}; NOWHERE={MCHT_NOWHERE:#x}, PREV={MCHT_TITLEBTNPREV:#x}, NEXT={MCHT_TITLEBTNNEXT:#x}").unwrap();
        nc.light + gutters.light
    }
}
fn save_bmp(path: &Path, sample: &Capture) -> std::io::Result<()> {
    let bytes = (sample.frame.width * sample.frame.height * 4) as u32;
    let mut file = std::io::BufWriter::new(
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?,
    );
    file.write_all(b"BM")?;
    file.write_all(&(54 + bytes).to_le_bytes())?;
    file.write_all(&[0; 4])?;
    file.write_all(&54u32.to_le_bytes())?;
    for value in [
        40u32,
        sample.frame.width as u32,
        (-sample.frame.height) as u32,
    ] {
        file.write_all(&value.to_le_bytes())?;
    }
    file.write_all(&1u16.to_le_bytes())?;
    file.write_all(&32u16.to_le_bytes())?;
    for value in [0u32, bytes, 0, 0, 0, 0] {
        file.write_all(&value.to_le_bytes())?;
    }
    // SAFETY: Capture's flushed DIB outlives this read; serialize BGRA with an
    // opaque alpha byte, without saving any pixels outside the fixture's outer.
    for pixel in unsafe { pixels(sample) } {
        file.write_all(&(*pixel | 0xff00_0000).to_le_bytes())?;
    }
    file.flush()
}
unsafe fn redraw(hwnd: HWND) {
    // SAFETY: Only the fixture's native window. No print or child traversal: the
    // separate host/calendar stages reveal which native repaint owns the band.
    unsafe {
        assert_ne!(
            RedrawWindow(
                hwnd,
                null(),
                null_mut(),
                RDW_INVALIDATE | RDW_ERASE | RDW_FRAME | RDW_UPDATENOW | RDW_NOCHILDREN
            ),
            0
        )
    };
}
unsafe fn pair(calendar: HWND, host: HWND) -> [Capture; 2] {
    // SAFETY: Flush synchronous native drawing, then read both complete native
    // windows from the screen, never WM_PRINT/GetWindowDC. Do not dispatch a
    // pending calendar repaint between the host-only and child-repaint samples.
    unsafe {
        GdiFlush();
        assert_eq!(DwmFlush(), 0);
        [capture(calendar), capture(host)]
    }
}

#[test]
#[ignore = "requires an unobscured interactive desktop; captures native DropDown outer baseline; run alone"]
fn calendar_popup_outer_screen_baseline_and_adapted_border_evidence() {
    let _styles = VisualStyles::new();
    let host = Host::new();
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    assert!(manifest.is_absolute());
    let manifest = manifest.canonicalize().unwrap();
    assert!(manifest.join("Cargo.toml").is_file());
    // /target/ is repository-ignored. Unique run paths never overwrite evidence.
    let target = manifest.join("target");
    std::fs::create_dir_all(&target).unwrap();
    let output_dir = target.join(format!(
        "calendar-outer-evidence-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&output_dir).unwrap();
    let mut report = String::new();
    writeln!(report, "Screen-only native popup evidence; COLORREF luma > {LIGHT}; no light app theme, no forced styles.").unwrap();
    // SAFETY: Private controls/date, no user data/network/clipboard/global theme.
    // Only popup presentation is detached for baseline; classic six colors stay.
    unsafe {
        SetWindowPos(host.0, HWND_TOPMOST, 80, 80, 700, 500, SWP_SHOWWINDOW);
        SetForegroundWindow(host.0);
        let picker = date_picker::create(host.0, 2224, "开始（含）").unwrap();
        MoveWindow(picker, 24, 30, 336, 28, 0);
        SendMessageW(picker, WM_SETTEXT, 0, wide("2026-10-15").as_ptr() as LPARAM);
        date_picker::apply(picker, Resolved::Dark, palette(), 96);
        let calendar = popup(picker);
        let popup_host = date_picker::popup::native_host(calendar).unwrap();
        // Drain the picker's deferred refresh BEFORE disabling local chrome.
        pump_for(Duration::from_millis(200));
        let snapshot = |hwnd| {
            (
                GetWindowLongPtrW(hwnd, GWL_STYLE),
                GetWindowLongPtrW(hwnd, GWL_EXSTYLE),
                rect(outer_client(hwnd).0),
                rect(outer_client(hwnd).1),
            )
        };
        let geometry = [calendar, popup_host].map(snapshot);
        let parts = [
            MCSC_BACKGROUND,
            MCSC_TEXT,
            MCSC_TITLEBK,
            MCSC_TITLETEXT,
            MCSC_MONTHBK,
            MCSC_TRAILINGTEXT,
        ];
        let colors = parts.map(|part| SendMessageW(calendar, MCM_GETCOLOR, part as WPARAM, 0));
        let selected = value(picker);

        assert_eq!(colors[4] as u32, palette().surface);
        assert!(!active(calendar), "calendar must not be whole-frame tinted");
        writeln!(
            report,
            "dpi={} calendar(style,ex,outer,client)={:?} DropDown={:?} colors={colors:06x?}",
            GetDpiForWindow(calendar),
            geometry[0],
            geometry[1]
        )
        .unwrap();
        writeln!(
            report,
            "system_COLORREF={:?}",
            [
                COLOR_BTNFACE,
                COLOR_BTNHIGHLIGHT,
                COLOR_BTNSHADOW,
                COLOR_WINDOW
            ]
            .map(|part| (part, GetSysColor(part)))
        )
        .unwrap();
        writeln!(report, "calendar_border={} view={} theme={:?}; repaint flags=INVALIDATE|ERASE|FRAME|UPDATENOW|NOCHILDREN; stage capture never dispatches pending paints",
            SendMessageW(calendar, MCM_GETCALENDARBORDER, 0, 0),
            SendMessageW(calendar, MCM_GETCURRENTVIEW, 0, 0), GetWindowTheme(calendar)).unwrap();
        for (name, part) in [
            ("previous", MCGIP_PREV),
            ("next", MCGIP_NEXT),
            ("body", MCGIP_CALENDARBODY),
            ("footer", MCGIP_FOOTER),
        ] {
            let mut info = MCGRIDINFO {
                cbSize: size_of::<MCGRIDINFO>() as u32,
                dwPart: part,
                dwFlags: MCGIF_RECT,
                ..Default::default()
            };
            let ok = SendMessageW(
                calendar,
                MCM_GETCALENDARGRIDINFO,
                0,
                (&mut info as *mut MCGRIDINFO) as LPARAM,
            );
            writeln!(
                report,
                "  native {name} client_rect={:?}, query={ok}",
                rect(info.rc)
            )
            .unwrap();
        }
        date_picker::popup::apply(calendar, Resolved::Light, palette());
        assert!(!date_picker::popup::active(calendar));
        assert!(!date_picker::popup::active(popup_host));

        assert_eq!(
            GetWindowTheme(calendar),
            0,
            "baseline must remain classic dark six-color"
        );
        // Detach invalidates both HWNDs. Validate the pending calendar damage
        // first, then observe native host painting without a later child paint.
        redraw(calendar);
        redraw(popup_host);
        let host_only = pair(calendar, popup_host);
        redraw(calendar);
        let native = pair(calendar, popup_host);
        assert_eq!([calendar, popup_host].map(snapshot), geometry);
        assert_eq!(
            parts.map(|part| SendMessageW(calendar, MCM_GETCOLOR, part as WPARAM, 0)),
            colors
        );
        date_picker::popup::apply(calendar, Resolved::Dark, palette());
        assert!(date_picker::popup::active(calendar));
        assert!(date_picker::popup::active(popup_host));
        redraw(popup_host);
        redraw(calendar);
        let adapted = pair(calendar, popup_host);
        assert_eq!([calendar, popup_host].map(snapshot), geometry);
        assert_eq!(
            parts.map(|part| SendMessageW(calendar, MCM_GETCOLOR, part as WPARAM, 0)),
            colors
        );
        assert_eq!(value(picker), selected);
        assert_popup_layout(calendar);
        let mut brightness = [0usize; 3];
        for (stage, (name, samples)) in [
            ("native-host-repaint", &host_only),
            ("native-host-calendar-repaint", &native),
            ("adapted", &adapted),
        ]
        .into_iter()
        .enumerate()
        {
            let calendar_client = screen_rect(&samples[0]);
            for (index, sample) in samples.iter().enumerate() {
                let label = format!("{name}-{}", ["calendar", "DropDown"][index]);
                brightness[stage] +=
                    measure(&label, sample, calendar, calendar_client, &mut report);
                save_bmp(&output_dir.join(format!("{label}.bmp")), sample).unwrap();
            }
        }
        writeln!(report, "Client-edge brightness may be native navigation/text; compare hit parts and geometry. The host-repaint vs child-repaint delta identifies an observed repaint stage, not a specific internal COMCTL/GDI call.").unwrap();
        if brightness[0] == 0 && brightness[1] == 0 {
            writeln!(report, "NOT REPRODUCED: neither native stage had bright NC/host gutters; this fixture does not prove the user's border reproduced or was fixed. Inspect client-edge geometry/hit diagnostics, not date/nav ink alone.").unwrap();
        } else if brightness[1] == 0 {
            writeln!(report, "HOST-REPAINT STAGE ONLY: bright NC/gutters={} disappeared after calendar repaint; not a reproduced persistent border. Inspect host-only geometry/BMP before assigning the user's screenshot root cause.", brightness[0]).unwrap();
        } else {
            writeln!(report, "Native bright NC/gutter pixels={} -> adapted={}; host-only intermediate={}. This local geometry is measured, but does not by itself prove the user's screenshot has the same cause. Client-edge brightness alone is not a border regression.", brightness[1], brightness[2], brightness[0]).unwrap();
        }
        println!("{report}Evidence directory: {}", output_dir.display());
        std::fs::write(output_dir.join("report.txt"), &report).unwrap();
        // Save evidence before assertions so a real unresolved border is visible.
        assert_frame_pixels(calendar, &adapted[0].frame);
        assert_frame_pixels(popup_host, &adapted[1].frame);
        let client = native[0].client;
        for y in client.top..client.bottom {
            for x in client.left..client.right {
                let index = (y * native[0].frame.width + x) as usize;
                assert_eq!(
                    pixels(&native[0])[index] & 0x00ff_ffff,
                    pixels(&adapted[0])[index] & 0x00ff_ffff,
                    "border adaptation changed native calendar client/navigation at {x},{y}"
                );
            }
        }
        for sample in [&native[0], &adapted[0]] {
            // Analyze a copy of captured client pixels in native hit-test
            // coordinates; a real NC inset must not shift the seven columns.
            let r = sample.client;
            let raster = Frame::new(r.right - r.left, r.bottom - r.top).unwrap();
            assert_ne!(
                BitBlt(
                    raster.dc,
                    0,
                    0,
                    raster.width,
                    raster.height,
                    sample.frame.dc,
                    r.left,
                    r.top,
                    SRCCOPY
                ),
                0
            );
            GdiFlush();
            assert_grid_and_ink(calendar, &raster);
        }
        SendMessageW(picker, DTM_CLOSEMONTHCAL, 0, 0);
    }
}
