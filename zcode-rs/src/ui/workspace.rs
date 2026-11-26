use super::composer::{ComposerElement, LaidLine};
use super::theme;
use crate::agent::{run_turn, ModelClient, UreqClient};
use crate::config::Config;
use crate::extension::ExtensionRegistry;
use crate::session::{ChatMessage, Role, Session, SessionStore};
use gpui::{
    div, prelude::*, px, App, Context, FocusHandle, Focusable, IntoElement, MouseButton,
    MouseUpEvent, Pixels, Point, Render, ScrollHandle, SharedString, Window,
};
use std::ops::Range;
use std::sync::mpsc::{self, TryRecvError};
use std::time::Duration;

pub struct Workspace {
    config: Config,
    store: Option<SessionStore>,
    registry: ExtensionRegistry,
    sessions: Vec<Session>,
    active_id: String,
    pub(crate) draft: String,
    pub(crate) cursor: usize,
    pub(crate) marked: Option<Range<usize>>,
    pub(crate) focus_handle: FocusHandle,
    pub(crate) laid_lines: Vec<LaidLine>,
    pub(crate) laid_origin: Option<Point<Pixels>>,
    pub(crate) laid_line_height: Pixels,
    scroll: ScrollHandle,
    running: bool,
    generation: u64,
    status: SharedString,
}

impl Workspace {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let config = Config::from_env();
        let registry = ExtensionRegistry::with_builtin();
        let (store, mut sessions, mut status) = match SessionStore::open(&config.data_dir) {
            Ok(store) => match store.list() {
                Ok(sessions) => (Some(store), sessions, String::new()),
                Err(err) => (Some(store), Vec::new(), format!("读取会话失败：{err}")),
            },
            Err(err) => (None, Vec::new(), format!("无法打开会话目录：{err}")),
        };
        if sessions.is_empty() {
            let session = Session::create(&config.workspace, &config.model);
            if let Some(store) = &store {
                if let Err(err) = store.save(&session) {
                    status = format!("保存新会话失败：{err}");
                }
            }
            sessions.push(session);
        }
        let active_id = sessions[0].id.clone();
        let mut workspace = Self {
            config,
            store,
            registry,
            sessions,
            active_id,
            draft: String::new(),
            cursor: 0,
            marked: None,
            focus_handle: cx.focus_handle(),
            laid_lines: Vec::new(),
            laid_origin: None,
            laid_line_height: px(22.),
            scroll: ScrollHandle::default(),
            running: false,
            generation: 0,
            status: SharedString::default(),
        };
        if status.is_empty() {
            status = workspace.ready_status();
        }
        workspace.status = status.into();
        workspace
    }

    pub(crate) fn submit(&mut self, cx: &mut Context<Self>) {
        if self.running {
            return;
        }
        let text = self.draft.trim().to_string();
        if text.is_empty() {
            return;
        }
        let Some(session) = self.active().cloned() else {
            return;
        };
        let Some(store) = self.store.clone() else {
            self.status = "会话目录不可用，这条消息不会被保存。".into();
            cx.notify();
            return;
        };

        self.draft.clear();
        self.cursor = 0;
        self.marked = None;
        self.running = true;
        self.generation = self.generation.saturating_add(1);
        let generation = self.generation;
        self.status = "正在运行…".into();
        let tools = self.registry.tools();
        let max_steps = self.config.max_steps;
        let config = self.config.clone();
        cx.notify();

        cx.spawn(async move |this, async_cx| {
            let (tx, rx) = mpsc::channel::<(Session, bool)>();
            std::thread::spawn(move || {
                let mut session = session;
                let result = if let Some(key) = config.api_key.clone() {
                    let client = UreqClient::new(&config.api_base, &key);
                    let progress_tx = tx.clone();
                    let progress_store = store.clone();
                    run_turn(
                        &client as &dyn ModelClient,
                        &mut session,
                        &text,
                        &tools,
                        max_steps,
                        move |session| {
                            let _ = progress_store.save(session);
                            let _ = progress_tx.send((session.clone(), false));
                        },
                    )
                } else {
                    session.note_user_title(&text);
                    session.messages.push(ChatMessage::user(text.clone()));
                    session.messages.push(ChatMessage::assistant(
                        "请求失败：未设置 API 密钥。请设置 ZCODE_API_KEY 或 OPENAI_API_KEY 后重新启动。",
                    ));
                    session.touch();
                    let _ = store.save(&session);
                    Ok(())
                };
                if let Err(err) = result {
                    session
                        .messages
                        .push(ChatMessage::assistant(format!("请求失败：{err}")));
                    session.touch();
                    let _ = store.save(&session);
                }
                let _ = tx.send((session, true));
            });

            loop {
                match rx.try_recv() {
                    Ok((session, done)) => {
                        let _ = this.update(async_cx, |view, cx| {
                            view.apply_remote(generation, session, done, cx);
                        });
                    }
                    Err(TryRecvError::Empty) => {
                        async_cx.background_executor().timer(Duration::from_millis(40)).await;
                    }
                    Err(TryRecvError::Disconnected) => break,
                }
            }
        })
        .detach();
    }

    fn apply_remote(
        &mut self,
        generation: u64,
        session: Session,
        done: bool,
        cx: &mut Context<Self>,
    ) {
        let id = session.id.clone();
        if let Some(slot) = self.sessions.iter_mut().find(|item| item.id == id) {
            *slot = session;
        }
        self.sessions
            .sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        if self.active_id == id {
            self.scroll.scroll_to_bottom();
        }
        if done && generation == self.generation {
            self.running = false;
            self.status = self.ready_status().into();
        }
        cx.notify();
    }

    fn new_session(&mut self, cx: &mut Context<Self>) {
        let session = Session::create(&self.config.workspace, &self.config.model);
        if let Some(store) = &self.store {
            if let Err(err) = store.save(&session) {
                self.status = format!("保存新会话失败：{err}").into();
            }
        }
        self.active_id = session.id.clone();
        self.sessions.insert(0, session);
        self.sessions
            .sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        self.draft.clear();
        self.cursor = 0;
        self.marked = None;
        self.scroll.scroll_to_bottom();
        cx.notify();
    }

    fn active(&self) -> Option<&Session> {
        self.sessions
            .iter()
            .find(|session| session.id == self.active_id)
    }

    fn ready_status(&self) -> String {
        let key = if self.config.key_is_set() {
            "密钥已配置"
        } else {
            "未设置 ZCODE_API_KEY"
        };
        let workspace = self
            .active()
            .map(|session| session.workspace.as_str())
            .unwrap_or("");
        let usage = self
            .active()
            .and_then(|session| session.last_usage.as_ref())
            .map(|usage| {
                format!(
                    " · tokens {}/{}",
                    fmt_tokens(usage.prompt_tokens),
                    fmt_tokens(usage.completion_tokens)
                )
            })
            .unwrap_or_default();
        format!("{key} · {} · {workspace}{usage}", self.config.model)
    }
}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let active_id = self.active_id.clone();
        let session_buttons = self.sessions.iter().map(|session| {
            let id = session.id.clone();
            let selected = id == active_id;
            let title = session.title.clone();
            div()
                .id(SharedString::from(format!("session-{id}")))
                .w_full()
                .px_2()
                .py_2()
                .rounded_md()
                .cursor_pointer()
                .bg(if selected {
                    theme::panel_raised()
                } else {
                    theme::sidebar_bg()
                })
                .hover(|style| style.bg(theme::panel()))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                        this.active_id = id.clone();
                        this.scroll.scroll_to_bottom();
                        cx.notify();
                    }),
                )
                .child(title)
        });

        let messages = self
            .active()
            .map(|session| session.messages.clone())
            .unwrap_or_default();
        let title = self
            .active()
            .map(|session| session.title.clone())
            .unwrap_or_else(|| "zcode-rs".into());
        let workspace = self
            .active()
            .map(|session| session.workspace.clone())
            .unwrap_or_default();
        let running = self.running;
        let status = self.status.clone();

        div()
            .size_full()
            .flex()
            .bg(theme::app_bg())
            .text_color(theme::text())
            .text_size(px(14.))
            .child(
                div()
                    .w(px(248.))
                    .h_full()
                    .flex()
                    .flex_col()
                    .bg(theme::sidebar_bg())
                    .border_r_1()
                    .border_color(theme::line())
                    .child(
                        div()
                            .px_3()
                            .py_3()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(div().text_sm().child("zcode-rs"))
                            .child(
                                div()
                                    .id("new-session")
                                    .px_2()
                                    .py_1()
                                    .rounded_md()
                                    .bg(theme::accent())
                                    .text_color(theme::sidebar_bg())
                                    .cursor_pointer()
                                    .child("新会话")
                                    .on_mouse_up(
                                        MouseButton::Left,
                                        cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                            this.new_session(cx);
                                        }),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .id("session-list")
                            .flex_1()
                            .overflow_scroll()
                            .px_2()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .children(session_buttons),
                    )
                    .child(
                        div()
                            .px_3()
                            .py_2()
                            .text_xs()
                            .text_color(theme::muted())
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child("内置工具")
                            .child("读文件 · 列目录")
                            .child("改文件 · 运行命令"),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .px_4()
                            .py_3()
                            .border_b_1()
                            .border_color(theme::line())
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div().flex().flex_col().child(title).child(
                                    div().text_xs().text_color(theme::muted()).child(workspace),
                                ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme::accent())
                                    .child(self.config.model.clone()),
                            ),
                    )
                    .child(
                        div()
                            .id("transcript")
                            .flex_1()
                            .overflow_scroll()
                            .track_scroll(&self.scroll)
                            .px_4()
                            .py_3()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .children(if messages.is_empty() {
                                vec![empty_transcript()]
                            } else {
                                messages.iter().map(message_view).collect()
                            }),
                    )
                    .child(
                        div()
                            .px_4()
                            .py_3()
                            .border_t_1()
                            .border_color(theme::line())
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .id("composer")
                                    .h(px(120.))
                                    .overflow_scroll()
                                    .bg(theme::panel())
                                    .border_1()
                                    .border_color(theme::line())
                                    .rounded_md()
                                    .px_3()
                                    .py_2()
                                    .track_focus(&self.focus_handle)
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _, window, cx| {
                                            window.focus(&this.focus_handle);
                                            cx.notify();
                                        }),
                                    )
                                    .on_key_down(cx.listener(Workspace::on_composer_key))
                                    .child(ComposerElement { view: cx.entity() }),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(theme::muted())
                                            .text_ellipsis()
                                            .child(status.to_string()),
                                    )
                                    .child(
                                        div()
                                            .id("send")
                                            .px_3()
                                            .py_1()
                                            .rounded_md()
                                            .cursor_pointer()
                                            .bg(if running {
                                                theme::panel_raised()
                                            } else {
                                                theme::accent()
                                            })
                                            .text_color(if running {
                                                theme::muted()
                                            } else {
                                                theme::sidebar_bg()
                                            })
                                            .child(if running { "运行中" } else { "发送" })
                                            .on_mouse_up(
                                                MouseButton::Left,
                                                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                                    this.submit(cx);
                                                }),
                                            ),
                                    ),
                            ),
                    ),
            )
    }
}

fn empty_transcript() -> gpui::AnyElement {
    div()
        .text_color(theme::muted())
        .whitespace_normal()
        .child("这个会话还没有消息。发送一条任务后，代理会调用模型，并在需要时读取文件、列出目录、修改文件或运行命令。")
        .into_any_element()
}

fn message_view(message: &ChatMessage) -> gpui::AnyElement {
    let (label, color) = match message.role {
        Role::User => ("你", theme::user()),
        Role::Assistant => ("助手", theme::accent()),
        Role::Tool => (message.name.as_deref().unwrap_or("工具"), theme::tool()),
    };
    let show_content = !message.content.is_empty();
    div()
        .p_3()
        .rounded_md()
        .bg(theme::panel())
        .border_1()
        .border_color(theme::line())
        .flex()
        .flex_col()
        .gap_1()
        .child(div().text_xs().text_color(color).child(label.to_string()))
        .when(show_content, |this| {
            this.child(div().whitespace_normal().child(message.content.clone()))
        })
        .children(message.tool_calls.iter().map(|call| {
            div()
                .text_xs()
                .text_color(theme::muted())
                .whitespace_normal()
                .child(format!(
                    "调用 {} {}",
                    call.name,
                    crate::truncate_chars(&call.arguments, 180)
                ))
        }))
        .into_any_element()
}

fn fmt_tokens(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "—".into())
}
