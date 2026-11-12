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
    if let Some(path) = args.first() {
        if path.contains("drift-edit-") {
            edit_probe(path);
            return;
        }
    }
    parent();
}

fn diff_host() {
    unsafe {
        libc::umask(0o022);
    }
    let screen = drift_tui::DiffScreen::new("same\n", "left\n", "right\n");
    let _ = lunatui::run(screen, lunatui::RunOptions::new(160, 40).pixels(false));
}

fn edit_probe(path: &str) {
    let mut out = std::io::stdout();
    let _ = writeln!(out, "probe ready");
    let _ = out.flush();
    let byte = read_byte(Duration::from_secs(3)).unwrap_or(b'?');
    consume_line();
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

fn parent() {
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
        .arg("--diff-host")
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
    let saw_conflict = wait_text(&captured, &text_rx, "conflict", Duration::from_secs(5));
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
        "conflict",
        Duration::from_secs(5),
    );
    let _ = master_write.write_all(b"q");
    let _ = master_write.flush();
    let (done_tx, done_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = child.wait();
        let _ = done_tx.send(());
    });
    if done_rx.recv_timeout(Duration::from_secs(2)).is_err() {
        unsafe { libc::kill(pid, libc::SIGKILL) };
    }
    let mut err = String::new();
    if let Some(mut stderr) = stderr {
        let _ = stderr.read_to_string(&mut err);
    }
    let tail = captured.lock().unwrap_or_else(|error| error.into_inner());
    let tail = tail.chars().rev().take(400).collect::<String>();
    let tail = tail.chars().rev().collect::<String>();
    assert!(
        saw_conflict && saw_ready && saw_byte && saw_redraw,
        "conflict {saw_conflict} ready {saw_ready} byte {saw_byte} redraw {saw_redraw} err {err} tail {tail}"
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
