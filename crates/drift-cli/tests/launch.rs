use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd};
use std::process::{Command, Stdio};
use std::time::Duration;

use drift_fixtures::RootBuilder;

#[test]
fn a_broken_config_does_not_open_the_screen() {
    let mut builder = RootBuilder::new();
    builder
        .file(".config/drift/config.toml", b"roots = [\n")
        .unwrap();
    let root = builder.materialize().unwrap();
    let (_master, slave) = open_pty();
    let slave_fd = slave.as_raw_fd();
    let stdout_fd = unsafe { libc::dup(slave_fd) };
    assert!(stdout_fd >= 0);
    drop(slave);
    let mut child = Command::new(env!("CARGO_BIN_EXE_drift"))
        .env("HOME", root.path())
        .env_remove("XDG_CONFIG_HOME")
        .stdin(Stdio::null())
        .stdout(unsafe { Stdio::from_raw_fd(stdout_fd) })
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let pid = child.id() as i32;
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let status = child.wait().unwrap();
        let _ = done_tx.send(status);
    });
    let status = done_rx.recv_timeout(Duration::from_secs(3));
    if status.is_err() {
        unsafe { libc::kill(pid, libc::SIGKILL) };
    }
    let status = status.expect("the screen stayed open");
    let mut err = String::new();
    stderr.read_to_string(&mut err).unwrap();
    assert!(!status.success(), "{err}");
    assert!(err.contains("config.toml"), "{err}");
}

fn open_pty() -> (std::fs::File, std::fs::File) {
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
    (unsafe { std::fs::File::from_raw_fd(master) }, unsafe {
        std::fs::File::from_raw_fd(slave)
    })
}
