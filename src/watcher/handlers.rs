//! Event classification and handling for file watcher events.

use std::path::PathBuf;

use notify::EventKind;

use super::WatchEvent;

/// Convert a raw notify event into zero or more `WatchEvent`s.
pub fn classify_event(event: &notify::Event, project_name: &str) -> Vec<WatchEvent> {
    let mut results = Vec::new();

    for path in &event.paths {
        // Only care about PDF files.
        if !is_pdf(path) {
            continue;
        }

        match event.kind {
            EventKind::Create(_) => {
                results.push(WatchEvent::NewPdf {
                    project_name: project_name.to_string(),
                    path: path.clone(),
                });
            }
            EventKind::Modify(_) => {
                results.push(WatchEvent::Modified {
                    project_name: project_name.to_string(),
                    path: path.clone(),
                });
            }
            EventKind::Remove(_) => {
                results.push(WatchEvent::Removed {
                    project_name: project_name.to_string(),
                    path: path.clone(),
                });
            }
            _ => {}
        }
    }

    results
}

/// Check if a path has a `.pdf` extension (case-insensitive).
fn is_pdf(path: &PathBuf) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("pdf"))
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use notify::Event;
    use notify::event::CreateKind;

    #[test]
    fn classify_create_pdf() {
        let event = Event {
            kind: EventKind::Create(CreateKind::Any),
            paths: vec![PathBuf::from("/tmp/test.pdf")],
            attrs: Default::default(),
        };

        let results = classify_event(&event, "MyProject");
        assert_eq!(results.len(), 1);
        match &results[0] {
            WatchEvent::NewPdf { project_name, path } => {
                assert_eq!(project_name, "MyProject");
                assert_eq!(path, &PathBuf::from("/tmp/test.pdf"));
            }
            _ => panic!("expected NewPdf event"),
        }
    }

    #[test]
    fn classify_ignores_non_pdf() {
        let event = Event {
            kind: EventKind::Create(CreateKind::Any),
            paths: vec![PathBuf::from("/tmp/notes.txt")],
            attrs: Default::default(),
        };

        let results = classify_event(&event, "MyProject");
        assert!(results.is_empty());
    }

    #[test]
    fn classify_remove_pdf() {
        use notify::event::RemoveKind;
        let event = Event {
            kind: EventKind::Remove(RemoveKind::Any),
            paths: vec![PathBuf::from("/tmp/old.PDF")],
            attrs: Default::default(),
        };

        let results = classify_event(&event, "Proj");
        assert_eq!(results.len(), 1);
        match &results[0] {
            WatchEvent::Removed { project_name, .. } => {
                assert_eq!(project_name, "Proj");
            }
            _ => panic!("expected Removed event"),
        }
    }
}
