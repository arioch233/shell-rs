//! 资源详情中对齐且可选择复制的字段
use gpui_kit::base::SelectableText;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::*;

pub fn detail_row(
    id: ElementId,
    label: SharedString,
    value: SharedString,
    cx: &App,
) -> impl IntoElement + InteractiveElement + ParentElement + Styled + use<> {
    h_flex()
        .id(id.clone())
        .test_support()
        .aria_label(value.clone())
        .items_start()
        .gap_3()
        .px_3()
        .py_2()
        .text_sm()
        .child(
            div()
                .w(rems(9.))
                .flex_shrink_0()
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
        .child(
            div()
                .id((id.clone(), "value-cell"))
                .test_support()
                .flex_1()
                .min_w_0()
                .child(
                    SelectableText::new((id, "value"), value).selection_color(cx.theme().selection),
                ),
        )
}

/// 窄面板的上下布局，长路径与多行内容使用整行宽度并保留原文供复制
pub fn stacked_detail_row(
    id: ElementId,
    label: SharedString,
    value: SharedString,
    cx: &App,
) -> impl IntoElement + InteractiveElement + ParentElement + Styled + use<> {
    v_flex()
        .id(id.clone())
        .test_support()
        .aria_label(value.clone())
        .w_full()
        .min_w_0()
        .gap_1()
        .text_xs()
        .child(div().text_color(cx.theme().muted_foreground).child(label))
        .child(
            div()
                .id((id.clone(), "value-cell"))
                .test_support()
                .w_full()
                .min_w_0()
                .whitespace_normal()
                .text_color(cx.theme().foreground)
                .child(
                    SelectableText::new((id, "value"), value).selection_color(cx.theme().selection),
                ),
        )
}

/// 窄面板的紧凑键值行，短数值右对齐，长标签和值可各自换行
pub fn compact_detail_row(
    id: ElementId,
    label: SharedString,
    value: SharedString,
    cx: &App,
) -> impl IntoElement + StatefulInteractiveElement + ParentElement + Styled + use<> {
    h_flex()
        .id(id.clone())
        .test_support()
        .aria_label(value.clone())
        .w_full()
        .min_w_0()
        .items_start()
        .gap_3()
        .text_xs()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
        .child(
            div()
                .id((id.clone(), "value-cell"))
                .test_support()
                .flex_1()
                .min_w_0()
                .text_right()
                .whitespace_normal()
                .text_color(cx.theme().foreground)
                .font_family(cx.theme().mono_font_family.clone())
                .child(
                    SelectableText::new((id, "value"), value).selection_color(cx.theme().selection),
                ),
        )
}
