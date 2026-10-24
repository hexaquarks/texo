use anyhow::{Result, ensure};
use crokey::KeyCombination;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

pub struct Prefix {
    key: KeyCombination,
    pub armed: bool,
}

pub enum PrefixInput {
    Pass,
    Pending,
    Action(KeyCode),
}

impl Default for Prefix {
    fn default() -> Self {
        Self {
            key: crokey::key!(ctrl - x),
            armed: false,
        }
    }
}

impl Prefix {
    pub fn new(binding: &str) -> Result<Self> {
        let key: KeyCombination = binding
            .trim()
            .replace('+', "-")
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid prefix: use a shortcut such as ctrl+x"))?;
        let supported = key.is_ansi_compatible()
            && matches!(key.codes.first(), KeyCode::Char(character) if character.is_ascii_alphabetic())
            && matches!(key.modifiers, KeyModifiers::CONTROL | KeyModifiers::ALT);
        ensure!(supported, "Prefix must be Ctrl or Alt plus a letter");
        ensure!(
            key != crokey::key!(ctrl - q),
            "Ctrl+Q is reserved for quitting"
        );
        Ok(Self { key, armed: false })
    }

    pub fn label(&self) -> String {
        self.key.to_string()
    }

    pub fn handle(&mut self, event: KeyEvent) -> PrefixInput {
        if event.kind == KeyEventKind::Release {
            return PrefixInput::Pending;
        }
        if self.armed {
            if event.kind == KeyEventKind::Repeat {
                return PrefixInput::Pending;
            }
            self.armed = false;
            return if event.modifiers.is_empty() && event.code != KeyCode::Esc {
                PrefixInput::Action(event.code)
            } else {
                PrefixInput::Pending
            };
        }
        if KeyCombination::from(event) == self.key {
            self.armed = true;
            return PrefixInput::Pending;
        }
        PrefixInput::Pass
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_prefix_and_consumes_sequences_without_leaking_text() -> Result<()> {
        for binding in ["x", "F2", "ctrl+q", "ctrl+enter", "ctrl+alt+x", "invalid"] {
            assert!(Prefix::new(binding).is_err(), "{binding}");
        }
        let mut prefix = Prefix::new("ctrl+a")?;
        let start = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL);
        let action = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE);
        assert!(matches!(prefix.handle(action), PrefixInput::Pass));
        assert!(matches!(prefix.handle(start), PrefixInput::Pending));

        let mut release = start;
        release.kind = KeyEventKind::Release;
        prefix.handle(release);
        assert!(prefix.armed);
        let mut repeat = start;
        repeat.kind = KeyEventKind::Repeat;
        prefix.handle(repeat);
        assert!(prefix.armed);
        assert!(matches!(
            prefix.handle(action),
            PrefixInput::Action(KeyCode::Char('s'))
        ));
        assert!(!prefix.armed);

        prefix.handle(start);
        assert!(matches!(
            prefix.handle(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            PrefixInput::Pending
        ));
        assert!(!prefix.armed);
        Ok(())
    }
}
