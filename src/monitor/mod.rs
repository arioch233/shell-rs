//! 系统监控: the right sidebar's tool that shows the CPU, memory, network
//! and disks of the host of the SSH terminal in front, read over that
//! terminal's own connection. Linux hosts only, for now.

mod access;
mod access_dialog;
mod collectors;
pub(crate) use access_dialog::open_access_dialog;
mod controller;
mod dvpp;
mod dvpp_cards;
mod hardware_cards;
mod linux;
mod mpp;
mod mpp_cards;
mod scheduler;
mod snapshot;
mod tegra;
mod tegra_cards;
pub use snapshot::{CollectorDomain, MonitorPage, MonitorPreferences};
mod model;
mod monitor_panel;

pub use monitor_panel::MonitorPanel;

/// A part of the system monitor that is folded away until asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MonitorDetail {
    /// Every core's load, under the average.
    Cores,
    /// Every network interface, not only the main one.
    Interfaces,
}
