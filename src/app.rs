//! TUI main loop: a scanner thread produces snapshots over mpsc; the UI
//! thread renders them and handles keys with a 100ms poll so input stays
//! responsive even if a scan stalls.

use std::sync::mpsc::{self, TryRecvError};
use std::time::{Duration, Instant};

use anyhow::Context;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};

use crate::snapshot::{self, Snapshot};
use crate::ui;

pub const POLL_INTERVAL: Duration = Duration::from_secs(2);
const INPUT_POLL: Duration = Duration::from_millis(100);

pub fn run() -> anyhow::Result<()> {
    let (tx, rx) = mpsc::channel::<Snapshot>();
    std::thread::spawn(move || {
        let mut store = crate::state::StateStore::new();
        loop {
            let snapshot = snapshot::scan_debounced(&mut store);
            // UI gone → channel closed → stop scanning.
            if tx.send(snapshot).is_err() {
                break;
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    });

    // ratatui::init installs a panic hook that restores the terminal.
    let mut terminal = ratatui::init();
    let result = ui_loop(&mut terminal, &rx);
    ratatui::restore();
    result
}

fn ui_loop(
    terminal: &mut ratatui::DefaultTerminal,
    rx: &mpsc::Receiver<Snapshot>,
) -> anyhow::Result<()> {
    let mut snapshot: Option<Snapshot> = None;
    let mut updated_at: Option<Instant> = None;
    let mut scroll: u16 = 0;

    loop {
        loop {
            match rx.try_recv() {
                Ok(next) => {
                    snapshot = Some(next);
                    updated_at = Some(Instant::now());
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    anyhow::bail!("scanner thread terminated unexpectedly");
                }
            }
        }

        let lines = ui::tree_lines(snapshot.as_ref());
        let header = ui::header_line(
            snapshot.as_ref(),
            updated_at.map(|at| at.elapsed().as_secs()),
        );
        let mut viewport = 0u16;
        terminal
            .draw(|frame| {
                viewport = ui::body_height(frame.area());
                let max_scroll = (lines.len() as u16).saturating_sub(viewport);
                scroll = scroll.min(max_scroll);
                ui::draw(frame, &lines, header.clone(), scroll);
            })
            .context("terminal draw failed")?;

        if event::poll(INPUT_POLL).context("input poll failed")?
            && let Event::Key(key) = event::read().context("input read failed")?
        {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match (key.code, key.modifiers) {
                (KeyCode::Char('q') | KeyCode::Esc, _) => return Ok(()),
                (KeyCode::Char('c'), KeyModifiers::CONTROL) => return Ok(()),
                (KeyCode::Char('j') | KeyCode::Down, _) => {
                    scroll = scroll.saturating_add(1);
                }
                (KeyCode::Char('k') | KeyCode::Up, _) => {
                    scroll = scroll.saturating_sub(1);
                }
                (KeyCode::Char('g'), _) => scroll = 0,
                (KeyCode::PageDown, _) => scroll = scroll.saturating_add(viewport),
                (KeyCode::PageUp, _) => scroll = scroll.saturating_sub(viewport),
                _ => {}
            }
        }
    }
}
