//! Input event handling for the TUI.

use crossterm::event::{self, Event, KeyCode, KeyEventKind};

use super::app::{AppState, View};
use crate::search::{self, SearchIndex};

/// Poll for and handle a single input event. Returns `Ok(true)` if the app
/// should continue running, `Ok(false)` to quit.
pub fn handle_events(state: &mut AppState) -> anyhow::Result<bool> {
    if event::poll(std::time::Duration::from_millis(100))? {
        if let Event::Key(key) = event::read()? {
            // Only respond to press events (ignore release/repeat on Windows).
            if key.kind != KeyEventKind::Press {
                return Ok(!state.should_quit);
            }

            match state.view {
                View::Projects => handle_projects_keys(state, key.code),
                View::Sources => handle_sources_keys(state, key.code),
                View::Search => handle_search_keys(state, key.code)?,
            }
        }
    }
    Ok(!state.should_quit)
}

fn handle_projects_keys(state: &mut AppState, code: KeyCode) {
    match code {
        KeyCode::Char('q') | KeyCode::Esc => state.go_back(),
        KeyCode::Up | KeyCode::Char('k') => state.select_prev(),
        KeyCode::Down | KeyCode::Char('j') => state.select_next(),
        KeyCode::Enter => state.enter_sources(),
        KeyCode::Char('/') => state.enter_search(),
        _ => {}
    }
}

fn handle_sources_keys(state: &mut AppState, code: KeyCode) {
    match code {
        KeyCode::Esc | KeyCode::Backspace | KeyCode::Char('q') => state.go_back(),
        _ => {}
    }
}

fn handle_search_keys(state: &mut AppState, code: KeyCode) -> anyhow::Result<()> {
    match code {
        KeyCode::Esc => state.go_back(),
        KeyCode::Enter => {
            if !state.search_input.is_empty() {
                run_search(state)?;
            }
        }
        KeyCode::Backspace => {
            state.search_input.pop();
        }
        KeyCode::Char(c) => {
            state.search_input.push(c);
        }
        KeyCode::Up | KeyCode::Left => state.select_prev(),
        KeyCode::Down | KeyCode::Right => state.select_next(),
        _ => {}
    }
    Ok(())
}

fn run_search(state: &mut AppState) -> anyhow::Result<()> {
    let idx = SearchIndex::open_or_create()?;
    state.search_results = search::search(&idx, &state.search_input, 20, None, None)?;
    state.selected_result = 0;
    Ok(())
}
