use std::io::{self, Write};
use std::mem;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, Instant};

const HOLD: i32 = 1;
const HELD: i32 = 2;
static STAGE: AtomicI32 = AtomicI32::new(0);
const FUTEX_WAIT: libc::c_long = 0;
const FUTEX_WAKE: libc::c_long = 1;
const LEAVE_SCREEN: &[u8] = b"\x1b[?1049l\x1b[?25h\x1b[?1006l\x1b[?1003l\x1b[?1002l\x1b[?1000l";
const ENTER_SCREEN: &[u8] = b"\x1b[?1049h\x1b[?25l\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h";

pub fn around_editor<T>(work: impl FnOnce() -> T) -> T {
    let Some(tty) = crate::wake::terminal() else {
        return work();
    };
    let Some(saved) = capture_termios(tty) else {
        return work();
    };
    let held = hold_reader();
    let paused = crate::wake::pause();
    let stdin = lend_terminal(tty);
    cook(tty, &saved);
    write_tty(LEAVE_SCREEN);
    let mut guard = Guard {
        tty,
        saved,
        held,
        paused,
        stdin,
    };
    let result = work();
    guard.finish();
    result
}

struct Guard {
    tty: i32,
    saved: libc::termios,
    held: bool,
    paused: bool,
    stdin: Option<i32>,
}

impl Guard {
    fn finish(&mut self) {
        restore_termios(self.tty, &self.saved);
        write_tty(ENTER_SCREEN);
        if let Some(original) = self.stdin.take() {
            unsafe {
                libc::dup2(original, libc::STDIN_FILENO);
                libc::close(original);
            }
        }
        if self.paused {
            crate::wake::resume();
            self.paused = false;
        }
        if self.held {
            release_reader();
            self.held = false;
        }
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.finish();
    }
}

fn lend_terminal(tty: i32) -> Option<i32> {
    if tty == libc::STDIN_FILENO {
        return None;
    }
    unsafe {
        let original = libc::dup(libc::STDIN_FILENO);
        if original < 0 {
            return None;
        }
        if libc::dup2(tty, libc::STDIN_FILENO) < 0 {
            libc::close(original);
            return None;
        }
        Some(original)
    }
}

fn capture_termios(tty: i32) -> Option<libc::termios> {
    unsafe {
        if libc::isatty(tty) != 1 {
            return None;
        }
        let mut term = mem::zeroed();
        if libc::tcgetattr(tty, &mut term) != 0 {
            return None;
        }
        Some(term)
    }
}

fn cook(tty: i32, saved: &libc::termios) {
    let mut cooked = *saved;
    cooked.c_iflag |= libc::ICRNL | libc::IXON;
    cooked.c_oflag |= libc::OPOST;
    cooked.c_lflag |= libc::ISIG | libc::ICANON | libc::ECHO | libc::IEXTEN;
    unsafe {
        libc::tcsetattr(tty, libc::TCSANOW, &cooked);
    }
}

fn restore_termios(tty: i32, saved: &libc::termios) {
    unsafe {
        libc::tcsetattr(tty, libc::TCSANOW, saved);
    }
}

fn write_tty(bytes: &[u8]) {
    let mut out = io::stdout();
    let _ = out.write_all(bytes);
    let _ = out.flush();
}

fn hold_reader() -> bool {
    let Some(tid) = input_tid() else {
        return false;
    };
    if !hook_reader() {
        return false;
    }
    STAGE.store(HOLD, Ordering::SeqCst);
    let sent = unsafe { libc::syscall(libc::SYS_tgkill, libc::getpid(), tid, libc::SIGUSR1) };
    if sent != 0 {
        STAGE.store(0, Ordering::SeqCst);
        return false;
    }
    let deadline = Instant::now() + Duration::from_secs(1);
    while STAGE.load(Ordering::SeqCst) != HELD {
        if Instant::now() >= deadline {
            if STAGE
                .compare_exchange(HOLD, 0, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
            {
                release_reader();
            }
            return false;
        }
        std::thread::yield_now();
    }
    true
}

fn release_reader() {
    STAGE.store(0, Ordering::SeqCst);
    unsafe {
        libc::syscall(
            libc::SYS_futex,
            STAGE.as_ptr(),
            FUTEX_WAKE,
            1i32,
            std::ptr::null::<libc::timespec>(),
        );
    }
}

fn hook_reader() -> bool {
    static HOOKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if HOOKED.load(Ordering::SeqCst) {
        return true;
    }
    unsafe {
        let mut action: libc::sigaction = mem::zeroed();
        action.sa_sigaction = on_suspend as *const () as usize;
        action.sa_flags = libc::SA_RESTART;
        libc::sigemptyset(&mut action.sa_mask);
        if libc::sigaction(libc::SIGUSR1, &action, std::ptr::null_mut()) != 0 {
            return false;
        }
    }
    HOOKED.store(true, Ordering::SeqCst);
    true
}

extern "C" fn on_suspend(_: libc::c_int) {
    if STAGE
        .compare_exchange(HOLD, HELD, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }
    while STAGE.load(Ordering::SeqCst) == HELD {
        unsafe {
            libc::syscall(
                libc::SYS_futex,
                STAGE.as_ptr(),
                FUTEX_WAIT,
                HELD,
                std::ptr::null::<libc::timespec>(),
            );
        }
    }
}

fn input_tid() -> Option<i32> {
    let tasks = std::fs::read_dir("/proc/self/task").ok()?;
    for entry in tasks.flatten() {
        let tid = entry.file_name().to_str()?.parse().ok()?;
        let comm = std::fs::read_to_string(entry.path().join("comm")).ok()?;
        if comm.trim() == "lunatui-input" {
            return Some(tid);
        }
    }
    None
}
