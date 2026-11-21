use std::sync::mpsc::{Receiver, TryRecvError};

use anyhow::{Result, anyhow};
use ratatui::{
    Frame,
    crossterm::event::{Event, KeyCode, MouseButton, MouseEventKind},
    layout::{Constraint, Layout, Margin, Position, Rect},
    style::Modifier,
    text::Line,
    widgets::{List, ListItem, ListState, Paragraph, Wrap},
};

use texo::{
    preview::{PageThumbnail, Preview},
    theme,
};

use crate::{
    dialog_ui::{ButtonFocus, DialogAction, FormLayout, buttons},
    resume_templates::{ResumeTemplate, TEMPLATES},
};

const ACTION: &str = "Choose template";
const BODY_ROWS: u16 = 26;
const TEMPLATE_LIST_WIDTH: u16 = 23;
const SPLIT_MIN_WIDTH: u16 = 48;
const STACKED_LIST_ROWS: u16 = 4;
const MIN_THUMBNAIL_ROWS: u16 = 3;
const BLANK_ROW: usize = 1;
const FIRST_TEMPLATE_ROW: usize = 4;

pub struct TemplateGallery {
    selection: ListState,
    preview_area: Option<Rect>,
    thumbnails: Vec<Option<Result<PageThumbnail>>>,
    pending: Option<ThumbnailJob>,
}

struct ThumbnailJob {
    template_index: usize,
    preview_area: Rect,
    receiver: Receiver<Result<PageThumbnail>>,
}

impl Default for TemplateGallery {
    fn default() -> Self {
        Self {
            selection: ListState::default().with_selected(Some(BLANK_ROW)),
            preview_area: None,
            thumbnails: (0..TEMPLATES.len()).map(|_| None).collect(),
            pending: None,
        }
    }
}

impl TemplateGallery {
    pub fn selected(&self) -> &'static ResumeTemplate {
        &TEMPLATES[self.selected_index()]
    }

    fn selected_index(&self) -> usize {
        self.selection
            .selected()
            .and_then(template_index_at_row)
            .unwrap_or_default()
    }

    fn select_template(&mut self, index: usize) {
        let row = if index == 0 {
            *self.selection.offset_mut() = 0;
            BLANK_ROW
        } else {
            FIRST_TEMPLATE_ROW + index - 1
        };
        self.selection.select(Some(row));
    }

    fn layout(screen: Rect) -> FormLayout {
        FormLayout::new(screen, BODY_ROWS, 0, ACTION)
    }

    fn content_areas(body: Rect) -> (Rect, Rect) {
        if body.width >= SPLIT_MIN_WIDTH {
            let body = body.inner(Margin::new(0, 1));
            let [list, _, preview] = Layout::horizontal([
                Constraint::Length(TEMPLATE_LIST_WIDTH),
                Constraint::Length(3),
                Constraint::Min(0),
            ])
            .areas(body);
            (list, preview)
        } else {
            let [list, _, preview] = Layout::vertical([
                Constraint::Length(STACKED_LIST_ROWS.min(body.height)),
                Constraint::Length(1),
                Constraint::Min(0),
            ])
            .areas(body);
            (list, preview)
        }
    }

    pub fn handle_event(&mut self, event: Event, screen: Rect) -> Option<DialogAction> {
        match event {
            Event::Key(key) => match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    self.select_template(self.selected_index().saturating_sub(1));
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    let next = (self.selected_index() + 1).min(TEMPLATES.len() - 1);
                    self.select_template(next);
                }
                KeyCode::Home => self.select_template(0),
                KeyCode::End => self.select_template(TEMPLATES.len() - 1),
                KeyCode::Enter => return Some(DialogAction::Submit),
                KeyCode::Esc => return Some(DialogAction::Cancel),
                _ => {}
            },
            Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left) => {
                let position = Position::new(mouse.column, mouse.row);
                let layout = Self::layout(screen);
                let [cancel, choose] = layout.buttons;

                if cancel.contains(position) {
                    return Some(DialogAction::Cancel);
                }
                if choose.contains(position) {
                    return Some(DialogAction::Submit);
                }

                let (list, _) = Self::content_areas(layout.body);
                if list.contains(position) {
                    let row = self.selection.offset() + usize::from(position.y - list.y);
                    if let Some(index) = template_index_at_row(row) {
                        self.select_template(index);
                    }
                }
            }
            _ => {}
        }

        None
    }

    pub fn draw(&mut self, frame: &mut Frame, document_preview: &Preview) {
        let layout = Self::layout(frame.area());
        layout.draw_with_help(
            frame,
            "New resume · Templates",
            &[("↑ ↓", "Browse"), ("Enter", "Choose"), ("Esc", "Cancel")],
        );

        let (list, preview) = Self::content_areas(layout.body);
        let [blank, templates @ ..] = TEMPLATES else {
            return;
        };
        let heading = theme::muted().add_modifier(Modifier::BOLD);
        let mut items = vec![
            ListItem::new("Start from scratch").style(heading),
            ListItem::new(blank.name),
            ListItem::new(""),
            ListItem::new("Templates").style(heading),
        ];
        items.extend(
            templates
                .iter()
                .map(|template| ListItem::new(template.name)),
        );

        // After widening the dialog, bring earlier rows back into the available space.
        let last_offset = items.len().saturating_sub(usize::from(list.height));
        *self.selection.offset_mut() = self.selection.offset().min(last_offset);

        frame.render_stateful_widget(
            List::new(items).highlight_symbol("› ").highlight_style(
                theme::dialog_title()
                    .bg(theme::SELECTION)
                    .add_modifier(Modifier::BOLD),
            ),
            list,
            &mut self.selection,
        );
        self.draw_preview(frame, preview, document_preview);
        buttons(frame, layout.buttons, ACTION, ButtonFocus::Action, false);
    }

    fn draw_preview(&mut self, frame: &mut Frame, area: Rect, preview: &Preview) {
        self.prepare_thumbnails(area, preview);

        let template = self.selected();
        let title = Line::styled(
            format!(
                "PDF preview · {}/{}",
                self.selected_index() + 1,
                TEMPLATES.len()
            ),
            theme::muted(),
        );
        let [title_area, description, image_area] = preview_areas(template, area);
        frame.render_widget(Paragraph::new(title), title_area);
        frame.render_widget(
            Paragraph::new(template.description).wrap(Wrap { trim: true }),
            description,
        );

        if let Some(image_area) = thumbnail_area(image_area) {
            self.draw_thumbnail(frame, image_area);
        }
    }

    fn draw_thumbnail(&mut self, frame: &mut Frame, area: Rect) {
        let index = self.selected_index();
        let message = match self.thumbnails[index].as_mut() {
            Some(Ok(image)) => {
                if image.draw(frame, area).is_ok() {
                    return;
                }

                "Preview unavailable"
            }
            Some(Err(_)) => "Preview unavailable",
            None => "Loading preview…",
        };
        frame.render_widget(
            Paragraph::new(message).centered().style(theme::muted()),
            area,
        );
    }

    fn prepare_thumbnails(&mut self, area: Rect, preview: &Preview) {
        if self.preview_area != Some(area) {
            self.thumbnails.fill_with(|| None);
            self.preview_area = Some(area);
        }

        self.receive_thumbnail();
        if self.pending.is_some() {
            return;
        }

        let Some((template_index, image_area)) = self.next_thumbnail(area) else {
            return;
        };

        let png = TEMPLATES[template_index].thumbnail;
        match preview.prepare_page_thumbnail(png, image_area) {
            Ok(receiver) => {
                self.pending = Some(ThumbnailJob {
                    template_index,
                    preview_area: area,
                    receiver,
                });
            }
            Err(error) => self.thumbnails[template_index] = Some(Err(error)),
        }
    }

    fn next_thumbnail(&self, area: Rect) -> Option<(usize, Rect)> {
        let selected = self.selected_index();
        TEMPLATES
            .iter()
            .enumerate()
            .filter(|(index, _)| self.thumbnails[*index].is_none())
            .filter_map(|(index, template)| {
                let [_, _, image_area] = preview_areas(template, area);
                thumbnail_area(image_area).map(|area| (index, area))
            })
            .min_by_key(|(index, _)| index.abs_diff(selected))
    }

    fn receive_thumbnail(&mut self) {
        let Some(job) = &self.pending else {
            return;
        };
        let result = match job.receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(anyhow!("Gallery preview worker stopped")),
        };

        // Background previews are useful after selection changes, but not after resizing.
        if self.preview_area == Some(job.preview_area) {
            self.thumbnails[job.template_index] = Some(result);
        }
        self.pending = None;
    }

    pub fn is_preparing_thumbnail(&self) -> bool {
        self.pending.is_some()
    }
}

fn preview_areas(template: &ResumeTemplate, area: Rect) -> [Rect; 3] {
    let description_rows =
        textwrap::wrap(template.description, usize::from(area.width.max(1))).len();
    Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(description_rows as u16),
        Constraint::Min(0),
    ])
    .areas(area)
}

fn thumbnail_area(area: Rect) -> Option<Rect> {
    (area.height >= MIN_THUMBNAIL_ROWS && area.width > 0).then(|| area.inner(Margin::new(0, 1)))
}

fn template_index_at_row(row: usize) -> Option<usize> {
    if row == BLANK_ROW {
        return Some(0);
    }

    let index = row.checked_sub(FIRST_TEMPLATE_ROW)? + 1;
    (index < TEMPLATES.len()).then_some(index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resized_gallery_discards_old_previews() -> Result<()> {
        let old_area = Rect::new(0, 0, 30, 20);
        let new_area = Rect::new(0, 0, 40, 20);
        let (sender, receiver) = std::sync::mpsc::channel();
        let mut gallery = TemplateGallery {
            preview_area: Some(new_area),
            pending: Some(ThumbnailJob {
                template_index: 0,
                preview_area: old_area,
                receiver,
            }),
            ..TemplateGallery::default()
        };

        sender.send(Err(anyhow!("From the earlier size")))?;
        gallery.receive_thumbnail();

        assert!(gallery.pending.is_none());
        assert!(gallery.thumbnails[0].is_none());
        Ok(())
    }
}
