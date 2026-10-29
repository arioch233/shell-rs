# ShellRS Fork Features

<p align="center"><b>English</b> | <a href="fork-features.zh.md">简体中文</a></p>

Updated 2026-10-30

This fork is maintained by [arioch233](https://github.com/arioch233/shell-rs) and based on [since2006/shell-rs](https://github.com/since2006/shell-rs), with the original copyright notices and [GPLv3 license](../LICENSE) preserved. This page describes additions and extensions implemented on the current branch. Existing SSH connections, terminals, dual-pane SFTP, port forwarding, credentials and sync continue to use the upstream implementation.

## At a glance

| Area | Additions or improvements in this fork |
| --- | --- |
| Resource monitoring | Combined system, hardware and process views; cards based on detected capabilities; distinct live, historical and error states |
| Rockchip | RGA, RKNN NPU, MPP/VPU, GPU/DDR frequency data, temperatures and resource pressure |
| Jetson Orin NX | GPU utilization and temperature, media-engine runtime states, ACTMON load and awake-time ratios |
| Ascend 310B | AI CPU, control CPU, memory bandwidth utilization, memory frequency, and DVPP video and image processing data |
| Monitoring permissions | In-app setup of passwordless, read-only access to fixed nodes, without editing sudoers by hand |
| File management | Remote files dragged out to Finder or the desktop on macOS; modification-time and type sorting in the terminal file sidebar |
| Bookmarks and terminals | Optional aliases, long-name truncation, alias/path search, and directory commands inserted into the corresponding terminal |

## Unified resource monitoring

The resource monitor uses the current remote terminal's SSH connection, targets Linux hosts, and is shown in the right sidebar by default. The previous system monitor and GPU/NPU tools are combined into three pages:

- **Overview:** existing system, CPU, memory, network and disk metrics.
- **Hardware:** cards for detected GPUs, NPUs, RGA units, codecs, temperatures and PSI resource pressure.
- **Processes:** system processes or GPU/NPU compute processes.

Hardware cards share a title, device information, live metrics and expandable details. GPU model names take priority over node paths, which move into details. RGA labels show hardware types and core numbers, with scheduler IDs explained in details. Temperatures are grouped by chip zones, CPU and GPU/NPU, with the highest temperature and its sensor highlighted. Long paths and raw output wrap and can be selected and copied.

Cards follow detected capabilities. A Jetson, Ascend or ordinary Linux host does not get unsupported Rockchip RGA/NPU cards merely because debugfs is inaccessible. The selected page, sudo setting and device expansion preferences are saved per host.

### Rockchip

| Module | Information shown |
| --- | --- |
| RGA | Per-unit load by driver scheduler, RGA2/RGA3 types, core numbers and hardware composition |
| RKNN NPU | Per-core load, frequency and driver version; device-wide load stays in details and is not counted as another core |
| GPU / DDR | Frequency and available load per devfreq device; GPU model names come from the driver when available |
| MPP / VPU | Codec sessions, PIDs, formats, resolutions, actual encoding FPS and its sum, core clocks, decoder task indices and their growth rates |
| Temperature / pressure | Thermal sensors and kernel-provided CPU, memory and I/O PSI metrics |

MPP details add configured frame rates, rate control, bitrate, GOP, profile, driver-reported memory, bus clocks, enable counts, driver version and diagnostics. Each source keeps its own status and last successful sample time.

Actual encoding FPS comes from the driver's `fps_calc` field and is separate from configured frame rates. Decoder task growth is not decoding FPS, and clock rates and enable counts are not hardware utilization. Load is shown only when the driver provides a valid load interface.

### NVIDIA Jetson Orin NX

When Jetson is detected, a bounded `tegrastats` sample supplies GPU utilization and temperature alongside devfreq frequency data, making activity visible beyond clock speeds alone. The integrated GPU has one combined card; shared system RAM is not presented as dedicated VRAM.

Orin NX media engines include NVENC, NVDEC, two NVJPG units, VIC and OFA: video encoding, video decoding, JPEG processing, image composition/conversion and optical flow. Where the interfaces are available, the monitor shows:

- Runtime PM active or suspended state, frequency and governor.
- ACTMON hardware activity load, subject to node read permissions.
- The awake-time ratio between two power-counter samples, with cumulative active and suspended times in details.

Suspended engines do not retain an old ACTMON load as a current reading. Awake-time ratios are labeled separately from codec utilization, and frequency is not used to estimate load. Public nodes have been checked on hardware; ACTMON values under real workloads after authorization still need further validation.

### Huawei Ascend 310B

The collector handles integrated devices reporting `Bus-Id = NA` and separate NPU/chip IDs. Alongside existing AI Core utilization, memory, temperature, power and health data, additional queries use the detected device IDs to collect:

- AI CPU and control CPU utilization.
- Memory bandwidth utilization and memory frequency.

DVPP collection reads the VDEC, VENC, VPC, H.264/H.265 encoder and rate-control nodes under `/proc/umap`. Cards group video decoding, video encoding and image processing, showing created channels, formats, dimensions, PIDs, driver-reported frame rates and available encoding bitrates. Configured frame rates, buffers, task counts, timings and diagnostics appear in details.

Created channels are not necessarily processing work. Driver frame rates and task timings are not converted into hardware utilization. Idle output from all six nodes has been checked on hardware; active-channel parsing is covered by simulated samples, and field meanings still need validation under real codec workloads.

## Monitoring permissions and states

Collection uses the SSH login user's permissions by default. For metrics that need elevated access, open the collection settings and configure passwordless reading. Enter the remote sudo password in the app; successful setup enables sudo collection and probes hardware again. The password is not saved.

Authorization is limited to bounded reads of twelve fixed nodes for Rockchip RGA/NPU, the MPP clock tree and Jetson ACTMON. Setup validates the dedicated sudoers file and rolls back on failure. Routine collection does not mount debugfs, change governors or enable driver tracing. Setup requires the SSH user to already have sudo administrative rights; turning off sudo collection does not remove the remote rule.

The monitor distinguishes these states:

- **Unsupported, permission denied, failed and timed out** remain separate; missing data is not filled with `0`.
- **Historical readings** are labeled so an earlier successful sample is not mistaken for current data.
- **Independent collection domains** isolate hardware failures while system sampling, including CPU and memory, continues.
- **Timeout suspension** stops new requests for the affected hardware domain until reconnection. Hidden panels do not start new requests.

Available metrics depend on the board, BSP, driver, kernel interfaces and permissions. Authorization fixes access restrictions; it cannot supply data absent from the driver. The current implementation does not estimate missing hardware utilization and does not offer monitoring-history export or a monitoring Agent/MCP interface.

## SFTP and the terminal file sidebar

**Drag-out downloads on macOS:** drag remote files or directories from an SFTP tab or the terminal file sidebar into a Finder folder or onto the desktop. Multiple selected items can be dragged together. The drop location becomes the download destination, with progress in the source panel; completion is reported to the system only after the download succeeds. Windows and Linux continue to use the app's download controls.

**Sidebar sorting:** the terminal file sidebar adds modification-time and type columns alongside name and size. Column headers cycle through sort directions and the default name order, keeping directories first. Sorting survives refreshes and directory changes, and column widths are adjustable.

Upstream dual-pane transfers, drag-in uploads and resumable transfers remain available. These additions extend system drag-out downloads and the terminal sidebar experience.

## Bookmark aliases and terminal directory navigation

In the SFTP or file sidebar directory/bookmark dialog, give a bookmark an optional alias, such as “Payment logs” for `/srv/services/payment/logs`. Existing aliases can be edited or cleared. Paths remain the bookmark identity and navigation target, so aliases may repeat. Local and remote bookmarks are stored separately; remote aliases travel through the existing sync mechanism, and older bookmarks remain usable.

Long aliases are truncated at the end; long paths are truncated in the middle to keep both ends visible. Hover for the full text. Display truncation never changes stored values, search matching or actual paths.

To use a bookmark in a remote terminal:

1. Press `⌘⇧J` on macOS or `Ctrl+Shift+J` elsewhere, or choose directory bookmarks from the terminal context menu.
2. Search the full alias or path, select with the arrow keys, and press Enter or click to insert a quoted `cd` command into that terminal.
3. Review the command and press Enter in the terminal to execute it. If directory following is enabled, the file sidebar updates from the shell's directory report.

The file sidebar toolbar, directory context menu and bookmark dialog also provide a command to enter a directory in the terminal. The action stays tied to the source terminal and connection; closing or reconnecting requires a new selection. Bookmarks saved in a standalone SFTP tab are also available through the same host's terminal bookmark search.

Terminal navigation currently targets SSH terminals using a POSIX shell; Windows remote shells are unsupported. Inserting the command replaces the current input line and does not execute it automatically. Input is rejected while a full-screen program such as vim controls the terminal. The shortcut can be changed in settings.

## Building and maintenance

This fork is currently available as source only. Upstream installers and update channels do not contain these additions. Build instructions are in the [README](../README.md#building-from-source). The [user manual](manual.md) covers upstream features; detailed fork procedures are currently in the [Chinese manual](manual.zh.md).

Maintenance uses one customized `main`, feature branches and temporary upstream-sync branches, preserving upstream history through merges. A workflow periodically checks upstream and creates a sync PR; checks must pass before merging into the protected main branch. Collection fixtures live in `tests/fixtures/` and are shared by parser and UI tests. See the [Fork maintenance guide](fork-maintenance.zh.md) (in Chinese).
