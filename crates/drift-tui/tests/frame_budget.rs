#![doc = "Frame budget for the tree screen with 5,000 rows, measured in CPU time."]

use std::time::Duration;

use drift_core::{BaseProvenance, ScanEntry, ScanReport, Status};
use drift_fixtures::{busy_for, cpu_cost};
use drift_tui::TreeScreen;
use lunatui::{render_once, App, ControlFlow};
use lunatui_core::{ClusterTable, Grid, Rect};
use lunatui_term::{Event, KeyCode, KeyEvent, MouseEvent, MouseKind, Scroll};

const BUDGET: Duration = Duration::from_millis(8);

struct Driver<'a>(&'a mut TreeScreen);

impl App for Driver<'_> {
    fn update(&mut self, event: &Event) -> ControlFlow {
        let _ = self.0.handle(event);
        ControlFlow::Continue
    }

    fn render(&mut self, grid: &mut Grid, area: Rect, clusters: &mut ClusterTable) {
        self.0.render(grid, area, clusters);
    }
}

fn report(count: usize) -> ScanReport {
    let entries = (0..count)
        .map(|n| ScanEntry {
            path: format!(".config/dir{:03}/file{n:04}.conf", n % 120),
            template: Some(format!("config/dir{:03}/file{n:04}.conf", n % 120)),
            status: Status::ALL[n % 6],
            badge: None,
            base: BaseProvenance::None,
            volatile: false,
            note: String::new(),
        })
        .collect();
    ScanReport {
        package_version: "4.5.0-3".to_string(),
        reviewed_version: None,
        entries,
        backups: Vec::new(),
        ignored: 0,
    }
}

fn press(screen: &mut TreeScreen, code: KeyCode) {
    let _ = screen.handle(&Event::Key(KeyEvent::press(code)));
}

#[test]
fn five_thousand_rows_render_under_the_budget_in_every_frame() {
    for (columns, rows) in [(80, 24), (200, 60)] {
        let mut screen = TreeScreen::new(&report(5000));
        for _ in 0..20 {
            let _ = render_once(&mut Driver(&mut screen), columns, rows);
        }
        let mut worst = (Duration::ZERO, 0);
        for step in 0..400 {
            match step % 4 {
                0 => press(&mut screen, KeyCode::PageDown),
                1 => press(&mut screen, KeyCode::Down),
                2 => {
                    let _ = screen.handle(&Event::Mouse(MouseEvent::new(
                        MouseKind::Scroll(Scroll::Down),
                        5,
                        5,
                    )));
                }
                _ => press(&mut screen, KeyCode::Char(' ')),
            }
            let took = cpu_cost(|| {
                let _ = render_once(&mut Driver(&mut screen), columns, rows);
            });
            if took > worst.0 {
                worst = (took, step);
            }
        }
        eprintln!("{columns}x{rows} worst {:?} at step {}", worst.0, worst.1);
        assert!(
            worst.0 < BUDGET,
            "{columns}x{rows} worst {:?} at step {}",
            worst.0,
            worst.1
        );
    }
}

#[test]
fn the_cpu_clock_ignores_waiting_and_counts_work() {
    let waiting = cpu_cost(|| std::thread::sleep(Duration::from_millis(40)));
    assert!(waiting < Duration::from_millis(5), "{waiting:?}");
    let working = cpu_cost(|| busy_for(Duration::from_millis(30)));
    assert!(working >= Duration::from_millis(20), "{working:?}");
}

#[test]
fn a_render_that_burns_cpu_over_the_budget_is_caught() {
    let mut screen = TreeScreen::new(&report(5000));
    let slow = cpu_cost(|| {
        busy_for(Duration::from_millis(12));
        let _ = render_once(&mut Driver(&mut screen), 80, 24);
    });
    assert!(slow >= BUDGET, "{slow:?}");
}
