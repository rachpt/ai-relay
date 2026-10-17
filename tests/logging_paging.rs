use ai_relay::logging::*;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    dir: PathBuf,
    service: LogService,
}
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ai-relay-pages-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let service = LogService::start(
            &dir,
            LogOptions {
                file_enabled: false,
                retention_days: None,
                ..Default::default()
            },
        )
        .unwrap();
        Self { dir, service }
    }
    fn write(&self, bytes: impl AsRef<[u8]>) {
        fs::write(self.dir.join(CURRENT_FILE), bytes).unwrap();
    }
    fn tail(&self, bytes: usize, lines: usize) -> LogPage {
        page(
            self.service
                .read_page(
                    CURRENT_FILE,
                    PageCursor::Tail,
                    ReadBounds {
                        max_bytes: bytes,
                        max_lines: lines,
                    },
                )
                .unwrap(),
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.service.shutdown(Duration::from_secs(2)).unwrap();
        let _ = fs::remove_dir_all(&self.dir);
    }
}
fn page(response: ReadResponse) -> LogPage {
    match response {
        ReadResponse::Page(page) => page,
        other => panic!("{other:?}"),
    }
}
#[test]
fn backwards_pages_preserve_every_complete_line_at_byte_and_line_boundaries() {
    let f = Fixture::new();
    let original = (0..200)
        .map(|i| format!("{i:03}:普通行\r\n"))
        .collect::<String>();
    f.write(&original);
    for bytes in [19, 32, 48, 97, MAX_TAIL_BYTES] {
        for lines in [1, 2, 7, MAX_TAIL_LINES] {
            let bounds = ReadBounds {
                max_bytes: bytes,
                max_lines: lines,
            };
            let mut current = f.tail(bytes, lines);
            let mut texts = Vec::new();
            loop {
                assert!(
                    !current.truncated,
                    "bytes={bytes}, lines={lines}: {current:?}"
                );
                assert!(current.text.len() <= bytes);
                assert!(current.text.lines().count() <= lines);
                texts.push(current.text);
                let Some(before) = current.before else { break };
                assert!(texts.len() <= 200);
                current = page(
                    f.service
                        .read_page(CURRENT_FILE, PageCursor::Before(before), bounds)
                        .unwrap(),
                );
            }
            texts.reverse();
            assert_eq!(texts.concat(), original, "bytes={bytes}, lines={lines}");
        }
    }
}
#[test]
fn tail_follow_drains_backlog_and_shares_with_append_writer() {
    let f = Fixture::new();
    f.write(b"old\n");
    let initial = f.tail(32, 2);
    let mut cursor = initial.follow;
    let mut writer = OpenOptions::new()
        .append(true)
        .open(f.dir.join(CURRENT_FILE))
        .unwrap();
    writer.write_all(b"one\ntwo\nthree\nfour\n").unwrap();
    let mut text = String::new();
    loop {
        let next = page(
            f.service
                .read_follow(
                    CURRENT_FILE,
                    cursor,
                    ReadBounds {
                        max_bytes: 12,
                        max_lines: 2,
                    },
                )
                .unwrap(),
        );
        assert!(!next.truncated);
        text.push_str(&next.text);
        cursor = next.follow;
        if !next.has_more {
            break;
        }
    }
    assert_eq!(text, "one\ntwo\nthree\nfour\n");
    let empty = page(
        f.service
            .read_follow(CURRENT_FILE, cursor, ReadBounds::default())
            .unwrap(),
    );
    assert_eq!(empty.text, "");
    writer.write_all(b"five\n").unwrap();
    assert_eq!(
        page(
            f.service
                .read_follow(CURRENT_FILE, empty.follow, ReadBounds::default())
                .unwrap()
        )
        .text,
        "five\n"
    );
}
#[test]
fn detects_real_rotation_same_length_replacement_deletion_and_truncation() {
    let f = Fixture::new();
    f.write(b"aaa\nbbb\n");
    let old = f.tail(1024, 1);
    let path = f.dir.join(CURRENT_FILE);
    fs::rename(&path, f.dir.join("ai-relay.1.log")).unwrap();
    assert!(matches!(
        f.service
            .read_follow(CURRENT_FILE, old.follow.clone(), ReadBounds::default())
            .unwrap(),
        ReadResponse::Missing
    ));
    f.write(b"ccc\nddd\n");
    // Equal size and deliberately equal timestamps must not mask a replacement.
    let old_meta = fs::metadata(f.dir.join("ai-relay.1.log")).unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(old_meta.modified().unwrap()))
        .unwrap();
    assert!(matches!(
        f.service
            .read_follow(CURRENT_FILE, old.follow, ReadBounds::default())
            .unwrap(),
        ReadResponse::Changed(FileChange::Replaced)
    ));
    assert!(matches!(
        f.service
            .read_page(
                CURRENT_FILE,
                PageCursor::Before(old.before.unwrap()),
                ReadBounds::default()
            )
            .unwrap(),
        ReadResponse::Changed(FileChange::Replaced)
    ));
    let current = f.tail(1024, 1);
    OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(0)
        .unwrap();
    assert!(matches!(
        f.service
            .read_follow(CURRENT_FILE, current.follow.clone(), ReadBounds::default())
            .unwrap(),
        ReadResponse::Changed(FileChange::Truncated)
    ));
    assert!(matches!(
        f.service
            .read_page(
                CURRENT_FILE,
                PageCursor::Before(current.before.unwrap()),
                ReadBounds::default()
            )
            .unwrap(),
        ReadResponse::Changed(FileChange::Truncated)
    ));
    fs::remove_file(&path).unwrap();
    f.write(b"new\n");
    assert!(matches!(
        f.service
            .read_follow(CURRENT_FILE, current.follow, ReadBounds::default())
            .unwrap(),
        ReadResponse::Changed(FileChange::Replaced)
    ));
}
#[test]
fn bounds_long_lines_and_lossy_utf8_are_explicit() {
    let f = Fixture::new();
    f.write(vec![0xff; MAX_TAIL_BYTES * 3]);
    let tail = f.tail(MAX_TAIL_BYTES, MAX_TAIL_LINES);
    assert!(tail.truncated && tail.utf8_loss && tail.has_more);
    assert!(tail.text.len() <= MAX_TAIL_BYTES);
    let previous = page(
        f.service
            .read_page(
                CURRENT_FILE,
                PageCursor::Before(tail.before.unwrap()),
                ReadBounds::default(),
            )
            .unwrap(),
    );
    assert!(previous.truncated && previous.utf8_loss);
    f.write(b"");
    let empty = f.tail(64, 1);
    assert!(empty.text.is_empty() && empty.before.is_none() && !empty.truncated);
    let mut writer = OpenOptions::new()
        .append(true)
        .open(f.dir.join(CURRENT_FILE))
        .unwrap();
    writer.write_all(&vec![b'x'; MAX_TAIL_BYTES * 2]).unwrap();
    let next = page(
        f.service
            .read_follow(CURRENT_FILE, empty.follow, ReadBounds::default())
            .unwrap(),
    );
    assert!(next.truncated && next.has_more && !next.utf8_loss);
    assert_eq!(next.text.len(), MAX_TAIL_BYTES);
}
#[test]
fn follow_marks_partial_line_continuations_even_when_the_newline_arrives_later() {
    let f = Fixture::new();
    f.write(b"unfinished");
    let initial = f.tail(64, 2);
    assert!(initial.truncated);
    let mut writer = OpenOptions::new()
        .append(true)
        .open(f.dir.join(CURRENT_FILE))
        .unwrap();
    writer.write_all(b" suffix\ncomplete\n").unwrap();
    let next = page(
        f.service
            .read_follow(CURRENT_FILE, initial.follow, ReadBounds::default())
            .unwrap(),
    );
    assert_eq!(next.text, " suffix\ncomplete\n");
    assert!(next.truncated);
    writer.write_all(b"another\n").unwrap();
    let next = page(
        f.service
            .read_follow(CURRENT_FILE, next.follow, ReadBounds::default())
            .unwrap(),
    );
    assert_eq!(next.text, "another\n");
    assert!(!next.truncated);
}

#[test]
fn rejects_paths_wrong_file_cursors_bounds_and_hardlinks() {
    let f = Fixture::new();
    f.write(b"data\n");
    let cursor = f.tail(64, 1).follow;
    for name in [
        "../ai-relay.current.log",
        "ai-relay.01.log",
        "ai-relay.100.log",
        "ai-relay.1.log:stream",
        "other.log",
        "C:\\ai-relay.current.log",
    ] {
        assert!(matches!(
            f.service
                .read_page(name, PageCursor::Tail, ReadBounds::default()),
            Err(LogError::InvalidName)
        ));
        assert!(matches!(
            f.service
                .read_follow(name, cursor.clone(), ReadBounds::default()),
            Err(LogError::InvalidName)
        ));
    }
    assert!(matches!(
        f.service
            .read_follow("ai-relay.1.log", cursor.clone(), ReadBounds::default()),
        Err(LogError::InvalidCursor)
    ));
    for bounds in [
        ReadBounds {
            max_bytes: 0,
            max_lines: 1,
        },
        ReadBounds {
            max_bytes: MAX_TAIL_BYTES + 1,
            max_lines: 1,
        },
        ReadBounds {
            max_bytes: 1,
            max_lines: 0,
        },
        ReadBounds {
            max_bytes: 1,
            max_lines: MAX_TAIL_LINES + 1,
        },
    ] {
        assert!(matches!(
            f.service.read_page(CURRENT_FILE, PageCursor::Tail, bounds),
            Err(LogError::InvalidTailBounds)
        ));
        assert!(matches!(
            f.service.read_follow(CURRENT_FILE, cursor.clone(), bounds),
            Err(LogError::InvalidTailBounds)
        ));
    }
    fs::hard_link(f.dir.join(CURRENT_FILE), f.dir.join("ai-relay.1.log")).unwrap();
    assert!(matches!(
        f.service
            .read_page(CURRENT_FILE, PageCursor::Tail, ReadBounds::default()),
        Err(LogError::UnsafePath)
    ));
}
