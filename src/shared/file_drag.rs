//! 系统文件拖出适配：系统提供落点，调用方异步写入后完成承诺

use gpui_kit::{App, Global, Window};
use std::{
    path::PathBuf,
    sync::{Arc, mpsc},
};

#[cfg(target_os = "macos")]
mod macos;

/// 拖拽开始时冻结的文件信息，不包含连接或远端路径
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromisedFile {
    name: String,
    directory: bool,
}

impl PromisedFile {
    pub fn new(name: impl Into<String>, directory: bool) -> Self {
        Self {
            name: name.into(),
            directory,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn is_directory(&self) -> bool {
        self.directory
    }
}

/// 一次系统落点请求，丢弃请求会明确通知系统失败，避免 Finder 无限等待
pub struct FilePromiseRequest {
    item: usize,
    target: PathBuf,
    completion: Option<mpsc::Sender<Result<(), String>>>,
}

impl FilePromiseRequest {
    pub fn new(item: usize, target: PathBuf) -> (Self, mpsc::Receiver<Result<(), String>>) {
        let (sender, receiver) = mpsc::channel();
        (
            Self {
                item,
                target,
                completion: Some(sender),
            },
            receiver,
        )
    }

    pub fn item(&self) -> usize {
        self.item
    }
    pub fn target(&self) -> &std::path::Path {
        &self.target
    }

    pub fn finish(mut self, result: Result<(), String>) {
        if let Some(sender) = self.completion.take() {
            let _ = sender.send(result);
        }
    }
}

impl Drop for FilePromiseRequest {
    fn drop(&mut self) {
        if let Some(sender) = self.completion.take() {
            let _ = sender.send(Err("File transfer cancelled".into()));
        }
    }
}

/// 可注入的系统拖拽边界，测试只发送落点请求，不访问真实桌面
pub trait FileDragProvider: Send + Sync + 'static {
    fn start(
        &self,
        files: Vec<PromisedFile>,
        window: &mut Window,
        cx: &mut App,
    ) -> anyhow::Result<async_channel::Receiver<FilePromiseRequest>>;
}

pub struct FileDragService(Arc<dyn FileDragProvider>);
impl Global for FileDragService {}

impl FileDragService {
    pub fn new(provider: Arc<dyn FileDragProvider>) -> Self {
        Self(provider)
    }
}

/// 在文件行收到左键按下时保留原生事件，供离开窗口后的系统拖拽使用
pub fn capture_mouse_down(window: &Window, cx: &mut App) {
    #[cfg(target_os = "macos")]
    macos::capture_mouse_down(window, cx);
    #[cfg(not(target_os = "macos"))]
    let _ = (window, cx);
}

pub fn start(
    files: Vec<PromisedFile>,
    window: &mut Window,
    cx: &mut App,
) -> anyhow::Result<async_channel::Receiver<FilePromiseRequest>> {
    anyhow::ensure!(!files.is_empty(), "No files to drag");
    for file in &files {
        anyhow::ensure!(
            !file.name.is_empty()
                && file.name != "."
                && file.name != ".."
                && !file.name.contains(['/', '\0']),
            "Invalid promised filename"
        );
    }
    if let Some(provider) = cx
        .try_global::<FileDragService>()
        .map(|service| service.0.clone())
    {
        return provider.start(files, window, cx);
    }
    #[cfg(target_os = "macos")]
    {
        macos::start(files, window, cx)
    }
    #[cfg(not(target_os = "macos"))]
    {
        anyhow::bail!("File promises are not supported on this platform")
    }
}
