//! 书签列表、菜单和终端跳转共用的名称与路径展示

use gpui_kit::component::{ActiveTheme as _, tooltip::Tooltip, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::host::Bookmark;

#[derive(IntoElement)]
pub struct BookmarkLabel {
    bookmark: Bookmark,
}

impl BookmarkLabel {
    pub fn new(bookmark: Bookmark) -> Self {
        Self { bookmark }
    }
}

impl RenderOnce for BookmarkLabel {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let bookmark = self.bookmark;
        let tooltip = bookmark.clone();
        v_flex()
            .id(SharedString::from(format!(
                "bookmark-label:{}",
                bookmark.path()
            )))
            .test_support()
            .w_full()
            .min_w_0()
            .flex_1()
            .overflow_hidden()
            .tooltip(move |window, cx| {
                let bookmark = tooltip.clone();
                Tooltip::element(move |_, cx| {
                    v_flex()
                        .max_w(rems(28.))
                        .gap_1()
                        .whitespace_normal()
                        .child(div().child(bookmark.label().to_owned()))
                        .when(bookmark.alias().is_some(), |this| {
                            this.child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(bookmark.path().to_owned()),
                            )
                        })
                })
                .build(window, cx)
            })
            .child(
                div()
                    .truncate()
                    .when(bookmark.alias().is_none(), |this| {
                        this.text_ellipsis_middle()
                    })
                    .child(bookmark.label().to_owned()),
            )
            .when(bookmark.alias().is_some(), |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .truncate()
                        .text_ellipsis_middle()
                        .child(bookmark.path().to_owned()),
                )
            })
    }
}
