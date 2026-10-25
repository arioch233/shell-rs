//! AppKit 文件承诺桥接，网络写入由调用方执行，主线程只负责拖拽

use super::{FilePromiseRequest, PromisedFile};
use anyhow::{Context as _, anyhow};
use gpui_kit::{App, Global, Window, WindowId};
use objc2::{
    AnyThread, DefinedClass, MainThreadOnly, define_class, msg_send, rc::Retained,
    runtime::ProtocolObject, sel,
};
use objc2_app_kit::{
    NSApplication, NSDragOperation, NSDraggingContext, NSDraggingItem, NSDraggingSession,
    NSDraggingSource, NSEvent, NSEventType, NSFilePromiseProvider, NSFilePromiseProviderDelegate,
    NSView, NSWindow, NSWorkspace,
};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSDictionary, NSError, NSLocalizedDescriptionKey, NSObject,
    NSObjectProtocol, NSOperationQueue, NSPoint, NSRect, NSSize, NSString, NSURL,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

struct PromiseIvars {
    item: usize,
    name: String,
    requests: async_channel::Sender<FilePromiseRequest>,
    queue: Retained<NSOperationQueue>,
}

/// 同一时刻只有一次鼠标手势，新的按下替换旧事件，开始拖出时取走
struct DragMouseDown {
    window: WindowId,
    event: Option<Retained<NSEvent>>,
}
impl Global for DragMouseDown {}

pub(super) fn capture_mouse_down(window: &Window, cx: &mut App) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let event = NSApplication::sharedApplication(mtm)
        .currentEvent()
        .filter(|event| event.r#type() == NSEventType::LeftMouseDown);
    cx.set_global(DragMouseDown {
        window: window.window_handle().window_id(),
        event,
    });
}

struct DragSourceIvars {
    ended: Arc<AtomicBool>,
    window: Retained<NSWindow>,
}

define_class!(
    // 安全性：NSObject 无额外子类约束，字段仅包含线程安全的消息通道和不可变信息
    #[unsafe(super = NSObject)]
    #[name = "ShellRSFilePromiseDelegate"]
    #[ivars = PromiseIvars]
    struct PromiseDelegate;

    unsafe impl NSObjectProtocol for PromiseDelegate {}

    unsafe impl NSFilePromiseProviderDelegate for PromiseDelegate {
        #[unsafe(method_id(filePromiseProvider:fileNameForType:))]
        fn filename(&self, _: &NSFilePromiseProvider, _: &NSString) -> Retained<NSString> {
            NSString::from_str(&self.ivars().name)
        }

        #[unsafe(method_id(operationQueueForFilePromiseProvider:))]
        fn operation_queue(&self, _: &NSFilePromiseProvider) -> Retained<NSOperationQueue> {
            self.ivars().queue.clone()
        }

        #[unsafe(method(filePromiseProvider:writePromiseToURL:completionHandler:))]
        fn write(
            &self,
            _: &NSFilePromiseProvider,
            url: &NSURL,
            completion: &block2::DynBlock<dyn Fn(*mut NSError)>,
        ) {
            // AppKit 已在文件协调写锁内调用此方法，再建协调器会等待自己释放锁
            // 仅在私有工作队列等待下载，完成回调交还 AppKit 的外层协调锁
            let result = (|| {
                anyhow::ensure!(
                    url.isFileURL(),
                    "File promise destination is not a file URL"
                );
                let target = url
                    .to_file_path()
                    .context("Invalid file promise destination")?;
                let (request, reply) = FilePromiseRequest::new(self.ivars().item, target);
                self.ivars()
                    .requests
                    .try_send(request)
                    .map_err(|_| anyhow!("File transfer cancelled"))?;
                reply
                    .recv()
                    .context("File transfer cancelled")?
                    .map_err(anyhow::Error::msg)
            })();
            finish(completion, result);
        }
    }
);

fn finish(completion: &block2::DynBlock<dyn Fn(*mut NSError)>, result: anyhow::Result<()>) {
    match result {
        Ok(()) => completion.call((std::ptr::null_mut(),)),
        Err(error) => {
            let description = NSString::from_str(&error.to_string());
            // 安全性：错误字典使用 Foundation 规定的字符串键和值，回调期间保持对象存活
            let error = unsafe {
                let info = NSDictionary::from_slices(
                    &[NSLocalizedDescriptionKey],
                    &[&*description as &objc2::runtime::AnyObject],
                );
                NSError::errorWithDomain_code_userInfo(
                    &NSString::from_str("com.shellrs.FilePromise"),
                    1,
                    Some(&info),
                )
            };
            completion.call((Retained::as_ptr(&error).cast_mut(),));
        }
    }
}

define_class!(
    // 安全性：拖拽源的 AppKit 回调限定在主线程，结束标记跨异步任务共享
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "ShellRSFileDraggingSource"]
    #[ivars = DragSourceIvars]
    struct DragSource;

    unsafe impl NSObjectProtocol for DragSource {}

    unsafe impl NSDraggingSource for DragSource {
        #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
        fn operation(&self, _: &NSDraggingSession, context: NSDraggingContext) -> NSDragOperation {
            if context == NSDraggingContext::OutsideApplication {
                NSDragOperation::Copy
            } else {
                NSDragOperation::None
            }
        }

        #[unsafe(method(ignoreModifierKeysForDraggingSession:))]
        fn ignore_modifiers(&self, _: &NSDraggingSession) -> bool {
            true
        }

        #[unsafe(method(draggingSession:endedAtPoint:operation:))]
        fn ended(&self, session: &NSDraggingSession, point: NSPoint, operation: NSDragOperation) {
            self.ivars().ended.store(true, Ordering::Release);
            // 转交给 GPUI 原生窗口，结束其合成拖动定时器并清理按下事件
            // 安全性：该选择器属于 NSDraggingSource，调用前确认窗口实现了它
            let window = &self.ivars().window;
            if window.respondsToSelector(sel!(draggingSession:endedAtPoint:operation:)) {
                unsafe {
                    let _: () = msg_send![window, draggingSession: session, endedAtPoint: point, operation: operation];
                }
            }
        }
    }
);

pub(super) fn start(
    files: Vec<PromisedFile>,
    window: &mut Window,
    cx: &mut App,
) -> anyhow::Result<async_channel::Receiver<FilePromiseRequest>> {
    let mtm = MainThreadMarker::new().context("File dragging requires the main thread")?;
    let handle = HasWindowHandle::window_handle(window).map_err(|error| anyhow!("{error}"))?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        anyhow::bail!("Not an AppKit window")
    };
    // 安全性：GPUI 的窗口句柄提供本窗口的 NSView，使用期间窗口保持存活
    let view = unsafe { &*handle.ns_view.as_ptr().cast::<NSView>() };
    // currentEvent 在出窗口和延迟动作之间可能已是退出、抬键或其他事件
    anyhow::ensure!(
        cx.try_global::<DragMouseDown>()
            .is_some_and(|gesture| gesture.window == Window::window_handle(window).window_id()),
        "No mouse-down event for this window"
    );
    let event = cx
        .global_mut::<DragMouseDown>()
        .event
        .take()
        .context("No mouse-down event for file dragging")?;
    let position = view.convertPoint_fromView(event.locationInWindow(), None);
    let (sender, receiver) = async_channel::unbounded();
    let queue = NSOperationQueue::new();
    queue.setMaxConcurrentOperationCount(1);
    let mut items = Vec::with_capacity(files.len());
    for (item, file) in files.into_iter().enumerate() {
        let delegate = PromiseDelegate::alloc().set_ivars(PromiseIvars {
            item,
            name: file.name.clone(),
            requests: sender.clone(),
            queue: queue.clone(),
        });
        // 安全性：调用 NSObject 初始化，随后通过 userInfo 强持有 weak delegate
        let delegate: Retained<PromiseDelegate> = unsafe { msg_send![super(delegate), init] };
        let file_type = NSString::from_str(if file.directory {
            "public.folder"
        } else {
            "public.data"
        });
        let provider = NSFilePromiseProvider::initWithFileType_delegate(
            NSFilePromiseProvider::alloc(),
            &file_type,
            ProtocolObject::from_ref(&*delegate),
        );
        unsafe {
            provider.setUserInfo(Some(&delegate));
        }
        let dragging_item = NSDraggingItem::initWithPasteboardWriter(
            NSDraggingItem::alloc(),
            ProtocolObject::from_ref(&*provider),
        );
        #[allow(deprecated)]
        let icon = NSWorkspace::sharedWorkspace().iconForFileType(&file_type);
        // 系统拖拽图标的点尺寸属于原生 API 边界，不参与应用布局
        unsafe {
            dragging_item.setDraggingFrame_contents(
                NSRect::new(position, NSSize::new(32., 32.)),
                Some(&icon),
            );
        }
        items.push(dragging_item);
    }
    let ended = Arc::new(AtomicBool::new(false));
    let source = DragSource::alloc(mtm).set_ivars(DragSourceIvars {
        ended: ended.clone(),
        window: view
            .window()
            .context("File dragging requires an attached view")?,
    });
    // 安全性：调用 NSObject 初始化，源对象由前台任务保留到原生拖拽结束
    let source: Retained<DragSource> = unsafe { msg_send![super(source), init] };
    let session = view.beginDraggingSessionWithItems_event_source(
        &NSArray::from_retained_slice(&items),
        &event,
        ProtocolObject::from_ref(&*source),
    );
    cx.spawn(async move |cx| {
        while !ended.load(Ordering::Acquire) {
            cx.background_executor()
                .timer(Duration::from_millis(16))
                .await;
        }
        drop((session, source));
    })
    .detach();
    Ok(receiver)
}

#[cfg(test)]
mod tests {
    use super::*;
    use block2::StackBlock;
    use objc2_foundation::{NSFileCoordinator, NSFileCoordinatorWritingOptions};
    use std::{ptr::NonNull, sync::mpsc};

    #[test]
    fn native_promise_inside_appkit_coordination_completes_or_cancels() {
        for succeed in [true, false] {
            let directory = tempfile::tempdir().unwrap();
            let target = directory.path().join("报告 2.txt");
            let expected = target.clone();
            let (requests, receiver) = async_channel::unbounded();
            let (finished, result) = mpsc::channel();
            let worker = std::thread::spawn(move || {
                objc2::rc::autoreleasepool(|_| {
                    let delegate = PromiseDelegate::alloc().set_ivars(PromiseIvars {
                        item: 3,
                        name: "报告.txt".into(),
                        requests,
                        queue: NSOperationQueue::new(),
                    });
                    // 安全性：使用与生产路径相同的 NSObject 初始化和原生协议调用
                    let delegate: Retained<PromiseDelegate> =
                        unsafe { msg_send![super(delegate), init] };
                    let provider = NSFilePromiseProvider::initWithFileType_delegate(
                        NSFilePromiseProvider::alloc(),
                        &NSString::from_str("public.data"),
                        ProtocolObject::from_ref(&*delegate),
                    );
                    let completion = StackBlock::new(move |error: *mut NSError| {
                        let result = if error.is_null() {
                            Ok(())
                        } else {
                            // 安全性：错误对象在完成回调期间有效
                            Err(unsafe { &*error }.localizedDescription().to_string())
                        };
                        finished.send(result).unwrap();
                    });
                    // Finder 经 AppKit 持有外层协调锁后才调用 delegate
                    // 直接调用 delegate 无法发现重复申请同一路径写锁造成的死锁
                    let writer = StackBlock::new(|url: NonNull<NSURL>| {
                        // 安全性：协调器保证 accessor 期间 URL 有效
                        let url = unsafe { url.as_ref() };
                        ProtocolObject::<dyn NSFilePromiseProviderDelegate>::from_ref(&*delegate)
                            .filePromiseProvider_writePromiseToURL_completionHandler(
                                &provider,
                                url,
                                &completion,
                            );
                    });
                    let mut error = None;
                    NSFileCoordinator::new().coordinateWritingItemAtURL_options_error_byAccessor(
                        &NSURL::from_file_path(&target).unwrap(),
                        NSFileCoordinatorWritingOptions::empty(),
                        Some(&mut error),
                        &writer,
                    );
                    assert!(error.is_none(), "Outer coordination failed: {error:?}");
                })
            });
            // 旧实现会卡在第二把写锁上，以有界等待报告回归而非挂住整个测试
            let request = async_io::block_on(async {
                use futures::FutureExt as _;
                futures::select_biased! {
                    request = receiver.recv().fuse() => request.expect("Native request channel closed"),
                    _ = async_io::Timer::after(Duration::from_secs(5)).fuse() => {
                        panic!("Native promise blocked under AppKit's write coordination")
                    }
                }
            });
            assert_eq!(request.item(), 3);
            assert_eq!(request.target(), expected);
            assert!(!expected.exists());
            if succeed {
                std::fs::write(request.target(), "downloaded bytes").unwrap();
                request.finish(Ok(()));
            } else {
                drop(request);
            }
            assert_eq!(
                result.recv_timeout(Duration::from_secs(3)).unwrap().is_ok(),
                succeed
            );
            worker.join().unwrap();
            if succeed {
                assert_eq!(
                    std::fs::read_to_string(expected).unwrap(),
                    "downloaded bytes"
                );
            } else {
                assert!(!expected.exists());
            }
            assert!(result.try_recv().is_err());
        }
    }
}
