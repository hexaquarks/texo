use std::{
    io::{self, Write},
    num::NonZeroU32,
    sync::{Arc, LazyLock, Mutex},
};

use image::DynamicImage;
use kitty_graphics_protocol::{Action, Command, CursorPolicy, DeleteTarget, ESC};
use ratatui::{
    buffer::Buffer,
    layout::{Position, Rect, Size},
    widgets::Widget,
};
use ratatui_image::{
    Image,
    picker::{Capability, Picker, cap_parser::Parser},
    protocol::{Protocol, kitty::Kitty},
};

// Kitty's q=2 suppresses replies, which must never enter the application's key input.
const NO_PROTOCOL_REPLIES: u8 = 2;
// Standard Kitty Unicode placeholder.
const IMAGE_PLACEHOLDER: char = '\u{10eeee}';
// Standard row/column encoding: https://sw.kovidgoyal.net/kitty/graphics-protocol/#unicode-placeholders
// ratatui-image supplies image uploads and identity, but exposes no horizontal viewport offset.
static COORDINATE_MARKS: LazyLock<Vec<char>> = LazyLock::new(|| {
    include_str!("kitty_diacritics.txt")
        .trim_end()
        .chars()
        .collect()
});

#[derive(Clone, Default)]
pub(crate) struct TerminalImages(Arc<Mutex<Vec<RetiredImage>>>);

struct RetiredImage {
    id: NonZeroU32,
    tmux: bool,
}

impl TerminalImages {
    pub(crate) fn flush_retired(&self, writer: &mut impl Write) -> io::Result<()> {
        let retired = std::mem::take(
            &mut *self
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );

        for image in retired {
            let command = delete(image.id, true);
            writer.write_all(command_wire(&command, image.tmux)?.as_bytes())?;
        }

        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct ImageView {
    pub(crate) page_size: Size,
    pub(crate) offset: Position,
    pub(crate) area: Rect,
}

impl ImageView {
    pub(crate) fn supports_placeholders(self) -> bool {
        usize::from(self.offset.x) < COORDINATE_MARKS.len()
            && usize::from(self.offset.y) + usize::from(self.area.height) <= COORDINATE_MARKS.len()
    }
}

pub(crate) struct RetainedImage {
    image: Protocol,
    id: NonZeroU32,
    tmux: bool,
    view: Option<ImageView>,
    uploaded: bool,
    terminal: TerminalImages,
}

impl RetainedImage {
    pub(crate) fn new(
        picker: &Picker,
        source: DynamicImage,
        terminal: TerminalImages,
    ) -> anyhow::Result<Self> {
        let id = rand::random::<NonZeroU32>();
        // This is a placeholder renderer's maximum extent, not a pixel allocation.
        // The placement command supplies the actual viewport size on every view change.
        let extent = Size::new(u16::MAX, u16::MAX);
        let image = Kitty::new(
            source,
            extent,
            id.get(),
            picker.tmux_detected(),
            picker
                .capabilities()
                .contains(&Capability::KittyCompression),
        )?;

        Ok(Self {
            image: Protocol::Kitty(image),
            id,
            tmux: picker.tmux_detected(),
            view: None,
            uploaded: false,
            terminal,
        })
    }

    pub(crate) fn last_view(&self) -> Option<ImageView> {
        self.view
    }

    pub(crate) fn render(&mut self, view: ImageView, buffer: &mut Buffer) -> io::Result<()> {
        if view.area.is_empty() {
            return Ok(());
        }

        if !view.supports_placeholders() {
            return Err(io::Error::other(
                "The page viewport exceeds Kitty's coordinate range",
            ));
        }

        let mut commands = String::new();
        if self.view != Some(view) {
            // Use the default virtual placement, which is what the library's placeholders
            // reference. Delete its old geometry without deleting the uploaded pixels.
            commands.push_str(&command_wire(&delete(self.id, false), self.tmux)?);
            let command = Command::builder()
                .action(Action::Place)
                .image_id(self.id.get())
                .quiet(NO_PROTOCOL_REPLIES)
                .display_area(
                    u32::from(view.page_size.width),
                    u32::from(view.page_size.height),
                )
                .unicode_placeholder(view.page_size.width, view.page_size.height)
                .cursor_policy(CursorPolicy::NoMove)
                .build();
            commands.push_str(&command_wire(&command, self.tmux)?);
        }

        // Let ratatui-image generate the upload, foreground color, and image-ID mark.
        // Supply page-relative row/column marks ourselves: Ghostty ignores source_rect
        // on virtual placements, so zoom uses the whole-page grid and pan uses these marks.
        let cell_area = Rect::new(0, 0, 1, 1);
        let mut identity = Buffer::empty(cell_area);
        Image::new(&self.image)
            .allow_clipping(true)
            .render(cell_area, &mut identity);
        let identity_cell = &identity[(0, 0)];
        let (upload, metadata) = identity_cell
            .symbol()
            .split_once(IMAGE_PLACEHOLDER)
            .ok_or_else(|| io::Error::other("Missing Kitty image identity"))?;
        let image_mark = metadata
            .chars()
            .nth(2)
            .ok_or_else(|| io::Error::other("Missing Kitty image-ID mark"))?;
        let mut template = identity_cell.clone();
        // The upload can be megabytes. Copy only cell styling/width into each placeholder.
        template.set_symbol("");

        for y in 0..view.area.height {
            let row_mark = COORDINATE_MARKS[usize::from(view.offset.y + y)];
            for x in 0..view.area.width {
                let column = usize::from(view.offset.x) + usize::from(x);
                let symbol = if let Some(column_mark) = COORDINATE_MARKS.get(column) {
                    format!("{IMAGE_PLACEHOLDER}{row_mark}{column_mark}{image_mark}")
                } else {
                    // Kitty continues columns beyond the table by inheriting the previous
                    // cell's column and image ID. The first visible column is always explicit.
                    format!("{IMAGE_PLACEHOLDER}{row_mark}")
                };
                let cell = &mut buffer[(view.area.x + x, view.area.y + y)];
                cell.clone_from(&template);
                cell.set_symbol(&symbol);
            }
        }

        let first = &mut buffer[view.area.as_position()];
        first.set_symbol(&format!("{upload}{commands}{}", first.symbol()));

        self.view = Some(view);
        self.uploaded = true;
        Ok(())
    }
}

impl Drop for RetainedImage {
    fn drop(&mut self) {
        if self.uploaded {
            self.terminal
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(RetiredImage {
                    id: self.id,
                    tmux: self.tmux,
                });
        }
    }
}

fn delete(id: NonZeroU32, free_data: bool) -> Command {
    Command::builder()
        .action(Action::Delete)
        .image_id(id.get())
        .delete_target(DeleteTarget::ById { free_data })
        .quiet(NO_PROTOCOL_REPLIES)
        .build()
}

fn command_wire(command: &Command, tmux: bool) -> io::Result<String> {
    let sequence = command.serialize(&[]).map_err(io::Error::other)?;
    // ratatui-image owns upload chunking; these small placement/delete commands
    // use its public tmux framing rules to reach the same outer terminal.
    let (start, escape, end) = Parser::tmux_start_escape_end(tmux);
    Ok(format!(
        "{start}{}{end}",
        sequence.replace(char::from(ESC), escape)
    ))
}

#[cfg(test)]
#[path = "retained_image_tests.rs"]
mod tests;
