use anyhow::Result;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Margin, Rect},
    style::Style,
    widgets::{Block, Paragraph, Wrap},
};
use ratatui_image::{
    StatefulImage,
    picker::{Picker, ProtocolType},
    protocol::StatefulProtocol,
};

use crate::pdf::PdfPages;
use crate::theme;

const EMPTY_STATE_HEIGHT: u16 = 7;
const EMPTY_NOTICE: &str =
    "Your PDF will appear here.\n\nBuild your document to update the preview.\n\nCtrl+R  to build";
const UNSUPPORTED_NOTICE: &str = "Preview needs a graphics-capable terminal.\n\nLaunch texo in its dedicated window to see your PDF here.";

pub struct Preview {
    picker: Option<Picker>,
    notice: String,
    pages: Option<PdfPages>,
    page_index: usize,
    image: Option<StatefulProtocol>,
}

impl Default for Preview {
    fn default() -> Self {
        Self {
            picker: None,
            notice: UNSUPPORTED_NOTICE.into(),
            pages: None,
            page_index: 0,
            image: None,
        }
    }
}

impl Preview {
    /// Query graphics support after entering the alternate screen, before reading input events.
    pub fn detect() -> Self {
        match Picker::from_query_stdio() {
            Ok(picker) if picker.protocol_type() != ProtocolType::Halfblocks => Self {
                picker: Some(picker),
                notice: EMPTY_NOTICE.into(),
                ..Self::default()
            },
            _ => Self::default(),
        }
    }

    pub fn clear(&mut self) {
        self.pages = None;
        self.image = None;
        self.page_index = 0;
        self.notice = if self.picker.is_some() {
            EMPTY_NOTICE.into()
        } else {
            UNSUPPORTED_NOTICE.into()
        };
    }

    pub fn set_pages(&mut self, pages: PdfPages) -> Result<()> {
        let index = self.page_index.min(pages.page_count() - 1);
        let image = pages.load_page(index)?;
        self.image = self
            .picker
            .as_ref()
            .map(|picker| picker.new_resize_protocol(image));
        self.pages = Some(pages);
        self.page_index = index;
        Ok(())
    }

    pub fn next_page(&mut self) -> Result<()> {
        if let Some(pages) = &self.pages
            && self.page_index + 1 < pages.page_count()
        {
            self.show_page(self.page_index + 1)?;
        }
        Ok(())
    }

    pub fn previous_page(&mut self) -> Result<()> {
        if self.page_index > 0 {
            self.show_page(self.page_index - 1)?;
        }
        Ok(())
    }

    fn show_page(&mut self, index: usize) -> Result<()> {
        if let Some(pages) = &self.pages {
            let image = pages.load_page(index)?;
            self.image = self
                .picker
                .as_ref()
                .map(|picker| picker.new_resize_protocol(image));
            self.page_index = index;
        }
        Ok(())
    }

    pub fn page_status(&self) -> Option<String> {
        self.pages
            .as_ref()
            .map(|pages| format!("{} / {}", self.page_index + 1, pages.page_count()))
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect) {
        frame.render_widget(
            Block::default().style(Style::default().bg(theme::PREVIEW)),
            area,
        );
        let content = area.inner(Margin::new(1, 0));

        match &mut self.image {
            Some(image) => {
                frame.render_stateful_widget(StatefulImage::default(), content, image);
                if let Some(Err(error)) = image.last_encoding_result() {
                    self.notice = format!("Could not display the preview: {error}");
                    self.image = None;
                }
            }
            None => {
                let [_, message, _] = Layout::vertical([
                    Constraint::Fill(1),
                    Constraint::Length(EMPTY_STATE_HEIGHT),
                    Constraint::Fill(1),
                ])
                .areas(content);
                frame.render_widget(
                    Paragraph::new(self.notice.as_str())
                        .centered()
                        .style(theme::muted())
                        .wrap(Wrap { trim: false }),
                    message,
                );
            }
        }
    }
}
