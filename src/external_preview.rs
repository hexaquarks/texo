use std::path::PathBuf;

use anyhow::{Context, Result, ensure};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Margin, Rect},
    style::{Color, Style},
    widgets::{Block, Paragraph, Wrap},
};
use texo::theme;

use super::App;

const BUTTON_HEIGHT: u16 = 1;
const ICON_AND_SPACING_ROWS: u16 = 2;
const NOTICE: &str = "This terminal does not support PDF rendering.";

impl App {
    /// Open the displayed version, never a different PDF left beside the source file.
    fn external_pdf_path(&self) -> Result<PathBuf> {
        let path = if let Some(view) = &self.session.history.view {
            view.pdf_path()
                .context("Build this revision before opening its PDF.")?
        } else if self.session.show_proposed && self.session.proposal_review.is_some() {
            ensure!(
                self.session.proposed_job.is_none() && !self.session.proposed_render_failed,
                "Build the proposed version successfully before opening its PDF."
            );
            &self
                .session
                .proposed_build
                .as_ref()
                .context("Build the proposed version before opening its PDF.")?
                .pdf_path
        } else {
            ensure!(
                self.session.job.is_none() && !self.session.render_failed,
                "Wait for a successful build before opening the PDF."
            );
            ensure!(
                self.session.rendered_source.as_deref()
                    == Some(self.session.editor.source().as_str())
                    && matches!(
                        self.session.editor.disk_state(),
                        Ok(crate::editor::DiskState::Unchanged)
                    ),
                "Build the current document before opening its PDF."
            );
            self.session
                .rendered_pdf
                .as_deref()
                .context("Build the document before opening its PDF.")?
        };

        ensure!(path.is_file(), "The PDF is missing. Build it again.");
        // An absolute filesystem path cannot be interpreted as a URL or viewer option.
        path.canonicalize().context("Could not locate the PDF")
    }

    pub(super) fn open_pdf_externally(&mut self) {
        let result = self.external_pdf_path();

        #[cfg(not(test))]
        let result = result.and_then(|path| {
            open::that_detached(&path).context("Could not open the default PDF viewer")?;
            Ok(path)
        });

        self.session.status = match result {
            Ok(_) => "Opening PDF in the default viewer…".into(),
            Err(error) => error.to_string(),
        };
    }

    pub(super) fn external_pdf_button(&mut self, preview_area: Rect) -> Rect {
        if !self.active_preview().uses_basic_rendering() || self.external_pdf_path().is_err() {
            return Rect::default();
        }
        let body = self.preview_body_area(preview_area);
        Rect {
            height: body.height.min(BUTTON_HEIGHT),
            ..body
        }
    }

    pub(super) fn draw_external_preview(&mut self, frame: &mut Frame, area: Rect) {
        let [button, body] =
            Layout::vertical([Constraint::Length(BUTTON_HEIGHT), Constraint::Min(0)]).areas(area);
        let available = self.external_pdf_path().is_ok();
        let label = format!("{} o  Open PDF", self.prefix.label());
        let label = if label.len() <= usize::from(button.width) {
            label.as_str()
        } else if button.width >= "Open PDF".len() as u16 {
            "Open PDF"
        } else {
            "Open"
        };
        let style = if available {
            Style::default().fg(theme::BACKGROUND).bg(theme::ACCENT)
        } else {
            theme::muted().bg(theme::BUTTON)
        };
        frame.render_widget(Paragraph::new(label).centered().style(style), button);

        let page = self
            .active_preview()
            .fitted_page_area(body.inner(Margin::new(1, 1)));
        let paper = Style::default().bg(Color::White).fg(Color::Black);
        frame.render_widget(Block::default().style(paper), page);

        let content = page.inner(Margin::new(1, 0));
        let text_rows = textwrap::wrap(NOTICE, usize::from(content.width.max(1))).len() as u16;
        let [_, message, _] = Layout::vertical([
            Constraint::Fill(1),
            Constraint::Length(text_rows + ICON_AND_SPACING_ROWS),
            Constraint::Fill(1),
        ])
        .areas(content);
        frame.render_widget(
            Paragraph::new(format!("×\n\n{NOTICE}"))
                .centered()
                .wrap(Wrap { trim: true })
                .style(paper),
            message,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Focus, test_support::application};
    use crate::{assistant::ProposedEdit, proposal::ProposalReview};
    use ratatui::{
        Terminal,
        backend::TestBackend,
        crossterm::event::{
            Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
        },
    };

    #[test]
    fn external_pdf_requires_the_selected_current_successful_build() -> Result<()> {
        let (directory, mut app) = application()?;
        assert!(app.external_pdf_path().is_err());

        let pdf = directory.path().join("resume.pdf");
        std::fs::write(&pdf, b"test PDF")?;
        app.session.rendered_source = Some(app.session.editor.source());
        app.session.rendered_pdf = Some(pdf.clone());
        assert_eq!(app.external_pdf_path()?, pdf.canonicalize()?);

        app.session.render_failed = true;
        assert!(app.external_pdf_path().is_err());
        app.session.render_failed = false;
        app.session.editor.text.insert_str("new text");
        assert!(app.external_pdf_path().is_err());
        app.session.rendered_source = Some(app.session.editor.source());

        let edit = ProposedEdit {
            source_before: app.session.editor.source(),
            source_after: "proposal".into(),
        };
        app.session.proposal_review = Some(ProposalReview::new(&edit));
        app.session.show_proposed = true;
        assert!(
            app.external_pdf_path().is_err(),
            "Never open the current PDF for a missing proposal"
        );
        app.session.show_proposed = false;
        assert_eq!(app.external_pdf_path()?, pdf.canonicalize()?);

        std::fs::remove_file(pdf)?;
        assert!(app.external_pdf_path().is_err());
        Ok(())
    }

    #[test]
    #[ignore = "requires Tectonic cache and Poppler"]
    fn external_pdf_selects_proposal_and_history_instead_of_current() -> Result<()> {
        use std::{
            sync::atomic::AtomicBool,
            time::{Duration, Instant},
        };
        const BUILD_TIMEOUT: Duration = Duration::from_secs(30);
        const POLL_INTERVAL: Duration = Duration::from_millis(10);

        let (directory, mut app) = application()?;
        let current = directory.path().join("current.pdf");
        std::fs::write(&current, b"current PDF")?;
        let source = r"\documentclass{article}\begin{document}Historical version\end{document}";
        app.session.editor.replace_source(source);
        let area = Rect::new(0, 0, 160, 45);
        app.handle_prefix_action(KeyCode::Char('w'), area);
        app.session.rendered_source = Some(source.into());
        app.session.rendered_pdf = Some(current.clone());

        let proposed = source.replace("Historical", "Proposed");
        app.session.proposal_review = Some(ProposalReview::new(&ProposedEdit {
            source_before: source.into(),
            source_after: proposed.clone(),
        }));
        let build = texo::sandbox::build(&proposed, directory.path(), &AtomicBool::new(false))?;
        let proposed_pdf = build.pdf_path.canonicalize()?;
        app.session.proposed_build = Some(build);
        app.session.show_proposed = true;
        assert_eq!(app.external_pdf_path()?, proposed_pdf);
        assert_ne!(app.external_pdf_path()?, current.canonicalize()?);

        app.clear_proposal();
        app.open_history();
        app.handle_history_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_history_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.external_pdf_path().is_err());
        let deadline = Instant::now() + BUILD_TIMEOUT;
        while app.history_preview_loading() {
            ensure!(Instant::now() < deadline, "Historical PDF build timed out");
            app.receive_history_preview();
            std::thread::sleep(POLL_INTERVAL);
        }
        let historical = app
            .session
            .history
            .view
            .as_ref()
            .unwrap()
            .pdf_path()
            .context("Historical PDF was not built")?
            .canonicalize()?;
        assert_eq!(app.external_pdf_path()?, historical);
        assert_ne!(historical, current.canonicalize()?);

        app.session.history.view.as_mut().unwrap().preview = texo::preview::Preview::default();
        let preview = app.preview_area(area);
        let button = app.external_pdf_button(preview);
        assert_eq!(
            button.y,
            preview.y + 3 + super::super::pdf_toolbar::TOOLBAR_HEIGHT,
            "Border, toolbar, and the two history header rows"
        );
        app.handle_event(
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: button.x + 1,
                row: button.y,
                modifiers: KeyModifiers::NONE,
            }),
            area,
        );
        assert_eq!(app.session.status, "Opening PDF in the default viewer…");
        Ok(())
    }

    #[test]
    fn placeholder_and_button_fit_wide_narrow_and_tiny_panes() -> Result<()> {
        let (_directory, mut app) = application()?;
        app.session.preview = texo::preview::Preview::default();
        for (width, height) in [(60, 30), (24, 20), (8, 6), (1, 1), (0, 0)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height))?;
            terminal.draw(|frame| app.draw_external_preview(frame, frame.area()))?;
            let buffer = terminal.backend().buffer();
            let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
            if width >= 24 {
                assert!(text.contains("Open PDF"));
                assert!(text.contains('×'));
                assert!(
                    text.split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                        .contains(NOTICE)
                );
                assert!(buffer.content.iter().any(|cell| cell.bg == Color::White));
            }
        }
        Ok(())
    }

    #[test]
    fn mouse_and_prefix_open_the_pdf_without_changing_the_document() -> Result<()> {
        let (directory, mut app) = application()?;
        app.session.preview = texo::preview::Preview::default();
        let source = app.session.editor.source();
        let pdf = directory.path().join("resume.pdf");
        std::fs::write(&pdf, b"test PDF")?;
        app.session.rendered_source = Some(source.clone());
        app.session.rendered_pdf = Some(pdf);
        let area = Rect::new(0, 0, 160, 45);
        let preview = app.preview_area(area);
        let button = app.external_pdf_button(preview);

        app.handle_event(
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: button.x + 1,
                row: button.y,
                modifiers: KeyModifiers::NONE,
            }),
            area,
        );
        assert!(app.focus == Focus::Preview);
        assert_eq!(app.session.status, "Opening PDF in the default viewer…");
        app.session.status.clear();

        app.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL)),
            area,
        );
        app.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE)),
            area,
        );
        assert_eq!(app.session.status, "Opening PDF in the default viewer…");
        assert_eq!(app.session.editor.source(), source);
        Ok(())
    }
}
