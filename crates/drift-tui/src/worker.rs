use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use drift_core::ScanReport;

use crate::wake::{self, Wake};

pub const DEBOUNCE: Duration = Duration::from_millis(200);

enum Request {
    Rescan,
    Changed,
    Stop,
    Due,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Note {
    Scanning,
    Ready(ScanReport),
    Failed(String),
}

#[derive(Clone)]
pub struct ChangeSink {
    requests: Sender<Request>,
}

impl ChangeSink {
    pub fn changed(&self) {
        let _ = self.requests.send(Request::Changed);
    }
}

pub struct WorkerHandle {
    requests: Sender<Request>,
    notes: Receiver<Note>,
    thread: Option<JoinHandle<()>>,
    wake: Wake,
}

impl WorkerHandle {
    pub(crate) fn rescan(&self) {
        let _ = self.requests.send(Request::Rescan);
    }

    pub fn changes(&self) -> ChangeSink {
        ChangeSink {
            requests: self.requests.clone(),
        }
    }

    pub(crate) fn try_recv(&self) -> Option<Note> {
        self.notes.try_recv().ok()
    }

    pub(crate) fn arm_terminal(&self) -> std::io::Result<()> {
        wake::arm(&self.wake)
    }
}

impl Drop for WorkerHandle {
    fn drop(&mut self) {
        let _ = self.requests.send(Request::Stop);
        drop(self.thread.take());
    }
}

pub(crate) fn launch(
    mut scan: Box<dyn FnMut() -> Result<ScanReport, String> + Send>,
    debounce: Duration,
) -> WorkerHandle {
    let (requests_tx, requests_rx) = mpsc::channel();
    let (notes_tx, notes_rx) = mpsc::channel();
    let wake = Wake::new();
    let threaded = wake.clone();
    let thread = thread::spawn(move || serve(&mut scan, debounce, requests_rx, notes_tx, threaded));
    WorkerHandle {
        requests: requests_tx,
        notes: notes_rx,
        thread: Some(thread),
        wake,
    }
}

pub fn spawn_scan<F>(scan: F) -> WorkerHandle
where
    F: FnMut() -> Result<ScanReport, String> + Send + 'static,
{
    launch(Box::new(scan), DEBOUNCE)
}

fn serve<F>(
    scan: &mut F,
    debounce: Duration,
    requests: Receiver<Request>,
    notes: Sender<Note>,
    wake: Wake,
) where
    F: FnMut() -> Result<ScanReport, String>,
{
    let mut schedule = Schedule::new(debounce);
    while let Some(message) = next_message(&mut schedule, &requests) {
        match message {
            Request::Stop => break,
            Request::Changed => schedule.changed(),
            Request::Rescan => schedule.rescan(),
            Request::Due => {
                if !schedule.begin() {
                    continue;
                }
                let _ = notes.send(Note::Scanning);
                wake.ping();
                let result = scan();
                if !absorb(&mut schedule, &requests) {
                    break;
                }
                if schedule.finish() == Outcome::Publish && publish(&notes, &wake, result).is_err()
                {
                    break;
                }
            }
        }
    }
}

fn publish(
    notes: &Sender<Note>,
    wake: &Wake,
    result: Result<ScanReport, String>,
) -> Result<(), mpsc::SendError<Note>> {
    let note = match result {
        Ok(report) => Note::Ready(report),
        Err(error) => Note::Failed(error),
    };
    let sent = notes.send(note);
    wake.ping();
    sent
}

fn absorb(schedule: &mut Schedule, requests: &Receiver<Request>) -> bool {
    let mut rescan = false;
    let mut changed = false;
    loop {
        match requests.try_recv() {
            Ok(Request::Stop) => return false,
            Ok(Request::Rescan | Request::Due) => rescan = true,
            Ok(Request::Changed) => changed = true,
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => return false,
        }
    }
    if rescan {
        schedule.rescan();
    } else if changed {
        schedule.changed();
    }
    true
}

fn next_message(schedule: &mut Schedule, requests: &Receiver<Request>) -> Option<Request> {
    match schedule.wait() {
        Wait::Forever => requests.recv().ok(),
        Wait::Timeout(duration) => {
            let started = Instant::now();
            let message = wait_for(requests, duration)?;
            schedule.advance(started.elapsed());
            Some(message)
        }
    }
}

fn wait_for(requests: &Receiver<Request>, duration: Duration) -> Option<Request> {
    if duration.is_zero() {
        return match requests.try_recv() {
            Ok(message) => Some(message),
            Err(TryRecvError::Empty) => Some(Request::Due),
            Err(TryRecvError::Disconnected) => None,
        };
    }
    match requests.recv_timeout(duration) {
        Ok(message) => Some(message),
        Err(RecvTimeoutError::Timeout) => Some(Request::Due),
        Err(RecvTimeoutError::Disconnected) => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Armed {
    Idle,
    Immediate,
    Later(Duration),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Follow {
    None,
    Immediate,
    Later,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Publish,
    Dropped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Wait {
    Forever,
    Timeout(Duration),
}

#[derive(Clone, Debug)]
struct Schedule {
    now: Duration,
    debounce: Duration,
    running: bool,
    armed: Armed,
    follow: Follow,
}

impl Schedule {
    fn new(debounce: Duration) -> Self {
        Self {
            now: Duration::ZERO,
            debounce,
            running: false,
            armed: Armed::Idle,
            follow: Follow::None,
        }
    }

    fn advance(&mut self, by: Duration) {
        self.now = self.now.saturating_add(by);
    }

    fn changed(&mut self) {
        if self.running {
            if self.follow != Follow::Immediate {
                self.follow = Follow::Later;
            }
            return;
        }
        if self.armed == Armed::Immediate {
            return;
        }
        self.armed = Armed::Later(self.now.saturating_add(self.debounce));
    }

    fn rescan(&mut self) {
        if self.running {
            self.follow = Follow::Immediate;
            return;
        }
        self.armed = Armed::Immediate;
    }

    fn begin(&mut self) -> bool {
        if self.running || !self.ready() {
            return false;
        }
        self.armed = Armed::Idle;
        self.running = true;
        self.follow = Follow::None;
        true
    }

    fn ready(&self) -> bool {
        match self.armed {
            Armed::Idle => false,
            Armed::Immediate => true,
            Armed::Later(deadline) => self.now >= deadline,
        }
    }

    fn finish(&mut self) -> Outcome {
        self.running = false;
        match self.follow {
            Follow::None => Outcome::Publish,
            Follow::Immediate => {
                self.follow = Follow::None;
                self.armed = Armed::Immediate;
                Outcome::Dropped
            }
            Follow::Later => {
                self.follow = Follow::None;
                self.armed = Armed::Later(self.now.saturating_add(self.debounce));
                Outcome::Dropped
            }
        }
    }

    fn wait(&self) -> Wait {
        if self.running {
            return Wait::Forever;
        }
        match self.armed {
            Armed::Idle => Wait::Forever,
            Armed::Immediate => Wait::Timeout(Duration::ZERO),
            Armed::Later(deadline) => Wait::Timeout(deadline.saturating_sub(self.now)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{mpsc, Arc};
    use std::thread;

    use super::*;

    fn clock() -> Schedule {
        Schedule::new(DEBOUNCE)
    }

    fn empty_report() -> ScanReport {
        report_named("")
    }

    fn report_named(version: &str) -> ScanReport {
        ScanReport {
            package_version: version.to_string(),
            reviewed_version: None,
            entries: Vec::new(),
            backups: Vec::new(),
            ignored: 0,
        }
    }

    #[test]
    fn a_burst_scans_once_when_the_debounce_elapses() {
        let mut schedule = clock();
        for _ in 0..100 {
            schedule.changed();
        }
        schedule.advance(Duration::from_millis(199));
        assert!(!schedule.begin());
        assert!(matches!(schedule.wait(), Wait::Timeout(left) if left == Duration::from_millis(1)));
        schedule.advance(Duration::from_millis(1));
        assert!(schedule.begin());
        assert_eq!(schedule.finish(), Outcome::Publish);
        assert!(!schedule.begin());
        schedule.advance(Duration::from_secs(30));
        assert!(!schedule.begin());
        assert!(matches!(schedule.wait(), Wait::Forever));
    }

    #[test]
    fn an_idle_schedule_does_not_start_as_time_passes() {
        let mut schedule = clock();
        assert!(matches!(schedule.wait(), Wait::Forever));
        schedule.advance(Duration::from_secs(3600));
        assert!(!schedule.begin());
        assert!(matches!(schedule.wait(), Wait::Forever));
    }

    #[test]
    fn rescan_is_immediate_and_a_change_does_not_postpone_it() {
        let mut schedule = clock();
        schedule.rescan();
        schedule.changed();
        assert!(matches!(schedule.wait(), Wait::Timeout(left) if left.is_zero()));
        assert!(schedule.begin());
    }

    #[test]
    fn a_rescan_during_a_run_drops_the_result_and_starts_one_follow_up() {
        let mut schedule = clock();
        schedule.rescan();
        assert!(schedule.begin());
        schedule.rescan();
        schedule.changed();
        assert_eq!(schedule.finish(), Outcome::Dropped);
        assert!(schedule.begin());
        assert_eq!(schedule.finish(), Outcome::Publish);
        assert!(!schedule.begin());
    }

    #[test]
    fn a_change_during_a_run_waits_another_debounce() {
        let mut schedule = clock();
        schedule.rescan();
        assert!(schedule.begin());
        schedule.changed();
        assert_eq!(schedule.finish(), Outcome::Dropped);
        schedule.advance(Duration::from_millis(199));
        assert!(!schedule.begin());
        schedule.advance(Duration::from_millis(1));
        assert!(schedule.begin());
        assert_eq!(schedule.finish(), Outcome::Publish);
    }

    #[test]
    fn an_idle_worker_never_scans() {
        let scans = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&scans);
        let worker = launch(
            Box::new(move || {
                counted.fetch_add(1, Ordering::SeqCst);
                Ok(empty_report())
            }),
            Duration::ZERO,
        );
        drop(worker);
        assert_eq!(scans.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_hundred_queued_changes_scan_once() {
        let scans = Arc::new(AtomicUsize::new(0));
        let (requests_tx, requests_rx) = mpsc::channel();
        let (notes_tx, notes_rx) = mpsc::channel();
        for _ in 0..100 {
            requests_tx.send(Request::Changed).unwrap();
        }
        let counted = Arc::clone(&scans);
        let thread = thread::spawn(move || {
            serve(
                &mut || {
                    counted.fetch_add(1, Ordering::SeqCst);
                    Ok(empty_report())
                },
                Duration::ZERO,
                requests_rx,
                notes_tx,
                Wake::new(),
            );
        });
        assert_eq!(notes_rx.recv().unwrap(), Note::Scanning);
        assert!(matches!(notes_rx.recv().unwrap(), Note::Ready(_)));
        assert_eq!(scans.load(Ordering::SeqCst), 1);
        requests_tx.send(Request::Stop).unwrap();
        thread.join().unwrap();
    }

    #[test]
    fn rescans_during_a_blocked_scan_collapse_to_one_follow_up() {
        let scans = Arc::new(AtomicUsize::new(0));
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (follow_tx, follow_rx) = mpsc::channel();
        let counted = Arc::clone(&scans);
        let worker = launch(
            Box::new(move || {
                let n = counted.fetch_add(1, Ordering::SeqCst);
                if n == 0 {
                    let _ = entered_tx.send(Duration::from_secs(2));
                    let _ = release_rx.recv();
                } else {
                    let _ = follow_tx.send(());
                }
                Ok(empty_report())
            }),
            Duration::ZERO,
        );
        worker.rescan();
        assert_eq!(entered_rx.recv().unwrap(), Duration::from_secs(2));
        for _ in 0..100 {
            worker.rescan();
        }
        release_tx.send(()).unwrap();
        follow_rx.recv().unwrap();
        assert_eq!(scans.load(Ordering::SeqCst), 2);
        drop(worker);
    }

    #[test]
    fn distinct_reports_cross_the_worker() {
        let (requests_tx, requests_rx) = mpsc::channel();
        let (notes_tx, notes_rx) = mpsc::channel();
        let step = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&step);
        let thread = thread::spawn(move || {
            serve(
                &mut || {
                    let index = count.fetch_add(1, Ordering::SeqCst);
                    Ok(report_named(if index == 0 { "one" } else { "two" }))
                },
                Duration::ZERO,
                requests_rx,
                notes_tx,
                Wake::new(),
            );
        });
        requests_tx.send(Request::Rescan).unwrap();
        assert_eq!(notes_rx.recv().unwrap(), Note::Scanning);
        match notes_rx.recv().unwrap() {
            Note::Ready(report) => assert_eq!(report.package_version, "one"),
            other => panic!("first report was {other:?}"),
        }
        requests_tx.send(Request::Rescan).unwrap();
        assert_eq!(notes_rx.recv().unwrap(), Note::Scanning);
        match notes_rx.recv().unwrap() {
            Note::Ready(report) => assert_eq!(report.package_version, "two"),
            other => panic!("second report was {other:?}"),
        }
        requests_tx.send(Request::Stop).unwrap();
        thread.join().unwrap();
    }
}
