use lunatui::{render_once, run, App, ControlFlow, RunOptions};
use lunatui_core::{clip_display_prefix, width, ClusterTable, Grid, Rect, Style};
use lunatui_runtime::keymap::{Command, KeyChord, Keymap};
use lunatui_term::{Event, KeyCode, Modifiers, MouseButton, MouseEvent, MouseKind};
use lunatui_theme::{Theme, Token, Variant};
use lunatui_widgets::{KeyHintBar, RenderCx, Tree, TreeItem, TreeState, Widget};

use drift_core::ScanReport;

use crate::worker::{Note, WorkerHandle};

const QUIT: &str = "app.quit";
const RESCAN: &str = "app.rescan";
const SPIN: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

enum Phase {
    Idle,
    Scanning,
    Failed,
}

pub struct Screen {
    keymap: Keymap,
    tree: TreeState,
    columns: u16,
    rows: u16,
    phase: Phase,
    report: Option<ScanReport>,
    spin: usize,
    kick: bool,
    worker: Option<WorkerHandle>,
}

impl Screen {
    pub fn new() -> Self {
        Self {
            keymap: keymap(),
            tree: TreeState::new(Vec::new()),
            columns: 0,
            rows: 0,
            phase: Phase::Idle,
            report: None,
            spin: 0,
            kick: false,
            worker: None,
        }
    }

    pub(crate) fn attached(worker: WorkerHandle) -> Self {
        Self {
            kick: true,
            worker: Some(worker),
            ..Self::new()
        }
    }
}

impl Default for Screen {
    fn default() -> Self {
        Self::new()
    }
}

pub fn keymap() -> Keymap {
    let mut map = Keymap::new();
    map.register(Command::new(QUIT, "Quit", QUIT_CHORDS));
    map.register(Command::new(RESCAN, "Rescan", RESCAN_CHORDS));
    map
}

const QUIT_CHORDS: &[KeyChord] = &[KeyChord::new(KeyCode::Char('q'), Modifiers::new())];
const RESCAN_CHORDS: &[KeyChord] = &[KeyChord::new(KeyCode::Char('r'), Modifiers::new())];

pub fn render_frame(width: u16, height: u16) -> (Grid, ClusterTable) {
    let mut screen = Screen::new();
    render_once(&mut screen, width, height)
}

pub fn frame_text(width: u16, height: u16) -> String {
    let (grid, clusters) = render_frame(width, height);
    grid.display_text(&clusters)
}

pub fn launch() -> std::io::Result<()> {
    let (columns, rows) = terminal_size();
    run(Screen::new(), RunOptions::new(columns, rows).pixels(false))
}

pub fn launch_with(worker: WorkerHandle) -> std::io::Result<()> {
    worker.arm_terminal()?;
    let (columns, rows) = terminal_size();
    run(
        Screen::attached(worker),
        RunOptions::new(columns, rows).pixels(false),
    )
}

pub fn quit_bounds(columns: u16, rows: u16) -> Option<Rect> {
    if columns == 0 || rows == 0 {
        return None;
    }
    let label = quit_label();
    let cells = u16::try_from(width(&label)).unwrap_or(u16::MAX);
    let hit_width = cells.min(columns);
    if hit_width == 0 {
        return None;
    }
    Some(Rect::new(0, rows - 1, hit_width, 1))
}

fn quit_label() -> String {
    let chord = QUIT_CHORDS[0];
    format!("[{chord}] Quit")
}

pub(crate) fn terminal_dimensions() -> (u16, u16) {
    terminal_size()
}

fn terminal_size() -> (u16, u16) {
    terminal_size::terminal_size()
        .map(|(columns, rows)| (columns.0, rows.0))
        .filter(|(columns, rows)| *columns > 0 && *rows > 0)
        .unwrap_or((80, 24))
}

fn regions(area: Rect) -> (Rect, Rect, Rect) {
    if area.height == 0 {
        let empty = Rect::new(area.x, area.y, area.width, 0);
        return (empty, empty, empty);
    }
    let header = Rect::new(area.x, area.y, area.width, 1);
    if area.height == 1 {
        let empty = Rect::new(area.x, area.y, 0, 0);
        return (header, empty, empty);
    }
    let hint = Rect::new(area.x, area.y + area.height - 1, area.width, 1);
    let tree = Rect::new(area.x, area.y + 1, area.width, area.height - 2);
    (header, tree, hint)
}

fn contains(area: Rect, column: u16, row: u16) -> bool {
    column >= area.x
        && row >= area.y
        && column < area.x.saturating_add(area.width)
        && row < area.y.saturating_add(area.height)
}

impl Screen {
    fn apply_notes(&mut self) {
        while let Some(note) = self.worker.as_ref().and_then(WorkerHandle::try_recv) {
            self.apply_note(note);
        }
    }

    fn apply_note(&mut self, note: Note) {
        self.phase = match note {
            Note::Scanning => Phase::Scanning,
            Note::Ready(report) => {
                self.report = Some(report);
                Phase::Idle
            }
            Note::Failed(_) => Phase::Failed,
        };
        if !matches!(self.phase, Phase::Scanning) {
            self.spin = 0;
        }
        self.refresh_tree();
    }

    fn kick_scan(&mut self) {
        if !self.kick {
            return;
        }
        self.kick = false;
        self.request_rescan();
    }

    fn request_rescan(&mut self) {
        let Some(worker) = &self.worker else {
            return;
        };
        worker.rescan();
        self.phase = Phase::Scanning;
        self.refresh_tree();
    }

    fn title(&self) -> String {
        match self.report.as_ref() {
            Some(report) if !report.package_version.is_empty() => {
                format!("Drift  {}", report.package_version)
            }
            _ => "Drift".to_string(),
        }
    }

    fn refresh_tree(&mut self) {
        let items = match self.phase {
            Phase::Idle => Vec::new(),
            Phase::Scanning => {
                let glyph = SPIN[self.spin % SPIN.len()];
                vec![TreeItem::leaf(format!("{glyph} scanning")).disabled()]
            }
            Phase::Failed => vec![TreeItem::leaf("scan failed").disabled()],
        };
        self.tree = TreeState::new(items);
    }
}

impl App for Screen {
    fn needs_animation_frames(&self) -> bool {
        matches!(self.phase, Phase::Scanning)
    }

    fn update(&mut self, event: &Event) -> ControlFlow {
        self.apply_notes();
        if let Event::Mouse(mouse) = event {
            if mouse_quits(self.columns, self.rows, mouse) {
                return ControlFlow::Break;
            }
        }
        match self.keymap.dispatch(event) {
            Some(QUIT) => ControlFlow::Break,
            Some(RESCAN) => {
                self.request_rescan();
                ControlFlow::Continue
            }
            _ => ControlFlow::Continue,
        }
    }

    fn render(&mut self, grid: &mut Grid, area: Rect, clusters: &mut ClusterTable) {
        self.apply_notes();
        self.kick_scan();
        if matches!(self.phase, Phase::Scanning) {
            self.spin = self.spin.wrapping_add(1);
            self.refresh_tree();
        }
        self.columns = area.width;
        self.rows = area.height;
        let theme = Theme::builtin(Variant::Dark);
        let mut cx = RenderCx::new(grid, clusters, &theme);
        let text = Style::new()
            .fg(cx.token(Token::TextPrimary))
            .bg(cx.token(Token::Surface0));
        let accent = Style::new()
            .fg(cx.token(Token::Accent))
            .bg(cx.token(Token::Surface0));
        cx.fill(area, text, ' ');
        let (header, tree, hint) = regions(area);
        if header.width > 0 && header.height > 0 {
            let title = self.title();
            let shown = clip_display_prefix(&title, header.width);
            cx.put_str(header.x, header.y, shown, accent);
        }
        if tree.width > 0 && tree.height > 0 {
            Tree::new(&self.tree).render(tree, &mut cx);
        }
        if hint.width > 0 && hint.height > 0 {
            KeyHintBar::new(&self.keymap).render(hint, &mut cx);
        }
    }
}

fn mouse_quits(columns: u16, rows: u16, mouse: &MouseEvent) -> bool {
    if !matches!(mouse.kind, MouseKind::Down(MouseButton::Left)) {
        return false;
    }
    quit_bounds(columns, rows).is_some_and(|hit| contains(hit, mouse.column, mouse.row))
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use lunatui::{render_once, App, ControlFlow};
    use lunatui_term::{KeyEvent, MouseEvent};
    use lunatui_test::styled_snapshot;

    use super::*;
    use crate::worker::Note;

    fn empty_report() -> ScanReport {
        ScanReport {
            package_version: String::new(),
            reviewed_version: None,
            entries: Vec::new(),
            backups: Vec::new(),
            ignored: 0,
        }
    }

    fn named_report(version: &str) -> ScanReport {
        ScanReport {
            package_version: version.to_string(),
            ..empty_report()
        }
    }

    const SIZES: [(u16, u16, &str); 3] =
        [(80, 24, "80x24"), (120, 36, "120x36"), (200, 60, "200x60")];

    #[test]
    fn snapshots_match_the_empty_screen() {
        for (columns, rows, label) in SIZES {
            let (grid, clusters) = render_frame(columns, rows);
            let snap = styled_snapshot(&grid, &clusters);
            let text = grid.display_text(&clusters);
            let lines: Vec<&str> = text.lines().collect();
            assert_eq!(lines.len(), usize::from(rows), "{label}");
            assert!(lines[0].contains("Drift"), "{label}");
            assert!(lines[1].trim().is_empty(), "{label}");
            assert!(lines[usize::from(rows) - 1].contains("[Q] Quit"), "{label}");
            assert!(
                lines[usize::from(rows) - 1].contains("[R] Rescan"),
                "{label}"
            );
            let expected = match label {
                "80x24" => include_str!("../snapshots/empty-80x24.txt"),
                "120x36" => include_str!("../snapshots/empty-120x36.txt"),
                "200x60" => include_str!("../snapshots/empty-200x60.txt"),
                _ => unreachable!(),
            };
            assert_eq!(snap, expected, "{label}");
        }
    }

    #[test]
    fn q_and_a_click_on_quit_both_leave() {
        let mut screen = Screen::new();
        let _ = render_once(&mut screen, 80, 24);
        let hit = quit_bounds(80, 24).unwrap();
        let quit_click = Event::Mouse(MouseEvent::new(
            MouseKind::Down(MouseButton::Left),
            hit.x + hit.width / 2,
            hit.y,
        ));
        assert_eq!(screen.update(&quit_click), ControlFlow::Break);

        let mut screen = Screen::new();
        let _ = render_once(&mut screen, 80, 24);
        let miss = Event::Mouse(MouseEvent::new(
            MouseKind::Down(MouseButton::Left),
            hit.x + hit.width,
            hit.y,
        ));
        assert_eq!(screen.update(&miss), ControlFlow::Continue);
        let key = Event::Key(KeyEvent::press(KeyCode::Char('q')));
        assert_eq!(screen.update(&key), ControlFlow::Break);
    }

    #[test]
    fn empty_frames_stay_under_the_budget() {
        for (columns, rows, _) in SIZES {
            if (columns, rows) == (120, 36) {
                continue;
            }
            let mut screen = Screen::new();
            let mut samples = Vec::new();
            for _ in 0..32 {
                let started = Instant::now();
                let _ = render_once(&mut screen, columns, rows);
                samples.push(started.elapsed());
            }
            let worst = samples.iter().copied().max().unwrap();
            assert!(
                worst < Duration::from_millis(8),
                "{columns}x{rows} worst {worst:?} over {} samples",
                samples.len()
            );
        }
    }

    #[test]
    fn rescan_without_a_worker_leaves_the_empty_screen() {
        let mut screen = Screen::new();
        let key = Event::Key(KeyEvent::press(KeyCode::Char('r')));
        assert_eq!(screen.update(&key), ControlFlow::Continue);
        assert!(!screen.needs_animation_frames());
        let text = frame_text(80, 24);
        assert!(!text.contains("scanning"));
        assert!(!text.contains("scan failed"));
    }

    struct Unlock(Option<std::sync::mpsc::Sender<()>>);

    impl Drop for Unlock {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }

    #[test]
    fn a_delayed_scan_keeps_the_screen_responsive() {
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let unlock = Unlock(Some(release_tx));
        let worker = crate::worker::launch(
            Box::new(move || {
                let _ = entered_tx.send(Duration::from_secs(2));
                let _ = release_rx.recv();
                Ok(empty_report())
            }),
            Duration::ZERO,
        );
        let mut screen = Screen::attached(worker);
        let started = Instant::now();
        let (grid, clusters) = render_once(&mut screen, 80, 24);
        let text = grid.display_text(&clusters);
        assert!(text.contains("scanning"), "{text}");
        let key = Event::Key(KeyEvent::press(KeyCode::Char('q')));
        assert_eq!(screen.update(&key), ControlFlow::Break);
        assert!(screen.needs_animation_frames());
        assert!(
            started.elapsed() < Duration::from_millis(200),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(entered_rx.recv().unwrap(), Duration::from_secs(2));
        drop(unlock);
        drop(screen);
    }

    #[test]
    fn notes_clear_a_finished_scan_and_show_a_failure() {
        let worker = crate::worker::launch(Box::new(|| Ok(empty_report())), Duration::ZERO);
        let mut screen = Screen::attached(worker);
        screen.kick = false;
        screen.apply_note(Note::Ready(empty_report()));
        assert!(!screen.needs_animation_frames());
        let (grid, clusters) = render_once(&mut screen, 80, 24);
        let idle = grid.display_text(&clusters);
        assert!(!idle.contains("scanning"), "{idle}");
        screen.apply_note(Note::Failed("unavailable".to_string()));
        let (grid, clusters) = render_once(&mut screen, 80, 24);
        let failed = grid.display_text(&clusters);
        assert!(failed.contains("scan failed"), "{failed}");
        assert!(!screen.needs_animation_frames());
    }

    #[test]
    fn rescan_marks_scanning_without_waiting_for_the_worker() {
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let unlock = Unlock(Some(release_tx));
        let worker = crate::worker::launch(
            Box::new(move || {
                let _ = release_rx.recv();
                Ok(empty_report())
            }),
            Duration::from_millis(200),
        );
        let mut screen = Screen::attached(worker);
        screen.kick = false;
        screen.apply_note(Note::Ready(empty_report()));
        let started = Instant::now();
        let key = Event::Key(KeyEvent::press(KeyCode::Char('r')));
        assert_eq!(screen.update(&key), ControlFlow::Continue);
        assert!(screen.needs_animation_frames());
        let (grid, clusters) = render_once(&mut screen, 80, 24);
        let text = grid.display_text(&clusters);
        assert!(text.contains("scanning"), "{text}");
        assert!(started.elapsed() < Duration::from_millis(200));
        drop(unlock);
        drop(screen);
    }

    #[test]
    fn quitting_returns_while_the_scan_is_blocked() {
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let worker = crate::worker::launch(
            Box::new(move || {
                let _ = entered_tx.send(());
                let _ = release_rx.recv();
                Ok(empty_report())
            }),
            Duration::ZERO,
        );
        let mut screen = Screen::attached(worker);
        let _ = render_once(&mut screen, 80, 24);
        entered_rx.recv().unwrap();
        let started = Instant::now();
        drop(screen);
        let elapsed = started.elapsed();
        drop(release_tx);
        assert!(
            elapsed < Duration::from_millis(200),
            "quit waited {elapsed:?}"
        );
    }

    #[test]
    fn finished_scans_keep_distinct_reports() {
        let step = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = std::sync::Arc::clone(&step);
        let worker = crate::worker::launch(
            Box::new(move || {
                let index = count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(named_report(if index == 0 { "one" } else { "two" }))
            }),
            Duration::ZERO,
        );
        let mut screen = Screen::attached(worker);
        screen.kick = false;
        screen.request_rescan();
        wait_for_version(&mut screen, "one");
        screen.request_rescan();
        wait_for_version(&mut screen, "two");
    }

    fn wait_for_version(screen: &mut Screen, version: &str) {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            screen.apply_notes();
            if screen
                .report
                .as_ref()
                .is_some_and(|report| report.package_version == version)
                && !matches!(screen.phase, Phase::Scanning)
            {
                return;
            }
            assert!(Instant::now() < deadline, "missing {version}");
            std::thread::yield_now();
        }
    }

    #[test]
    fn a_file_change_wakes_the_idle_screen_without_a_key() {
        if std::env::var_os("DRIFT_HOST_CHILD").is_some() {
            host_child();
            return;
        }
        host_parent();
    }

    fn host_child() {
        use std::io::Read;
        use std::os::fd::FromRawFd;
        let mut trigger = unsafe { std::fs::File::from_raw_fd(3) };
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let step = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = std::sync::Arc::clone(&step);
        let worker = crate::worker::launch(
            Box::new(move || {
                let index = count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if index == 0 {
                    let _ = release_rx.recv();
                    Ok(named_report("report-one"))
                } else {
                    Err("unavailable".to_string())
                }
            }),
            Duration::ZERO,
        );
        let changes = worker.changes();
        std::thread::spawn(move || {
            let mut byte = [0_u8; 1];
            loop {
                if trigger.read(&mut byte).ok() != Some(1) {
                    break;
                }
                if byte[0] == b'R' {
                    let _ = release_tx.send(());
                } else if byte[0] == b'C' {
                    changes.changed();
                }
            }
        });
        let _ = launch_with(worker);
    }

    fn host_parent() {
        use std::io::{Read, Write};
        use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd};
        use std::os::unix::process::CommandExt;
        use std::process::{Command, Stdio};
        use std::sync::mpsc;

        let (master, slave) = open_pty();
        let slave_fd = slave.as_raw_fd();
        let stdin_fd = unsafe { libc::dup(slave_fd) };
        let stdout_fd = unsafe { libc::dup(slave_fd) };
        assert!(stdin_fd >= 0 && stdout_fd >= 0);
        drop(slave);
        let mut master_write = master.try_clone().unwrap();
        let (mut parent_trigger, child_trigger) = std::os::unix::net::UnixStream::pair().unwrap();
        let trigger_fd = child_trigger.into_raw_fd();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .arg("screen::tests::a_file_change_wakes_the_idle_screen_without_a_key")
            .arg("--exact")
            .arg("--nocapture")
            .arg("--test-threads=1")
            .env("DRIFT_HOST_CHILD", "1")
            .stdin(unsafe { Stdio::from_raw_fd(stdin_fd) })
            .stdout(unsafe { Stdio::from_raw_fd(stdout_fd) })
            .stderr(Stdio::piped());
        unsafe {
            command.pre_exec(move || {
                if trigger_fd != 3 {
                    let copied = libc::dup2(trigger_fd, 3);
                    if copied < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    libc::close(trigger_fd);
                }
                Ok(())
            });
        }
        let mut child = command.spawn().unwrap();
        let pid = child.id() as i32;
        let captured = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
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
        let started = wait_text(&captured, &text_rx, "scanning", Duration::from_secs(5));
        parent_trigger.write_all(b"R").unwrap();
        parent_trigger.flush().unwrap();
        let ready = wait_text(&captured, &text_rx, "report-one", Duration::from_secs(5));
        parent_trigger.write_all(b"C").unwrap();
        parent_trigger.flush().unwrap();
        let failed = wait_text(&captured, &text_rx, "scan failed", Duration::from_secs(5));
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
        assert!(
            started && ready && failed,
            "started {started} ready {ready} failed {failed}"
        );
    }

    fn wait_text(
        captured: &std::sync::Mutex<String>,
        rx: &std::sync::mpsc::Receiver<()>,
        needle: &str,
        timeout: Duration,
    ) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if captured
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .contains(needle)
            {
                return true;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            match rx.recv_timeout(remaining) {
                Ok(()) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => return false,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return captured
                        .lock()
                        .unwrap_or_else(|err| err.into_inner())
                        .contains(needle);
                }
            }
        }
    }

    fn open_pty() -> (std::fs::File, std::fs::File) {
        use std::os::fd::FromRawFd;
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
            ws_row: 24,
            ws_col: 80,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        unsafe {
            libc::ioctl(master, libc::TIOCSWINSZ, &size);
        }
        (unsafe { std::fs::File::from_raw_fd(master) }, unsafe {
            std::fs::File::from_raw_fd(slave)
        })
    }
}
