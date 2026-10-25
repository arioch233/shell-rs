//! GPU/NPU: the right sidebar's tool for an AI host's accelerators, read
//! over the SSH terminal's own connection; and 侧栏功能, which offers it.

use gpui_kit::SharedString;
use shellrs::app::{EndProcess, RefreshGpu, ToolKind};
use shellrs::monitor::MonitorPage;

use crate::support::*;

/// Eight H100s, the first four busy with a job of alice's, which `ps`
/// names; a fifth process `ps` could not see.
fn eight_gpus() -> String {
    let mut output = String::from("@@uptime\n5359162.33\n@@nvidia\n");
    for index in 0..8u32 {
        let busy = index < 4;
        output += &format!(
            "{index}, GPU-{index:08}-0000-0000-0000-000000000000, NVIDIA H100 80GB HBM3, \
             00000000:{:02X}:00.0, 550.54.15, {}, {}, {}, 81559, {}, {}, 700.00, [N/A], 0\n",
            0x18 + index,
            if busy { "P0" } else { "P8" },
            if busy { 96 } else { 0 },
            if busy { 61440 } else { 1 },
            if busy { 70 } else { 30 },
            if busy { "600.00" } else { "70.00" },
        );
    }
    output += "@@status 0\n@@nvidia-apps\n";
    for index in 0..4u32 {
        output += &format!(
            "GPU-{index:08}-0000-0000-0000-000000000000, {}, 60000, /usr/bin/python3\n",
            4242 + index
        );
    }
    output += "GPU-00000001-0000-0000-0000-000000000000, 999, 1024, /opt/infer\n";
    output += "@@status 0\n@@ps\n";
    for index in 0..4u32 {
        output += &format!("   {} alice    torchrun train.py\n", 4242 + index);
    }
    output += "@@end\n";
    output
}

/// What the panel says of the eight, up 62 days and some.
const EIGHT_SUMMARY: &str = "共 8 个设备 · 已运行 62 天 00:39:22";

/// One Ascend 910B3 of an Atlas 800T A2, idle.
const ONE_NPU: &str = "\
@@npu
+------------------------------------------------------------------------------------------------+
| npu-smi 24.1.0.3                 Version: 24.1.0.3                                             |
+---------------------------+---------------+----------------------------------------------------+
| NPU   Name                | Health        | Power(W)    Temp(C)           Hugepages-Usage(page)|
| Chip                      | Bus-Id        | AICore(%)   Memory-Usage(MB)  HBM-Usage(MB)        |
+===========================+===============+====================================================+
| 0     910B3               | OK            | 92.9        37                0    / 0             |
| 0                         | 0000:C1:00.0  | 0           0    / 0          3379 / 65536         |
+===========================+===============+====================================================+
+---------------------------+---------------+----------------------------------------------------+
| NPU     Chip              | Process id    | Process name             | Process memory(MB)      |
+===========================+===============+====================================================+
| No running processes found in NPU 0                                                            |
+===========================+===============+====================================================+
@@status 0
@@end
";

/// 旧 GPU 样例按采集域注入，结束进程的响应独立
fn open_gpu(
    cx: &mut TestAppContext,
    factory: Arc<FakeTerminalFactory>,
) -> (WindowHandle<Root>, Entity<Workspace>) {
    let samples: Vec<String> = factory
        .exec_outputs
        .lock()
        .unwrap()
        .iter()
        .filter(|sample| sample.as_str() != "@@status 0\n")
        .cloned()
        .collect();
    factory.monitor_answers(
        "accelerator",
        &samples.iter().map(String::as_str).collect::<Vec<_>>(),
    );
    *factory.exec_outputs.lock().unwrap() = ["@@status 0\n".to_owned()].into();
    let (handle, workspace) = open_workspace_with_remote_factory(cx, HostStore::seed(), factory);
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, cx| window.click("tool-monitor", cx));
    page(cx, handle, MonitorPage::Hardware);
    (handle, workspace)
}
fn page(cx: &mut TestAppContext, handle: WindowHandle<Root>, page: MonitorPage) {
    in_frame(cx, handle, |window, cx| {
        window.within("monitor-pages").click(
            match page {
                MonitorPage::Overview => 0usize,
                MonitorPage::Hardware => 1,
                MonitorPage::Processes => 2,
            },
            cx,
        )
    });
}
fn gpu_processes(cx: &mut TestAppContext, handle: WindowHandle<Root>) {
    page(cx, handle, MonitorPage::Processes);
    in_frame(cx, handle, |window, cx| {
        window.within("monitor-process-source").click(1usize, cx)
    });
}
async fn wait_for_label(
    cx: &mut TestAppContext,
    handle: WindowHandle<Root>,
    id: impl Into<ElementId> + Clone,
    label: &str,
) {
    cx.wait_for(handle.into(), Duration::from_secs(5), |window, _| {
        window
            .try_find(id.clone())
            .is_some_and(|element| element.label() == Some(label))
    })
    .await;
}
fn label(window: &mut gpui_kit::Window, id: impl Into<ElementId>) -> Option<String> {
    window.find(id).label().map(str::to_owned)
}
fn detail_id(device: usize, name: &str) -> ElementId {
    ElementId::Name(format!("gpu-detail:{device}:{name}").into())
}

#[gpui_kit::test]
async fn eight_gpus_keep_summaries_devices_and_processes_in_the_unified_monitor(
    cx: &mut TestAppContext,
) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[&eight_gpus()]));
    let (handle, _) = open_gpu(cx, factory.clone());
    wait_for_label(cx, handle, "gpu-summary", EIGHT_SUMMARY).await;
    in_frame(cx, handle, |window, _| {
        assert!(window.try_find("tool-gpu").is_none());
        assert_eq!(
            label(window, "gpu-model").as_deref(),
            Some("NVIDIA H100 80GB HBM3 · 驱动 550.54.15")
        );
        assert_eq!(label(window, "gpu-utilization").as_deref(), Some("48%"));
        assert_eq!(label(window, "gpu-memory").as_deref(), Some("38%"));
        assert_eq!(
            label(window, "gpu-memory-amount").as_deref(),
            Some("240.0 / 637 GB")
        );
        assert_eq!(label(window, "gpu-temperature").as_deref(), Some("70 °C"));
        assert_eq!(label(window, "gpu-power").as_deref(), Some("2680 W"));
        assert_eq!(
            label(window, ("gpu-device", 0u64)).as_deref(),
            Some("GPU 0 · NVIDIA H100 80GB HBM3 · 利用率 96% · 显存 60.0 / 80 GB · 70 °C · 600 W")
        );
        assert_eq!(
            label(window, ("gpu-device", 7u64)).as_deref(),
            Some("GPU 7 · NVIDIA H100 80GB HBM3 · 利用率 0% · 显存 1 MB / 80 GB · 30 °C · 70 W")
        );
        assert!(window.try_find(("gpu-device", 8u64)).is_none());
    });
    gpu_processes(cx, handle);
    in_frame(cx, handle, |window, _| {
        assert_eq!(label(window, "gpu-processes-count").as_deref(), Some("5"));
        assert_eq!(
            label(window, ("gpu-process", 4242u64)).as_deref(),
            Some("torchrun train.py · GPU 0 · PID 4242 · alice · 显存 58.6 GB")
        );
        assert_eq!(
            label(window, ("gpu-process", 999u64)).as_deref(),
            Some("/opt/infer · GPU 1 · PID 999 · — · 显存 1.0 GB")
        );
        assert!(
            window.find(("gpu-process", 4243u64)).bounds().top()
                < window.find(("gpu-process", 999u64)).bounds().top()
        );
    });
    in_frame(cx, handle, |window, cx| window.click("tool-monitor", cx));
    let count = factory.monitor_count("accelerator");
    cx.executor().advance_clock(Duration::from_secs(10));
    cx.run_until_parked();
    assert_eq!(factory.monitor_count("accelerator"), count);
    assert!(factory.monitor_peak.load(Ordering::SeqCst) <= 2);
}

#[gpui_kit::test]
async fn gpu_details_and_fold_all_keep_their_actions(cx: &mut TestAppContext) {
    let (handle, _) = open_gpu(
        cx,
        Arc::new(FakeTerminalFactory::answering(&[&eight_gpus()])),
    );
    wait_for_label(cx, handle, "gpu-summary", EIGHT_SUMMARY).await;
    in_frame(cx, handle, |window, cx| {
        window.click(
            "gpu-device-toggle:GPU-00000000-0000-0000-0000-000000000000",
            cx,
        )
    });
    in_frame(cx, handle, |window, _| {
        for (name, value) in [
            ("model", "NVIDIA H100 80GB HBM3"),
            ("uuid", "GPU-00000000-0000-0000-0000-000000000000"),
            ("bus", "00000000:18:00.0"),
            ("pstate", "P0"),
            ("power", "600 / 700 W"),
            ("health", "正常"),
        ] {
            assert_eq!(label(window, detail_id(0, name)).as_deref(), Some(value));
        }
        assert!(window.try_find(detail_id(0, "fan")).is_none());
        assert!(window.try_find(("gpu-device-details", 3u64)).is_none());
    });
    in_frame(cx, handle, |window, cx| {
        window.click("gpu-devices-fold", cx)
    });
    in_frame(cx, handle, |window, _| {
        for id in 0..8u64 {
            assert!(window.try_find(("gpu-device-details", id)).is_some());
        }
    });
    in_frame(cx, handle, |window, cx| {
        window.click("gpu-devices-fold", cx)
    });
    in_frame(cx, handle, |window, _| {
        for id in 0..8u64 {
            assert!(window.try_find(("gpu-device-details", id)).is_none());
        }
    });
}

#[gpui_kit::test]
async fn ascend_keeps_details_and_empty_process_state(cx: &mut TestAppContext) {
    let (handle, _) = open_gpu(cx, Arc::new(FakeTerminalFactory::answering(&[ONE_NPU])));
    wait_for_label(cx, handle, "gpu-summary", "共 1 个设备").await;
    in_frame(cx, handle, |window, cx| {
        assert!(window.try_find(("gpu-device-details", 0u64)).is_none());
        assert!(window.try_find("gpu-overview").is_none());
        assert_eq!(
            label(window, ("gpu-device-name", 0u64)).as_deref(),
            Some("910B3")
        );
        let memory = window.find("gpu-device-memory-0").bounds();
        let temperature = window.find(("gpu-device-temperature", 0u64)).bounds();
        let power = window.find(("gpu-device-power", 0u64)).bounds();
        assert_eq!(memory.left(), temperature.left());
        assert_eq!(temperature.right(), power.right());
        assert!(memory.bottom() <= temperature.top());
        assert!(temperature.bottom() <= power.top());
        window.click("gpu-device-toggle:0000:C1:00.0", cx)
    });
    in_frame(cx, handle, |window, _| {
        assert_eq!(
            label(window, "gpu-model").as_deref(),
            Some("910B3 · 驱动 24.1.0.3")
        );
        assert_eq!(
            label(window, detail_id(0, "bus")).as_deref(),
            Some("0000:C1:00.0")
        );
        assert_eq!(
            label(window, detail_id(0, "power")).as_deref(),
            Some("93 W")
        );
        assert!(window.try_find("gpu-devices-fold").is_none());
    });
    gpu_processes(cx, handle);
    in_frame(cx, handle, |window, _| {
        assert_eq!(
            label(window, "gpu-no-processes").as_deref(),
            Some("没有计算进程")
        );
    });
}

#[gpui_kit::test]
async fn ending_a_gpu_process_confirms_then_refreshes_its_domain(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[&eight_gpus()]));
    let (handle, _) = open_gpu(cx, factory.clone());
    wait_for_label(cx, handle, "gpu-summary", EIGHT_SUMMARY).await;
    gpu_processes(cx, handle);
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(
            Box::new(EndProcess {
                pid: 4242,
                force: false,
            }),
            cx,
        )
    });
    assert!(
        !factory
            .exec_commands()
            .iter()
            .any(|command| command.contains("kill -TERM"))
    );
    let before = factory.monitor_count("accelerator");
    in_frame(cx, handle, |window, cx| window.click("ok", cx));
    cx.wait_for(handle.into(), Duration::from_secs(5), |_, _| {
        factory.monitor_count("accelerator") > before
    })
    .await;
    assert!(
        factory
            .exec_commands()
            .contains(&"sh -c 'kill -TERM 4242 2>&1; echo @@status $?'".to_owned())
    );
}

#[gpui_kit::test]
async fn missing_gpu_tools_pause_only_the_accelerator_domain(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&["@@end\n"]));
    let (handle, _) = open_gpu(cx, factory.clone());
    wait_for_label(cx, handle, "monitor-no-hardware", "未检测到可用的硬件指标").await;
    let before = factory.monitor_count("accelerator");
    cx.executor().advance_clock(Duration::from_secs(10));
    cx.run_until_parked();
    assert_eq!(factory.monitor_count("accelerator"), before);
}

#[gpui_kit::test]
async fn a_timed_out_driver_is_not_restarted_by_repeated_refresh(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[
        "@@nvidia\n\n@@status 137\n@@end\n",
    ]));
    let (handle, _) = open_gpu(cx, factory.clone());
    cx.wait_for(handle.into(), Duration::from_secs(5), |window, _| {
        window
            .try_find("monitor-status-accelerator")
            .is_some_and(|element| {
                element
                    .label()
                    .is_some_and(|label| label.contains("采样超时"))
            })
    })
    .await;
    let before = factory.monitor_count("accelerator");
    for _ in 0..3 {
        in_frame(cx, handle, |window, cx| {
            window.dispatch_action(Box::new(RefreshGpu), cx)
        });
    }
    cx.executor().advance_clock(Duration::from_secs(10));
    cx.run_until_parked();
    assert_eq!(factory.monitor_count("accelerator"), before);
}

#[gpui_kit::test]
fn legacy_gpu_selection_opens_hardware_and_new_writes_use_monitor(cx: &mut TestAppContext) {
    let mut store = HostStore::seed();
    store.set_host_sidebar_tool_unnotified(HostId(WEB_01), Some(ToolKind::Gpu));
    let (handle, workspace) = open_workspace_with_store(cx, store);
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, _| {
        assert!(window.try_find("tool-gpu").is_none());
        assert_eq!(window.find("tool-monitor").checked(), Some(true));
        assert_eq!(
            window.within("monitor-pages").find(1usize).checked(),
            Some(true)
        );
    });
    workspace.read_with(cx, |workspace, cx| {
        assert_eq!(
            workspace
                .store()
                .read(cx)
                .host(HostId(WEB_01))
                .unwrap()
                .sidebar_tool,
            Some(ToolKind::Monitor)
        );
    });
}

#[gpui_kit::test]
fn unified_monitor_visibility_changes_immediately(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, cx| window.click("tool-monitor", cx));
    workspace.update(cx, |workspace, cx| {
        workspace.settings().update(cx, |store, cx| {
            store.update(
                |settings| settings.sidebar_tools.set(ToolKind::Monitor, false),
                cx,
            )
        });
    });
    in_frame(cx, handle, |window, _| {
        assert!(window.try_find("tool-monitor").is_none());
        assert!(window.try_find("tool-gpu").is_none());
    });
    workspace.update(cx, |workspace, cx| {
        workspace.settings().update(cx, |store, cx| {
            store.update(|settings| settings.sidebar_tools.reset(), cx)
        });
    });
    in_frame(cx, handle, |window, _| {
        assert_eq!(window.find("tool-monitor").checked(), Some(true));
    });
}

fn sidebar_tools(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> serde_json::Value {
    workspace.read_with(cx, |workspace, cx| {
        serde_json::to_value(workspace.settings().read(cx).settings().sidebar_tools).unwrap()
    })
}

/// 设置 › 终端 › 侧栏功能, the page's third group: the tools shown in a
/// row, the rest in another; a click or a drag moves one across.
#[gpui_kit::test]
fn the_tools_move_between_the_rows_in_the_settings(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    // Tall enough for the whole group.
    cx.simulate_window_resize(handle.into(), size(px(1280.), px(1400.)));
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, _| {
        assert!(window.find("tool-monitor").visible());
        assert!(
            window.try_find("tool-kubernetes").is_none(),
            "off to begin with"
        );
    });

    let open = |cx: &mut TestAppContext| {
        in_frame(cx, handle, |window, cx| {
            window.dispatch_action(Box::new(OpenSettings), cx)
        });
        in_frame(cx, handle, |window, cx| {
            window.within("settings").click("0-1", cx)
        });
        // The group's entry under 终端 in the left column, which scrolls to
        // it: the page draws only the groups near the view.
        in_frame(cx, handle, |window, cx| {
            window.within("settings").click("0-1-2", cx)
        });
    };
    let row_of = |window: &mut gpui_kit::Window, tool: &str| {
        let id = format!("sidebar-tool-{tool}");
        if window
            .within("sidebar-tools-shown")
            .try_find(id.clone())
            .is_some()
        {
            "shown"
        } else if window.within("sidebar-tools-hidden").try_find(id).is_some() {
            "hidden"
        } else {
            "nowhere"
        }
    };
    open(cx);
    in_frame(cx, handle, |window, cx| {
        assert_eq!(row_of(window, "monitor"), "shown");
        assert_eq!(row_of(window, "kubernetes"), "hidden");
        assert_eq!(
            window.find("sidebar-tool-kubernetes").label(),
            Some("Kubernetes")
        );
        assert!(!enabled(&window.find("sidebar-tools-reset")));
        // A click moves it up.
        window.click("sidebar-tool-kubernetes", cx);
    });
    assert_eq!(
        sidebar_tools(cx, &workspace),
        serde_json::json!({"kubernetes": true})
    );
    in_frame(cx, handle, |window, cx| {
        assert_eq!(row_of(window, "kubernetes"), "shown");
        assert!(enabled(&window.find("sidebar-tools-reset")));
        // A drag takes it to the other row.
        window.drag_to("sidebar-tool-monitor", "sidebar-tools-hidden", cx);
    });
    assert_eq!(
        sidebar_tools(cx, &workspace),
        serde_json::json!({"kubernetes": true, "monitor": false})
    );
    in_frame(cx, handle, |window, cx| {
        assert_eq!(row_of(window, "monitor"), "hidden");
        // Dropped on its own row, it stays.
        window.drag_to("sidebar-tool-kubernetes", "sidebar-tools-shown", cx);
    });
    assert_eq!(
        sidebar_tools(cx, &workspace),
        serde_json::json!({"kubernetes": true, "monitor": false})
    );

    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, _| {
        assert!(window.find("tool-kubernetes").visible());
        assert!(window.try_find("tool-monitor").is_none());
    });

    // 恢复默认, for these rows alone.
    open(cx);
    in_frame(cx, handle, |window, cx| {
        window.click("sidebar-tools-reset", cx)
    });
    assert_eq!(sidebar_tools(cx, &workspace), serde_json::json!({}));
    in_frame(cx, handle, |window, cx| {
        assert_eq!(row_of(window, "monitor"), "shown");
        assert_eq!(row_of(window, "kubernetes"), "hidden");
        window.click("sidebar-tool-kubernetes", cx);
    });
    assert_eq!(
        sidebar_tools(cx, &workspace),
        serde_json::json!({"kubernetes": true})
    );

    // The page's 重置 takes them back with the rest of the page.
    in_frame(cx, handle, |window, cx| {
        window.within("settings").click("reset", cx)
    });
    assert_eq!(sidebar_tools(cx, &workspace), serde_json::json!({}));
}

#[gpui_kit::test]
async fn cancelling_or_reconnecting_during_confirmation_never_kills_a_process(
    cx: &mut TestAppContext,
) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[&eight_gpus()]));
    let (handle, _) = open_gpu(cx, factory.clone());
    wait_for_label(cx, handle, "gpu-summary", EIGHT_SUMMARY).await;
    gpu_processes(cx, handle);
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(
            Box::new(EndProcess {
                pid: 4242,
                force: false,
            }),
            cx,
        )
    });
    in_frame(cx, handle, |window, cx| window.click("cancel", cx));
    assert!(
        !factory
            .exec_commands()
            .iter()
            .any(|command| command.contains("kill -TERM"))
    );
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx);
        window.dispatch_action(
            Box::new(EndProcess {
                pid: 4242,
                force: false,
            }),
            cx,
        );
    });
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(
            Box::new(ReconnectTerminal(RemoteTerminalId(INITIAL_WEB_TERMINAL))),
            cx,
        )
    });
    in_frame(cx, handle, |window, cx| window.click("ok", cx));
    assert!(
        !factory
            .exec_commands()
            .iter()
            .any(|command| command.contains("kill -TERM"))
    );
}

#[gpui_kit::test]
async fn monitor_preferences_use_the_hosts_public_identity(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[ONE_NPU]));
    let (handle, workspace) = open_gpu(cx, factory);
    wait_for_label(cx, handle, "gpu-summary", "共 1 个设备").await;
    gpu_processes(cx, handle);
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(
            Box::new(shellrs::app::ToggleMonitorDetail(
                shellrs::monitor::MonitorDetail::Cores,
            )),
            cx,
        );
        window.dispatch_action(
            Box::new(shellrs::app::ToggleGpuDevice("0000:C1:00.0".into())),
            cx,
        );
    });
    workspace.read_with(cx, |workspace, cx| {
        let key = workspace
            .store()
            .read(cx)
            .host(HostId(WEB_01))
            .unwrap()
            .public_id
            .to_string();
        let settings = workspace.settings().read(cx).settings();
        let preferences = settings.monitor.get(&key).unwrap();
        assert_eq!(preferences.page, MonitorPage::Processes);
        assert!(preferences.gpu_processes);
        assert!(preferences.cores);
        assert!(preferences.gpu_toggled.contains("0000:C1:00.0"));
        let encoded = serde_json::to_string(&settings.monitor).unwrap();
        let decoded: std::collections::BTreeMap<String, shellrs::monitor::MonitorPreferences> =
            serde_json::from_str(&encoded).unwrap();
        assert_eq!(&decoded[&key], preferences);
    });
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_STAGING_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, cx| window.click("tool-monitor", cx));
    in_frame(cx, handle, |window, _| {
        assert_eq!(
            window.within("monitor-pages").find(0usize).checked(),
            Some(true)
        )
    });
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, _| {
        assert_eq!(
            window.within("monitor-pages").find(2usize).checked(),
            Some(true)
        );
        assert_eq!(
            window
                .within("monitor-process-source")
                .find(1usize)
                .checked(),
            Some(true)
        );
    });
}

#[gpui_kit::test]
async fn ascend_310b_displays_board_readings_without_a_pci_bus(cx: &mut TestAppContext) {
    let (handle, _) = open_gpu(
        cx,
        Arc::new(FakeTerminalFactory::answering(&[include_str!(
            "../fixtures/gpu/ascend-310b.txt"
        )])),
    );
    wait_for_label(cx, handle, ("gpu-device-name", 0u64), "310B1").await;
    in_frame(cx, handle, |window, cx| {
        assert_eq!(
            label(window, "gpu-device-utilization-0").as_deref(),
            Some("0.0%")
        );
        assert_eq!(
            label(window, ("gpu-device-temperature", 0u64)).as_deref(),
            Some("45 °C")
        );
        assert!(window.try_find("gpu-overview").is_none());
        assert_eq!(
            label(window, ("gpu-device-power", 0u64)).as_deref(),
            Some("4.7 W")
        );
        for (key, value) in [
            ("ai-cpu", "0.0%"),
            ("control-cpu", "3.0%"),
            ("memory-bandwidth", "1.0%"),
            ("memory-frequency", "2131 MHz"),
        ] {
            assert_eq!(
                label(
                    window,
                    SharedString::from(format!("gpu-ascend:ascend/0/0:{key}"))
                )
                .as_deref(),
                Some(value)
            );
        }
        window.click("gpu-device-toggle:ascend/0/0", cx);
    });
    in_frame(cx, handle, |window, _| {
        assert!(window.try_find(detail_id(0, "bus")).is_none());
        assert_eq!(
            label(window, detail_id(0, "source")).as_deref(),
            Some("npu-smi info")
        );
        assert_eq!(
            label(window, detail_id(0, "ascend-usages")).as_deref(),
            Some("已获取\nnpu-smi info -t usages -i 0 -c 0")
        );
        assert_eq!(
            label(window, detail_id(0, "ascend-memory")).as_deref(),
            Some("已获取\nnpu-smi info -t memory -i 0 -c 0")
        );
    });
}
