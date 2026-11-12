use std::collections::HashMap;
use std::io::{ErrorKind, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::thread::{self, JoinHandle};

use drift_core::Registry;
use drift_tui::ChangeSink;
use inotify::{EventMask, Inotify, WatchDescriptor, WatchMask};

pub(crate) struct Watch {
    stop: Option<UnixStream>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Watch {
    fn drop(&mut self) {
        if let Some(mut stop) = self.stop.take() {
            let _ = stop.write_all(&[1]);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub(crate) fn dirs(registry: &Registry, home: &Path, omarchy: &Path) -> Vec<PathBuf> {
    registry
        .roots()
        .iter()
        .flat_map(|root| [root.live().under(home), root.template().under(omarchy)])
        .collect()
}

pub(crate) fn arm(paths: &[PathBuf], sink: ChangeSink) -> Option<Watch> {
    let mut inotify = Inotify::init().ok()?;
    let mut watched = HashMap::new();
    let mut armed = false;
    for path in paths {
        if watch_tree(&mut inotify, path, &mut watched) {
            armed = true;
        }
    }
    if !armed {
        return None;
    }
    let (stop, wake) = UnixStream::pair().ok()?;
    let thread = thread::spawn(move || listen(inotify, watched, wake, sink));
    Some(Watch {
        stop: Some(stop),
        thread: Some(thread),
    })
}

fn watch_mask() -> WatchMask {
    WatchMask::ATTRIB
        | WatchMask::CLOSE_WRITE
        | WatchMask::CREATE
        | WatchMask::DELETE
        | WatchMask::DELETE_SELF
        | WatchMask::DONT_FOLLOW
        | WatchMask::MODIFY
        | WatchMask::MOVE_SELF
        | WatchMask::MOVED_FROM
        | WatchMask::MOVED_TO
        | WatchMask::ONLYDIR
}

fn watch_tree(
    inotify: &mut Inotify,
    root: &Path,
    watched: &mut HashMap<WatchDescriptor, PathBuf>,
) -> bool {
    let Ok(descriptor) = inotify.add_watch(root, watch_mask()) else {
        return false;
    };
    watched.insert(descriptor, root.to_path_buf());
    let Ok(entries) = std::fs::read_dir(root) else {
        return true;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            let _ = watch_tree(inotify, &entry.path(), watched);
        }
    }
    true
}

fn listen(
    mut inotify: Inotify,
    mut watched: HashMap<WatchDescriptor, PathBuf>,
    wake: UnixStream,
    sink: ChangeSink,
) {
    let mut buffer = [0_u8; 8192];
    let inotify_fd = inotify.as_raw_fd();
    let wake_fd = wake.as_raw_fd();
    loop {
        let mut fds = [
            libc::pollfd {
                fd: inotify_fd,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: wake_fd,
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        let ready = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, -1) };
        if ready < 0 {
            if std::io::Error::last_os_error().kind() == ErrorKind::Interrupted {
                continue;
            }
            break;
        }
        if fds[1].revents != 0 {
            break;
        }
        if !readable(fds[0].revents) {
            continue;
        }
        if !take_events(&mut inotify, &mut watched, &mut buffer, &sink) {
            break;
        }
    }
}

fn take_events(
    inotify: &mut Inotify,
    watched: &mut HashMap<WatchDescriptor, PathBuf>,
    buffer: &mut [u8],
    sink: &ChangeSink,
) -> bool {
    let events = match inotify.read_events(buffer) {
        Ok(events) => events,
        Err(error) if error.kind() == ErrorKind::WouldBlock => return true,
        Err(_) => return false,
    };
    let mut interesting = false;
    let mut born = Vec::new();
    let mut gone = Vec::new();
    for event in events {
        if relevant(event.mask) {
            interesting = true;
        }
        if event.mask.contains(EventMask::IGNORED) {
            gone.push(event.wd.clone());
        }
        if new_directory(event.mask) {
            if let (Some(parent), Some(name)) = (watched.get(&event.wd), event.name) {
                born.push(parent.join(name));
            }
        }
    }
    for descriptor in gone {
        watched.remove(&descriptor);
    }
    for path in born {
        let _ = watch_tree(inotify, &path, watched);
    }
    if interesting {
        sink.changed();
    }
    true
}

fn relevant(mask: EventMask) -> bool {
    mask.intersects(
        EventMask::ATTRIB
            | EventMask::CLOSE_WRITE
            | EventMask::CREATE
            | EventMask::DELETE
            | EventMask::DELETE_SELF
            | EventMask::MODIFY
            | EventMask::MOVE_SELF
            | EventMask::MOVED_FROM
            | EventMask::MOVED_TO
            | EventMask::Q_OVERFLOW
            | EventMask::UNMOUNT,
    )
}

fn new_directory(mask: EventMask) -> bool {
    mask.contains(EventMask::ISDIR) && mask.intersects(EventMask::CREATE | EventMask::MOVED_TO)
}

fn readable(revents: libc::c_short) -> bool {
    (revents & (libc::POLLIN | libc::POLLERR | libc::POLLHUP)) != 0
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use drift_core::Registry;
    use inotify::EventMask;

    use super::{dirs, relevant};

    #[test]
    fn tracked_roots_watch_the_live_tree_and_the_package_templates() {
        let registry = Registry::v1();
        let watched = dirs(
            &registry,
            Path::new("/home/user"),
            Path::new("/usr/share/omarchy"),
        );
        assert_eq!(
            watched,
            vec![
                PathBuf::from("/home/user/.config"),
                PathBuf::from("/usr/share/omarchy/config"),
            ]
        );
    }

    #[test]
    fn content_changes_request_a_rescan_and_reads_do_not() {
        assert!(relevant(EventMask::MODIFY));
        assert!(relevant(EventMask::CREATE | EventMask::ISDIR));
        assert!(relevant(EventMask::Q_OVERFLOW));
        assert!(!relevant(EventMask::ACCESS));
        assert!(!relevant(EventMask::OPEN));
        assert!(!relevant(EventMask::CLOSE_NOWRITE));
        assert!(!relevant(EventMask::IGNORED));
    }
}
