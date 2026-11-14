use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

const PATIENCE: Duration = Duration::from_secs(30);

pub const ENTER: &str = "\r";
pub const ESC: &str = "\x1b";
pub const SPACE: &str = " ";
pub const UP: &str = "\x1b[A";
pub const DOWN: &str = "\x1b[B";
pub const RIGHT: &str = "\x1b[C";
pub const LEFT: &str = "\x1b[D";
pub const HOME: &str = "\x1b[H";
pub const END: &str = "\x1b[F";
pub const PAGE_UP: &str = "\x1b[5~";
pub const PAGE_DOWN: &str = "\x1b[6~";

const SYNC_BEGIN: &[u8] = b"\x1b[?2026h";
const SYNC_END: &[u8] = b"\x1b[?2026l";

struct Terminal {
    parser: vt100::Parser,
    shown: Vec<String>,
    pending: Vec<u8>,
    open: bool,
    closed: bool,
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn held_back(bytes: &[u8]) -> usize {
    (1..SYNC_BEGIN.len())
        .rev()
        .find(|length| {
            bytes.len() >= *length
                && (SYNC_BEGIN.starts_with(&bytes[bytes.len() - length..])
                    || SYNC_END.starts_with(&bytes[bytes.len() - length..]))
        })
        .unwrap_or(0)
}

impl Terminal {
    fn rows(&self) -> Vec<String> {
        let screen = self.parser.screen();
        let (_, columns) = screen.size();
        screen
            .rows(0, columns)
            .map(|row| row.trim_end().to_string())
            .collect()
    }

    fn publish(&mut self) {
        self.shown = self.rows();
    }

    fn feed(&mut self, bytes: &[u8]) {
        self.pending.extend_from_slice(bytes);
        loop {
            let begin = find(&self.pending, SYNC_BEGIN);
            let end = find(&self.pending, SYNC_END);
            let (at, length, opens) = match (begin, end) {
                (Some(b), Some(e)) if b < e => (b, SYNC_BEGIN.len(), true),
                (Some(b), None) => (b, SYNC_BEGIN.len(), true),
                (_, Some(e)) => (e, SYNC_END.len(), false),
                (None, None) => break,
            };
            let chunk: Vec<u8> = self.pending.drain(..at + length).collect();
            self.parser.process(&chunk);
            self.open = opens;
            if !opens {
                self.publish();
            }
        }
        let keep = held_back(&self.pending);
        let ready = self.pending.len() - keep;
        let chunk: Vec<u8> = self.pending.drain(..ready).collect();
        self.parser.process(&chunk);
        if !self.open {
            self.publish();
        }
    }
}

struct Shared {
    terminal: Mutex<Terminal>,
    changed: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Terminal> {
        self.terminal.lock().unwrap_or_else(|err| err.into_inner())
    }
}

#[derive(Clone)]
pub struct Screen {
    rows: Vec<String>,
}

impl Screen {
    pub fn text(&self) -> String {
        self.rows.join("\n")
    }

    pub fn rows(&self) -> &[String] {
        &self.rows
    }

    pub fn contains(&self, needle: &str) -> bool {
        self.rows.iter().any(|row| row.contains(needle))
    }

    pub fn first(&self) -> &str {
        self.rows.first().map(String::as_str).unwrap_or("")
    }

    pub fn position(&self, needle: &str) -> Option<(u16, u16)> {
        self.rows.iter().enumerate().find_map(|(y, row)| {
            row.find(needle).map(|byte| {
                let column = row[..byte].chars().count();
                (column as u16, y as u16)
            })
        })
    }
}

pub struct Session {
    master: File,
    child: Child,
    shared: Arc<Shared>,
}

pub struct Launch<'a> {
    pub program: &'a str,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub size: (u16, u16),
}

fn set_size(fd: i32, size: (u16, u16)) {
    let winsize = libc::winsize {
        ws_row: size.1,
        ws_col: size.0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    unsafe {
        libc::ioctl(fd, libc::TIOCSWINSZ, &winsize);
    }
}

impl Session {
    pub fn start(launch: Launch<'_>) -> Session {
        let mut master = -1;
        let mut slave = -1;
        let opened = unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(opened, 0, "openpty failed");
        set_size(master, launch.size);
        let master = unsafe { File::from_raw_fd(master) };
        let slave = unsafe { File::from_raw_fd(slave) };
        let mut command = Command::new(launch.program);
        command.args(&launch.args).env_clear();
        for (key, value) in &launch.env {
            command.env(key, value);
        }
        command
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave.try_clone().unwrap()));
        let slave_fd = slave.as_raw_fd();
        unsafe {
            command.pre_exec(move || {
                libc::setsid();
                libc::ioctl(slave_fd, libc::TIOCSCTTY as _, 0);
                Ok(())
            });
        }
        let child = command.spawn().expect("the drift binary starts");
        drop(slave);
        let (columns, rows) = launch.size;
        let shared = Arc::new(Shared {
            terminal: Mutex::new(Terminal {
                parser: vt100::Parser::new(rows, columns, 0),
                shown: vec![String::new(); usize::from(rows)],
                pending: Vec::new(),
                open: false,
                closed: false,
            }),
            changed: Condvar::new(),
        });
        let feed = Arc::clone(&shared);
        let mut reader = master.try_clone().unwrap();
        std::thread::spawn(move || {
            let mut buffer = [0_u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(size) if size > 0 => {
                        let mut terminal = feed.lock();
                        terminal.feed(&buffer[..size]);
                        feed.changed.notify_all();
                    }
                    _ => break,
                }
            }
            let mut terminal = feed.lock();
            terminal.open = false;
            terminal.feed(b"");
            terminal.closed = true;
            drop(terminal);
            feed.changed.notify_all();
        });
        Session {
            master,
            child,
            shared,
        }
    }

    pub fn screen(&self) -> Screen {
        Screen {
            rows: self.shared.lock().shown.clone(),
        }
    }

    pub fn wait_for(&self, what: &str, ready: impl Fn(&Screen) -> bool) -> Screen {
        let guard = self.shared.lock();
        let (guard, outcome) = self
            .shared
            .changed
            .wait_timeout_while(guard, PATIENCE, |terminal| {
                !ready(&Screen {
                    rows: terminal.shown.clone(),
                })
            })
            .unwrap_or_else(|err| err.into_inner());
        let screen = Screen {
            rows: guard.shown.clone(),
        };
        assert!(
            !outcome.timed_out(),
            "gave up waiting for {what}; the screen is:\n{}",
            screen.text()
        );
        screen
    }

    pub fn wait_for_text(&self, needle: &str) -> Screen {
        self.wait_for(&format!("{needle:?} on the screen"), |screen| {
            screen.contains(needle)
        })
    }

    pub fn send(&mut self, bytes: &str) {
        self.master.write_all(bytes.as_bytes()).unwrap();
        self.master.flush().unwrap();
    }

    pub fn press(&mut self, key: &str) {
        self.send(key);
    }

    pub fn click(&mut self, column: u16, row: u16) {
        self.send(&format!(
            "\x1b[<0;{};{}M\x1b[<0;{};{}m",
            column + 1,
            row + 1,
            column + 1,
            row + 1
        ));
    }

    pub fn click_text(&mut self, needle: &str) {
        let screen = self.screen();
        let (column, row) = screen
            .position(needle)
            .unwrap_or_else(|| panic!("{needle:?} is not on the screen:\n{}", screen.text()));
        self.click(column + 1, row);
    }

    pub fn wait_exit(&mut self) -> i32 {
        let guard = self.shared.lock();
        let (_guard, outcome) = self
            .shared
            .changed
            .wait_timeout_while(guard, PATIENCE, |terminal| !terminal.closed)
            .unwrap_or_else(|err| err.into_inner());
        assert!(
            !outcome.timed_out(),
            "the program did not exit; the screen is:\n{}",
            self.screen().text()
        );
        let status = self.child.wait().unwrap();
        status.code().unwrap_or(-1)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        unsafe { libc::kill(self.child.id() as i32, libc::SIGKILL) };
        let _ = self.child.wait();
    }
}
