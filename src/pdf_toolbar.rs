use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::Style,
    widgets::Paragraph,
};
use texo::theme;

use super::{App, workspace_view::pane_content};

// One action row and a gap separate the buttons from the PDF status and version tabs.
pub(super) const TOOLBAR_HEIGHT: u16 = 2;
const BUTTON_GAP: u16 = 1;
const TOOLBAR_LEFT_PADDING: u16 = 1;

impl App {
    fn pdf_building(&self) -> bool {
        if let Some(view) = &self.session.history.view {
            view.job.is_some()
        } else if self.session.proposal_review.is_some() {
            self.session.proposed_job.is_some()
        } else {
            self.session.job.is_some()
        }
    }

    fn compile_label(&self) -> &'static str {
        if self.pdf_building() {
            "Compiling…"
        } else {
            let has_pdf = if let Some(view) = &self.session.history.view {
                view.pdf_path().is_some()
            } else if self.session.proposal_review.is_some() {
                self.session.proposed_build.is_some()
            } else {
                self.session.rendered_pdf.is_some()
            };

            match has_pdf {
                true => "Recompile",
                false => "Compile",
            }
        }
    }

    fn toolbar_labels(&self, width: u16) -> [String; 2] {
        let compile = self.compile_label();
        let prefix = self.prefix.label();
        let full = [
            format!(" {compile}  {prefix} b "),
            format!(" Export  {prefix} e "),
        ];
        let required = full
            .iter()
            .map(|label| label.chars().count())
            .sum::<usize>()
            + usize::from(BUTTON_GAP);

        if required <= usize::from(width.saturating_sub(TOOLBAR_LEFT_PADDING)) {
            full
        } else {
            [format!(" {compile} "), " Export ".into()]
        }
    }

    pub(super) fn pdf_toolbar_buttons(&self, preview_area: Rect) -> [Rect; 2] {
        let content = pane_content(preview_area);
        self.toolbar_layout(content)
    }

    fn toolbar_layout(&self, content: Rect) -> [Rect; 2] {
        let labels = self.toolbar_labels(content.width);
        let [compile, export, _] = Layout::horizontal([
            Constraint::Length(labels[0].chars().count() as u16),
            Constraint::Length(labels[1].chars().count() as u16),
            Constraint::Min(0),
        ])
        .spacing(BUTTON_GAP)
        .areas(Rect {
            x: content.x + TOOLBAR_LEFT_PADDING.min(content.width),
            width: content.width.saturating_sub(TOOLBAR_LEFT_PADDING),
            height: content.height.min(1),
            ..content
        });

        [compile, export]
    }

    pub(super) fn draw_pdf_toolbar(&self, frame: &mut Frame, content: Rect) {
        let buttons = self.toolbar_layout(content);
        let labels = self.toolbar_labels(content.width);
        let compile_enabled = !self.session.empty_workspace && !self.pdf_building();
        let export_enabled = self.ensure_export_ready().is_ok();

        let compile_style = if compile_enabled {
            Style::default()
                .fg(theme::BACKGROUND)
                .bg(theme::GREEN)
                .bold()
        } else {
            theme::muted().bg(theme::BUTTON)
        };
        let export_style = if export_enabled {
            theme::text().bg(theme::KEYCAP).bold()
        } else {
            theme::muted().bg(theme::BUTTON)
        };

        frame.render_widget(
            Paragraph::new(labels[0].as_str()).style(compile_style),
            buttons[0],
        );
        frame.render_widget(
            Paragraph::new(labels[1].as_str()).style(export_style),
            buttons[1],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Focus, test_support::application};
    use ratatui::{
        Terminal,
        backend::TestBackend,
        crossterm::event::{
            Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
        },
    };
    use std::fs;

    const SCREEN: Rect = Rect::new(0, 0, 160, 45);

    #[test]
    fn toolbar_labels_and_clicks_follow_build_state() -> anyhow::Result<()> {
        let (directory, mut app) = application()?;
        assert_eq!(app.compile_label(), "Compile");

        let pdf = directory.path().join("resume.pdf");
        fs::write(&pdf, "compiled PDF")?;
        app.session.rendered_pdf = Some(pdf.clone());
        app.session.rendered_source = Some(app.session.editor.source());
        assert_eq!(app.compile_label(), "Recompile");

        let [compile, export] = app.pdf_toolbar_buttons(app.preview_area(SCREEN));
        assert!(!compile.intersects(export));
        assert!(export.bottom() <= app.preview_body_area(app.preview_area(SCREEN)).y);

        app.handle_event(
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: export.x,
                row: export.y,
                modifiers: KeyModifiers::NONE,
            }),
            SCREEN,
        );
        assert!(app.export.is_some());

        let dialog = app.export.as_mut().unwrap();
        assert!(!dialog.folder.is_selecting());
        assert!(!dialog.input.is_selecting());
        assert_eq!(
            dialog.input.cursor(),
            (0, dialog.input.lines()[0].chars().count())
        );
        dialog.input.select_all();
        app.handle_event(Event::Paste("Alex_Morgan_Backend.pdf".into()), SCREEN);
        app.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            SCREEN,
        );
        assert!(app.export.is_none());
        assert_eq!(
            fs::read(directory.path().join("Alex_Morgan_Backend.pdf"))?,
            fs::read(pdf)?
        );

        app.session.editor.text.insert_str("New source");
        app.open_export();
        assert!(app.export.is_none());
        assert!(app.session.status.contains("Build the current document"));
        Ok(())
    }

    #[test]
    fn toolbar_keeps_actions_visible_in_split_and_narrow_layouts() -> anyhow::Result<()> {
        let (_directory, mut app) = application()?;
        app.focus = Focus::Preview;

        for width in [40, 80, 120, 160] {
            let area = Rect::new(0, 0, width, 35);
            let mut terminal = Terminal::new(TestBackend::new(area.width, area.height))?;
            terminal.draw(|frame| app.draw(frame))?;

            let [compile, export] = app.pdf_toolbar_buttons(app.preview_area(area));
            let row: String = (compile.x..export.right())
                .map(|x| terminal.backend().buffer()[(x, compile.y)].symbol())
                .collect();
            assert!(row.contains("Compile"), "Missing Compile at {width}: {row}");
            assert!(row.contains("Export"), "Missing Export at {width}: {row}");
        }
        Ok(())
    }

    #[test]
    fn nested_entrypoint_exports_to_project_root() -> anyhow::Result<()> {
        let workspace = tempfile::tempdir()?;
        let mut project = crate::resume_project::create(workspace.path(), "Backend")?;
        let nested = project.root.join("source");
        fs::create_dir(&nested)?;
        fs::rename(project.document(), nested.join("main.tex"))?;
        project.manifest.entrypoint = "source/main.tex".into();
        fs::write(
            project.root.join(crate::resume_project::MANIFEST_FILE),
            toml_edit::ser::to_string_pretty(&project.manifest)?,
        )?;

        let mut app = App::with_workspace(
            crate::editor::Editor::open(project.document())?,
            crate::app::Preview::default(),
            workspace.path().to_owned(),
        )?;
        let pdf = nested.join("main.pdf");
        fs::write(&pdf, "compiled PDF")?;
        app.session.rendered_pdf = Some(pdf);
        app.session.rendered_source = Some(app.session.editor.source());
        app.open_export();

        let dialog = app.export.as_ref().unwrap();
        assert_eq!(
            dialog.folder.lines().join(""),
            crate::workspace::display_path(&project.root)
        );
        app.export.as_mut().unwrap().input.select_all();
        app.handle_event(Event::Paste("Application.pdf".into()), SCREEN);
        app.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            SCREEN,
        );
        assert_eq!(
            fs::read_to_string(project.root.join("Application.pdf"))?,
            "compiled PDF"
        );
        assert!(!nested.join("Application.pdf").exists());
        Ok(())
    }
}
