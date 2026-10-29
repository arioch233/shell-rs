//! 目录书签复用 SFTP 数据，填入固定终端且保留完整路径

use crate::support::*;
use shellrs::app::{EnterTerminalDirectory, OpenTerminalBookmarks, ShowTerminalBookmarks};
use shellrs::host::BookmarkSide;

fn row(path: &str) -> ElementId {
    format!("terminal-bookmark-row:{path}").into()
}

#[gpui_kit::test]
async fn terminal_bookmarks_search_full_aliases_and_paths_then_prepare_cd(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::default());
    let mut store = HostStore::seed();
    let path = "/srv/项目 a'b/$(touch nope)";
    let long_alias = format!("{}关键尾部", "很长的项目书签名称".repeat(30));
    store.add_bookmark_with_alias_unnotified(
        HostId(WEB_01),
        BookmarkSide::Remote,
        path,
        Some(&long_alias),
    );
    store.add_bookmark_with_alias_unnotified(
        HostId(WEB_01),
        BookmarkSide::Remote,
        "/etc",
        Some(&long_alias),
    );
    store.add_bookmark_unnotified(HostId(WEB_01), BookmarkSide::Local, "/local-only");
    store.add_bookmark_unnotified(HostId(DB_01), BookmarkSide::Remote, "/other-host");
    let (handle, workspace) = open_workspace_with_remote_factory(cx, store, factory.clone());
    in_frame(cx, handle, |window, cx| {
        window.activate_window();
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx);
    });
    cx.wait_for(handle.into(), Duration::from_secs(2), |_, cx| {
        remote_lifecycle(&workspace, RemoteTerminalId(INITIAL_WEB_TERMINAL), cx).accepts_input()
    })
    .await;
    let before = factory.written_text();
    in_frame(cx, handle, |window, cx| {
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-shift-j"
            } else {
                "ctrl-shift-j"
            },
            cx,
        );
    });
    in_frame(cx, handle, |window, cx| {
        let palette = window.find("terminal-bookmarks").bounds();
        let label = window
            .find(ElementId::Name(format!("bookmark-label:{path}").into()))
            .bounds();
        assert!(
            label.right() <= palette.right(),
            "{label:?} outside {palette:?}"
        );
        assert!(
            label.size.height < px(65.),
            "long names must remain two lines"
        );
        assert!(window.find(row(path)).label().unwrap().contains("关键尾部"));
        assert!(window.try_find(row("/local-only")).is_none());
        assert!(window.try_find(row("/other-host")).is_none());
        window.input("关键尾部 TOUCH", cx);
    });
    cx.wait_for(handle.into(), Duration::from_secs(2), |window, cx| {
        window.render_frame(cx);
        window.try_find(row("/etc")).is_none()
    })
    .await;
    in_frame(cx, handle, |window, cx| {
        assert!(window.find(row(path)).visible());
        window.press("enter", cx);
    });
    cx.wait_for(handle.into(), Duration::from_secs(2), |_, _| {
        factory.written_text() != before
    })
    .await;
    assert_eq!(
        &factory.written_text()[before.len()..],
        "\x05\x15cd -- '/srv/项目 a'\\''b/$(touch nope)'"
    );
    assert_eq!(factory.exec_count(), 0);
    in_frame(cx, handle, |window, _| {
        assert!(window.try_find("terminal-bookmarks").is_none());
        assert_eq!(
            window.find(("terminal", INITIAL_WEB_TERMINAL)).focused(),
            Some(true)
        );
    });
}

#[gpui_kit::test]
async fn terminal_bookmarks_empty_search_and_escape_do_not_send_input(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::default());
    let (handle, workspace) =
        open_workspace_with_remote_factory(cx, HostStore::seed(), factory.clone());
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx);
    });
    cx.wait_for(handle.into(), Duration::from_secs(2), |_, cx| {
        remote_lifecycle(&workspace, RemoteTerminalId(INITIAL_WEB_TERMINAL), cx).accepts_input()
    })
    .await;
    let before = factory.written_text();
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(OpenTerminalBookmarks), cx)
    });
    in_frame(cx, handle, |window, cx| {
        assert!(
            window
                .find("terminal-bookmarks-empty")
                .label()
                .unwrap()
                .contains("还没有远程书签")
        );
        window.press("escape", cx);
    });
    in_frame(cx, handle, |window, cx| {
        assert_eq!(
            window.find(("terminal", INITIAL_WEB_TERMINAL)).focused(),
            Some(true)
        );
        workspace.read(cx).store().clone().update(cx, |store, cx| {
            store.add_bookmark(HostId(WEB_01), BookmarkSide::Remote, "/var/log", cx);
        });
        window.dispatch_action(Box::new(OpenTerminalBookmarks), cx);
    });
    in_frame(cx, handle, |window, cx| window.input("does-not-exist", cx));
    cx.wait_for(handle.into(), Duration::from_secs(2), |window, cx| {
        window.render_frame(cx);
        window
            .try_find("terminal-bookmarks-empty")
            .is_some_and(|empty| empty.label() == Some("没有匹配的目录书签"))
    })
    .await;
    in_frame(cx, handle, |window, cx| window.press("enter", cx));
    in_frame(cx, handle, |window, cx| {
        assert!(window.find("terminal-bookmarks").visible());
        window.press("escape", cx);
    });
    assert_eq!(factory.written_text(), before);
}

#[gpui_kit::test]
async fn terminal_bookmarks_reject_stale_connections_and_control_characters(
    cx: &mut TestAppContext,
) {
    let factory = Arc::new(FakeTerminalFactory::default());
    let (handle, workspace) =
        open_workspace_with_remote_factory(cx, HostStore::seed(), factory.clone());
    let terminal = RemoteTerminalId(INITIAL_WEB_TERMINAL);
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", terminal.0), cx)
    });
    cx.wait_for(handle.into(), Duration::from_secs(2), |_, cx| {
        remote_lifecycle(&workspace, terminal, cx).accepts_input()
    })
    .await;
    let generation = cx.update(|cx| {
        workspace
            .read(cx)
            .remote_terminal(terminal)
            .unwrap()
            .read(cx)
            .terminal()
            .read(cx)
            .generation(cx)
    });
    let before = factory.written_text();
    for path in ["/tmp\nexit", "/tmp\x1b[31m", "relative"] {
        in_frame(cx, handle, |window, cx| {
            window.dispatch_action(
                Box::new(EnterTerminalDirectory::new(
                    terminal,
                    generation,
                    path.into(),
                )),
                cx,
            );
        });
    }
    assert_eq!(factory.written_text(), before);
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(ReconnectTerminal(terminal)), cx)
    });
    cx.wait_for(handle.into(), Duration::from_secs(2), |_, cx| {
        let view = workspace
            .read(cx)
            .remote_terminal(terminal)
            .unwrap()
            .read(cx)
            .terminal()
            .read(cx);
        view.generation(cx) != generation && view.lifecycle(cx).accepts_input()
    })
    .await;
    let before = factory.written_text();
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(
            Box::new(EnterTerminalDirectory::new(
                terminal,
                generation,
                "/etc".into(),
            )),
            cx,
        );
    });
    assert_eq!(factory.written_text(), before);
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(DisconnectTerminal(terminal)), cx);
    });
    let generation = cx.update(|cx| {
        workspace
            .read(cx)
            .remote_terminal(terminal)
            .unwrap()
            .read(cx)
            .terminal()
            .read(cx)
            .generation(cx)
    });
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(
            Box::new(EnterTerminalDirectory::new(
                terminal,
                generation,
                "/etc".into(),
            )),
            cx,
        );
    });
    assert_eq!(factory.written_text(), before);
}

#[gpui_kit::test]
async fn terminal_bookmarks_do_not_type_into_full_screen_programs(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::printing("\x1b[?1049hVIM"));
    let mut store = HostStore::seed();
    store.add_bookmark_unnotified(HostId(WEB_01), BookmarkSide::Remote, "/var/log");
    let (handle, workspace) = open_workspace_with_remote_factory(cx, store, factory.clone());
    let terminal = RemoteTerminalId(INITIAL_WEB_TERMINAL);
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", terminal.0), cx)
    });
    cx.wait_for(handle.into(), Duration::from_secs(2), |_, cx| {
        remote_lifecycle(&workspace, terminal, cx).accepts_input()
            && workspace
                .read(cx)
                .remote_terminal(terminal)
                .unwrap()
                .read(cx)
                .terminal()
                .read(cx)
                .screen_text(cx)
                .contains("VIM")
    })
    .await;
    let before = factory.written_text();
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(ShowTerminalBookmarks(terminal)), cx)
    });
    in_frame(cx, handle, |window, cx| window.click(row("/var/log"), cx));
    in_frame(cx, handle, |window, cx| {
        assert!(!window.notifications(cx).is_empty())
    });
    assert_eq!(factory.written_text(), before);
}
