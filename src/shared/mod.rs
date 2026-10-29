//! Small presentation pieces shared by more than one feature.

mod bookmark_label;
mod closable_tab;
mod command_line;
mod detail_row;
mod dialog;
pub mod file_drag;
mod format;
mod host_mark;
mod latency_label;
mod metrics;
mod rename_tab_dialog;
mod row_tooltip;
mod segmented_control;
mod tab_menu;
mod tint;

pub use bookmark_label::BookmarkLabel;
pub use closable_tab::ClosableTabTitle;
pub use command_line::{command_tooltip, one_line};
pub use detail_row::{compact_detail_row, detail_row, stacked_detail_row};
pub use dialog::{
    DeleteHandler, commit_footer, confirm_danger, confirm_danger_with_cancel, confirm_delete,
    dismiss_form_error, form_error_notification, parse_port,
};
pub use format::{format_bytes, format_duration, format_percent};
pub use host_mark::HostMark;
pub use latency_label::LatencyLabel;
pub use metrics::{Load, amount, card, gauge, load_color, meter, meter_with_color, numeric};
pub use rename_tab_dialog::{RenamableTab, open_rename_tab_dialog};
pub use row_tooltip::{RowTooltip, RowTooltipTrigger, RowTooltips};
pub use segmented_control::{Segment, SegmentedControl};
pub use tab_menu::close_tab_items;
pub use tint::{soft_tag, tinted};
