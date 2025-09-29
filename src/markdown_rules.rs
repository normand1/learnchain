use crate::{config, session_manager::SessionEvent};

const EXECUTION_ERROR_PREFIX: &str = "execution error:";
const OPERATION_NOT_PERMITTED_PHRASE: &str = "operation not permitted";
/// Applies the repository's markdown inclusion rules with optional limits.
#[derive(Debug, Clone, Copy)]
pub struct MarkdownRules {
    max_events: usize,
}

impl Default for MarkdownRules {
    fn default() -> Self {
        Self {
            max_events: config::default_max_events(),
        }
    }
}

impl MarkdownRules {
    /// Create new rules with a custom maximum number of events.
    #[allow(dead_code)]
    pub fn with_max_events(max_events: usize) -> Self {
        Self { max_events }
    }

    /// Determines whether a single event should appear in the markdown output.
    pub fn should_include_event(&self, event: &SessionEvent) -> bool {
        should_include_event(event)
    }

    /// Return up to `max_events` of the most recent entries that satisfy [`Self::should_include_event`].
    pub fn select_events<'a>(&self, events: &'a [SessionEvent]) -> Vec<&'a SessionEvent> {
        let mut selected: Vec<&SessionEvent> = events
            .iter()
            .rev()
            .filter(|event| self.should_include_event(event))
            .take(self.max_events)
            .collect();

        selected.reverse();
        selected
    }

    /// Expose the configured maximum count for callers that need to inspect it.
    pub fn max_events(&self) -> usize {
        self.max_events
    }
}

fn should_include_event(event: &SessionEvent) -> bool {
    !includes_execution_error(event)
        && !includes_operation_not_permitted(event)
        && (has_content_texts(event)
            || has_non_blank(event.arguments.as_deref())
            || has_non_blank(event.output.as_deref()))
}

fn includes_execution_error(event: &SessionEvent) -> bool {
    event
        .content_texts
        .iter()
        .find_map(|text| non_empty_trimmed(text))
        .map_or(false, starts_with_execution_error)
        || event
            .output
            .as_deref()
            .and_then(non_empty_trimmed)
            .map_or(false, starts_with_execution_error)
}

fn includes_operation_not_permitted(event: &SessionEvent) -> bool {
    event
        .content_texts
        .iter()
        .any(|text| contains_operation_not_permitted(text))
        || event
            .output
            .as_deref()
            .map_or(false, contains_operation_not_permitted)
        || event
            .arguments
            .as_deref()
            .map_or(false, contains_operation_not_permitted)
}

fn contains_operation_not_permitted(value: &str) -> bool {
    non_empty_trimmed(value)
        .map(|text| {
            text.to_ascii_lowercase()
                .contains(OPERATION_NOT_PERMITTED_PHRASE)
        })
        .unwrap_or(false)
}

fn has_content_texts(event: &SessionEvent) -> bool {
    event
        .content_texts
        .iter()
        .any(|text| non_empty_trimmed(text).is_some())
}

fn has_non_blank(value: Option<&str>) -> bool {
    value.and_then(non_empty_trimmed).is_some()
}

fn non_empty_trimmed(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn starts_with_execution_error(value: &str) -> bool {
    value
        .to_ascii_lowercase()
        .starts_with(EXECUTION_ERROR_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_most_recent_events_up_to_max() {
        let rules = MarkdownRules::with_max_events(3);
        let events = vec![event("1"), event("2"), event("3"), event("4"), event("5")];

        let selected = rules.select_events(&events);

        assert_eq!(selected.len(), 3);
        assert_eq!(selected[0].timestamp, "3");
        assert_eq!(selected[1].timestamp, "4");
        assert_eq!(selected[2].timestamp, "5");
    }

    #[test]
    fn respects_custom_max_events() {
        let rules = MarkdownRules::with_max_events(2);
        let events = vec![event("a"), event("b"), event("c")];

        let selected = rules.select_events(&events);

        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].timestamp, "b");
        assert_eq!(selected[1].timestamp, "c");
    }

    #[test]
    fn excludes_operation_not_permitted_messages() {
        let rules = MarkdownRules::default();
        let mut event = event("operation");
        event.content_texts.clear();
        event.output = Some(
            "/Users/example/.rvm/scripts/rvm: line 29: /bin/ps: Operation not permitted"
                .to_string(),
        );

        assert!(!rules.should_include_event(&event));
    }

    fn event(label: &str) -> SessionEvent {
        SessionEvent {
            timestamp: label.to_string(),
            payload_type: "call".to_string(),
            call_id: Some(format!("call-{label}")),
            arguments: None,
            output: None,
            content_texts: vec![format!("content-{label}")],
        }
    }
}
