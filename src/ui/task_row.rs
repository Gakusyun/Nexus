//! One download row: name, state, progress, and the actions available for it.

use gpui::prelude::*;
use gpui::{ClickEvent, ColorExt, Context, FontWeight, IntoElement, TextAlign, div, px};

use super::{IconButton, ProgressBar, StatusBadge, Tone};
use crate::i18n::Strings;
use crate::model::{Status, Task, fmt_eta, fmt_percent, fmt_size, fmt_speed};
use crate::state::NexusApp;
use crate::theme::Theme;

pub(super) fn task_row(
    task: Task,
    strings: &'static Strings,
    theme: &Theme,
    cx: &mut Context<NexusApp>,
) -> impl IntoElement {
    let seq = task.seq;
    let progress = task.progress();
    let tone = match task.status {
        Status::Complete => Tone::Success,
        Status::Error => Tone::Danger,
        _ => Tone::Accent,
    };

    let (surface, border, accent) = (theme.surface, theme.border, theme.accent);

    div()
        .id(("task", seq))
        .flex()
        .flex_col()
        .flex_none()
        .gap(px(11.0))
        .px(px(16.0))
        .py(px(14.0))
        .rounded(px(15.0))
        .bg(surface)
        .border_1()
        .border_color(border)
        .hover(move |style| style.border_color(accent.opacity(0.38)))
        .child(header(&task, strings, theme, seq, cx))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(14.0))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(ProgressBar::new(progress, tone)),
                )
                .child(
                    div()
                        .flex_none()
                        .w(px(42.0))
                        .text_size(px(12.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_align(TextAlign::Right)
                        .text_color(match task.status {
                            Status::Complete => theme.success,
                            Status::Error => theme.danger,
                            _ => theme.text_muted,
                        })
                        .child(fmt_percent(progress)),
                ),
        )
        .child(meta(&task, strings, theme))
}

fn header(
    task: &Task,
    strings: &'static Strings,
    theme: &Theme,
    seq: u64,
    cx: &mut Context<NexusApp>,
) -> impl IntoElement + use<> {
    let (text, muted, danger, hover_bg) = (
        theme.text,
        theme.text_muted,
        theme.danger,
        theme.surface_hover,
    );

    let mut actions = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(2.0))
        .flex_none();

    // Finished downloads have nothing to pause, so they only offer "show me" and "forget".
    if task.status != Status::Complete {
        let (glyph, tint) = match task.status {
            Status::Error => ("icons/retry.svg", danger),
            Status::Paused => ("icons/play.svg", text),
            _ => ("icons/pause.svg", muted),
        };
        actions = actions.child(
            IconButton::new("task-toggle", seq, glyph, tint, text)
                .hover_bg(hover_bg)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle(seq, cx))),
        );
    }

    actions = actions
        .child(
            IconButton::new("task-reveal", seq, "icons/folder.svg", muted, text)
                .hover_bg(hover_bg)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.reveal(seq, cx))),
        )
        .child(
            IconButton::new("task-remove", seq, "icons/x.svg", muted, danger)
                .hover_bg(hover_bg)
                .on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| this.request_remove(seq, cx)),
                ),
        );

    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .text_size(px(13.5))
                .font_weight(FontWeight::MEDIUM)
                .child(task.name.clone()),
        )
        .child(StatusBadge::new(task.status, strings))
        .child(actions)
}

/// The small print: sizes on the left, live throughput on the right.
fn meta(task: &Task, strings: &Strings, theme: &Theme) -> impl IntoElement {
    let (left, right) = describe(task, strings);
    let has_right = !right.is_empty();
    let (faint, muted, danger) = (theme.text_faint, theme.text_muted, theme.danger);
    let left_tint = if task.status == Status::Error {
        danger
    } else {
        faint
    };

    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(14.0))
        .text_size(px(11.5))
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .text_color(left_tint)
                .child(left),
        )
        .when(has_right, |element| {
            element.child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .text_color(muted)
                    .child(right),
            )
        })
}

fn describe(task: &Task, strings: &Strings) -> (String, String) {
    match task.status {
        Status::Active => {
            let left = if task.total == 0 {
                strings.fetching_metadata.to_string()
            } else {
                format!("{} / {}", fmt_size(task.completed), fmt_size(task.total))
            };
            let mut right = fmt_speed(task.speed);
            if let Some(eta) = task.eta() {
                right.push_str(&format!(" · {} {}", fmt_eta(eta), strings.time_left));
            }
            if task.connections > 0 {
                right.push_str(&format!(" · {}", strings.connections(task.connections)));
            }
            (left, right)
        }
        Status::Paused => (
            format!("{} / {}", fmt_size(task.completed), fmt_size(task.total)),
            format!("{} {}", strings.paused_at, fmt_percent(task.progress())),
        ),
        Status::Waiting => {
            let left = if task.total == 0 {
                strings.queued.to_string()
            } else {
                format!("{} / {}", fmt_size(task.completed), fmt_size(task.total))
            };
            (left, String::new())
        }
        Status::Complete => (
            format!("{} · {}", fmt_size(task.total), task.dir),
            String::new(),
        ),
        Status::Error => (
            task.error
                .clone()
                .unwrap_or_else(|| strings.download_failed.to_string()),
            String::new(),
        ),
    }
}
