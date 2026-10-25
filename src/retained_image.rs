use std::{
    io::{self, Write},
    num::NonZeroU32,
    sync::{Arc, Mutex},
};

use image::DynamicImage;
use kitty_graphics_protocol::{Action, Command, CursorPolicy, DeleteTarget, ESC};
use ratatui::{
    buffer::Buffer,
    layout::{Rect, Size},
    widgets::Widget,
};
use ratatui_image::{
    Image,
    picker::{Capability, Picker, cap_parser::Parser},
    protocol::{Protocol, kitty::Kitty},
};

// Kitty's q=2 suppresses replies, which must never enter the application's key input.
const NO_PROTOCOL_REPLIES: u8 = 2;
// Standard Kitty Unicode placeholder; ratatui-image encodes its row/column metadata.
const IMAGE_PLACEHOLDER: char = '\u{10eeee}';

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
    pub(crate) source: image::math::Rect,
    pub(crate) area: Rect,
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

    pub(crate) fn render(&mut self, view: ImageView, buffer: &mut Buffer) -> io::Result<()> {
        if view.area.is_empty() {
            return Ok(());
        }

        let mut commands = String::new();
        if !self.uploaded {
            // Replace the upload's default placement while retaining the uploaded pixels.
            commands.push_str(&command_wire(&delete(self.id, false), self.tmux)?);
        }
        if self.view != Some(view) {
            let command = Command::builder()
                .action(Action::Place)
                .image_id(self.id.get())
                .placement_id(self.id.get())
                .quiet(NO_PROTOCOL_REPLIES)
                .source_rect(
                    view.source.x,
                    view.source.y,
                    view.source.width,
                    view.source.height,
                )
                .display_area(u32::from(view.area.width), u32::from(view.area.height))
                .unicode_placeholder(view.area.width, view.area.height)
                .cursor_policy(CursorPolicy::NoMove)
                .build();
            commands.push_str(&command_wire(&command, self.tmux)?);
        }

        Image::new(&self.image)
            .allow_clipping(true)
            .render(view.area, buffer);
        if !commands.is_empty() {
            let cell = &mut buffer[view.area.as_position()];
            let symbol = cell.symbol();
            let placeholder = symbol.find(IMAGE_PLACEHOLDER).ok_or_else(|| {
                io::Error::other("The terminal image renderer did not produce a placeholder")
            })?;

            // Keep the library's upload and placeholder encoding. Insert the view command
            // between them so the terminal receives upload, crop/scale, then visible cells.
            let (upload, cells) = symbol.split_at(placeholder);
            let symbol = format!("{upload}{commands}{cells}");
            cell.set_symbol(&symbol);
        }

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
