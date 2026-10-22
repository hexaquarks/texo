use anyhow::Result;
use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Block, Paragraph, Wrap},
};
use ratatui_image::{
    StatefulImage,
    picker::{Picker, ProtocolType},
    protocol::StatefulProtocol,
};

use crate::pdf::PdfPages;

pub struct Preview {
    picker: Option<Picker>,
    notice: String,
    pages: Option<PdfPages>,
    page_index: usize,
    image: Option<StatefulProtocol>,
}

impl Preview {
    /// Query graphics support after entering the alternate screen, before reading input events.
    pub fn detect() -> Self {
        let (picker, notice) = match Picker::from_query_stdio() {
            Ok(picker) if picker.protocol_type() != ProtocolType::Halfblocks => (
                Some(picker),
                String::from("Render your TeX document to preview it here."),
            ),
            _ => (
                None,
                String::from(
                    "PDF preview needs a terminal with Kitty, iTerm2, or Sixel graphics support.",
                ),
            ),
        };
        Self {
            picker,
            notice,
            pages: None,
            page_index: 0,
            image: None,
        }
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

    pub fn draw(&mut self, frame: &mut Frame, area: Rect) {
        let title = match &self.pages {
            Some(pages) => format!(
                " PDF · page {}/{} · Alt+Left/Right ",
                self.page_index + 1,
                pages.page_count()
            ),
            None => String::from(" PDF preview "),
        };
        let border = Block::bordered().title(title);
        let content = border.inner(area);
        frame.render_widget(border, area);

        match &mut self.image {
            Some(image) => frame.render_stateful_widget(StatefulImage::default(), content, image),
            None => frame.render_widget(
                Paragraph::new(self.notice.as_str()).wrap(Wrap { trim: false }),
                content,
            ),
        }
    }
}
