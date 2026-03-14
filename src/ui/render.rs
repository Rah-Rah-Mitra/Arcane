//! Layout dispatch and rendering for the TUI.

use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

use super::app::{AppState, View};

/// Render the current application state to the terminal frame.
pub fn render(frame: &mut Frame, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Min(0),   // body
            Constraint::Length(1), // footer
        ])
        .split(frame.area());

    // ── Header ────────────────────────────────────────────────────────
    let header_text = match state.view {
        View::Projects => " Arcane — Projects ",
        View::Sources => " Arcane — Sources ",
        View::Search => " Arcane — Search ",
    };
    let header = Paragraph::new(header_text)
        .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
        .block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(header, chunks[0]);

    // ── Body ──────────────────────────────────────────────────────────
    match state.view {
        View::Projects => render_projects(frame, state, chunks[1]),
        View::Sources => render_sources(frame, state, chunks[1]),
        View::Search => render_search(frame, state, chunks[1]),
    }

    // ── Footer ────────────────────────────────────────────────────────
    let help = match state.view {
        View::Projects => "j/k: navigate  Enter: view sources  /: search  q: quit",
        View::Sources => "Esc: back  q: back",
        View::Search => "Type to search  Enter: run query  Esc: back",
    };
    let footer = Paragraph::new(help)
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(footer, chunks[2]);
}

fn render_projects(frame: &mut Frame, state: &AppState, area: ratatui::layout::Rect) {
    let items: Vec<ListItem> = state
        .projects
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let tags = if p.tags.is_empty() {
                String::new()
            } else {
                format!("  [{}]", p.tags.join(", "))
            };
            let content = format!(
                "{} ({} sources){}",
                p.name,
                p.sources.len(),
                tags
            );
            let style = if i == state.selected_project {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(content).style(style)
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Projects"));
    frame.render_widget(list, area);
}

fn render_sources(frame: &mut Frame, state: &AppState, area: ratatui::layout::Rect) {
    let project_name = state.viewing_project.as_deref().unwrap_or("?");
    let project = state.projects.iter().find(|p| p.name == project_name);

    let items: Vec<ListItem> = match project {
        Some(p) => p
            .sources
            .iter()
            .map(|s| {
                let kind = if s.needs_chunking { "textbook" } else { "report" };
                ListItem::new(format!("{} ({})", s.title, kind))
            })
            .collect(),
        None => vec![ListItem::new("(project not found)")],
    };

    let title = format!("Sources — {}", project_name);
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title));
    frame.render_widget(list, area);
}

fn render_search(frame: &mut Frame, state: &AppState, area: ratatui::layout::Rect) {
    let inner = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    // Search input.
    let input = Paragraph::new(Line::from(vec![
        Span::styled("> ", Style::default().fg(Color::Cyan)),
        Span::raw(&state.search_input),
        Span::styled("_", Style::default().fg(Color::DarkGray)),
    ]))
    .block(Block::default().borders(Borders::ALL).title("Query"));
    frame.render_widget(input, inner[0]);

    // Results list.
    let items: Vec<ListItem> = state
        .search_results
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let content = format!(
                "[{}] {} — ch. \"{}\" (p{}, score {:.3})",
                r.project_name,
                r.source_title,
                r.chapter_title,
                r.page + 1,
                r.score
            );
            let style = if i == state.selected_result {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(content).style(style)
        })
        .collect();

    let results_title = format!("Results ({})", state.search_results.len());
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(results_title));
    frame.render_widget(list, inner[1]);
}
