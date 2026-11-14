use std::env;
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--diff-host") {
        diff_host();
        return;
    }
    if args.first().map(String::as_str) == Some("--screen-host") {
        screen_host();
        return;
    }
    if let Some(path) = args.first() {
        if path.contains("drift-edit-") {
            edit_probe(path);
            return;
        }
    }
    parent("--diff-host", false);
    parent("--screen-host", true);
}

fn diff_host() {
    unsafe {
        libc::umask(0o022);
    }
    let screen = drift_tui::DiffScreen::new("same\n", "left\n", "right\n");
    lunatui::run(screen, lunatui::RunOptions::new(160, 40).pixels(false)).unwrap();
}

struct NoResolver;

impl drift_tui::Resolver for NoResolver {
    fn propose(
        &mut self,
        _: drift_core::Action,
        _: &[String],
    ) -> Result<drift_tui::Proposal, String> {
        Err("not used".to_string())
    }

    fn propose_result(&mut self, _: &drift_tui::EditedFile) -> Result<drift_tui::Proposal, String> {
        Err("Writing is unavailable in this example.".to_string())
    }

    fn apply(&mut self, _: &drift_tui::Proposal) -> Result<String, String> {
        Err("not used".to_string())
    }
}

struct NoHistory;

impl drift_tui::Historian for NoHistory {
    fn rows(&mut self) -> Result<drift_core::HistoryView, String> {
        Err("not used".to_string())
    }

    fn propose(&mut self, _: &str, _: bool) -> Result<drift_tui::UndoProposal, String> {
        Err("not used".to_string())
    }

    fn apply(&mut self, _: &drift_tui::UndoProposal) -> Result<String, String> {
        Err("not used".to_string())
    }
}

struct SameDiff;

impl drift_tui::Differ for SameDiff {
    fn diff(&mut self, path: &str) -> Result<drift_core::FileDiff, String> {
        let text = |text: &str| drift_core::DiffContent::Text {
            sha256: "ab".repeat(32),
            text: text.to_string(),
        };
        Ok(drift_core::FileDiff {
            path: path.to_string(),
            status: drift_core::Status::Conflict,
            badge: None,
            provenance: drift_core::BaseProvenance::Reviewed,
            note: String::new(),
            base: text("same\n"),
            ours: text("left\n"),
            theirs: text("right\n"),
        })
    }
}

fn screen_host() {
    unsafe {
        libc::umask(0o022);
    }
    let worker = drift_tui::spawn_scan(|| {
        Ok(drift_core::ScanReport {
            package_version: "4.5.0-3".to_string(),
            reviewed_version: None,
            entries: vec![drift_core::ScanEntry {
                path: "a.conf".to_string(),
                template: Some("config/a.conf".to_string()),
                status: drift_core::Status::Conflict,
                badge: None,
                base: drift_core::BaseProvenance::Reviewed,
                volatile: false,
                note: String::new(),
            }],
            backups: Vec::new(),
            ignored: 0,
        })
    });
    drift_tui::launch_worker_with(
        worker,
        Box::new(NoResolver),
        Box::new(NoHistory),
        Box::new(SameDiff),
    )
    .unwrap();
}

fn edit_probe(path: &str) {
    let mut out = std::io::stdout();
    let _ = writeln!(out, "probe ready");
    let _ = out.flush();
    let byte = read_byte(Duration::from_secs(3)).unwrap_or(b'?');
    consume_line();
    write_back(path, &format!("kept-{}\n", byte as char));
    let mode = std::fs::metadata(path)
        .map(|meta| meta.permissions().mode() & 0o777)
        .unwrap_or(0);
    let (icanon, echo) = local_flags();
    let _ = writeln!(
        out,
        "probe byte={} mode={mode:o} icanon={icanon} echo={echo}",
        byte as char
    );
    let _ = out.flush();
}

fn write_back(path: &str, text: &str) {
    let Ok(path) = std::ffi::CString::new(path) else {
        return;
    };
    let fd = unsafe { libc::open(path.as_ptr(), libc::O_WRONLY | libc::O_TRUNC) };
    if fd < 0 {
        return;
    }
    let bytes = text.as_bytes();
    unsafe {
        libc::write(fd, bytes.as_ptr().cast(), bytes.len());
        libc::close(fd);
    }
}

fn read_byte(timeout: Duration) -> Option<u8> {
    let mut fd = libc::pollfd {
        fd: libc::STDIN_FILENO,
        events: libc::POLLIN,
        revents: 0,
    };
    let waited = unsafe { libc::poll(&mut fd, 1, timeout.as_millis() as libc::c_int) };
    if waited != 1 {
        return None;
    }
    let mut buf = [0_u8; 1];
    let n = unsafe { libc::read(libc::STDIN_FILENO, buf.as_mut_ptr().cast(), 1) };
    (n == 1).then_some(buf[0])
}

fn consume_line() {
    let flags = unsafe { libc::fcntl(libc::STDIN_FILENO, libc::F_GETFL) };
    if flags < 0 {
        return;
    }
    unsafe {
        libc::fcntl(libc::STDIN_FILENO, libc::F_SETFL, flags | libc::O_NONBLOCK);
    }
    let mut buf = [0_u8; 64];
    loop {
        let n = unsafe { libc::read(libc::STDIN_FILENO, buf.as_mut_ptr().cast(), buf.len()) };
        if n <= 0 {
            break;
        }
    }
    unsafe {
        libc::fcntl(libc::STDIN_FILENO, libc::F_SETFL, flags);
    }
}

fn local_flags() -> (u8, u8) {
    let mut term: libc::termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(libc::STDIN_FILENO, &mut term) } != 0 {
        return (0, 0);
    }
    let icanon = u8::from(term.c_lflag & libc::ICANON != 0);
    let echo = u8::from(term.c_lflag & libc::ECHO != 0);
    (icanon, echo)
}

fn parent(host: &str, through_tree: bool) {
    let (master, slave) = open_pty();
    let slave_fd = slave.as_raw_fd();
    let stdin_fd = unsafe { libc::dup(slave_fd) };
    let stdout_fd = unsafe { libc::dup(slave_fd) };
    assert!(stdin_fd >= 0 && stdout_fd >= 0);
    drop(slave);
    let mut master_write = master.try_clone().unwrap();
    let exe = env::current_exe().unwrap();
    let mut command = Command::new(&exe);
    command
        .arg(host)
        .env("EDITOR", &exe)
        .stdin(unsafe { Stdio::from_raw_fd(stdin_fd) })
        .stdout(unsafe { Stdio::from_raw_fd(stdout_fd) })
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    let pid = child.id() as i32;
    let stderr = child.stderr.take();
    let captured = std::sync::Arc::new(Mutex::new(String::new()));
    let stored = std::sync::Arc::clone(&captured);
    let (text_tx, text_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut master = master;
        let mut buffer = [0_u8; 4096];
        loop {
            match master.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(size) => {
                    let mut text = stored.lock().unwrap_or_else(|err| err.into_inner());
                    text.push_str(&String::from_utf8_lossy(&buffer[..size]));
                    drop(text);
                    if text_tx.send(()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    let saw_conflict = if through_tree {
        let saw_tree = wait_text(&captured, &text_rx, "a.conf", Duration::from_secs(5));
        let _ = master_write.write_all(b"\r");
        let _ = master_write.flush();
        saw_tree && wait_text(&captured, &text_rx, "Diff  a.conf", Duration::from_secs(5))
    } else {
        wait_text(&captured, &text_rx, "conflict", Duration::from_secs(5))
    };
    let _ = master_write.write_all(b"e");
    let _ = master_write.flush();
    let saw_ready = wait_text(&captured, &text_rx, "probe ready", Duration::from_secs(5));
    let _ = master_write.write_all(b"Z\n");
    let _ = master_write.flush();
    let saw_byte = wait_text(
        &captured,
        &text_rx,
        "probe byte=Z mode=600 icanon=1 echo=1",
        Duration::from_secs(5),
    );
    let saw_redraw = wait_after(
        &captured,
        &text_rx,
        "probe byte=Z",
        "kept-Z",
        Duration::from_secs(5),
    ) && wait_after(
        &captured,
        &text_rx,
        "probe byte=Z",
        if through_tree { "Diff  a.conf" } else { "Diff" },
        Duration::from_secs(5),
    ) && wait_after(
        &captured,
        &text_rx,
        "probe byte=Z",
        "Quit",
        Duration::from_secs(5),
    );
    let _ = master_write.write_all(b"e");
    let _ = master_write.flush();
    let saw_second_ready = wait_after(
        &captured,
        &text_rx,
        "probe byte=Z",
        "probe ready",
        Duration::from_secs(5),
    );
    let _ = master_write.write_all(b"Y\n");
    let _ = master_write.flush();
    let saw_second = wait_after(
        &captured,
        &text_rx,
        "probe byte=Z",
        "probe byte=Y mode=600 icanon=1 echo=1",
        Duration::from_secs(5),
    );
    let saw_second_redraw = wait_after(
        &captured,
        &text_rx,
        "probe byte=Y",
        "kept-Y",
        Duration::from_secs(5),
    );
    let _ = master_write.write_all(b"q");
    let _ = master_write.flush();
    if through_tree {
        let _ = wait_after(
            &captured,
            &text_rx,
            "probe byte=Y",
            "omarchy-settings",
            Duration::from_secs(5),
        );
        let _ = master_write.write_all(b"q");
        let _ = master_write.flush();
    }
    let (done_tx, done_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let status = child.wait();
        let _ = done_tx.send(status);
    });
    let finished = done_rx.recv_timeout(Duration::from_secs(5));
    if finished.is_err() {
        unsafe { libc::kill(pid, libc::SIGKILL) };
    }
    let mut err = String::new();
    if let Some(mut stderr) = stderr {
        let _ = stderr.read_to_string(&mut err);
    }
    let tail = captured.lock().unwrap_or_else(|error| error.into_inner());
    let tail = tail.chars().rev().take(500).collect::<String>();
    let tail = tail.chars().rev().collect::<String>();
    let exit_ok = matches!(finished, Ok(Ok(status)) if status.success());
    assert!(
        saw_conflict
            && saw_ready
            && saw_byte
            && saw_redraw
            && saw_second_ready
            && saw_second
            && saw_second_redraw
            && exit_ok,
        "host {host} conflict {saw_conflict} ready {saw_ready} byte {saw_byte} redraw {saw_redraw} second_ready {saw_second_ready} second {saw_second} second_redraw {saw_second_redraw} exit {exit_ok} finished {finished:?} err {err} tail {tail}"
    );
}

fn wait_text(
    captured: &Mutex<String>,
    rx: &mpsc::Receiver<()>,
    needle: &str,
    timeout: Duration,
) -> bool {
    contains_after(captured, rx, "", needle, timeout)
}

fn wait_after(
    captured: &Mutex<String>,
    rx: &mpsc::Receiver<()>,
    marker: &str,
    needle: &str,
    timeout: Duration,
) -> bool {
    contains_after(captured, rx, marker, needle, timeout)
}

fn contains_after(
    captured: &Mutex<String>,
    rx: &mpsc::Receiver<()>,
    marker: &str,
    needle: &str,
    timeout: Duration,
) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if after_marker(
            &captured.lock().unwrap_or_else(|err| err.into_inner()),
            marker,
            needle,
        ) {
            return true;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return false;
        }
        match rx.recv_timeout(remaining) {
            Ok(()) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => return false,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return after_marker(
                    &captured.lock().unwrap_or_else(|err| err.into_inner()),
                    marker,
                    needle,
                );
            }
        }
    }
}

fn after_marker(text: &str, marker: &str, needle: &str) -> bool {
    if marker.is_empty() {
        return text.contains(needle);
    }
    text.split_once(marker)
        .is_some_and(|(_, rest)| rest.contains(needle))
}

fn open_pty() -> (File, File) {
    let mut master = -1;
    let mut slave = -1;
    let rc = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    assert_eq!(rc, 0);
    let size = libc::winsize {
        ws_row: 40,
        ws_col: 160,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    unsafe {
        libc::ioctl(master, libc::TIOCSWINSZ, &size);
    }
    (unsafe { File::from_raw_fd(master) }, unsafe {
        File::from_raw_fd(slave)
    })
}
