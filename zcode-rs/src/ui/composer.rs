use super::theme;
use super::workspace::Workspace;
use gpui::{
    fill, point, px, relative, size, App, Bounds, Context, Element, ElementInputHandler, Entity,
    EntityInputHandler, GlobalElementId, IntoElement, KeyDownEvent, LayoutId, PaintQuad, Pixels,
    Point, ShapedLine, SharedString, Style, TextRun, UTF16Selection, Window,
};
use std::ops::Range;

pub(crate) struct LaidLine {
    pub byte_start: usize,
    pub text_len: usize,
    pub shaped: ShapedLine,
}

struct DraftLine {
    byte_start: usize,
    text: String,
}

pub(crate) struct ComposerElement {
    pub view: Entity<Workspace>,
}

impl Workspace {
    pub(crate) fn on_composer_key(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.to_string().to_ascii_lowercase();
        let modifiers = event.keystroke.modifiers;
        if key == "enter" {
            cx.stop_propagation();
            if modifiers.shift {
                self.insert_text("\n", cx);
            } else {
                self.submit(cx);
            }
            return;
        }
        if key == "backspace" {
            cx.stop_propagation();
            self.backspace(cx);
            return;
        }
        if key == "left"
            || key == "right"
            || key == "up"
            || key == "down"
            || key == "home"
            || key == "end"
        {
            cx.stop_propagation();
            self.move_cursor(&key);
            cx.notify();
            return;
        }
        if key == "v" && (modifiers.control || modifiers.platform) {
            cx.stop_propagation();
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                self.insert_text(&text, cx);
            }
        }
    }

    pub(crate) fn insert_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let text = normalize_newlines(text);
        let range = self.marked.clone().unwrap_or(self.cursor..self.cursor);
        self.replace_utf8(range, &text);
        cx.notify();
    }

    pub(crate) fn backspace(&mut self, cx: &mut Context<Self>) {
        if let Some(marked) = self.marked.clone() {
            self.replace_utf8(marked, "");
        } else if self.cursor > 0 {
            let previous = previous_boundary(&self.draft, self.cursor);
            self.replace_utf8(previous..self.cursor, "");
        }
        cx.notify();
    }

    fn move_cursor(&mut self, key: &str) {
        self.marked = None;
        self.cursor = match key {
            "left" => previous_boundary(&self.draft, self.cursor),
            "right" => next_boundary(&self.draft, self.cursor),
            "home" => line_bounds(&self.draft, self.cursor).0,
            "end" => line_bounds(&self.draft, self.cursor).1,
            "up" => move_line(&self.draft, self.cursor, true),
            "down" => move_line(&self.draft, self.cursor, false),
            _ => self.cursor,
        };
    }

    fn replace_utf8(&mut self, range: Range<usize>, text: &str) {
        let range = clamp_range(&self.draft, range);
        self.draft.replace_range(range.start..range.end, text);
        self.cursor = range.start + text.len();
        self.marked = None;
        self.cursor = clamp_range(&self.draft, self.cursor..self.cursor).start;
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8 = 0;
        let mut utf16 = 0;
        for ch in self.draft.chars() {
            if utf16 >= offset {
                break;
            }
            utf16 += ch.len_utf16();
            utf8 += ch.len_utf8();
        }
        utf8
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf8 = 0;
        let mut utf16 = 0;
        for ch in self.draft.chars() {
            if utf8 >= offset {
                break;
            }
            utf8 += ch.len_utf8();
            utf16 += ch.len_utf16();
        }
        utf16
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }
}

impl EntityInputHandler for Workspace {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.draft.get(range).unwrap_or("").to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let cursor = self.offset_to_utf16(self.cursor);
        Some(UTF16Selection {
            range: cursor..cursor,
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked.as_ref().map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .or_else(|| self.marked.clone())
            .unwrap_or(self.cursor..self.cursor);
        self.replace_utf8(range, &normalize_newlines(new_text));
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let new_text = normalize_newlines(new_text);
        self.replace_text_in_range(range_utf16, &new_text, window, cx);
        if new_text.is_empty() {
            self.marked = None;
        } else {
            let end = self.cursor;
            let start = end.saturating_sub(new_text.len());
            self.marked = Some(start..end);
            if let Some(selection) = new_selected_range {
                let relative = utf16_index_in(&new_text, selection.end);
                self.cursor = start + relative.min(new_text.len());
            }
        }
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        Some(self.caret_bounds(element_bounds))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let origin = self.laid_origin?;
        if self.laid_lines.is_empty() {
            return Some(0);
        }
        let mut top = origin.y;
        for line in &self.laid_lines {
            let bottom = top + self.laid_line_height;
            if point.y < bottom {
                let column = line
                    .shaped
                    .index_for_x(point.x - origin.x)
                    .unwrap_or(line.text_len)
                    .min(line.text_len);
                return Some(self.offset_to_utf16(line.byte_start + column));
            }
            top = bottom;
        }
        Some(self.offset_to_utf16(self.draft.len()))
    }
}

impl Workspace {
    fn caret_bounds(&self, fallback: Bounds<Pixels>) -> Bounds<Pixels> {
        let Some(origin) = self.laid_origin else {
            return fallback;
        };
        let (line_index, column) = line_column(&self.draft, self.cursor);
        let x = self
            .laid_lines
            .get(line_index)
            .map(|line| line.shaped.x_for_index(column.min(line.text_len)))
            .unwrap_or(px(0.));
        let mut y = origin.y;
        for _ in 0..line_index {
            y += self.laid_line_height;
        }
        Bounds::new(point(origin.x + x, y), size(px(2.), self.laid_line_height))
    }
}

pub(crate) struct ComposerPrepaint {
    lines: Vec<LaidLine>,
    cursor: Option<PaintQuad>,
    origin: Point<Pixels>,
    line_height: Pixels,
}

impl IntoElement for ComposerElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for ComposerElement {
    type RequestLayoutState = ();
    type PrepaintState = ComposerPrepaint;

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let draft = self.view.read(cx).draft.clone();
        let rows = split_draft(&draft).len().max(1) as f32;
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = px((rows * 22.0).max(88.0)).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let (draft, cursor, focused) = {
            let view = self.view.read(cx);
            (
                view.draft.clone(),
                view.cursor,
                view.focus_handle.is_focused(window),
            )
        };
        let style = window.text_style();
        let font = style.font();
        let font_size = px(15.);
        let line_height = px(22.);
        let placeholder = draft.is_empty();
        let source = if placeholder {
            vec![DraftLine {
                byte_start: 0,
                text: "描述要做的事。Enter 发送，Shift+Enter 换行。".into(),
            }]
        } else {
            split_draft(&draft)
        };
        let color = if placeholder {
            theme::muted()
        } else {
            theme::text()
        };
        let mut lines = Vec::new();
        let mut top = bounds.top();
        let (cursor_line, cursor_column) = line_column(&draft, cursor);
        let mut cursor_quad = None;
        for (index, line) in source.into_iter().enumerate() {
            let display: SharedString = if line.text.is_empty() {
                " ".to_string().into()
            } else {
                line.text.clone().into()
            };
            let shaped = window.text_system().shape_line(
                display,
                font_size,
                &[TextRun {
                    len: if line.text.is_empty() {
                        1
                    } else {
                        line.text.len()
                    },
                    font: font.clone(),
                    color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }],
                None,
            );
            if focused && !placeholder && index == cursor_line {
                let x = shaped.x_for_index(cursor_column.min(line.text.len()));
                cursor_quad = Some(fill(
                    Bounds::new(point(bounds.left() + x, top), size(px(2.), line_height)),
                    theme::accent(),
                ));
            }
            lines.push(LaidLine {
                byte_start: line.byte_start,
                text_len: if placeholder { 0 } else { line.text.len() },
                shaped,
            });
            top += line_height;
        }
        ComposerPrepaint {
            lines,
            cursor: cursor_quad,
            origin: bounds.origin,
            line_height,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.view.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.view.clone()),
            cx,
        );
        let mut top = bounds.top();
        for line in &prepaint.lines {
            let _ = line
                .shaped
                .paint(point(bounds.left(), top), prepaint.line_height, window, cx);
            top += prepaint.line_height;
        }
        if let Some(cursor) = prepaint.cursor.take() {
            window.paint_quad(cursor);
        }
        let lines = std::mem::take(&mut prepaint.lines);
        let origin = prepaint.origin;
        let line_height = prepaint.line_height;
        self.view.update(cx, |view, _cx| {
            view.laid_lines = lines;
            view.laid_origin = Some(origin);
            view.laid_line_height = line_height;
        });
    }
}

fn split_draft(draft: &str) -> Vec<DraftLine> {
    if draft.is_empty() {
        return vec![DraftLine {
            byte_start: 0,
            text: String::new(),
        }];
    }
    let mut lines = Vec::new();
    let mut start = 0;
    for (index, ch) in draft.char_indices() {
        if ch == '\n' {
            lines.push(DraftLine {
                byte_start: start,
                text: draft[start..index].to_string(),
            });
            start = index + ch.len_utf8();
        }
    }
    lines.push(DraftLine {
        byte_start: start,
        text: draft[start..].to_string(),
    });
    lines
}

fn line_column(draft: &str, cursor: usize) -> (usize, usize) {
    let lines = split_draft(draft);
    let cursor = cursor.min(draft.len());
    for (index, line) in lines.iter().enumerate() {
        let end = line.byte_start + line.text.len();
        let last = index + 1 == lines.len();
        if cursor < end || cursor == end && (last || cursor < lines[index + 1].byte_start) {
            return (index, cursor.saturating_sub(line.byte_start));
        }
    }
    let last = lines.len() - 1;
    (last, lines[last].text.len())
}

fn line_bounds(draft: &str, cursor: usize) -> (usize, usize) {
    let cursor = cursor.min(draft.len());
    let start = draft[..cursor]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let end = draft[cursor..]
        .find('\n')
        .map(|index| cursor + index)
        .unwrap_or(draft.len());
    (start, end)
}

fn move_line(draft: &str, cursor: usize, up: bool) -> usize {
    let (start, end) = line_bounds(draft, cursor);
    if up {
        if start == 0 {
            0
        } else {
            draft[..start - 1]
                .rfind('\n')
                .map(|index| index + 1)
                .unwrap_or(0)
        }
    } else if end >= draft.len() {
        draft.len()
    } else {
        end + 1
    }
}

fn previous_boundary(text: &str, cursor: usize) -> usize {
    text[..cursor.min(text.len())]
        .char_indices()
        .next_back()
        .map(|(index, _)| index)
        .unwrap_or(0)
}

fn next_boundary(text: &str, cursor: usize) -> usize {
    let cursor = cursor.min(text.len());
    text[cursor..]
        .char_indices()
        .nth(1)
        .map(|(index, _)| cursor + index)
        .unwrap_or(text.len())
}

fn clamp_range(text: &str, range: Range<usize>) -> Range<usize> {
    let start = snap(text, range.start);
    let end = snap(text, range.end.max(start));
    start..end
}

fn snap(text: &str, mut index: usize) -> usize {
    if index > text.len() {
        index = text.len();
    }
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn normalize_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn utf16_index_in(text: &str, utf16_index: usize) -> usize {
    let mut utf8 = 0;
    let mut utf16 = 0;
    for ch in text.chars() {
        if utf16 >= utf16_index {
            break;
        }
        utf16 += ch.len_utf16();
        utf8 += ch.len_utf8();
    }
    utf8
}
