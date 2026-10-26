use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Clone)]
pub(super) enum EditInput {
    Key(KeyEvent),
    Paste(String),
}

impl EditInput {
    fn is_count_digit(&self) -> bool {
        match self {
            Self::Key(key) => {
                matches!(key.code, KeyCode::Char(character) if character.is_ascii_digit())
            }
            Self::Paste(_) => false,
        }
    }
}

#[derive(Default)]
pub(super) struct Repeat {
    pending: Vec<EditInput>,
    changed: bool,
    last: Vec<EditInput>,
}

impl Repeat {
    pub(super) fn record(&mut self, input: EditInput, changed: bool, finished: bool) {
        self.pending.push(input);
        self.changed |= changed;

        if finished {
            if self.changed {
                self.last = std::mem::take(&mut self.pending);
            }
            self.cancel();
        }
    }

    pub(super) fn is_recording(&self) -> bool {
        !self.pending.is_empty()
    }

    pub(super) fn cancel(&mut self) {
        self.pending.clear();
        self.changed = false;
    }

    pub(super) fn for_count(&mut self, count: Option<usize>) -> Vec<EditInput> {
        if self.last.is_empty() {
            return Vec::new();
        }
        let Some(count) = count else {
            return self.last.clone();
        };
        // An explicit count on dot replaces both counts in a command such as 2d3w.
        let mut inputs: Vec<_> = self
            .last
            .iter()
            .skip_while(|input| input.is_count_digit())
            .cloned()
            .collect();
        let operator = match inputs.first() {
            Some(EditInput::Key(key)) => matches!(key.code, KeyCode::Char('d' | 'c' | 'y')),
            _ => false,
        };
        if operator {
            let digits = inputs
                .iter()
                .skip(1)
                .take_while(|input| input.is_count_digit())
                .count();
            inputs.drain(1..1 + digits);
        }

        let mut prefix: Vec<_> = count
            .to_string()
            .chars()
            .map(|character| {
                EditInput::Key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE))
            })
            .collect();
        prefix.extend(inputs);
        self.last = prefix.clone();
        prefix
    }
}
