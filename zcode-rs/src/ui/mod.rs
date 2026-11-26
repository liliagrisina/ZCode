mod composer;
mod theme;
mod workspace;

use gpui::{
    prelude::*, px, size, App, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions,
};
use workspace::Workspace;

pub fn run() {
    Application::new().run(|cx: &mut App| {
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let bounds = Bounds::centered(None, size(px(1120.), px(760.)), cx);
        let opened = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("zcode-rs".into()),
                    ..Default::default()
                }),
                app_id: Some("zcode-rs".into()),
                window_min_size: Some(size(px(860.), px(560.))),
                ..Default::default()
            },
            |window, cx| {
                let view = cx.new(Workspace::new);
                view.read(cx).focus_handle.focus(window);
                view
            },
        );
        if let Err(err) = opened {
            eprintln!("无法打开窗口: {err:#}");
            cx.quit();
        }
    });
}
