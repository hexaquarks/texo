use std::io::{self, Write};

use ratatui::{
    DefaultTerminal,
    crossterm::{
        SynchronizedUpdate,
        cursor::SetCursorStyle,
        event::{self, Event},
        execute,
    },
};

use super::{App, Duration, EVENT_POLL_INTERVAL};

// Bound input work so a continuous trackpad stream cannot starve drawing or job completion.
const MAX_INPUT_EVENTS_PER_FRAME: usize = 256;
const IMAGE_POLL_INTERVAL: Duration = Duration::from_millis(16);
const IDLE_POLL_INTERVAL: Duration = Duration::from_secs(1);

impl App {
    pub(super) fn run_event_loop(&mut self, terminal: &mut DefaultTerminal) -> anyhow::Result<()> {
        let mut repaint = false;
        let mut cursor_style = SetCursorStyle::DefaultUserShape;

        loop {
            let area = terminal.get_frame().area();
            let previous_preview_area = self.preview_area(area);
            let was_proposed_loading = self.session.proposed_job.is_some();
            self.receive_render();
            self.receive_assistant();
            self.receive_proposed_render();
            repaint |= (was_proposed_loading != self.session.proposed_job.is_some()
                || previous_preview_area != self.preview_area(area))
                && self.active_preview().uses_inline_graphics();

            // Flush graphics cleanup and the replacement frame as one synchronized update.
            std::io::stdout().sync_update(|_| -> io::Result<()> {
                self.session
                    .preview
                    .flush_retired_images(terminal.backend_mut())?;
                if repaint {
                    terminal.clear()?;
                }
                let next_cursor_style = self.terminal_cursor_style();
                if next_cursor_style != cursor_style {
                    execute!(terminal.backend_mut(), next_cursor_style)?;
                    cursor_style = next_cursor_style;
                }

                terminal.draw(|frame| self.draw(frame))?;
                Ok(())
            })??;
            repaint = false;

            if !event::poll(self.input_poll_interval())? {
                continue;
            }

            let area = terminal.get_frame().area();
            let previous_preview_area = self.preview_area(area);
            let previous_version = self.session.show_proposed;
            let previous_document = self.session.editor.path.clone();
            let projects_was_visible = self.projects_visible;
            let overlay_was_open = self.overlay_open();
            let review_was_open = self.session.proposal_review.is_some();
            let preview_was_loading = self.session.job.is_some();
            let proposed_was_loading = self.session.proposed_job.is_some();

            let quit = drain_ready_events(
                event::read()?,
                || {
                    if event::poll(Duration::ZERO)? {
                        event::read().map(Some)
                    } else {
                        Ok(None)
                    }
                },
                |event| self.handle_event(event, area),
            )?;
            if quit {
                return Ok(());
            }

            // Kitty placeholders participate in ordinary cell diffs. Only inline images
            // need a screen clear to remove pixels formerly covered by another placement.
            repaint = self.overlays_require_clear(overlay_was_open)
                || (self.active_preview().uses_inline_graphics()
                    && (previous_version != self.session.show_proposed
                        || review_was_open != self.session.proposal_review.is_some()
                        || proposed_was_loading != self.session.proposed_job.is_some()
                        || previous_preview_area != self.preview_area(area)
                        || projects_was_visible != self.projects_visible
                        || previous_document != self.session.editor.path
                        || preview_was_loading != self.session.job.is_some()));
        }
    }

    pub(super) fn terminal_cursor_style(&self) -> SetCursorStyle {
        if self.editor_has_focus() {
            self.session.editor.terminal_cursor_style()
        } else {
            SetCursorStyle::DefaultUserShape
        }
    }

    fn input_poll_interval(&mut self) -> Duration {
        let preparing_image = !self.overlay_open() && self.active_preview().is_refreshing();
        let running_job = self.session.job.is_some()
            || self.session.proposed_job.is_some()
            || self.session.assistant.is_running()
            || self.session.assistant.login_open();

        poll_interval(preparing_image, running_job)
    }

    pub(super) fn release_terminal_images(
        &mut self,
        writer: &mut impl Write,
    ) -> anyhow::Result<()> {
        self.sessions.clear();
        self.session.preview.clear();
        self.session.proposed_preview.clear();
        self.session.preview.flush_retired_images(writer)?;
        writer.flush()?;
        Ok(())
    }
}

fn poll_interval(preparing_image: bool, running_job: bool) -> Duration {
    if preparing_image {
        IMAGE_POLL_INTERVAL
    } else if running_job {
        EVENT_POLL_INTERVAL
    } else {
        IDLE_POLL_INTERVAL
    }
}

// Process events already waiting in the input queue before drawing. Never wait to fill
// a batch, and stop after resize so subsequent pointer events use the new frame geometry.
fn drain_ready_events(
    first: Event,
    mut next: impl FnMut() -> io::Result<Option<Event>>,
    mut handle: impl FnMut(Event) -> bool,
) -> io::Result<bool> {
    let mut current = first;

    for index in 0..MAX_INPUT_EVENTS_PER_FRAME {
        let resized = matches!(current, Event::Resize(_, _));
        if handle(current) {
            return Ok(true);
        }
        if resized || index + 1 == MAX_INPUT_EVENTS_PER_FRAME {
            break;
        }

        match next()? {
            Some(event) => current = event,
            None => break,
        }
    }

    Ok(false)
}

#[cfg(test)]
#[path = "event_loop_tests.rs"]
mod tests;
