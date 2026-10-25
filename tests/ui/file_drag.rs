//! 系统拖出下载的边界测试，替换 Finder 落点但保留真实指针事件与传输队列

use crate::support::*;
use shellrs::app::{ExplorerAction, ExplorerCommand};

#[gpui_kit::test]
async fn native_drag_freezes_sources_and_uses_finders_exact_destination(cx: &mut TestAppContext) {
    let provider = Arc::new(FakeSftpProvider::default());
    let native = Arc::new(FakeFileDrag::default());
    let (handle, _) = open_workspace_with_sftp(cx, provider.clone());
    cx.update(|cx| cx.set_global(FileDragService::new(native.clone())));
    open_test_explorer(cx, handle).await;
    in_frame(cx, handle, |window, cx| {
        window
            .within(("remote-pane", SFTP_TAB))
            .click("name:文件 甲.txt", cx);
        modified_click(
            window,
            ("remote-pane", SFTP_TAB),
            "name:目录",
            gpui_kit::Modifiers::secondary_key(),
            cx,
        );
    });
    cx.run_until_parked();
    drag_remote_file_out(cx, handle, SFTP_TAB, &native);
    assert_eq!(
        *native.files.lock().unwrap(),
        [
            PromisedFile::new("目录", true),
            PromisedFile::new("文件 甲.txt", false)
        ]
    );
    assert!(
        provider.downloads.lock().unwrap().is_empty(),
        "Entering native drag must not download yet"
    );

    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(
            Box::new(ExplorerAction::new(
                ExplorerId(SFTP_TAB),
                ExplorerCommand::Navigate {
                    remote: true,
                    path: "/other".into(),
                },
            )),
            cx,
        )
    });
    let file = native.drop_file(1, "/tmp/Finder 落点/文件 甲 2.txt");
    let folder = native.drop_file(0, "/tmp/Finder 落点/目录 2");
    cx.wait_for(handle.into(), Duration::from_secs(2), |window, cx| {
        window.render_frame(cx);
        !provider.downloads.lock().unwrap().is_empty()
    })
    .await;
    {
        let requests = provider.downloads.lock().unwrap();
        assert_eq!(
            requests[0].sources()[0].as_str(),
            "/home/tester/文件 甲.txt"
        );
        assert_eq!(requests[0].destination().to_str(), Some("/tmp/Finder 落点"));
        assert_eq!(requests[0].target_name(), Some("文件 甲 2.txt"));
    }
    assert!(file.try_recv().is_err());
    in_frame(cx, handle, |window, cx| {
        assert!(window.try_find("download-confirm").is_none());
        window.click("cancel-transfer", cx);
    });
    cx.wait_for(handle.into(), Duration::from_secs(2), |window, cx| {
        window.render_frame(cx);
        window
            .try_find("transfer-status")
            .is_some_and(|status| status.label() == Some("已停止"))
    })
    .await;
    assert!(
        file.try_recv().is_err(),
        "Pause keeps the promise resumable"
    );
    in_frame(cx, handle, |window, cx| {
        window.click("discard-transfer", cx)
    });
    cx.wait_for(handle.into(), Duration::from_secs(2), |window, cx| {
        window.render_frame(cx);
        provider.downloads.lock().unwrap().len() == 2
    })
    .await;
    assert!(file.try_recv().unwrap().is_err());
    {
        let requests = provider.downloads.lock().unwrap();
        assert_eq!(requests[1].sources()[0].as_str(), "/home/tester/目录");
        assert_eq!(requests[1].target_name(), Some("目录 2"));
    }
    provider.events.lock().unwrap()[0]
        .send_blocking(SftpEvent::Disconnected("Connection lost".into()))
        .unwrap();
    cx.wait_for(handle.into(), Duration::from_secs(2), |window, cx| {
        window.render_frame(cx);
        native
            .requests
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .is_closed()
    })
    .await;
    assert!(folder.try_recv().unwrap().is_err());
    assert!(
        native
            .drop_file(0, "/tmp/late")
            .try_recv()
            .unwrap()
            .is_err()
    );
}

#[gpui_kit::test]
async fn cancelling_native_drag_before_drop_starts_no_transfer(cx: &mut TestAppContext) {
    let provider = Arc::new(FakeSftpProvider::default());
    let native = Arc::new(FakeFileDrag::default());
    let (handle, _) = open_workspace_with_sftp(cx, provider.clone());
    cx.update(|cx| cx.set_global(FileDragService::new(native.clone())));
    open_test_explorer(cx, handle).await;
    drag_remote_file_out(cx, handle, SFTP_TAB, &native);
    assert_eq!(native.files.lock().unwrap().len(), 1);
    native.requests.lock().unwrap().take();
    cx.executor().advance_clock(Duration::from_millis(50));
    cx.run_until_parked();
    assert!(provider.downloads.lock().unwrap().is_empty());
    in_frame(cx, handle, |window, _| {
        assert!(window.try_find(("transfer-queue", SFTP_TAB)).is_none())
    });
}
