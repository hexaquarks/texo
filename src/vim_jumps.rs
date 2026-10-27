use ratatui_textarea::DataCursor;

// Vim keeps at most 100 jump locations per window.
const MAX_JUMP_LOCATIONS: usize = 100;

#[derive(Default)]
pub(super) struct JumpList {
    entries: Vec<DataCursor>,
    current: Option<usize>,
}

impl JumpList {
    pub(super) fn record(&mut self, from: DataCursor, to: DataCursor) {
        if from == to {
            return;
        }

        // Match Vim's single entry per source line, retaining older forward locations on a new jump.
        self.entries.retain(|position| position.0 != from.0);
        self.push(from);
        self.current = None;
    }

    pub(super) fn older(&mut self, position: DataCursor) -> Option<DataCursor> {
        if self.entries.is_empty() {
            return None;
        }

        let current = match self.current {
            Some(current) => current,
            None => {
                if self.entries.last() != Some(&position) {
                    self.push(position);
                }
                self.entries.len() - 1
            }
        };
        let older = current.checked_sub(1)?;
        self.current = Some(older);
        Some(self.entries[older])
    }

    pub(super) fn newer(&mut self) -> Option<DataCursor> {
        let newer = self.current? + 1;
        let position = *self.entries.get(newer)?;
        self.current = Some(newer);
        Some(position)
    }

    fn push(&mut self, position: DataCursor) {
        self.entries.push(position);
        if self.entries.len() > MAX_JUMP_LOCATIONS {
            self.entries.remove(0);
        }
    }
}
