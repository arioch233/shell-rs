//! 终端目录书签搜索，打开时固定主机、终端和连接代次

use gpui_kit::component::{
    ActiveTheme as _, IndexPath, WindowExt as _,
    list::{List, ListDelegate, ListItem, ListState},
};
use gpui_kit::*;

use super::RemoteTerminalId;
use crate::{app::EnterTerminalDirectory, host::Bookmark, i18n::t, shared::BookmarkLabel};

struct BookmarkList {
    bookmarks: Vec<Bookmark>,
    shown: Vec<usize>,
    selected: Option<usize>,
    target: (RemoteTerminalId, u64),
    dispatch: FocusHandle,
}

impl ListDelegate for BookmarkList {
    type Item = ListItem;

    fn perform_search(
        &mut self,
        query: &str,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) -> Task<()> {
        self.shown = self
            .bookmarks
            .iter()
            .enumerate()
            .filter(|(_, bookmark)| bookmark.matches(query))
            .map(|(index, _)| index)
            .collect();
        Task::ready(())
    }

    fn items_count(&self, _: usize, _: &App) -> usize {
        self.shown.len()
    }

    fn set_selected_index(
        &mut self,
        index: Option<IndexPath>,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) {
        self.selected = index.map(|index| index.row);
    }

    fn render_item(
        &mut self,
        index: IndexPath,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) -> Option<ListItem> {
        let bookmark = self.bookmarks.get(*self.shown.get(index.row)?)?;
        Some(
            ListItem::new(SharedString::from(format!(
                "terminal-bookmark:{}",
                bookmark.path()
            )))
            .child(
                div()
                    .id(SharedString::from(format!(
                        "terminal-bookmark-row:{}",
                        bookmark.path()
                    )))
                    .test_support()
                    .aria_label(format!("{} — {}", bookmark.label(), bookmark.path()))
                    .w_full()
                    .min_w_0()
                    .child(BookmarkLabel::new(bookmark.clone())),
            ),
        )
    }

    fn confirm(&mut self, _: bool, window: &mut Window, cx: &mut Context<ListState<Self>>) {
        let Some(bookmark) = self
            .selected
            .and_then(|index| self.shown.get(index))
            .and_then(|index| self.bookmarks.get(*index))
        else {
            return;
        };
        let action =
            EnterTerminalDirectory::new(self.target.0, self.target.1, bookmark.path().to_owned());
        let dispatch = self.dispatch.clone();
        window.close_dialog(cx);
        window.defer(cx, move |window, cx| {
            dispatch.dispatch_action(&action, window, cx)
        });
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        let text = if self.bookmarks.is_empty() {
            t!("terminal.bookmarks.empty")
        } else {
            t!("terminal.bookmarks.no_matches")
        };
        div()
            .id("terminal-bookmarks-empty")
            .test_support()
            .aria_label(text.clone())
            .p_3()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(text)
    }
}

pub(crate) fn open_terminal_bookmarks(
    bookmarks: Vec<Bookmark>,
    target: (RemoteTerminalId, u64),
    name: SharedString,
    dispatch: FocusHandle,
    return_focus: FocusHandle,
    window: &mut Window,
    cx: &mut App,
) {
    let list = cx.new(|cx| {
        ListState::new(
            BookmarkList {
                shown: (0..bookmarks.len()).collect(),
                bookmarks,
                selected: None,
                target,
                dispatch,
            },
            window,
            cx,
        )
        .searchable(true)
    });
    window.open_dialog(cx, {
        let list = list.clone();
        move |dialog, _, cx| {
            dialog
                .title(t!("terminal.bookmarks.title", name = name))
                .overlay_closable(false)
                .child(
                    div()
                        .id("terminal-bookmarks")
                        .test_support()
                        .w_full()
                        .min_w_0()
                        .h(rems(20.))
                        .overflow_hidden()
                        .border_1()
                        .border_color(cx.theme().border)
                        .rounded(cx.theme().radius)
                        .child(
                            List::new(&list).search_placeholder(t!("terminal.bookmarks.search")),
                        ),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("terminal.bookmarks.hint")),
                )
                .on_close({
                    let focus = return_focus.clone();
                    move |_, window, cx| window.focus(&focus, cx)
                })
        }
    });
    list.update(cx, |list, cx| list.focus(window, cx));
}
