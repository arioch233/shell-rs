//! 系统监控: the right sidebar's tool that reads the SSH terminal's host
//! over the terminal's own connection.

use crate::support::*;
use gpui_kit::SharedString;
use gpui_kit::component::{Theme, ThemeMode};

/// A two-core Debian host, then the same host a moment later: 100 more busy
/// ticks of 200, and some traffic on eth0.
const FIRST: &str = "\
@@host
web-01-vm
@@arch
x86_64
@@os
Debian GNU/Linux 12 (bookworm)
@@uptime
5359162.33 1.00
@@cpu
model name\t: AMD EPYC-Milan Processor
@@stat
cpu  100 0 100 800 0 0 0 0
cpu0 50 0 50 400 0 0 0 0
cpu1 50 0 50 400 0 0 0 0
@@memory
MemTotal:         996608 kB
MemAvailable:     565480 kB
SwapTotal:             0 kB
SwapFree:              0 kB
@@net
    lo: 10 0 0 0 0 0 0 0 10 0 0 0 0 0 0 0
  eth0: 1000 0 0 0 0 0 0 0 500 0 0 0 0 0 0 0
docker0: 50000 0 0 0 0 0 0 0 50000 0 0 0 0 0 0 0
@@route
Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask
eth0\t00000000\t0101A8C0\t0003\t0\t0\t100\t00000000
@@df
Filesystem     1024-blocks    Used Available Capacity Mounted on
/dev/vda1          9656904 3240180   5994708      36% /
tmpfs                99664     560     99104       1% /run
";

const SECOND: &str = "\
@@host
web-01-vm
@@arch
x86_64
@@os
Debian GNU/Linux 12 (bookworm)
@@uptime
5359164.33 1.00
@@cpu
model name\t: AMD EPYC-Milan Processor
@@stat
cpu  150 0 150 900 0 0 0 0
cpu0 100 0 50 400 0 0 0 0
cpu1 50 0 100 500 0 0 0 0
@@memory
MemTotal:         996608 kB
MemAvailable:     565480 kB
SwapTotal:             0 kB
SwapFree:              0 kB
@@net
    lo: 10 0 0 0 0 0 0 0 10 0 0 0 0 0 0 0
  eth0: 9192 0 0 0 0 0 0 0 4596 0 0 0 0 0 0 0
docker0: 60000 0 0 0 0 0 0 0 60000 0 0 0 0 0 0 0
@@route
Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask
eth0\t00000000\t0101A8C0\t0003\t0\t0\t100\t00000000
@@df
Filesystem     1024-blocks    Used Available Capacity Mounted on
/dev/vda1          9656904 3240180   5994708      36% /
tmpfs                99664     560     99104       1% /run
";

/// The seeded workspace with web-01's terminal in front and the monitor
/// open; every terminal's commands are answered by `factory`.
fn open_monitor(cx: &mut TestAppContext, factory: Arc<FakeTerminalFactory>) -> WindowHandle<Root> {
    open_monitor_with_workspace(cx, factory).0
}

fn open_monitor_with_workspace(
    cx: &mut TestAppContext,
    factory: Arc<FakeTerminalFactory>,
) -> (WindowHandle<Root>, Entity<Workspace>) {
    let samples: Vec<String> = factory
        .exec_outputs
        .lock()
        .unwrap()
        .iter()
        .cloned()
        .collect();
    let outputs: Vec<&str> = samples.iter().map(String::as_str).collect();
    if !outputs.is_empty() {
        factory.monitor_answers("system", &outputs);
        factory.monitor_answers("disks", &outputs[outputs.len() - 1..]);
    }
    let (handle, workspace) = open_workspace_with_remote_factory(cx, HostStore::seed(), factory);
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, cx| window.click("tool-monitor", cx));
    (handle, workspace)
}

async fn wait_for_label(
    cx: &mut TestAppContext,
    handle: WindowHandle<Root>,
    id: &'static str,
    label: &str,
) {
    cx.wait_for(handle.into(), Duration::from_secs(5), |window, _| {
        window
            .try_find(id)
            .is_some_and(|element| element.label() == Some(label))
    })
    .await;
}

#[gpui_kit::test]
async fn the_monitor_shows_the_hosts_cpu_memory_network_and_disks(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[FIRST, SECOND]));
    let handle = open_monitor(cx, factory.clone());

    // The load is the second reading's against the first.
    wait_for_label(cx, handle, "monitor-cpu-usage", "50.0%").await;
    wait_for_label(cx, handle, "monitor-disk:/", "35.1%，3.09 GB / 9.21 GB").await;
    in_frame(cx, handle, |window, _| {
        assert_eq!(window.find("monitor-host").label(), Some("web-01-vm"));
        assert_eq!(window.find("monitor-arch").label(), Some("x86_64"));
        assert_eq!(
            window.find("monitor-os").label(),
            Some("Debian GNU/Linux 12 (bookworm)")
        );
        // The second reading's uptime: 62 days and 2364 seconds.
        assert_eq!(
            window.find("monitor-uptime").label(),
            Some("62 天 00:39:24")
        );
        // Two cores fit the row shown at once: nothing to unfold.
        assert_eq!(
            window.find("monitor-cpu-cores").label(),
            Some("核 0：100.0%，核 1：33.3%")
        );
        assert!(window.try_find("monitor-cores-toggle").is_none());
        assert_eq!(
            window.find("monitor-memory-amount").label(),
            Some("421.02 MB / 973.25 MB")
        );
        // No swap, no swap row.
        assert!(window.try_find("monitor-swap-usage").is_none());
        // The main interface only, though docker0 carried more: the
        // default route goes out of eth0.
        let eth0 = window.find("monitor-interface:eth0");
        assert!(
            eth0.label()
                .is_some_and(|label| label.starts_with("上传 ") && !label.contains('—')),
            "{eth0:?}"
        );
        assert!(window.try_find("monitor-interface:docker0").is_none());
        assert_eq!(
            window.find("monitor-interfaces-toggle").checked(),
            Some(false)
        );
        // Loopback is not worth a row, nor is a memory file system a disk.
        assert!(window.try_find("monitor-interface:lo").is_none());
        assert_eq!(
            window.find("monitor-disk:/").label(),
            Some("35.1%，3.09 GB / 9.21 GB")
        );
        assert!(window.try_find("monitor-disk:/run").is_none());
    });

    in_frame(cx, handle, |window, cx| {
        window.scroll(
            "monitor",
            gpui_kit::ScrollDelta::Lines(gpui_kit::point(0., -12.)),
            cx,
        );
    });
    // Unfolded, every interface; and folded away again.
    in_frame(cx, handle, |window, cx| {
        window.click("monitor-interfaces-toggle", cx)
    });
    in_frame(cx, handle, |window, _| {
        assert_eq!(
            window.find("monitor-interfaces-toggle").checked(),
            Some(true)
        );
        assert!(window.try_find("monitor-interface:eth0").is_some());
        assert!(window.try_find("monitor-interface:docker0").is_some());
    });
    in_frame(cx, handle, |window, cx| {
        window.click("monitor-interfaces-toggle", cx)
    });
    in_frame(cx, handle, |window, _| {
        assert!(window.try_find("monitor-interface:docker0").is_none());
    });

    // The first reading asked for everything; the next one for the load
    // only, which starts no process on the host but its `sh`.
    let commands = factory.exec_commands();
    assert!(
        commands
            .iter()
            .any(|command| command.contains("@@host") && !command.contains("@@df"))
    );
    assert!(commands.iter().any(|command| command.contains("@@df")));
}

#[gpui_kit::test]
async fn the_monitor_reads_the_host_only_while_it_shows(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[FIRST]));
    let handle = open_monitor(cx, factory.clone());
    wait_for_label(cx, handle, "monitor-host", "web-01-vm").await;

    // Hidden: nothing more runs on the host.
    in_frame(cx, handle, |window, cx| window.click("tool-monitor", cx));
    let hidden = factory.exec_count();
    cx.executor().advance_clock(Duration::from_secs(10));
    cx.run_until_parked();
    assert_eq!(factory.exec_count(), hidden);

    // Back on screen, it reads again, and shows what it had meanwhile.
    in_frame(cx, handle, |window, cx| window.click("tool-monitor", cx));
    in_frame(cx, handle, |window, _| {
        assert_eq!(window.find("monitor-host").label(), Some("web-01-vm"));
    });
    cx.wait_for(handle.into(), Duration::from_secs(5), |_, _| {
        factory.exec_count() > hidden
    })
    .await;

    // Nor while another tool is showing.
    in_frame(cx, handle, |window, cx| window.click("tool-snippets", cx));
    let other = factory.exec_count();
    cx.executor().advance_clock(Duration::from_secs(10));
    cx.run_until_parked();
    assert_eq!(factory.exec_count(), other);
}

#[gpui_kit::test]
async fn the_monitor_says_when_the_terminal_is_not_connected(cx: &mut TestAppContext) {
    // web-01's terminal is the first started, and exits at once.
    let handle = open_monitor(cx, Arc::new(FakeTerminalFactory::exit_first()));
    wait_for_label(cx, handle, "monitor-message", "暂不可用").await;
}

#[gpui_kit::test]
async fn the_monitor_says_it_reads_linux_hosts_only(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&["@@unsupported\nDarwin\n"]));
    let handle = open_monitor(cx, factory);
    wait_for_label(cx, handle, "monitor-message", "驱动未提供").await;
}

#[gpui_kit::test]
fn the_monitor_is_not_offered_on_a_host_known_not_to_run_linux(cx: &mut TestAppContext) {
    let mut store = HostStore::seed();
    store.set_host_os_unnotified(HostId(WEB_01), Some(HostOs::Debian));
    store.set_host_os_unnotified(HostId(STAGING_API), Some(HostOs::MacOs));
    let (handle, _) = open_workspace_with_store(cx, store);
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, cx| window.click("tool-monitor", cx));
    in_frame(cx, handle, |window, _| {
        assert_eq!(
            window.find(("tool-sidebar", INITIAL_WEB_TERMINAL)).label(),
            Some("资源监控")
        );
    });

    // On the Mac the monitor has no button, and the sidebar showing it
    // goes; the other tools stay on offer.
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_STAGING_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, _| {
        assert!(window.try_find("tool-monitor").is_none());
        assert!(window.try_find("tool-connections").is_none());
        assert!(window.try_find("tool-processes").is_none());
        assert!(window.try_find("tool-services").is_none());
        assert!(window.find("tool-history").visible());
        assert!(
            window
                .try_find(("tool-sidebar", INITIAL_STAGING_TERMINAL))
                .is_none()
        );
    });

    // Back on the Linux host it is as it was.
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, _| {
        assert_eq!(window.find("tool-monitor").checked(), Some(true));
        assert!(
            window
                .try_find(("tool-sidebar", INITIAL_WEB_TERMINAL))
                .is_some()
        );
    });

    // The shortcut on the Mac shows the first tool it has instead.
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_STAGING_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(ToggleToolSidebar), cx)
    });
    in_frame(cx, handle, |window, _| {
        assert_eq!(
            window
                .find(("tool-sidebar", INITIAL_STAGING_TERMINAL))
                .label(),
            Some("命令片段")
        );
    });
}

#[gpui_kit::test]
async fn the_monitor_goes_once_the_host_turns_out_not_to_run_linux(cx: &mut TestAppContext) {
    // Every terminal reports a Mac as soon as it connects.
    let (handle, workspace) = open_workspace_with_remote_factory(
        cx,
        HostStore::seed(),
        Arc::new(FakeTerminalFactory::reports_os(HostOs::MacOs)),
    );
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx)
    });
    cx.wait_for(handle.into(), Duration::from_secs(2), |window, _| {
        window.try_find("tool-monitor").is_none()
    })
    .await;
    in_frame(cx, handle, |window, _| {
        assert!(window.find("tool-snippets").visible());
    });
    let os = workspace.read_with(cx, |workspace, cx| {
        workspace
            .store()
            .read(cx)
            .host(HostId(WEB_01))
            .and_then(|host| host.os)
    });
    assert_eq!(os, Some(HostOs::MacOs));
}

/// A host with `cores` cores, every one a little busier than the last.
fn many_cores(cores: u64, busy: u64) -> String {
    let lines: String = (0..cores)
        .map(|core| format!("cpu{core} {} 0 0 1000 0 0 0 0\n", busy * core))
        .collect();
    FIRST.replace(
        "cpu  100 0 100 800 0 0 0 0\ncpu0 50 0 50 400 0 0 0 0\ncpu1 50 0 50 400 0 0 0 0\n",
        &format!(
            "cpu  {} 0 0 {} 0 0 0 0\n{lines}",
            busy * cores * cores / 2,
            1000 * cores
        ),
    )
}

#[gpui_kit::test]
async fn sixty_four_cores_show_one_row_until_unfolded(cx: &mut TestAppContext) {
    let (first, second) = (many_cores(64, 1), many_cores(64, 2));
    let factory = Arc::new(FakeTerminalFactory::answering(&[&first, &second]));
    let handle = open_monitor(cx, factory);
    // A reading against the one before, so there are loads to draw.
    cx.wait_for(handle.into(), Duration::from_secs(5), |window, _| {
        window
            .try_find("monitor-cpu-usage")
            .is_some_and(|usage| usage.label() != Some("—"))
    })
    .await;
    // The first frame measures the row; the next one fills it.
    in_frame(cx, handle, |window, cx| window.simulate_next_frame(cx));
    let bars = |cx: &mut TestAppContext| {
        in_frame(cx, handle, |window, _| {
            (0..64_u64)
                .filter_map(|core| window.try_find(("monitor-core", core)))
                .map(|bar| bar.bounds())
                .collect::<Vec<_>>()
        })
    };
    let rows = |bars: &[gpui_kit::Bounds<gpui_kit::Pixels>]| {
        let mut tops: Vec<_> = bars.iter().map(|bar| bar.top()).collect();
        tops.dedup();
        tops.len()
    };

    // One row at first: as many as fit beside the gauge.
    let first_row = bars(cx);
    assert_eq!(first_row.len(), 23);
    assert_eq!(rows(&first_row), 1);
    in_frame(cx, handle, |window, _| {
        let toggle = window.find("monitor-cores-toggle");
        assert_eq!(toggle.label(), Some("64 核"));
        assert_eq!(toggle.checked(), Some(false));
    });

    // Unfolded, all of them, wrapping onto three rows inside the column;
    // the share sits at the column's right edge.
    in_frame(cx, handle, |window, cx| {
        window.click("monitor-cores-toggle", cx)
    });
    let all = bars(cx);
    assert_eq!(all.len(), 64);
    assert_eq!(rows(&all), 3);
    in_frame(cx, handle, |window, _| {
        let column_right = window.find("monitor-cpu-usage").bounds().right();
        assert!(all.iter().all(|bar| bar.right() <= column_right));
    });
}

const ROCKCHIP_PROBE: &str = "@@node /proc/device-tree/compatible ready\nrockchip,rk3588\n@@endnode\n@@node /sys/kernel/debug/rkrga/load denied\n@@endnode\n@@node /sys/kernel/debug/rknpu/load ready\n@@endnode\n@@node /proc/mpp_service/load unsupported\n@@endnode\n@@complete\n";
const NPU_READING: &str = "@@node /sys/kernel/debug/rknpu/load ready\nNPU load: Core0: 1%, Core1: 0%, Core2: 0%,\n@@endnode\n@@complete\n";

#[gpui_kit::test]
async fn rockchip_keeps_permission_and_absent_load_separate_from_zero(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[FIRST, SECOND]));
    factory.monitor_answers("probe", &[ROCKCHIP_PROBE]);
    factory.monitor_answers("npu", &[NPU_READING]);
    let handle = open_monitor(cx, factory.clone());
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(
            Box::new(shellrs::app::SelectMonitorPage(
                shellrs::monitor::MonitorPage::Hardware,
            )),
            cx,
        )
    });
    wait_for_label(
        cx,
        handle,
        "monitor-metric-rockchip/npu/core/0",
        "核心 0: 1.0%",
    )
    .await;
    wait_for_label(
        cx,
        handle,
        "monitor-metric-/sys/kernel/debug/rkrga/load",
        "负载: 无读取权限",
    )
    .await;
    in_frame(cx, handle, |window, _| {
        assert!(window.try_find("monitor-domain-mpp-mpp").is_none());
    });
    assert_eq!(factory.monitor_count("rga"), 0);
    assert_eq!(factory.monitor_count("mpp"), 0);
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(
            Box::new(shellrs::app::SelectMonitorPage(
                shellrs::monitor::MonitorPage::Overview,
            )),
            cx,
        )
    });
    wait_for_label(cx, handle, "monitor-cpu-usage", "50.0%").await;
}

#[gpui_kit::test]
async fn a_slow_hardware_read_does_not_block_system_and_reconnect_discards_it(
    cx: &mut TestAppContext,
) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[FIRST, SECOND]));
    factory.monitor_answers("probe", &[ROCKCHIP_PROBE]);
    factory.monitor_answers("npu", &[NPU_READING]);
    factory
        .monitor_delays
        .lock()
        .unwrap()
        .insert("npu".into(), Duration::from_secs(4));
    let handle = open_monitor(cx, factory.clone());
    wait_for_label(cx, handle, "monitor-cpu-usage", "50.0%").await;
    assert_eq!(factory.monitor_count("npu"), 1);
    assert!(factory.monitor_count("system") >= 2);
    for _ in 0..4 {
        in_frame(cx, handle, |window, cx| {
            window.dispatch_action(
                Box::new(shellrs::app::RefreshMonitor(Some(
                    shellrs::monitor::CollectorDomain::Npu,
                ))),
                cx,
            )
        });
    }
    assert_eq!(factory.monitor_count("npu"), 1);
    let probes = factory.monitor_count("probe");
    factory.monitor_answers("npu", &["@@node /sys/kernel/debug/rknpu/load ready\nNPU load: Core0: 17%, Core1: 0%, Core2: 0%,\n@@endnode\n@@complete"]);
    factory.monitor_delays.lock().unwrap().remove("npu");
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(
            Box::new(ReconnectTerminal(RemoteTerminalId(INITIAL_WEB_TERMINAL))),
            cx,
        )
    });
    cx.wait_for(handle.into(), Duration::from_secs(5), |_, _| {
        factory.monitor_count("probe") > probes
    })
    .await;
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(
            Box::new(shellrs::app::SelectMonitorPage(
                shellrs::monitor::MonitorPage::Hardware,
            )),
            cx,
        )
    });
    wait_for_label(
        cx,
        handle,
        "monitor-metric-rockchip/npu/core/0",
        "核心 0: 17.0%",
    )
    .await;
}

#[gpui_kit::test]
async fn a_bad_hardware_sample_preserves_the_last_value_as_stale(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[FIRST]));
    factory.monitor_answers("probe", &[ROCKCHIP_PROBE]);
    factory.monitor_answers("npu", &[NPU_READING, "@@node /sys/kernel/debug/rknpu/load ready\nNPU load: Core0: 101%,\n@@endnode\n@@complete"]);
    let handle = open_monitor(cx, factory.clone());
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(
            Box::new(shellrs::app::SelectMonitorPage(
                shellrs::monitor::MonitorPage::Hardware,
            )),
            cx,
        )
    });
    wait_for_label(
        cx,
        handle,
        "monitor-metric-rockchip/npu/core/0",
        "核心 0: 1.0%",
    )
    .await;
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(
            Box::new(shellrs::app::RefreshMonitor(Some(
                shellrs::monitor::CollectorDomain::Npu,
            ))),
            cx,
        )
    });
    wait_for_label(
        cx,
        handle,
        "monitor-metric-rockchip/npu/core/0",
        "核心 0: 1.0% · 历史数据 · 采集失败，可刷新重试",
    )
    .await;
    let count = factory.monitor_count("npu");
    cx.executor().advance_clock(Duration::from_secs(20));
    cx.run_until_parked();
    assert_eq!(factory.monitor_count("npu"), count);
}

#[gpui_kit::test]
async fn hardware_navigation_fits_the_narrow_sidebar_across_theme_and_zoom(
    cx: &mut TestAppContext,
) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[FIRST]));
    factory.monitor_answers("probe", &[ROCKCHIP_PROBE]);
    factory.monitor_answers("npu", &[NPU_READING]);
    let handle = open_monitor(cx, factory);
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(
            Box::new(shellrs::app::SelectMonitorPage(
                shellrs::monitor::MonitorPage::Hardware,
            )),
            cx,
        )
    });
    wait_for_label(
        cx,
        handle,
        "monitor-metric-rockchip/npu/core/0",
        "核心 0: 1.0%",
    )
    .await;
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        for font_size in [12., 20.] {
            cx.update_window(handle.into(), |_, window, cx| {
                Theme::change(mode, Some(window), cx);
                Theme::global_mut(cx).font_size = px(font_size);
            })
            .unwrap();
            cx.run_until_parked();
            in_frame(cx, handle, |window, _| {
                let panel = window.find("monitor").bounds();
                assert!(panel.size.width <= px(320.));
                for id in [
                    "monitor-settings-toggle",
                    "monitor-metric-rockchip/npu/core/0",
                ] {
                    let item = window.find(id);
                    assert!(item.bounds().size.width > px(0.), "{id}");
                    assert!(
                        item.bounds().left() >= panel.left()
                            && item.bounds().right() <= panel.right(),
                        "{id}"
                    );
                }
                for page in 0..3usize {
                    let item = window.within("monitor-pages").find(page);
                    assert!(item.visible());
                    assert!(item.bounds().right() <= panel.right());
                }
            });
            in_frame(cx, handle, |window, cx| {
                window.click("monitor-settings-toggle", cx)
            });
            in_frame(cx, handle, |window, _| {
                let panel = window.find("monitor").bounds();
                for id in [
                    "monitor-sudo",
                    "monitor-configure-access",
                    "monitor-refresh",
                    "monitor-probe",
                ] {
                    let item = window.find(id);
                    assert!(item.visible(), "{id}");
                    assert!(item.bounds().left() >= panel.left(), "{id}");
                    assert!(item.bounds().right() <= panel.right(), "{id}");
                }
            });
            in_frame(cx, handle, |window, cx| {
                window.click("monitor-settings-toggle", cx)
            });
        }
    }
}

#[gpui_kit::test]
async fn hardware_details_and_permission_settings_are_disclosed_on_demand(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[FIRST, SECOND]));
    factory.monitor_answers(
        "probe",
        &[&ROCKCHIP_PROBE.replace(
            "@@complete",
            "@@node /sys/kernel/debug/rknpu/version ready\n@@endnode\n@@complete",
        )],
    );
    factory.monitor_answers("npu", &[&NPU_READING.replace("@@complete", "@@node /sys/kernel/debug/rknpu/version ready\nRKNPU driver: v0.9.8\n@@endnode\n@@complete")]);
    let handle = open_monitor(cx, factory);
    wait_for_label(cx, handle, "monitor-cpu-usage", "50.0%").await;
    in_frame(cx, handle, |window, cx| {
        assert!(window.try_find("monitor-sudo").is_none());
        assert!(
            window
                .try_find("monitor-metric-rockchip/npu/core/0")
                .is_none()
        );
        assert!(window.try_find("monitor-message").is_none());
        window.within("monitor-pages").click(1usize, cx);
    });
    wait_for_label(cx, handle, "monitor-value-rockchip/npu/core/1", "0.0%").await;
    in_frame(cx, handle, |window, cx| {
        assert!(window.try_find("monitor-cpu-usage").is_none());
        assert!(window.try_find("monitor-sources-npu-npu").is_none());
        assert!(
            window
                .try_find("monitor-detail-value-/sys/kernel/debug/rknpu/version")
                .is_none()
        );
        assert!(
            window
                .try_find("monitor-value-/sys/kernel/debug/rkrga/load")
                .is_none()
        );
        window.click("monitor-details-npu-npu", cx);
    });
    in_frame(cx, handle, |window, cx| {
        assert_eq!(
            window.find("monitor-source-rockchip/npu/core/0").label(),
            Some("/sys/kernel/debug/rknpu/load")
        );
        assert_eq!(
            window
                .find("monitor-detail-value-/sys/kernel/debug/rknpu/version")
                .label(),
            Some("RKNPU driver: v0.9.8")
        );
        assert_eq!(window.find("monitor-details-npu-npu").checked(), Some(true));
        window.click("monitor-details-npu-npu", cx);
    });
    in_frame(cx, handle, |window, cx| {
        assert!(window.try_find("monitor-sources-npu-npu").is_none());
        assert!(
            window
                .try_find("monitor-detail-value-/sys/kernel/debug/rknpu/version")
                .is_none()
        );
        window.click("monitor-permission-rga-rga", cx);
    });
    in_frame(cx, handle, |window, cx| {
        assert_eq!(window.find("monitor-settings-toggle").checked(), Some(true));
        assert_eq!(window.find("monitor-sudo").checked(), Some(false));
        window.click("monitor-settings-toggle", cx);
    });
    in_frame(cx, handle, |window, _| {
        assert!(window.try_find("monitor-sudo").is_none());
    });
}

#[gpui_kit::test]
async fn devfreq_devices_keep_their_own_load_and_frequency_cards(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[FIRST]));
    factory.monitor_answers("probe", &["@@node /sys/class/devfreq/fb000000.gpu/device/gpuinfo ready\nMali-G610 MP4 r0p0 0xA8670000\n@@endnode\n@@node /sys/class/devfreq/fb000000.gpu/cur_freq ready\n@@endnode\n@@node /sys/class/devfreq/fb000000.gpu/load ready\n@@endnode\n@@node /sys/class/devfreq/dmc/cur_freq ready\n@@endnode\n@@complete\n"]);
    factory.monitor_answers("devfreq", &["@@node /sys/class/devfreq/fb000000.gpu/device/gpuinfo ready\nMali-G610 MP4 r0p0 0xA8670000\n@@endnode\n@@node /sys/class/devfreq/fb000000.gpu/cur_freq ready\n600000000\n@@endnode\n@@node /sys/class/devfreq/fb000000.gpu/load ready\n18@600000000Hz\n@@endnode\n@@node /sys/class/devfreq/dmc/cur_freq ready\n1560000000\n@@endnode\n@@complete\n"]);
    let handle = open_monitor(cx, factory);
    in_frame(cx, handle, |window, cx| {
        window.within("monitor-pages").click(1usize, cx)
    });
    wait_for_label(
        cx,
        handle,
        "monitor-metric-/sys/class/devfreq/fb000000.gpu/cur_freq",
        "当前频率: 600 MHz",
    )
    .await;
    wait_for_label(
        cx,
        handle,
        "monitor-metric-/sys/class/devfreq/dmc/cur_freq",
        "当前频率: 1560 MHz",
    )
    .await;
    wait_for_label(
        cx,
        handle,
        "monitor-value-/sys/class/devfreq/fb000000.gpu/load",
        "18.0%",
    )
    .await;
    wait_for_label(
        cx,
        handle,
        "monitor-subtitle-devfreq-fb000000.gpu",
        "Mali-G610 MP4",
    )
    .await;
    in_frame(cx, handle, |window, _| {
        assert!(
            window
                .try_find("monitor-device-devfreq-fb000000.gpu")
                .is_none()
        );
        assert!(
            window
                .try_find("monitor-node-devfreq-fb000000.gpu")
                .is_none()
        );
    });
    in_frame(cx, handle, |window, cx| {
        window.click("monitor-details-devfreq-fb000000.gpu", cx);
    });
    in_frame(cx, handle, |window, _| {
        assert!(
            window
                .find("monitor-sources-devfreq-fb000000.gpu")
                .visible()
        );
        assert!(window.try_find("monitor-sources-devfreq-dmc").is_none());
    });

    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        for size in [12., 20.] {
            cx.update_window(handle.into(), |_, window, cx| {
                Theme::change(mode, Some(window), cx);
                Theme::global_mut(cx).font_size = px(size);
            })
            .unwrap();
            cx.run_until_parked();
            in_frame(cx, handle, |window, _| {
                assert_eq!(
                    window.find("monitor-node-devfreq-fb000000.gpu").label(),
                    Some("fb000000.gpu")
                );
                let panel = window.find("monitor").bounds();
                let source_id = SharedString::from(
                    "monitor-source-/sys/class/devfreq/fb000000.gpu/device/gpuinfo",
                );
                let source = window
                    .find((ElementId::from(source_id), "value-cell"))
                    .bounds();
                assert!(source.left() >= panel.left() && source.right() <= panel.right());
                assert!(
                    source.size.height > px(size),
                    "the source path should wrap in the sidebar"
                );
            });
        }
    }
}

#[gpui_kit::test]
async fn sudo_reads_denied_nodes_and_keeps_the_choice_per_host(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[FIRST, SECOND]));
    factory.monitor_answers(
        "probe",
        &[&ROCKCHIP_PROBE.replace("/rknpu/load ready", "/rknpu/load denied")],
    );
    factory.monitor_answers("rga", &["@@node /sys/kernel/debug/rkrga/load ready\nscheduler[0]: rga3\n load = 2%\n@@endnode\n@@complete"]);
    factory.monitor_answers("npu", &[NPU_READING]);
    let (handle, workspace) = open_monitor_with_workspace(cx, factory.clone());
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(
            Box::new(shellrs::app::SelectMonitorPage(
                shellrs::monitor::MonitorPage::Hardware,
            )),
            cx,
        )
    });
    wait_for_label(
        cx,
        handle,
        "monitor-metric-/sys/kernel/debug/rkrga/load",
        "负载: 无读取权限",
    )
    .await;
    assert_eq!(factory.monitor_count("rga"), 0);
    assert_eq!(factory.monitor_count("npu"), 0);
    let probes = factory.monitor_count("probe");
    in_frame(cx, handle, |window, cx| {
        window.click("monitor-settings-toggle", cx)
    });
    in_frame(cx, handle, |window, cx| window.click("monitor-sudo", cx));
    wait_for_label(
        cx,
        handle,
        "monitor-metric-rockchip/rga/scheduler/0",
        "RGA3 · 核心 0: 2.0%",
    )
    .await;
    wait_for_label(
        cx,
        handle,
        "monitor-metric-rockchip/npu/core/0",
        "核心 0: 1.0%",
    )
    .await;
    assert!(factory.monitor_count("probe") > probes);
    workspace.read_with(cx, |workspace, cx| {
        let key = workspace
            .store()
            .read(cx)
            .host(HostId(WEB_01))
            .unwrap()
            .public_id
            .to_string();
        let settings = workspace.settings().read(cx).settings();
        assert!(settings.monitor[&key].allow_sudo);
        let round_trip: shellrs::monitor::MonitorPreferences =
            serde_json::from_value(serde_json::to_value(&settings.monitor[&key]).unwrap()).unwrap();
        assert!(round_trip.allow_sudo);
    });
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_STAGING_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, cx| window.click("tool-monitor", cx));
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(
            Box::new(shellrs::app::SelectMonitorPage(
                shellrs::monitor::MonitorPage::Hardware,
            )),
            cx,
        )
    });
    in_frame(cx, handle, |window, cx| {
        window.click("monitor-settings-toggle", cx)
    });
    in_frame(cx, handle, |window, _| {
        assert_eq!(window.find("monitor-sudo").checked(), Some(false))
    });
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, cx| {
        window.click("monitor-settings-toggle", cx)
    });
    in_frame(cx, handle, |window, cx| {
        assert_eq!(window.find("monitor-sudo").checked(), Some(true));
        window.click("monitor-sudo", cx);
    });
    wait_for_label(
        cx,
        handle,
        "monitor-metric-/sys/kernel/debug/rkrga/load",
        "负载: 无读取权限",
    )
    .await;
    let reads = factory.monitor_count("rga");
    cx.executor().advance_clock(Duration::from_secs(10));
    cx.run_until_parked();
    assert_eq!(factory.monitor_count("rga"), reads);
}

#[gpui_kit::test]
async fn rejected_sudo_pauses_hardware_without_stopping_system_metrics(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[FIRST, SECOND]));
    factory.monitor_answers("probe", &[ROCKCHIP_PROBE]);
    factory.monitor_answers(
        "rga",
        &["@@node /sys/kernel/debug/rkrga/load sudo_denied\n@@endnode\n@@complete"],
    );
    factory.monitor_answers("npu", &[NPU_READING]);
    let handle = open_monitor(cx, factory.clone());
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(
            Box::new(shellrs::app::SelectMonitorPage(
                shellrs::monitor::MonitorPage::Hardware,
            )),
            cx,
        )
    });
    in_frame(cx, handle, |window, cx| {
        window.click("monitor-settings-toggle", cx)
    });
    in_frame(cx, handle, |window, cx| window.click("monitor-sudo", cx));
    wait_for_label(
        cx,
        handle,
        "monitor-metric-/sys/kernel/debug/rkrga/load",
        "负载: sudo 读取失败，请检查免密授权",
    )
    .await;
    let reads = factory.monitor_count("rga");
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(
            Box::new(shellrs::app::SelectMonitorPage(
                shellrs::monitor::MonitorPage::Overview,
            )),
            cx,
        )
    });
    wait_for_label(cx, handle, "monitor-cpu-usage", "50.0%").await;
    cx.executor().advance_clock(Duration::from_secs(10));
    cx.run_until_parked();
    assert_eq!(factory.monitor_count("rga"), reads);
}

#[gpui_kit::test]
async fn toggling_sudo_cannot_restart_a_timed_out_driver_until_reconnect(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[FIRST, SECOND]));
    factory.monitor_answers("probe", &[ROCKCHIP_PROBE]);
    factory.monitor_answers(
        "rga",
        &["@@node /sys/kernel/debug/rkrga/load timeout\n@@endnode\n@@complete"],
    );
    factory.monitor_answers("npu", &[NPU_READING]);
    let handle = open_monitor(cx, factory.clone());
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(
            Box::new(shellrs::app::SelectMonitorPage(
                shellrs::monitor::MonitorPage::Hardware,
            )),
            cx,
        )
    });
    in_frame(cx, handle, |window, cx| {
        window.click("monitor-settings-toggle", cx)
    });
    in_frame(cx, handle, |window, cx| window.click("monitor-sudo", cx));
    wait_for_label(
        cx,
        handle,
        "monitor-metric-/sys/kernel/debug/rkrga/load",
        "负载: 采样超时，已暂停；重连后重试",
    )
    .await;
    let count = factory.monitor_count("rga");
    for _ in 0..4 {
        in_frame(cx, handle, |window, cx| window.click("monitor-sudo", cx));
    }
    cx.executor().advance_clock(Duration::from_secs(10));
    cx.run_until_parked();
    assert_eq!(factory.monitor_count("rga"), count);
    factory.monitor_answers("rga", &["@@node /sys/kernel/debug/rkrga/load ready\nscheduler[0]: rga3\n load = 7%\n@@endnode\n@@complete"]);
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(
            Box::new(ReconnectTerminal(RemoteTerminalId(INITIAL_WEB_TERMINAL))),
            cx,
        )
    });
    wait_for_label(
        cx,
        handle,
        "monitor-metric-rockchip/rga/scheduler/0",
        "RGA3 · 核心 0: 7.0%",
    )
    .await;
    assert!(factory.monitor_count("rga") > count);
}

#[gpui_kit::test]
async fn monitor_access_requires_confirmation_then_enables_reads_and_reprobes(
    cx: &mut TestAppContext,
) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[FIRST]));
    factory.monitor_answers("monitor-authorize", &["@@shellrs-monitor-access ready\n"]);
    factory
        .monitor_delays
        .lock()
        .unwrap()
        .insert("monitor-authorize".into(), Duration::from_millis(300));
    let (handle, workspace) = open_monitor_with_workspace(cx, factory.clone());
    wait_for_label(cx, handle, "monitor-host", "web-01-vm").await;
    in_frame(cx, handle, |window, cx| {
        window.click("monitor-settings-toggle", cx)
    });
    in_frame(cx, handle, |window, cx| {
        window.click("monitor-configure-access", cx)
    });
    in_frame(cx, handle, |window, cx| {
        assert!(window.find("monitor-sudo-password").value().is_none());
        window.press("escape", cx);
    });
    assert_eq!(factory.monitor_count("monitor-authorize"), 0);
    let probes = factory.monitor_count("probe");
    in_frame(cx, handle, |window, cx| {
        window.click("monitor-configure-access", cx)
    });
    in_frame(cx, handle, |window, cx| {
        window.click("monitor-sudo-password", cx);
        window.input("sudo-test-secret", cx);
    });
    in_frame(cx, handle, |window, cx| window.click("commit", cx));
    in_frame(cx, handle, |window, cx| {
        window.press("escape", cx);
        assert!(window.try_find("monitor-sudo-password").is_some());
    });
    cx.wait_for(handle.into(), Duration::from_secs(5), |window, _| {
        window.try_find("monitor-sudo-password").is_none()
            && window
                .try_find("monitor-sudo")
                .is_some_and(|toggle| toggle.checked() == Some(true))
    })
    .await;
    assert_eq!(factory.monitor_count("monitor-authorize"), 1);
    assert!(
        !factory
            .exec_commands
            .lock()
            .unwrap()
            .iter()
            .any(|command| command.contains("sudo-test-secret"))
    );
    cx.wait_for(handle.into(), Duration::from_secs(5), |_, _| {
        factory.monitor_count("probe") > probes
    })
    .await;
    workspace.read_with(cx, |workspace, cx| {
        let key = workspace
            .store()
            .read(cx)
            .host(HostId(WEB_01))
            .unwrap()
            .public_id
            .to_string();
        assert!(workspace.settings().read(cx).settings().monitor[&key].allow_sudo);
    });
}

#[gpui_kit::test]
async fn monitor_access_failure_keeps_reads_disabled_and_allows_retry(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[FIRST]));
    // 超出执行输出上限，模拟配置请求失败
    factory.monitor_answers("monitor-authorize", &[&"error".repeat(4 * 1024)]);
    let handle = open_monitor(cx, factory.clone());
    wait_for_label(cx, handle, "monitor-host", "web-01-vm").await;
    in_frame(cx, handle, |window, cx| {
        window.click("monitor-settings-toggle", cx)
    });
    in_frame(cx, handle, |window, cx| {
        window.click("monitor-configure-access", cx)
    });
    in_frame(cx, handle, |window, cx| window.click("commit", cx));
    cx.wait_for(handle.into(), Duration::from_secs(5), |window, _| {
        window.try_find("form-error").is_some()
    })
    .await;
    in_frame(cx, handle, |window, _| {
        assert_eq!(window.find("monitor-sudo").checked(), Some(false));
        assert!(window.try_find("monitor-sudo-password").is_some());
    });
    factory.monitor_answers("monitor-authorize", &["@@shellrs-monitor-access ready\n"]);
    in_frame(cx, handle, |window, cx| window.click("commit", cx));
    cx.wait_for(handle.into(), Duration::from_secs(5), |window, _| {
        window.try_find("monitor-sudo-password").is_none()
    })
    .await;
    assert_eq!(factory.monitor_count("monitor-authorize"), 2);
}

#[gpui_kit::test]
async fn monitor_access_refuses_a_connection_that_reconnected_after_opening(
    cx: &mut TestAppContext,
) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[FIRST]));
    let (handle, workspace) = open_monitor_with_workspace(cx, factory.clone());
    let view = workspace.read_with(cx, |workspace, cx| {
        workspace
            .remote_terminal(RemoteTerminalId(INITIAL_WEB_TERMINAL))
            .unwrap()
            .read(cx)
            .terminal()
            .clone()
    });
    let generation = view.read_with(cx, |view, cx| view.generation(cx));
    wait_for_label(cx, handle, "monitor-host", "web-01-vm").await;
    in_frame(cx, handle, |window, cx| {
        window.click("monitor-settings-toggle", cx)
    });
    in_frame(cx, handle, |window, cx| {
        window.click("monitor-configure-access", cx)
    });
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(
            Box::new(ReconnectTerminal(RemoteTerminalId(INITIAL_WEB_TERMINAL))),
            cx,
        )
    });
    assert!(view.read_with(cx, |view, cx| view.generation(cx)) > generation);
    // 重连会把焦点移回终端，先恢复到授权表单再提交
    in_frame(cx, handle, |window, cx| {
        window.click("monitor-sudo-password", cx)
    });
    in_frame(cx, handle, |window, cx| window.click("commit", cx));
    assert_eq!(factory.monitor_count("monitor-authorize"), 0);
    wait_for_label(
        cx,
        handle,
        "form-error",
        "连接已断开或重新建立，请关闭此窗口后重试",
    )
    .await;
    assert_eq!(factory.monitor_count("monitor-authorize"), 0);
    in_frame(cx, handle, |window, cx| window.press("escape", cx));
}

#[gpui_kit::test]
async fn npu_separates_core_load_from_devfreq_and_uses_a_compact_subtitle(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[FIRST]));
    let extra = "@@node /sys/class/devfreq/fdab0000.npu/cur_freq ready\n@@endnode\n@@node /sys/class/devfreq/fdab0000.npu/load ready\n@@endnode\n@@node /sys/kernel/debug/rknpu/version ready\n@@endnode\n@@complete";
    factory.monitor_answers("probe", &[&ROCKCHIP_PROBE.replace("@@complete", extra)]);
    factory.monitor_answers("npu", &["@@node /sys/class/devfreq/fdab0000.npu/cur_freq ready\n1000000000\n@@endnode\n@@node /sys/class/devfreq/fdab0000.npu/load ready\n100@1000000000Hz\n@@endnode\n@@node /sys/kernel/debug/rknpu/load ready\nNPU load: Core0: 35%, Core1: 5%, Core2: 0%,\n@@endnode\n@@node /sys/kernel/debug/rknpu/version ready\nRKNPU driver: v0.9.8\n@@endnode\n@@complete"]);
    let handle = open_monitor(cx, factory);
    in_frame(cx, handle, |window, cx| {
        window.within("monitor-pages").click(1usize, cx)
    });
    wait_for_label(cx, handle, "monitor-device-npu-npu", "3 核").await;
    wait_for_label(
        cx,
        handle,
        "monitor-subtitle-npu-npu",
        "1000 MHz · 驱动 v0.9.8",
    )
    .await;
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        for size in [12., 20.] {
            cx.update_window(handle.into(), |_, window, cx| {
                Theme::change(mode, Some(window), cx);
                Theme::global_mut(cx).font_size = px(size);
            })
            .unwrap();
            cx.run_until_parked();
            in_frame(cx, handle, |window, _| {
                assert!(
                    window
                        .try_find("monitor-metric-/sys/class/devfreq/fdab0000.npu/load")
                        .is_none()
                );
                assert!(
                    window
                        .try_find("monitor-metric-/sys/class/devfreq/fdab0000.npu/cur_freq")
                        .is_none()
                );
                assert_eq!(
                    window.find("monitor-value-rockchip/npu/core/0").label(),
                    Some("35.0%")
                );
                let panel = window.find("monitor").bounds();
                let subtitle = window.find("monitor-subtitle-npu-npu").bounds();
                assert!(subtitle.left() >= panel.left() && subtitle.right() <= panel.right());
            });
        }
    }
    in_frame(cx, handle, |window, cx| {
        window.click("monitor-details-npu-npu", cx)
    });
    wait_for_label(
        cx,
        handle,
        "monitor-diagnostic-/sys/class/devfreq/fdab0000.npu/load",
        "设备负载（devfreq）: 100.0%",
    )
    .await;
}

#[gpui_kit::test]
async fn thermal_names_and_rga_core_identity_are_readable_and_values_align(
    cx: &mut TestAppContext,
) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[FIRST]));
    let names = [
        "soc-thermal",
        "bigcore0-thermal",
        "bigcore1-thermal",
        "littlecore-thermal",
        "center-thermal",
        "gpu-thermal",
        "npu-thermal",
        "board-extra",
    ];
    let labels = [
        "芯片",
        "CPU 大核组 0",
        "CPU 大核组 1",
        "CPU 小核组",
        "中央温区",
        "GPU",
        "NPU",
        "board-extra",
    ];
    let mut probe = "@@node /sys/kernel/debug/rkrga/load ready\n@@endnode\n".to_string();
    let mut sample = String::new();
    for (ix, name) in names.iter().enumerate() {
        for node in ["type", "temp"] {
            probe.push_str(&format!(
                "@@node /sys/class/thermal/thermal_zone{ix}/{node} ready\n@@endnode\n"
            ));
        }
        let temperature = if ix == 1 { 50300 } else { 34200 };
        sample.push_str(&format!("@@node /sys/class/thermal/thermal_zone{ix}/type ready\n{name}\n@@endnode\n@@node /sys/class/thermal/thermal_zone{ix}/temp ready\n{temperature}\n@@endnode\n"));
    }
    probe.push_str("@@complete");
    sample.push_str("@@complete");
    factory.monitor_answers("probe", &[&probe]);
    factory.monitor_answers("thermal", &[&sample]);
    factory.monitor_answers("rga", &["@@node /sys/kernel/debug/rkrga/load ready\nnum of scheduler = 3\nscheduler[0]: rga3\n load = 2%\nscheduler[1]: rga3\n load = 0%\nscheduler[7]: rga2\n load = 0%\n@@endnode\n@@complete"]);
    let handle = open_monitor(cx, factory);
    in_frame(cx, handle, |window, cx| {
        window.within("monitor-pages").click(1usize, cx)
    });
    wait_for_label(
        cx,
        handle,
        "monitor-subtitle-rga-rga",
        "2 × RGA3 · 1 × RGA2",
    )
    .await;
    wait_for_label(
        cx,
        handle,
        "monitor-metric-rockchip/rga/scheduler/7",
        "RGA2 · 核心 7: 0.0%",
    )
    .await;
    wait_for_label(cx, handle, "monitor-device-thermal-thermal", "8 个传感器").await;
    wait_for_label(
        cx,
        handle,
        "monitor-thermal-highest",
        "当前最高温度: 50.3 °C · CPU 大核组 0",
    )
    .await;
    in_frame(cx, handle, |window, cx| {
        window.click("monitor-details-rga-rga", cx);
    });
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        for size in [12., 20.] {
            cx.update_window(handle.into(), |_, window, cx| {
                Theme::change(mode, Some(window), cx);
                Theme::global_mut(cx).font_size = px(size);
            })
            .unwrap();
            cx.run_until_parked();
            in_frame(cx, handle, |window, _| {
                let panel = window.find("monitor").bounds();
                for (index, kind) in [(0, "RGA3"), (1, "RGA3"), (7, "RGA2")] {
                    let unit = window.find(SharedString::from(format!(
                        "monitor-rga-unit-rockchip/rga/scheduler/{index}"
                    )));
                    assert_eq!(
                        unit.label(),
                        Some(format!("硬件类型：{kind}\n驱动调度器：scheduler[{index}]").as_str())
                    );
                    assert!(
                        unit.bounds().left() >= panel.left()
                            && unit.bounds().right() <= panel.right()
                    );
                    assert!(
                        window
                            .try_find(SharedString::from(format!(
                                "monitor-original-name-rockchip/rga/scheduler/{index}"
                            )))
                            .is_none()
                    );
                }
                assert_eq!(
                    window
                        .find("monitor-source-rockchip/rga/scheduler/0")
                        .label(),
                    Some("/sys/kernel/debug/rkrga/load")
                );
                assert!(
                    window
                        .try_find("monitor-source-rockchip/rga/scheduler/1")
                        .is_none()
                );
                assert!(
                    window
                        .try_find("monitor-source-rockchip/rga/scheduler/7")
                        .is_none()
                );
                let summary = window.find("monitor-thermal-highest").bounds();
                assert!(summary.left() >= panel.left() && summary.right() <= panel.right());
                assert_eq!(window.find("monitor-thermal-group-1").label(), Some("CPU"));
                assert_eq!(
                    window.find("monitor-thermal-group-2").label(),
                    Some("GPU / NPU")
                );
                let mut previous = None;
                for (ix, label) in labels.iter().enumerate() {
                    let row = window.find(SharedString::from(format!(
                        "monitor-metric-/sys/class/thermal/thermal_zone{ix}/temp"
                    )));
                    let value_text = if ix == 1 { "50.3" } else { "34.2" };
                    assert_eq!(
                        row.label(),
                        Some(format!("{label}: {value_text} °C").as_str())
                    );
                    let value = window
                        .find(SharedString::from(format!(
                            "monitor-value-/sys/class/thermal/thermal_zone{ix}/temp"
                        )))
                        .bounds();
                    if let Some((left, right)) = previous {
                        assert_eq!(row.bounds().left(), left);
                        assert_eq!(value.right(), right);
                    }
                    assert!(value.left() >= panel.left() && value.right() <= panel.right());
                    previous = Some((row.bounds().left(), value.right()));
                }
            });
        }
    }
}

#[gpui_kit::test]
async fn mpp_shows_sessions_throughput_clocks_and_task_indices_without_inventing_usage(
    cx: &mut TestAppContext,
) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[FIRST]));
    let nodes = [
        (
            "/proc/mpp_service/sessions-summary",
            include_str!("../fixtures/monitor/mpp-sessions-summary.txt"),
        ),
        (
            "/sys/kernel/debug/clk/clk_summary",
            include_str!("../fixtures/monitor/mpp-clk-summary.txt"),
        ),
        ("/proc/mpp_service/rkvdec-core0/task_count", "100"),
        ("/proc/mpp_service/timing_en", "0"),
        (
            "/proc/mpp_service/supports-device",
            "DEVICE[16]:RKVENC\nDEVICE[17]:RKVDEC",
        ),
        (
            "/proc/mpp_service/rkvenc-core0/sessions-info",
            "encoder diagnostic sample",
        ),
    ];
    let mut probe = "@@node /proc/mpp_service/load unsupported\n@@endnode\n".to_string();
    let mut sample = String::new();
    for (path, body) in nodes {
        probe.push_str(&format!("@@node {path} ready\n@@endnode\n"));
        sample.push_str(&format!("@@node {path} ready\n{body}\n@@endnode\n"));
    }
    probe.push_str("@@complete");
    sample.push_str("@@complete");
    factory.monitor_answers("probe", &[&probe]);
    factory.monitor_answers("mpp", &[&sample]);
    let handle = open_monitor(cx, factory.clone());
    in_frame(cx, handle, |window, cx| {
        window.within("monitor-pages").click(1usize, cx)
    });
    wait_for_label(
        cx,
        handle,
        "mpp-session-count",
        "2 路会话 · 编码 1 / 解码 1",
    )
    .await;
    wait_for_label(cx, handle, "mpp-fps-sum", "12.95 FPS").await;
    wait_for_label(
        cx,
        handle,
        "mpp-session-1354197-495",
        "H.264 · 3840 × 2160 · 实际编码: 12.95 FPS",
    )
    .await;
    wait_for_label(
        cx,
        handle,
        "mpp-clock-clk_rkvenc0_core",
        "编码核心 0: 800 MHz",
    )
    .await;
    wait_for_label(
        cx,
        handle,
        "mpp-task-rkvdec-core0",
        "任务索引 100 · 增长 — 次/秒",
    )
    .await;
    wait_for_label(
        cx,
        handle,
        "monitor-metric-/proc/mpp_service/load",
        "负载: 驱动未提供",
    )
    .await;
    in_frame(cx, handle, |window, cx| {
        window.click("monitor-details-mpp-mpp", cx)
    });
    in_frame(cx, handle, |window, _| {
        let detail = window.find("mpp-session-detail-1354197-495");
        let text = detail.label().unwrap();
        assert!(text.contains("配置输入 FPS: 21"));
        assert!(text.contains("配置输出 FPS: 21"));
        assert!(text.contains("配置码率: 22.71 Mbps"));
        assert!(text.contains("GOP: 44"));
        assert_eq!(
            window
                .find("mpp-detail-/proc/mpp_service/timing_en")
                .label(),
            Some("关闭（只读）")
        );
        assert!(
            window
                .find("mpp-detail-/sys/kernel/debug/clk/clk_summary")
                .label()
                .unwrap()
                .contains("aclk_rkvenc0: 500 MHz · 使能计数 1")
        );
        assert!(
            window
                .find("mpp-detail-/proc/mpp_service/rkvenc-core0/sessions-info")
                .label()
                .unwrap()
                .contains("encoder diagnostic sample")
        );
    });
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        for size in [12., 20.] {
            cx.update_window(handle.into(), |_, window, cx| {
                Theme::change(mode, Some(window), cx);
                Theme::global_mut(cx).font_size = px(size);
            })
            .unwrap();
            cx.run_until_parked();
            in_frame(cx, handle, |window, _| {
                let panel = window.find("monitor").bounds();
                let mut right = None;
                for id in [
                    "mpp-fps-sum",
                    "mpp-clock-clk_rkvenc0_core",
                    "mpp-clock-clk_rkvdec1_core",
                    "mpp-task-rate-rkvdec-core0",
                    "mpp-task-index-rkvdec-core0",
                ] {
                    let cell = window
                        .find((ElementId::from(SharedString::from(id)), "value-cell"))
                        .bounds();
                    if let Some(previous) = right {
                        assert_eq!(cell.right(), previous);
                    }
                    assert!(cell.left() >= panel.left() && cell.right() <= panel.right());
                    right = Some(cell.right());
                }
                let session_id =
                    ElementId::from(SharedString::from("mpp-session-detail-1354197-495"));
                for key in ["fps_in", "fps_out", "bitrate", "gop_size"] {
                    let field_id = ElementId::from((session_id.clone(), key));
                    assert_eq!(
                        window.find((field_id, "value-cell")).bounds().right(),
                        right.unwrap()
                    );
                }
                assert_eq!(
                    window
                        .find((
                            ElementId::from(SharedString::from("mpp-session-1354197-495")),
                            "fps"
                        ))
                        .bounds()
                        .right(),
                    right.unwrap()
                );
                for id in [
                    "mpp-session-1354197-495",
                    "mpp-clock-clk_rkvenc0_core",
                    "mpp-task-rkvdec-core0",
                    "mpp-session-detail-1354197-495",
                ] {
                    let bounds = window.find(id).bounds();
                    assert!(
                        bounds.left() >= panel.left() && bounds.right() <= panel.right(),
                        "{id}"
                    );
                }
            });
        }
    }
    // 会话结束后移除旧条目，空响应与读取失败不混淆
    factory.monitor_answers(
        "mpp",
        &["@@node /proc/mpp_service/sessions-summary ready\n@@endnode\n@@complete"],
    );
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(
            Box::new(shellrs::app::RefreshMonitor(Some(
                shellrs::monitor::CollectorDomain::Mpp,
            ))),
            cx,
        )
    });
    wait_for_label(
        cx,
        handle,
        "mpp-session-count",
        "0 路会话 · 编码 0 / 解码 0",
    )
    .await;
    in_frame(cx, handle, |window, _| {
        assert!(window.try_find("mpp-session-1354197-495").is_none())
    });
}

#[gpui_kit::test]
async fn mpp_many_sessions_can_be_expanded_without_changing_the_summary(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[FIRST]));
    factory.monitor_answers(
        "probe",
        &["@@node /proc/mpp_service/sessions-summary ready\n@@endnode\n@@complete"],
    );
    let mut sample = "@@node /proc/mpp_service/sessions-summary ready\n".to_owned();
    for id in 0..33 {
        sample.push_str(&format!(
            "session: pid=10 index={id}\n device: rkvdec-core\n"
        ));
    }
    sample.push_str("@@endnode\n@@complete");
    factory.monitor_answers("mpp", &[&sample]);
    let handle = open_monitor(cx, factory);
    in_frame(cx, handle, |window, cx| {
        window.within("monitor-pages").click(1usize, cx)
    });
    wait_for_label(
        cx,
        handle,
        "mpp-session-count",
        "33 路会话 · 编码 0 / 解码 33",
    )
    .await;
    in_frame(cx, handle, |window, cx| {
        assert!(window.try_find("mpp-session-10-32").is_none());
        window.click("mpp-all-sessions", cx);
    });
    in_frame(cx, handle, |window, cx| {
        assert!(window.try_find("mpp-session-10-32").is_some());
        window.click("mpp-all-sessions", cx);
    });
    in_frame(cx, handle, |window, _| {
        assert!(window.try_find("mpp-session-10-32").is_none())
    });
}

#[gpui_kit::test]
async fn jetson_groups_engines_and_temperatures_without_duplicate_gpu_cards(
    cx: &mut TestAppContext,
) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[FIRST]));
    let model = "NVIDIA Jetson Orin NX Engineering Reference Developer Kit Super";
    let mut probe = format!(
        "@@node /proc/device-tree/model ready\n{model}\n@@endnode\n@@node /proc/device-tree/compatible ready\nnvidia,tegra234\n@@endnode\n"
    );
    for path in [
        "/sys/kernel/debug/rkrga/load",
        "/sys/kernel/debug/rkrga/hardware",
        "/sys/kernel/debug/rknpu/load",
        "/sys/kernel/debug/rknpu/version",
        "/sys/kernel/debug/clk/clk_summary",
    ] {
        probe.push_str(&format!("@@node {path} denied\n@@endnode\n"));
    }
    let mut devfreq = String::new();
    for (device, frequency) in [
        ("17000000.gpu", 306000000),
        ("154c0000.nvenc", 793600000),
        ("15340000.vic", 729600000),
    ] {
        let path = format!("/sys/class/devfreq/{device}/cur_freq");
        probe.push_str(&format!("@@node {path} ready\n@@endnode\n"));
        devfreq.push_str(&format!("@@node {path} ready\n{frequency}\n@@endnode\n"));
    }
    let mut thermal = String::new();
    for (ix, name) in ["cpu-thermal", "cv0-thermal", "soc0-thermal", "tj-thermal"]
        .into_iter()
        .enumerate()
    {
        for (field, value) in [("type", name), ("temp", "50000")] {
            let path = format!("/sys/class/thermal/thermal_zone{ix}/{field}");
            probe.push_str(&format!("@@node {path} ready\n@@endnode\n"));
            thermal.push_str(&format!("@@node {path} ready\n{value}\n@@endnode\n"));
        }
    }
    for output in [&mut probe, &mut devfreq, &mut thermal] {
        output.push_str("@@complete\n");
    }
    factory.monitor_answers("probe", &[&probe]);
    factory.monitor_answers("devfreq", &[&devfreq]);
    factory.monitor_answers(
        "tegra",
        &[include_str!(
            "../fixtures/monitor/jetson-orin-nx-runtime.txt"
        )],
    );
    factory.monitor_answers("thermal", &[&thermal]);
    factory.monitor_answers(
        "accelerator",
        &[include_str!("../fixtures/gpu/jetson-orin-nx.txt")],
    );
    let handle = open_monitor(cx, factory.clone());
    in_frame(cx, handle, |window, cx| {
        window.within("monitor-pages").click(1usize, cx);
    });
    wait_for_label(cx, handle, "monitor-board-model", model).await;
    let samples = factory.monitor_count("tegra");
    in_frame(cx, handle, |window, cx| {
        window.click("monitor-settings-toggle", cx)
    });
    in_frame(cx, handle, |window, cx| window.click("monitor-sudo", cx));
    cx.wait_for(handle.into(), Duration::from_secs(5), |_, _| {
        factory.monitor_count("tegra") > samples
    })
    .await;
    in_frame(cx, handle, |window, cx| {
        window.click("monitor-settings-toggle", cx)
    });

    wait_for_label(cx, handle, "gpu-device-utilization-0", "0.0%").await;
    wait_for_label(cx, handle, "tegra-154c0000.nvenc-runtime", "已休眠").await;
    wait_for_label(
        cx,
        handle,
        "monitor-metric-/sys/class/thermal/thermal_zone1/temp",
        "视觉计算温区 0: 50.0 °C",
    )
    .await;
    in_frame(cx, handle, |window, _| {
        for domain in ["rga", "npu", "mpp"] {
            assert!(
                window
                    .try_find(SharedString::from(format!(
                        "monitor-domain-{domain}-{domain}"
                    )))
                    .is_none()
            );
            assert_eq!(factory.monitor_count(domain), 0);
        }
        assert!(
            window
                .try_find("monitor-details-devfreq-17000000.gpu")
                .is_none()
        );
        assert!(window.try_find("gpu-device-memory-0").is_none());
        assert_eq!(
            window.find(("gpu-device-frequency", 0u64)).label(),
            Some("306 MHz")
        );
        assert_eq!(
            window
                .find("monitor-metric-/sys/class/thermal/thermal_zone2/temp")
                .label(),
            Some("SoC 温区 0: 50.0 °C")
        );
        assert_eq!(
            window
                .find("monitor-metric-/sys/class/thermal/thermal_zone3/temp")
                .label(),
            Some("芯片结温: 50.0 °C")
        );
        assert!(window.try_find("gpu-errors").is_none());
    });
}

#[gpui_kit::test]
async fn ascend_dvpp_channels_details_and_failures_keep_their_meaning(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[FIRST]));
    let sample = include_str!("../fixtures/monitor/ascend-dvpp-simulated.txt");
    let idle = include_str!("../fixtures/monitor/ascend-310b-dvpp-idle.txt");
    let mut probe =
        "@@node /proc/device-tree/compatible ready\nhisilicon,ascend610\n@@endnode\n".to_owned();
    for line in sample.lines().filter(|line| line.starts_with("@@node ")) {
        probe.push_str(line);
        probe.push_str("\n@@endnode\n");
    }
    probe.push_str("@@complete");
    factory.monitor_answers("probe", &[&probe]);
    factory.monitor_answers("dvpp", &[sample]);
    let handle = open_monitor(cx, factory.clone());
    in_frame(cx, handle, |window, cx| {
        window.within("monitor-pages").click(1usize, cx)
    });
    wait_for_label(cx, handle, "dvpp-venc-count", "1 路通道").await;
    wait_for_label(cx, handle, "dvpp-venc-3", "H.264 · 3840 × 2160 · PID 2200").await;
    wait_for_label(cx, handle, "dvpp-vdec-3-decode_fps", "24.5 FPS").await;
    wait_for_label(cx, handle, "dvpp-venc-3-driver_fps", "24.75 FPS").await;
    wait_for_label(cx, handle, "dvpp-venc-3-bitrate", "7.53 Mbps").await;
    in_frame(cx, handle, |window, cx| {
        window.click("monitor-details-dvpp", cx)
    });
    wait_for_label(cx, handle, "dvpp-venc-3-detail-source_fps", "30 FPS").await;
    wait_for_label(cx, handle, "dvpp-venc-3-detail-queued_frames", "2").await;
    wait_for_label(cx, handle, "dvpp-vdec-3-detail-average_task_us", "2300 µs").await;
    wait_for_label(cx, handle, "dvpp-rc-3-detail-configured_kbps", "8.00 Mbps").await;
    wait_for_label(cx, handle, "dvpp-h264e-3-detail-encode_succeeded", "998").await;
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        for size in [12., 20.] {
            cx.update_window(handle.into(), |_, window, cx| {
                Theme::change(mode, Some(window), cx);
                Theme::global_mut(cx).font_size = px(size);
            })
            .unwrap();
            cx.run_until_parked();
            in_frame(cx, handle, |window, _| {
                let panel = window.find("monitor").bounds();
                let mut edge = None;
                for id in [
                    "dvpp-vdec-3-detail-average_task_us",
                    "dvpp-venc-3-detail-queued_frames",
                    "dvpp-rc-3-detail-configured_kbps",
                ] {
                    let bounds = window
                        .find((ElementId::from(SharedString::from(id)), "value-cell"))
                        .bounds();
                    assert!(bounds.left() >= panel.left() && bounds.right() <= panel.right());
                    if let Some(edge) = edge {
                        assert_eq!(bounds.right(), edge);
                    }
                    edge = Some(bounds.right());
                }
            });
        }
    }
    // 失败保留旧值但不再作为实时通道数，其他来源继续保持本轮状态
    let broken = sample.replace(
        "@@node /proc/umap/vdec ready",
        "@@node /proc/umap/vdec failed",
    );
    factory.monitor_answers("dvpp", &[&broken]);
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(
            Box::new(shellrs::app::RefreshMonitor(Some(
                shellrs::monitor::CollectorDomain::Dvpp,
            ))),
            cx,
        )
    });
    wait_for_label(cx, handle, "dvpp-vdec-count", "— 路通道").await;
    wait_for_label(cx, handle, "dvpp-venc-count", "1 路通道").await;
    wait_for_label(cx, handle, "dvpp-vdec-3-detail-average_task_us", "2300 µs").await;
    factory.monitor_answers("dvpp", &[idle]);
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(
            Box::new(shellrs::app::RefreshMonitor(Some(
                shellrs::monitor::CollectorDomain::Dvpp,
            ))),
            cx,
        )
    });
    wait_for_label(cx, handle, "dvpp-venc-count", "0 路通道").await;
    in_frame(cx, handle, |window, _| {
        assert!(window.try_find("dvpp-venc-3").is_none())
    });
}

#[gpui_kit::test]
async fn ascend_dvpp_many_channels_can_be_expanded(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[FIRST]));
    factory.monitor_answers("probe", &["@@node /proc/device-tree/compatible ready\nhisilicon,ascend610\n@@endnode\n@@node /proc/umap/venc ready\n@@endnode\n@@complete"]);
    let mut sample = "@@node /proc/umap/venc ready\n[VENC] Version: [HiDVPP]\n-----VENC CHN ATTR 1-----\n ID Width Height Type\n".to_owned();
    for id in 0..10 {
        sample.push_str(&format!(" {id} 1920 1080 H264\n"));
    }
    sample.push_str("@@endnode\n@@complete");
    factory.monitor_answers("dvpp", &[&sample]);
    let handle = open_monitor(cx, factory);
    in_frame(cx, handle, |window, cx| {
        window.within("monitor-pages").click(1usize, cx)
    });
    wait_for_label(cx, handle, "dvpp-venc-count", "10 路通道").await;
    wait_for_label(cx, handle, "dvpp-venc-0-driver_fps", "—").await;
    in_frame(cx, handle, |window, cx| {
        assert!(window.try_find("dvpp-venc-8").is_none());
        window.click("dvpp-all-channels", cx);
    });
    in_frame(cx, handle, |window, cx| {
        assert!(window.try_find("dvpp-venc-9").is_some());
        window.click("dvpp-all-channels", cx);
    });
    in_frame(cx, handle, |window, _| {
        assert!(window.try_find("dvpp-venc-8").is_none())
    });
}

#[gpui_kit::test]
async fn jetson_authorization_load_runtime_and_details_remain_distinct(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[FIRST]));
    let base = "/sys/class/devfreq/154c0000.nvenc";
    let usage = "/sys/kernel/debug/tegra-host1x/actmon/nvenc/module0/usage";
    let mut probe =
        "@@node /proc/device-tree/compatible ready\nnvidia,tegra234\n@@endnode\n".to_owned();
    let mut public = String::new();
    for (field, value) in [
        ("cur_freq", "793600000"),
        ("governor", "tegra_wmark"),
        ("device/power/runtime_status", "active"),
        ("device/power/runtime_active_time", "1000"),
        ("device/power/runtime_suspended_time", "4000"),
    ] {
        probe.push_str(&format!("@@node {base}/{field} ready\n@@endnode\n"));
        public.push_str(&format!(
            "@@node {base}/{field} ready\n{value}\n@@endnode\n"
        ));
    }
    probe.push_str(&format!("@@node {usage} denied\n@@endnode\n@@complete"));
    let active = format!("{public}@@node {usage} ready\n375\n@@endnode\n@@complete");
    let denied = format!("{public}@@complete");
    factory.monitor_answers("probe", &[&probe]);
    factory.monitor_answers("tegra", &[&denied]);
    factory.monitor_answers("monitor-authorize", &["@@shellrs-monitor-access ready\n"]);
    let handle = open_monitor(cx, factory.clone());
    in_frame(cx, handle, |window, cx| {
        window.within("monitor-pages").click(1usize, cx)
    });
    wait_for_label(cx, handle, "tegra-154c0000.nvenc-runtime", "已唤醒").await;
    wait_for_label(cx, handle, "tegra-154c0000.nvenc-frequency", "794 MHz").await;
    wait_for_label(cx, handle, "tegra-154c0000.nvenc-load", "—").await;
    in_frame(cx, handle, |window, cx| {
        window.click("tegra-154c0000.nvenc-authorize", cx)
    });
    in_frame(cx, handle, |window, cx| {
        window.click("monitor-sudo-password", cx);
        window.input("jetson-test-secret", cx);
    });
    factory.monitor_answers("tegra", &[&active]);
    in_frame(cx, handle, |window, cx| window.click("commit", cx));
    wait_for_label(cx, handle, "tegra-154c0000.nvenc-load", "37.5%").await;
    assert_eq!(factory.monitor_count("monitor-authorize"), 1);
    assert!(
        !factory
            .exec_commands()
            .iter()
            .any(|c| c.contains("jetson-test-secret"))
    );
    in_frame(cx, handle, |window, cx| {
        window.click("monitor-details-tegra-154c0000.nvenc", cx)
    });
    wait_for_label(
        cx,
        handle,
        "tegra-154c0000.nvenc-detail-usage",
        "375 / 1000",
    )
    .await;
    wait_for_label(
        cx,
        handle,
        "tegra-154c0000.nvenc-detail-governor",
        "tegra_wmark",
    )
    .await;
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        for size in [12., 20.] {
            cx.update_window(handle.into(), |_, window, cx| {
                Theme::change(mode, Some(window), cx);
                Theme::global_mut(cx).font_size = px(size);
            })
            .unwrap();
            cx.run_until_parked();
            in_frame(cx, handle, |window, _| {
                let panel = window.find("monitor").bounds();
                let mut edge = None;
                for id in [
                    "tegra-154c0000.nvenc-detail-usage",
                    "tegra-154c0000.nvenc-detail-governor",
                    "tegra-154c0000.nvenc-detail-cur_freq",
                ] {
                    let bounds = window
                        .find((ElementId::from(SharedString::from(id)), "value-cell"))
                        .bounds();
                    assert!(bounds.left() >= panel.left());
                    assert!(bounds.right() <= panel.right());
                    if let Some(right) = edge {
                        assert_eq!(bounds.right(), right);
                    }
                    edge = Some(bounds.right());
                }
            });
        }
    }
    let sleeping = active.replace("\nactive\n", "\nsuspended\n");
    factory.monitor_answers("tegra", &[&sleeping]);
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(
            Box::new(shellrs::app::RefreshMonitor(Some(
                shellrs::monitor::CollectorDomain::Tegra,
            ))),
            cx,
        )
    });
    wait_for_label(cx, handle, "tegra-154c0000.nvenc-runtime", "已休眠").await;
    wait_for_label(cx, handle, "tegra-154c0000.nvenc-load", "—").await;
    wait_for_label(
        cx,
        handle,
        "tegra-154c0000.nvenc-detail-usage",
        "375 / 1000",
    )
    .await;
}

#[gpui_kit::test]
async fn psi_real_collector_output_displays_values_and_refreshes(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[FIRST]));
    let sample = include_str!("../fixtures/monitor/linux-5.15-psi.txt");
    let mut probe = String::new();
    for path in [
        "/proc/pressure/cpu",
        "/proc/pressure/memory",
        "/proc/pressure/io",
    ] {
        probe.push_str(&format!("@@node {path} ready\n@@endnode\n"));
    }
    probe.push_str("@@complete");
    factory.monitor_answers("probe", &[&probe]);
    factory.monitor_answers("psi", &[sample]);
    let handle = open_monitor(cx, factory.clone());
    in_frame(cx, handle, |window, cx| {
        window.within("monitor-pages").click(1usize, cx)
    });
    wait_for_label(cx, handle, "monitor-value-/proc/pressure/cpu/some", "0.1%").await;
    wait_for_label(
        cx,
        handle,
        "monitor-value-/proc/pressure/memory/some",
        "0.0%",
    )
    .await;
    wait_for_label(
        cx,
        handle,
        "monitor-value-/proc/pressure/memory/full",
        "0.0%",
    )
    .await;
    wait_for_label(cx, handle, "monitor-value-/proc/pressure/io/some", "2.9%").await;
    wait_for_label(cx, handle, "monitor-value-/proc/pressure/io/full", "2.9%").await;
    in_frame(cx, handle, |window, _| {
        assert!(
            window
                .try_find("monitor-value-/proc/pressure/cpu/full")
                .is_none()
        );
        assert!(window.try_find("monitor-status-psi").is_none());
    });
    factory.monitor_answers("psi", &[&sample.replace("avg10=0.12", "avg10=12.34")]);
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(
            Box::new(shellrs::app::RefreshMonitor(Some(
                shellrs::monitor::CollectorDomain::Psi,
            ))),
            cx,
        )
    });
    wait_for_label(cx, handle, "monitor-value-/proc/pressure/cpu/some", "12.3%").await;
}
