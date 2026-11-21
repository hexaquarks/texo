use anyhow::Result;
use image::{DynamicImage, imageops::FilterType};
use ratatui::{Frame, layout::Rect};
use ratatui_image::{
    FontSize, Image, Resize,
    picker::{Picker, ProtocolType},
    protocol::Protocol,
};

use crate::retained_image::{ImageView, RetainedImage, TerminalImages};

// Some terminals report logical pixels per cell. Keep enough samples for a 2× display.
const NATIVE_RASTER_SCALE: u32 = 2;

pub struct PageThumbnail {
    backend: ThumbnailBackend,
}

enum ThumbnailBackend {
    Kitty {
        image: RetainedImage,
        view: ImageView,
    },
    Halfblocks(Protocol),
}

impl PageThumbnail {
    pub(crate) fn new(
        image: DynamicImage,
        area: Rect,
        picker: &Picker,
        terminal: TerminalImages,
    ) -> Result<Self> {
        let backend = if picker.protocol_type() == ProtocolType::Kitty {
            let font = picker.font_size();
            let font = FontSize::new(font.width.max(1), font.height.max(1));
            let size = Resize::Scale(None).size_for(&image, font, area.as_size());
            let placement = centered(area, size.width, size.height);

            let raster_width = u32::from(area.width) * u32::from(font.width) * NATIVE_RASTER_SCALE;
            let raster_height =
                u32::from(area.height) * u32::from(font.height) * NATIVE_RASTER_SCALE;
            let raster = image.resize(
                raster_width.min(image.width()),
                raster_height.min(image.height()),
                FilterType::CatmullRom,
            );

            let view = ImageView {
                page_size: placement.as_size(),
                offset: Default::default(),
                area: placement,
            };

            match RetainedImage::new(picker, raster, terminal) {
                Ok(image) => ThumbnailBackend::Kitty { image, view },
                Err(_) => halfblocks(image, area)?,
            }
        } else {
            // Character cells work even where inline image commands can escape the dialog.
            halfblocks(image, area)?
        };

        Ok(Self { backend })
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect) -> Result<()> {
        match &mut self.backend {
            ThumbnailBackend::Kitty { image, view } => image.render(*view, frame.buffer_mut())?,
            ThumbnailBackend::Halfblocks(image) => {
                let size = image.size();
                frame.render_widget(Image::new(image), centered(area, size.width, size.height));
            }
        }

        Ok(())
    }
}

fn halfblocks(image: DynamicImage, area: Rect) -> Result<ThumbnailBackend> {
    let protocol = Picker::halfblocks().new_protocol(
        image,
        area.as_size(),
        Resize::Fit(Some(FilterType::Triangle)),
    )?;
    Ok(ThumbnailBackend::Halfblocks(protocol))
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y,
        width,
        height,
    )
}
