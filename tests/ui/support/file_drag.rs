//! 可注入的系统文件拖拽边界和真实指针手势，供标签页与侧边栏共用

use super::*;
use shellrs::shared::file_drag::{FileDragProvider, FilePromiseRequest};
pub use shellrs::shared::file_drag::{FileDragService, PromisedFile};

#[derive(Default)]
pub struct FakeFileDrag {
    pub files: Mutex<Vec<PromisedFile>>,
    pub requests: Mutex<Option<async_channel::Sender<FilePromiseRequest>>>,
}

impl FileDragProvider for FakeFileDrag {
    fn start(
        &self,
        files: Vec<PromisedFile>,
        _: &mut gpui_kit::Window,
        _: &mut App,
    ) -> anyhow::Result<async_channel::Receiver<FilePromiseRequest>> {
        *self.files.lock().unwrap() = files;
        let (sender, receiver) = async_channel::unbounded();
        *self.requests.lock().unwrap() = Some(sender);
        Ok(receiver)
    }
}

impl FakeFileDrag {
    pub fn drop_file(&self, item: usize, path: &str) -> mpsc::Receiver<Result<(), String>> {
        let (request, receiver) = FilePromiseRequest::new(item, path.into());
        let _ = self
            .requests
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .try_send(request);
        receiver
    }
}

pub fn drag_remote_file_out(
    cx: &mut TestAppContext,
    handle: WindowHandle<Root>,
    explorer: u64,
    native: &FakeFileDrag,
) {
    in_frame(cx, handle, |window, cx| {
        let from = window
            .within(("remote-pane", explorer))
            .find("name:文件 甲.txt")
            .bounds()
            .center();
        window.dispatch_event(
            gpui_kit::MouseDownEvent {
                position: from,
                button: MouseButton::Left,
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            }
            .to_platform_input(),
            cx,
        );
        for position in [
            from + point(px(10.), px(0.)),
            point(window.viewport_size().width + px(20.), from.y),
        ] {
            window.dispatch_event(
                MouseMoveEvent {
                    position,
                    pressed_button: Some(MouseButton::Left),
                    modifiers: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
        }
        assert!(
            !native.files.lock().unwrap().is_empty(),
            "Native drag must begin during the boundary event, before mouse-up or deferred actions"
        );
        window.dispatch_event(
            gpui_kit::MouseUpEvent {
                position: point(window.viewport_size().width + px(20.), from.y),
                button: MouseButton::Left,
                modifiers: Default::default(),
                click_count: 1,
            }
            .to_platform_input(),
            cx,
        );
    });
    cx.run_until_parked();
}
