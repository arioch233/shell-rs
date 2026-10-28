//! 系统拖出下载：冻结来源及连接代次，将系统落点接入现有队列

use super::{ExplorerPanel, QueueEntry, QueueState, TransferJob, explorer_panel::transfer_command};
use crate::{
    host::ConnectionState,
    i18n::t,
    shared::file_drag::{self, FilePromiseRequest, PromisedFile},
};
use gpui_kit::component::{WindowExt as _, notification::Notification};
use gpui_kit::{Context, Window};
use std::time::Duration;

impl ExplorerPanel {
    pub(super) fn start_external_download(
        &mut self,
        paths: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.connection_state() != ConnectionState::Connected || paths.is_empty() {
            return;
        }
        let remote = self.remote.read(cx);
        let entries = remote.listed_entries(cx);
        let files = paths
            .iter()
            .map(|path| {
                let entry = entries.iter().find(|entry| {
                    !entry.is_parent() && remote.child_path_of(&entry.name) == *path
                })?;
                Some(PromisedFile::new(entry.name.to_string(), entry.is_dir()))
            })
            .collect::<Option<Vec<_>>>();
        let Some(files) = files else {
            return;
        };
        let requests = match file_drag::start(files, window, cx) {
            Ok(requests) => requests,
            Err(error) => {
                window.push_notification(
                    Notification::error(t!("explorer.drag.start_failed", error = error)),
                    cx,
                );
                return;
            }
        };
        cx.stop_active_drag(window);
        let generation = self.generation();
        let task = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                let pending: Vec<_> = (0..128).map_while(|_| requests.try_recv().ok()).collect();
                if !pending.is_empty()
                    && this
                        .update_in(cx, |panel, window, cx| {
                            for request in pending {
                                if panel.generation() == generation
                                    && panel.connection_state() == ConnectionState::Connected
                                    && let Some(source) = paths.get(request.item())
                                {
                                    panel.enqueue_file_promise(source.clone(), request, window, cx);
                                }
                            }
                        })
                        .is_err()
                {
                    break;
                }
                if requests.is_closed() && requests.is_empty() {
                    break;
                }
                if this.upgrade().is_none() {
                    break;
                }
            }
        });
        self.external_drags.push(task);
    }

    fn enqueue_file_promise(
        &mut self,
        source: String,
        request: FilePromiseRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = request.target().to_str() else {
            request.finish(Err("Destination is not valid UTF-8".into()));
            return;
        };
        let job = TransferJob::DownloadToPath {
            source,
            target: target.to_string(),
        };
        if let Err(error) = transfer_command(&job) {
            request.finish(Err(crate::sftp::describe_sftp_error(&error)));
            return;
        }
        let id = self.queue.push(job);
        self.file_promises.insert(id, request);
        self.start_next_if_idle(window, cx);
        cx.notify();
    }

    /// 只有完整成功才兑现承诺，跳过或部分失败不能让 Finder 报告成功
    pub(super) fn finish_file_promises(&mut self) {
        let finished: Vec<_> = self
            .file_promises
            .keys()
            .filter_map(|id| {
                let entry = self.queue.entries().iter().find(|entry| entry.id() == *id);
                promise_result(entry).map(|result| (*id, result))
            })
            .collect();
        for (id, result) in finished {
            if let Some(request) = self.file_promises.remove(&id) {
                request.finish(result);
            }
        }
    }

    pub(super) fn cancel_file_promises(&mut self) {
        self.external_drags.clear();
        for (id, _) in self.file_promises.drain() {
            self.queue.remove(id);
        }
    }
}

/// 停止代表可继续，只有完成或移出队列才结束系统承诺
fn promise_result(entry: Option<&QueueEntry>) -> Option<Result<(), String>> {
    match entry {
        Some(entry) if entry.state() == QueueState::Done => {
            let success = entry
                .progress()
                .is_some_and(|p| p.failed() == 0 && p.skipped() == 0 && p.succeeded() > 0);
            Some(if success {
                Ok(())
            } else {
                Err("Download did not complete all items".into())
            })
        }
        Some(entry) if entry.state() != QueueState::Removed => None,
        _ => Some(Err("File transfer cancelled".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::{promise_result, transfer_command};
    use crate::{
        explorer::{TransferJob, TransferQueue},
        sftp::{SftpCommand, TransferDetail, TransferPhase, TransferProgress},
    };

    #[test]
    fn finder_renamed_destination_reaches_the_download_engine() {
        let job = TransferJob::DownloadToPath {
            source: "/远程/报告.txt".into(),
            target: "/tmp/报告 2.txt".into(),
        };
        let SftpCommand::Download(request) = transfer_command(&job).unwrap() else {
            panic!("Expected download")
        };
        assert_eq!(request.target_name(), Some("报告 2.txt"));
        assert_eq!(request.destination().to_str(), Some("/tmp"));
        assert_eq!(request.sources()[0].as_str(), "/远程/报告.txt");
        assert!(!request.is_scp());
        assert_eq!(job.target(), "/tmp/报告 2.txt");
        assert_eq!(job.source_label(), "/远程/报告.txt");
        assert!(
            transfer_command(&TransferJob::DownloadToPath {
                source: "/a".into(),
                target: "relative/a".into()
            })
            .is_err()
        );
    }

    #[test]
    fn promise_waits_through_pause_and_rejects_partial_downloads() {
        for (detail, succeeded) in [
            (TransferDetail::done("/tmp/a"), true),
            (TransferDetail::skipped("/tmp/a"), false),
            (
                TransferDetail::failed("/tmp/a", anyhow::anyhow!("denied")),
                false,
            ),
        ] {
            let mut queue = TransferQueue::default();
            queue.push(TransferJob::DownloadToPath {
                source: "/a".into(),
                target: "/tmp/a".into(),
            });
            queue.start_next();
            assert_eq!(promise_result(queue.head()), None);
            queue.on_progress(TransferProgress::new(TransferPhase::Stopped));
            assert_eq!(promise_result(queue.head()), None);
            queue.on_progress(
                TransferProgress::new(TransferPhase::Completed).with_details(vec![detail]),
            );
            assert_eq!(
                promise_result(queue.entries().first()).unwrap().is_ok(),
                succeeded
            );
        }
        assert!(promise_result(None).unwrap().is_err());
    }
}
