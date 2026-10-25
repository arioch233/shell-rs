# Fork 开发与上游同步

本仓库为 [arioch233/shell-rs](https://github.com/arioch233/shell-rs)，基于 [since2006/shell-rs](https://github.com/since2006/shell-rs)。定制基线日期为 2026-10-25，上游基点为 `0eaf97be22814680769f8f709c1efcb191d5e95c`

主要修改包括统一资源监控，Rockchip RGA/NPU/MPP、Jetson 和 Ascend 310B/DVPP 的只读采集与设备专属展示，受限 sudo 授权，SFTP 系统拖出下载、文件排序，以及统一测试样例目录。具体变更由 Git 提交和 PR 记录，历史设计稿不代表所有功能均已交付

## 分支与远程

| 名称 | 用途 |
| --- | --- |
| `main` | 自己的稳定定制版本，跟踪 `origin/main` |
| `origin` | 自己的 Fork，可推送 |
| `upstream/main` | 原作者主分支的远程跟踪引用，不是本地开发分支 |
| `feature/*` | 独立功能和维护修改 |
| `sync/upstream-update` | 当前唯一的上游同步 PR 分支 |
| `release/*` | 将来确需维护旧版本时再创建 |

新克隆配置：

```sh
git clone git@github.com:arioch233/shell-rs.git
cd shell-rs
git remote add upstream https://github.com/since2006/shell-rs.git
git remote set-url --push upstream DISABLED
git fetch --no-tags upstream main
```

不维护 `develop` 或本地上游镜像分支。共享分支只用 Merge，不 Rebase、不强推；上游同步 PR 必须保留上游提交的祖先关系，不能 Squash

## 日常开发

先提交或妥善保存手头改动，再开始下一项功能：

```sh
git switch main
git pull --ff-only origin main
git switch -c feature/example
# 开发并运行下文检查
git add <本次改动的文件>
git commit -m 'feat: 描述具体变化'
git push -u origin feature/example
gh pr create --repo arioch233/shell-rs --base main --head feature/example
```

`main` 要求通过 PR 合并、必需的 `macos` 检查成功、与最新主分支同步且讨论已解决；禁止强推和删除，管理员也遵守。单人维护不要求第二位审查者批准，维护者检查后手动选择 Merge commit。不开启自动合并，合并后自动删除来源分支

## 自动同步与冲突

`sync-upstream` 在北京时间每周一 09:00 检查上游，也可在 Actions 页面手动运行。GitHub 调度可能延迟；定时任务只在默认分支运行，因此首次维护 PR 合并前不会启动

无新提交时结束；有更新时从自己的 `main` 或已有远程同步分支继续，依次 Merge 自己的 `main` 和 `upstream/main`，再推送并创建或更新同一个 PR。已有人工提交会保留，工作流不强推；并发推送被拒时本轮失败，重新运行会获取新的远程状态

仓库 Actions 默认 token 权限保持只读；只有同步工作流声明 `contents: write` 和 `pull-requests: write`。仓库需允许 Actions 创建 PR，该设置虽也允许批准，但工作流不会批准或合并 PR。不需要额外的 PAT 或 GitHub App 密钥

由 `GITHUB_TOKEN` 创建或更新 PR 时，检查可能处于等待批准状态，在 PR 页面选择 **Approve workflows to run**。不能把“工作流创建了 PR”视为测试已经完成；只有必需检查成功后才合并。参考 [GitHub 工作流触发规则](https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/trigger-a-workflow)

合并发生冲突时，工作流撤销本轮未完成的合并，保持远程分支原样；Actions 摘要列出冲突文件和处理命令。首次就冲突时不会创建没有合并结果的 PR，需本地解决后推送并创建 PR：

```sh
git fetch origin
git fetch --no-tags upstream main
# 已有远程同步分支时使用这一条；本地已有该分支时直接切换并 ff-only 拉取
git switch -c sync/upstream-update --track origin/sync/upstream-update
# 尚无远程同步分支时，改用：git switch -c sync/upstream-update origin/main
git merge --no-ff origin/main
# 有冲突先逐文件解决，git add 后 git commit 完成该次合并，再执行下一次
git merge --no-ff upstream/main
# 再次解决冲突、完成合并，然后运行检查
git push -u origin sync/upstream-update
# 尚无 PR 时再创建，已有 PR 会随推送更新
gh pr create --repo arioch233/shell-rs --base main --head sync/upstream-update
```

不要用整文件“全部采用上游”解决监控、SSH 旁路执行或 SFTP 拖放代码的冲突。验证原有功能与定制行为后才合并。需要撤回已经合入的同步时，通过新分支对相应 merge commit 执行 `git revert -m 1 <merge-commit>`，验证后提交回退 PR，不重写 `main`

## 检查与维护入口

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
RUST_TEST_THREADS=4 cargo test --locked
cargo build --locked
bash -n scripts/sync-upstream.sh
python3 scripts/test_sync_upstream.py
```

`ci` 的 macOS 检查覆盖所有主分支 PR、主分支提交和手动运行，不使用路径过滤；Windows 和 Linux shell 专项按相关路径运行，不作为所有 PR 的必需检查。默认测试不启用访问真实钥匙串、板卡或 Docker 的 ignored 场景；本地测试仍需允许本机回环连接、Unix socket 及 AppKit 文件协调

同步脚本的临时仓库测试覆盖无更新、正常合并、重复运行、人工提交保留、自己的主分支前进、冲突撤销和未提交改动保护，不访问远程仓库。CI 不设置发布渠道，不产生可自动更新的发行包

首次维护 PR 包含两个提交：完整定制基线、维护配置。首次 PR 检查成功后配置主分支保护；该 PR 仍由维护者手动合并。本次不顺带同步上游新功能

## 许可证与发行

保留原作者版权、贡献记录、`LICENSE` 和第三方许可；Fork 修改继续遵循 GPL-3.0-only。README 标明来源、主要修改与日期，后续功能在 PR 和提交中持续记录。对外分发修改版二进制时，应提供与该版本对应的完整源码及构建、安装所需资料，不能仅链接不匹配的最新分支。参见 [GPLv3 第 5、6 条](https://www.gnu.org/licenses/gpl)

当前仅维护源码，使用普通 `cargo run` 或 `cargo build --release --locked`，不设置 `SHELLRS_UPDATE_CHANNEL`。普通构建不检查更新。原作者的 release 工作流仅允许在 `since2006/shell-rs` 运行，Fork 中推送版本标签或手动触发都会跳过发布链

README 中原作者下载链接、`docs/release.md` 的签名及 CDN 配置说明均属于上游发行方式。当前不更改应用标识、数据目录、版本号或更新实现，不沿用上游渠道发行自己的包。未来发行前应单独建立 Fork 的版本、签名、下载地址、更新密钥和对应源码交付流程
