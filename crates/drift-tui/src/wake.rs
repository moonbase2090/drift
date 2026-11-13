use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

static ARMED: AtomicBool = AtomicBool::new(false);
static TERMINAL: AtomicI32 = AtomicI32::new(-1);
static CONTROL: AtomicI32 = AtomicI32::new(-1);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Gate {
    Run,
    Asked,
    Held,
}

static GATE: Mutex<Gate> = Mutex::new(Gate::Run);
static GATE_CHANGED: Condvar = Condvar::new();

pub(crate) fn terminal() -> Option<i32> {
    let fd = TERMINAL.load(Ordering::SeqCst);
    if fd >= 0 {
        return Some(fd);
    }
    (unsafe { libc::isatty(libc::STDIN_FILENO) } == 1).then_some(libc::STDIN_FILENO)
}

pub(crate) fn pause() -> bool {
    let control = CONTROL.load(Ordering::SeqCst);
    if control < 0 {
        return true;
    }
    let mut gate = GATE.lock().unwrap_or_else(|err| err.into_inner());
    *gate = Gate::Asked;
    let sent = unsafe { libc::write(control, [1_u8].as_ptr().cast(), 1) };
    if sent != 1 {
        *gate = Gate::Run;
        return false;
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while *gate != Gate::Held {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            *gate = Gate::Run;
            return false;
        }
        gate = GATE_CHANGED
            .wait_timeout(gate, left)
            .unwrap_or_else(|err| err.into_inner())
            .0;
    }
    true
}

pub(crate) fn resume() {
    let mut gate = GATE.lock().unwrap_or_else(|err| err.into_inner());
    *gate = Gate::Run;
    GATE_CHANGED.notify_all();
}

#[derive(Clone)]
pub(crate) struct Wake {
    writer: Arc<Mutex<Option<std::io::PipeWriter>>>,
}

impl Wake {
    pub(crate) fn new() -> Self {
        Self {
            writer: Arc::new(Mutex::new(None)),
        }
    }

    pub(crate) fn ping(&self) {
        let mut guard = self.writer.lock().unwrap_or_else(|err| err.into_inner());
        let Some(pipe) = guard.as_mut() else {
            return;
        };
        let _ = pipe.write_all(b"\x1b[I");
        let _ = pipe.flush();
    }

    fn attach(&self, pipe: std::io::PipeWriter) {
        let mut guard = self.writer.lock().unwrap_or_else(|err| err.into_inner());
        *guard = Some(pipe);
    }
}

pub(crate) fn arm(wake: &Wake) -> io::Result<()> {
    if ARMED.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    if let Err(error) = install(wake) {
        ARMED.store(false, Ordering::SeqCst);
        return Err(error);
    }
    Ok(())
}

fn install(wake: &Wake) -> io::Result<()> {
    let input = duplicate_stdin()?;
    let (merged_read, merged_write) = io::pipe()?;
    let (wake_read, wake_write) = io::pipe()?;
    let (control_read, control_write) = io::pipe()?;
    if unsafe { libc::isatty(input.as_raw_fd()) } == 1 {
        TERMINAL.store(input.as_raw_fd(), Ordering::SeqCst);
    }
    CONTROL.store(control_write.into_raw_fd(), Ordering::SeqCst);
    let merged_fd = merged_read.into_raw_fd();
    let copied = unsafe { libc::dup2(merged_fd, libc::STDIN_FILENO) };
    unsafe { libc::close(merged_fd) };
    if copied < 0 {
        return Err(io::Error::last_os_error());
    }
    thread::Builder::new()
        .name(String::from("drift-wake"))
        .spawn(move || forward(input, wake_read, control_read, merged_write))?;
    wake.attach(wake_write);
    Ok(())
}

fn duplicate_stdin() -> io::Result<File> {
    let fd = unsafe { libc::dup(io::stdin().as_raw_fd()) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn forward(
    mut input: File,
    mut wake: impl Read + AsRawFd,
    mut control: impl Read + AsRawFd,
    mut out: impl Write,
) {
    let mut buffer = [0_u8; 4096];
    loop {
        let mut fds = [
            poll_fd(input.as_raw_fd()),
            poll_fd(wake.as_raw_fd()),
            poll_fd(control.as_raw_fd()),
        ];
        let ready = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, -1) };
        if ready < 0 {
            if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            break;
        }
        if fds[2].revents != 0 {
            let _ = control.read(&mut buffer);
            hold();
            continue;
        }
        if fds[0].revents != 0 && !pump(&mut input, &mut buffer, &mut out) {
            break;
        }
        if fds[1].revents != 0 {
            let _ = pump(&mut wake, &mut buffer, &mut out);
        }
    }
}

fn hold() {
    let mut gate = GATE.lock().unwrap_or_else(|err| err.into_inner());
    if *gate != Gate::Asked {
        return;
    }
    *gate = Gate::Held;
    GATE_CHANGED.notify_all();
    while *gate == Gate::Held {
        gate = GATE_CHANGED
            .wait(gate)
            .unwrap_or_else(|err| err.into_inner());
    }
}

fn poll_fd(fd: i32) -> libc::pollfd {
    libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    }
}

fn pump(source: &mut impl Read, buffer: &mut [u8], out: &mut impl Write) -> bool {
    match source.read(buffer) {
        Ok(0) => false,
        Ok(size) => out.write_all(&buffer[..size]).is_ok(),
        Err(error) if error.kind() == io::ErrorKind::Interrupted => true,
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready(fd: i32, wait_ms: i32) -> bool {
        let mut fds = [poll_fd(fd)];
        unsafe { libc::poll(fds.as_mut_ptr(), 1, wait_ms) == 1 }
    }

    #[test]
    fn a_paused_forwarder_leaves_the_terminal_input_alone_until_it_is_resumed() {
        let (input_read, mut input_write) = io::pipe().unwrap();
        let (wake_read, _wake_write) = io::pipe().unwrap();
        let (control_read, control_write) = io::pipe().unwrap();
        let (mut out_read, out_write) = io::pipe().unwrap();
        CONTROL.store(control_write.into_raw_fd(), Ordering::SeqCst);
        let input = File::from(std::os::fd::OwnedFd::from(input_read));
        thread::spawn(move || forward(input, wake_read, control_read, out_write));

        input_write.write_all(b"a").unwrap();
        assert!(ready(out_read.as_raw_fd(), 2000));
        assert_eq!(take_pipe(&mut out_read), b"a");

        assert!(pause());
        input_write.write_all(b"b").unwrap();
        assert!(!ready(out_read.as_raw_fd(), 200));

        resume();
        assert!(ready(out_read.as_raw_fd(), 2000));
        assert_eq!(take_pipe(&mut out_read), b"b");
        CONTROL.store(-1, Ordering::SeqCst);
    }

    fn take_pipe(source: &mut std::io::PipeReader) -> Vec<u8> {
        let mut buffer = [0_u8; 16];
        let size = source.read(&mut buffer).unwrap_or(0);
        buffer[..size].to_vec()
    }
}
