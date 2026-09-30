# MSIX 打包

使用 Windows SDK 的官方 MakeAppx 工具，将普通生产包转换为 MSIX。保留原始 EXE、桌面 DLL 和构建信息，不使用便携数据目录。完整清单校验默认启用。

## 桌面组件加载

MSIX 打包脚本会在 EXE 同目录生成空文件 `msix`；普通安装包和便携包不包含该标记。应用无标记时保持原来的同目录 DLL 加载，不查询包身份或创建组件缓存。

有标记时查询 Windows 包身份：如果系统明确返回无包身份，则按普通包处理，直接加载 EXE 同目录 DLL，不创建组件缓存；有包身份时通过 `ApplicationData.Current.LocalFolder` 获取实际 LocalState 目录。查询失败或无法获取包数据目录时仍报告错误，不静默回退。标记用于选择部署方式，不用于证明商店来源或进行安全认证。

包内 DLL 按完整 SHA256 部署到 `LocalState\DesktopComponent\<SHA256>\luciddesk_desktop.dll`。每次连接前比对副本内容，缺失或损坏时使用临时文件原子替换；部署锁避免并发写入，验证后的文件句柄保持到加载完成，阻止期间写入和删除。旧哈希目录只尝试删除其中已释放的 DLL，不递归删除未知内容或跟随重解析点；占用时保留并在下次连接时重试。现有 Explorer 旧组件检查继续阻止新旧组件混用。

普通版无需复制 DLL。MSIX 仍由系统安装、更新和卸载，应用不自行触发更新；LocalState 缓存随包卸载清理。

## 打包与签名

Microsoft Store 要求包版本为四段数字，各段不超过 65535，第一段不能为 0，第四段必须为 0。默认映射为 `(应用主版本 + 1).次版本.修订版本.0`：应用 `0.15.0` 对应包版本 `1.15.0.0`，`0.16.0` 对应 `1.16.0.0`，应用 `1.0.0` 对应 `2.0.0.0`，保持升级顺序。应用自身版本不变。可通过 `-PackageVersion` 显式指定符合要求的 Store 包版本；给现有用户发布更新时，应大于已发布的适用包版本。文件名使用包版本，`build.json` 保留应用版本。

```powershell
.\tools\package-msix.ps1 -SourcePath '<普通包解压目录>'
```

产物位于 `target/msix/<时间>/`，包含 MSIX、SHA256 和说明。默认生成使用商店身份的 `-unsigned.msix`，不能直接双击安装。普通生产包仍可通过 `tools/package.ps1` 构建。

有签名证书时，指定当前用户个人证书存储中的证书指纹和与证书 Subject 完全相同的发布者：

```powershell
.\tools\package-msix.ps1 -SourcePath '<普通包解压目录>' -Publisher 'CN=Your Publisher' -CertificateThumbprint '<证书指纹>'
```

脚本不会生成证书或修改证书信任。侧载安装要求签名证书被目标设备信任；自签名证书仅适合测试。Microsoft Store 发布需要将 `IdentityName`、`Publisher`、`PublisherDisplayName` 替换为 Partner Center 分配的身份。

默认使用配置的 Partner Center 身份：包名 `Yuchen95.LucidDesk`，发布者 `CN=407B0E68-BE57-40C1-908A-6AD24F037975`，显示名称 `Yuchen95`。Store 提交包可以保持未签名，由 Microsoft Store 在发布时签名；未签名包不能直接双击安装。应用以 full-trust Win32 方式运行。包清单验证不等于安装及运行测试通过；尤其需要验证 Explorer 桌面 DLL 的加载、退出释放，以及 MSIX 升级、卸载和用户数据行为。商店分发和目标系统的验收要求见[验证与兼容边界](development/validation.md)。

参考：[MakeAppx](https://learn.microsoft.com/en-us/windows/msix/package/create-app-package-with-makeappx-tool)、[MSIX 签名](https://learn.microsoft.com/en-us/windows/msix/package/signing-package-overview)。

## 更新与卸载

更新由 Windows 的包部署机制处理。旧主程序退出并释放 Explorer 中的桌面组件后，新版启动时根据包内 DLL 的哈希准备匹配副本；相同内容复用同一目录，不覆盖正在使用的不同版本。主程序保留旧组件占用检查，组件尚未释放时等待重连。

使用 PowerShell 侧载更新时，普通 `Add-AppxPackage -Path <新版包>` 在应用占用包资源时可能返回 `0x80073D02`。`-ForceTargetApplicationShutdown` 可关闭目标应用后更新；此命令不负责重新启动 LucidDesk。App Installer 和 Microsoft Store 的更新交互应分别验收。

卸载由 Windows 清理包注册及 LocalState 中的组件缓存。应用配置与工作区的数据规则见[存储说明](development/storage.md)，不要把 DLL 缓存等同于全部用户数据。

## 验证

运行 `cargo test -p luciddesk --bin luciddesk desktop_component::tests --locked` 检查模式选择、内容校验、损坏修复、部署锁及旧副本清理。

安装验证必须使用实际部署到 WindowsApps 的可信签名包，检查 Explorer 加载路径、退出释放、运行中更新、损坏缓存修复和卸载清理。仅注册展开目录不能替代安装验证。平台与商店分发的限制见[验证与兼容边界](development/validation.md)。
