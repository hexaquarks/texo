use super::*;

fn page(terminal: TerminalImages) -> RetainedImage {
    RetainedImage::new(
        &Picker::halfblocks(),
        DynamicImage::new_rgb8(120, 200),
        terminal,
    )
    .unwrap()
}

fn view() -> ImageView {
    ImageView {
        source: image::math::Rect {
            x: 0,
            y: 0,
            width: 120,
            height: 200,
        },
        area: Rect::new(0, 0, 20, 12),
    }
}

#[test]
fn page_upload_is_followed_by_a_retained_placement() {
    let mut page = page(TerminalImages::default());
    let view = view();
    let mut buffer = Buffer::empty(view.area);
    page.render(view, &mut buffer).unwrap();
    let symbol = buffer[view.area.as_position()].symbol();

    let upload = symbol.find("a=T").unwrap();
    let remove_default = symbol.find("a=d").unwrap();
    let placement = symbol.find("a=p").unwrap();
    assert!(upload < remove_default && remove_default < placement);
    assert!(symbol.contains("U=1"));
    assert!(symbol.contains("q=2"));

    page.render(view, &mut buffer).unwrap();
    assert!(!buffer[view.area.as_position()].symbol().contains("a="));
}

#[test]
fn zoom_and_pan_change_only_the_placement_command() {
    let mut page = page(TerminalImages::default());
    let mut view = view();
    let mut buffer = Buffer::empty(view.area);
    page.render(view, &mut buffer).unwrap();

    for offset in 1..=20 {
        view.source = image::math::Rect {
            x: offset,
            y: offset,
            width: 60,
            height: 100,
        };
        page.render(view, &mut buffer).unwrap();
        let symbol = buffer[view.area.as_position()].symbol();

        assert!(symbol.contains("a=p"));
        assert!(!symbol.contains("a=T"));
        assert!(symbol.contains("w=60,h=100"));
        assert!(
            symbol.len() < 256,
            "Interaction must not retransmit pixel data"
        );
    }
}

#[test]
fn uploaded_pages_release_terminal_memory_and_unseen_pages_do_not_emit_commands() {
    let terminal = TerminalImages::default();
    drop(page(terminal.clone()));
    let mut bytes = Vec::new();
    terminal.flush_retired(&mut bytes).unwrap();
    assert!(bytes.is_empty());

    let mut page = page(terminal.clone());
    page.render(view(), &mut Buffer::empty(view().area))
        .unwrap();
    let id = page.id.get();
    drop(page);
    terminal.flush_retired(&mut bytes).unwrap();
    let command = String::from_utf8(bytes).unwrap();
    assert!(command.contains(&format!("i={id}")));
    assert!(command.contains("d=I"));
    assert!(command.contains("q=2"));

    let mut bytes = Vec::new();
    terminal.flush_retired(&mut bytes).unwrap();
    assert!(bytes.is_empty());
}

#[test]
fn placement_commands_use_the_uploads_tmux_framing() {
    let command = delete(NonZeroU32::MIN, false);
    let direct = command_wire(&command, false).unwrap();
    let wrapped = command_wire(&command, true).unwrap();

    assert!(direct.starts_with("\x1b_G"));
    assert!(wrapped.starts_with("\x1bPtmux;\x1b\x1b_G"));
    assert!(wrapped.ends_with("\x1b\x1b\\\x1b\\"));
}
