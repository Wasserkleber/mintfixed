//! Regression coverage for delayed priority editing in egui 0.31.
use super::{priority_drag_value, priority_preview_value};

fn frame(ctx: &egui::Context, priority: &mut i32, events: Vec<egui::Event>) -> egui::Id {
    let mut field_id = None;
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(500.0, 200.0),
            )),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                field_id = Some(priority_drag_value(ui, priority).id);
            });
        },
    );
    field_id.expect("priority input was rendered")
}

fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

#[test]
fn priority_input_multiple_digits_only_commit_on_enter() {
    let ctx = egui::Context::default();
    let mut priority = 42;
    let id = frame(&ctx, &mut priority, vec![]);
    ctx.memory_mut(|memory| memory.request_focus(id));
    frame(&ctx, &mut priority, vec![]);

    frame(
        &ctx,
        &mut priority,
        vec![
            key(egui::Key::A, egui::Modifiers::COMMAND),
            egui::Event::Text("1".into()),
        ],
    );
    assert_eq!(priority, 42, "first digit must not save the value");
    frame(&ctx, &mut priority, vec![egui::Event::Text("2".into())]);
    assert_eq!(priority, 42, "second digit must not save the value");
    frame(&ctx, &mut priority, vec![egui::Event::Text("3".into())]);
    assert_eq!(priority, 42, "third digit must not save the value");

    frame(
        &ctx,
        &mut priority,
        vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
    );
    assert_eq!(priority, 123);
}

#[test]
fn priority_input_clear_does_not_commit_until_enter() {
    let ctx = egui::Context::default();
    let mut priority = 17;
    let id = frame(&ctx, &mut priority, vec![]);
    ctx.memory_mut(|memory| memory.request_focus(id));
    frame(&ctx, &mut priority, vec![]);

    frame(
        &ctx,
        &mut priority,
        vec![
            key(egui::Key::A, egui::Modifiers::COMMAND),
            key(egui::Key::Backspace, egui::Modifiers::NONE),
        ],
    );
    assert_eq!(
        priority, 17,
        "clearing text must not immediately reset the priority"
    );

    frame(
        &ctx,
        &mut priority,
        vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
    );
    assert_eq!(
        priority, 0,
        "an empty committed field means default priority"
    );
}

#[test]
fn priority_input_live_preview_uses_pending_text_without_saving() {
    // egui stores DragValue's editing buffer as String keyed by the widget ID.
    // Test both positive and negative pending text, plus the cleared state.
    for (pending, expected) in [("1", 1), ("123", 123), ("-3", -3), ("", 0), ("-", 0)] {
        let ctx = egui::Context::default();
        let mut committed = 0;
        let id = frame(&ctx, &mut committed, vec![]);
        ctx.memory_mut(|memory| memory.request_focus(id));
        ctx.data_mut(|data| data.insert_temp(id, pending.to_owned()));
        let mut preview = None;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(500.0, 200.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    preview = Some(priority_preview_value(ui, committed));
                    // Render the field after preview, just as the real mod row does.
                    priority_drag_value(ui, &mut committed);
                });
            },
        );
        assert_eq!(preview, Some(expected), "preview for {pending:?}");
        assert_eq!(committed, 0, "preview must not commit stored priority");
    }
}
