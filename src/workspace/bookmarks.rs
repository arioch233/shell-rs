//! 终端书签和文件侧栏共用的目录输入路由

use gpui_kit::component::{WindowExt as _, notification::Notification};
use gpui_kit::*;

use super::Workspace;
use crate::{
    app::{CenterTab, EnterTerminalDirectory, OpenTerminalBookmarks, ShowTerminalBookmarks},
    host::{BookmarkSide, HostOs},
    i18n::t,
    terminal::{directory_command, open_terminal_bookmarks},
};

impl Workspace {
    pub(super) fn on_open_terminal_bookmarks(
        &mut self,
        _: &OpenTerminalBookmarks,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(CenterTab::Terminal(id)) = self.active_tab {
            self.on_show_terminal_bookmarks(&ShowTerminalBookmarks(id), window, cx);
        }
    }

    pub(super) fn on_show_terminal_bookmarks(
        &mut self,
        action: &ShowTerminalBookmarks,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if window.has_active_dialog(cx) {
            return;
        }
        let Some(panel) = self.terminals.get(&action.0) else {
            return;
        };
        let panel = panel.read(cx);
        if !panel.host().is_ssh() || panel.host().os == Some(HostOs::Windows) {
            window.push_notification(
                Notification::error(t!("terminal.bookmarks.unsupported")),
                cx,
            );
            return;
        }
        let view = panel.terminal().read(cx);
        if !view.lifecycle(cx).accepts_input() {
            window.push_notification(
                Notification::error(t!("terminal.command.not_connected")),
                cx,
            );
            return;
        }
        let target = (action.0, view.generation(cx));
        let focus = view.focus_handle();
        let name = panel.host().name.clone();
        let bookmarks = self
            .store
            .read(cx)
            .bookmarks_with_aliases(panel.host_id(), BookmarkSide::Remote);
        open_terminal_bookmarks(
            bookmarks,
            target,
            name,
            self.focus_handle.clone(),
            focus,
            window,
            cx,
        );
    }

    pub(super) fn on_enter_terminal_directory(
        &mut self,
        action: &EnterTerminalDirectory,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(panel) = self.terminals.get(&action.terminal()) else {
            window.push_notification(Notification::error(t!("terminal.bookmarks.expired")), cx);
            return;
        };
        let panel = panel.read(cx);
        if !panel.host().is_ssh() || panel.host().os == Some(HostOs::Windows) {
            window.push_notification(
                Notification::error(t!("terminal.bookmarks.unsupported")),
                cx,
            );
            return;
        }
        let view = panel.terminal().clone();
        if view.read(cx).generation(cx) != action.generation() {
            window.push_notification(Notification::error(t!("terminal.bookmarks.expired")), cx);
            return;
        }
        let Some(command) = directory_command(action.path()) else {
            window.push_notification(
                Notification::error(t!("terminal.bookmarks.invalid_path")),
                cx,
            );
            return;
        };
        match view.update(cx, |view, cx| view.enter_command(&command, false, true, cx)) {
            Ok(()) => window.focus(&view.read(cx).focus_handle(), cx),
            Err(why) => window.push_notification(
                Notification::error(t!("tools.command.enter_failed", error = why)),
                cx,
            ),
        }
    }
}
