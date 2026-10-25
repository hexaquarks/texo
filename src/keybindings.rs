use std::time::{Duration, Instant};

use anyhow::{Result, ensure};
use crokey::KeyCombination;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

// Match tmux's default repeat window for bindings declared with `bind -r`.
const RESIZE_REPEAT_WINDOW: Duration = Duration::from_millis(500);

#[derive(Clone)]
pub struct Prefix {
    key: KeyCombination,
    pub armed: bool,
    resize_repeat_until: Option<Instant>,
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
            resize_repeat_until: None,
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
        Ok(Self {
            key,
            armed: false,
            resize_repeat_until: None,
        })
    }

    pub fn label(&self) -> String {
        self.key.to_string()
    }

    pub fn handle(&mut self, event: KeyEvent) -> PrefixInput {
        self.handle_at(event, Instant::now())
    }

    pub fn cancel(&mut self) {
        self.armed = false;
        self.resize_repeat_until = None;
    }

    fn handle_at(&mut self, event: KeyEvent, now: Instant) -> PrefixInput {
        if event.kind == KeyEventKind::Release {
            return PrefixInput::Pending;
        }

        if self.armed {
            if event.kind == KeyEventKind::Repeat {
                return PrefixInput::Pending;
            }

            self.armed = false;
            let action_modifiers = event.modifiers.is_empty()
                || (event.modifiers == KeyModifiers::SHIFT
                    && matches!(event.code, KeyCode::Char(character) if character.is_ascii_uppercase()));

            return if action_modifiers && event.code != KeyCode::Esc {
                self.record_resize_repeat(event, now);
                PrefixInput::Action(event.code)
            } else {
                self.resize_repeat_until = None;
                PrefixInput::Pending
            };
        }

        if KeyCombination::from(event) == self.key {
            self.armed = true;
            self.resize_repeat_until = None;
            return PrefixInput::Pending;
        }

        let can_repeat_resize = self
            .resize_repeat_until
            .is_some_and(|deadline| now < deadline);

        if can_repeat_resize && is_resize_key(event) {
            self.record_resize_repeat(event, now);
            return PrefixInput::Action(event.code);
        }

        self.resize_repeat_until = None;
        PrefixInput::Pass
    }

    fn record_resize_repeat(&mut self, event: KeyEvent, now: Instant) {
        self.resize_repeat_until = is_resize_key(event).then_some(now + RESIZE_REPEAT_WINDOW);
    }
}

fn is_resize_key(event: KeyEvent) -> bool {
    matches!(event.code, KeyCode::Char('H' | 'J' | 'K' | 'L'))
        && (event.modifiers.is_empty() || event.modifiers == KeyModifiers::SHIFT)
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

    #[test]
    fn resize_repeat_expires_and_cancellation_restores_normal_input() {
        let mut prefix = Prefix::default();
        let now = Instant::now();
        let start = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL);
        let mut resize = KeyEvent::new(KeyCode::Char('L'), KeyModifiers::SHIFT);

        prefix.handle_at(start, now);
        assert!(matches!(
            prefix.handle_at(resize, now),
            PrefixInput::Action(KeyCode::Char('L'))
        ));
        resize.kind = KeyEventKind::Repeat;
        assert!(matches!(
            prefix.handle_at(resize, now),
            PrefixInput::Action(KeyCode::Char('L'))
        ));
        assert!(matches!(
            prefix.handle_at(resize, now + RESIZE_REPEAT_WINDOW),
            PrefixInput::Pass
        ));

        resize.kind = KeyEventKind::Press;
        prefix.handle_at(start, now);
        prefix.handle_at(resize, now);
        prefix.cancel();
        assert!(matches!(prefix.handle_at(resize, now), PrefixInput::Pass));
    }
}
