//! GPU/NPU: the right sidebar's tool for the accelerators of an AI host,
//! read over the SSH terminal's own connection: how busy each is, its
//! memory, heat and health, and the processes computing on them, which it
//! ends. NVIDIA GPUs (`nvidia-smi`) and Huawei Ascend NPUs (`npu-smi`), on
//! Linux hosts.

mod gpu_panel;
mod linux;
mod model;

pub use gpu_panel::GpuPanel;

// 统一监控通过明确边界复用解析器与数据模型
pub(crate) use linux::{Parsed as Collected, command as collect_command, parse as parse_reading};
pub(crate) use model::Reading as AcceleratorReading;

pub(crate) use gpu_panel::GpuCards;
