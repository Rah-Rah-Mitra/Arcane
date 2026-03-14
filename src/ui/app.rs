//! Application state machine for the TUI.

use crate::models::Project;
use crate::search::SearchResult;

/// Which view is currently active.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum View {
    Projects,
    Sources,
    Search,
}

/// Shared application state across all views.
pub struct AppState {
    /// Current active view.
    pub view: View,
    /// Whether the app should exit.
    pub should_quit: bool,

    // ── Projects view ─────────────────────────────────────────────────
    /// All loaded projects.
    pub projects: Vec<Project>,
    /// Currently selected project index.
    pub selected_project: usize,

    // ── Sources view ──────────────────────────────────────────────────
    /// Name of the project whose sources are being viewed.
    pub viewing_project: Option<String>,

    // ── Search view ───────────────────────────────────────────────────
    /// Current search query input.
    pub search_input: String,
    /// Search results.
    pub search_results: Vec<SearchResult>,
    /// Currently selected result index.
    pub selected_result: usize,
}

impl AppState {
    pub fn new(projects: Vec<Project>) -> Self {
        Self {
            view: View::Projects,
            should_quit: false,
            projects,
            selected_project: 0,
            viewing_project: None,
            search_input: String::new(),
            search_results: Vec::new(),
            selected_result: 0,
        }
    }

    /// Move selection up in the current list.
    pub fn select_prev(&mut self) {
        match self.view {
            View::Projects => {
                if self.selected_project > 0 {
                    self.selected_project -= 1;
                }
            }
            View::Sources => {}
            View::Search => {
                if self.selected_result > 0 {
                    self.selected_result -= 1;
                }
            }
        }
    }

    /// Move selection down in the current list.
    pub fn select_next(&mut self) {
        match self.view {
            View::Projects => {
                if !self.projects.is_empty()
                    && self.selected_project < self.projects.len() - 1
                {
                    self.selected_project += 1;
                }
            }
            View::Sources => {}
            View::Search => {
                if !self.search_results.is_empty()
                    && self.selected_result < self.search_results.len() - 1
                {
                    self.selected_result += 1;
                }
            }
        }
    }

    /// Enter the sources view for the currently selected project.
    pub fn enter_sources(&mut self) {
        if let Some(p) = self.projects.get(self.selected_project) {
            self.viewing_project = Some(p.name.clone());
            self.view = View::Sources;
        }
    }

    /// Go back to the projects view.
    pub fn go_back(&mut self) {
        match self.view {
            View::Sources => {
                self.viewing_project = None;
                self.view = View::Projects;
            }
            View::Search => {
                self.view = View::Projects;
            }
            View::Projects => {
                self.should_quit = true;
            }
        }
    }

    /// Switch to the search view.
    pub fn enter_search(&mut self) {
        self.view = View::Search;
        self.search_input.clear();
        self.search_results.clear();
        self.selected_result = 0;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_machine_transitions() {
        let projects = vec![
            Project::new("Alpha"),
            Project::new("Beta"),
        ];
        let mut state = AppState::new(projects);

        assert_eq!(state.view, View::Projects);
        assert_eq!(state.selected_project, 0);

        // Navigate down.
        state.select_next();
        assert_eq!(state.selected_project, 1);

        // Can't go past the end.
        state.select_next();
        assert_eq!(state.selected_project, 1);

        // Navigate up.
        state.select_prev();
        assert_eq!(state.selected_project, 0);

        // Enter sources view.
        state.enter_sources();
        assert_eq!(state.view, View::Sources);
        assert_eq!(state.viewing_project, Some("Alpha".to_string()));

        // Go back.
        state.go_back();
        assert_eq!(state.view, View::Projects);

        // Enter search.
        state.enter_search();
        assert_eq!(state.view, View::Search);

        // Go back from search.
        state.go_back();
        assert_eq!(state.view, View::Projects);

        // Quit from projects.
        state.go_back();
        assert!(state.should_quit);
    }
}
