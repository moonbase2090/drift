use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

static ARMED: AtomicBool = AtomicBool::new(false);

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
    let merged_fd = merged_read.into_raw_fd();
    let copied = unsafe { libc::dup2(merged_fd, libc::STDIN_FILENO) };
    unsafe { libc::close(merged_fd) };
    if copied < 0 {
        return Err(io::Error::last_os_error());
    }
    thread::Builder::new()
        .name(String::from("drift-wake"))
        .spawn(move || forward(input, wake_read, merged_write))?;
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

fn forward(mut input: File, mut wake: impl Read + AsRawFd, mut out: impl Write) {
    let mut buffer = [0_u8; 4096];
    loop {
        let mut fds = [poll_fd(input.as_raw_fd()), poll_fd(wake.as_raw_fd())];
        let ready = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, -1) };
        if ready < 0 {
            if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            break;
        }
        if fds[0].revents != 0 && !pump(&mut input, &mut buffer, &mut out) {
            break;
        }
        if fds[1].revents != 0 {
            let _ = pump(&mut wake, &mut buffer, &mut out);
        }
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
