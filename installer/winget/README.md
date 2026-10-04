# WinGet 发布

首个包标识为 `Yuchen95.LucidDesk`，使用 GitHub Release 中的正式 x64 Inno Setup EXE，当前仅声明当前用户安装。清单中的 `/CURRENTUSER` 固定安装范围；静默参数采用 WinGet 对 `InstallerType: inno` 的内置支持。

0.20.1 清单提交：[microsoft/winget-pkgs #446565](https://github.com/microsoft/winget-pkgs/pull/446565)。提交记录不代表已经合并或可从公共源安装；以该 PR 的审核状态为准。

## 本地校验

```powershell
winget validate --manifest installer/winget/manifests/y/Yuchen95/LucidDesk/0.20.1
```

本地安装测试需要管理员启用 `LocalManifestFiles`。记录原状态，测试结束后恢复；安装会关闭正在运行的 LucidDesk。测试应优先在 Windows Sandbox 或测试机器上进行。

```powershell
winget settings --enable LocalManifestFiles
winget install --manifest installer/winget/manifests/y/Yuchen95/LucidDesk/0.20.1 --silent --force --scope user
winget list --name LucidDesk
winget settings --disable LocalManifestFiles
```

## 0.20.1 验证记录

测试环境：Windows 10.0.26300.9550 x64、WinGet 1.29.380。

- 正式 Release EXE 的 SHA256 与发布校验文件一致。
- `winget validate` 通过；`winget install --manifest ... --silent --force --scope user` 返回 0。
- WinGet 的已安装应用列表识别 `0.20.1`，卸载登记 `DisplayVersion` 为 `0.20.1`。
- 正式 EXE 在程序运行时静默覆盖安装返回 0；配置文件及数据库逻辑内容保持不变。
- 安装后的 GUI、CLI、桌面组件、CLI 文档、协议 schema 和全部 6 个 Skill 文件均与 Release 构建清单哈希一致。
- 独立 AppId、名称、数据目录及互斥锁的测试包验证了静默首次安装、仅显示进度的升级、正常关闭等待、降级拦截和静默卸载；登记与程序被清除，未跟踪的用户文件被保留。
- 测试完成后恢复禁用 `LocalManifestFiles`，重新启动正式程序，确认桌面组件连接正常。最终核对配置与面板数据未变；桌面清单仅更新了安装时重建的 LucidDesk 快捷方式的 `identity_key`、`file_id`。

范围：真实安装原本已经是 `0.20.1`，因此正式包测试为同版本覆盖安装。跨版本流程使用同一安装脚本生成的 `99.0.1 → 99.0.2` 隔离测试包；不等同于历史正式版本升级实测。未验证全用户安装或其他 Windows 版本。

## 后续版本

Release 安装包发布完成后，更新版本、固定下载地址和 SHA256，重新验证并向 `microsoft/winget-pkgs` 提交单个版本的清单。不要覆盖已被 WinGet 清单引用的同版本 EXE，否则现有哈希会失效。

```powershell
wingetcreate update Yuchen95.LucidDesk --version <版本> --urls <正式EXE地址>
```

检查生成结果并完成安装测试后再提交。主程序发布不会自动更新 WinGet 源；本仓库 CI 尚未接入清单自动提交。
