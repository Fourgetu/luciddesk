# MSIX 打包与运行

使用 Windows SDK 的官方 MakeAppx 工具，将普通生产包转换为 MSIX。保留原始 EXE、桌面 DLL 和构建信息，不使用便携数据目录。完整清单校验默认启用。

## 输入与产物

在 Windows 上使用 `tools/package-msix.ps1`，输入由 `tools/package.ps1` 生成的普通生产包解压目录。脚本读取 `build.json`，拒绝便携包和渲染诊断包，并核对 `luciddesk.exe`、`luciddesk_explorer.dll` 与构建记录中的 SHA256；校验失败时先修复输入，不要手动修改哈希绕过检查。

脚本通过 `tools/use-windows-toolchain.ps1` 定位 Windows SDK，使用 MakeAppx 打包；签名时还使用 SignTool。不会重新编译 EXE 或 DLL。

产物位于仓库 `target/msix/<时间戳>/`：

| 产物 | 用途 |
| --- | --- |
| `stage/` | EXE、DLL、构建信息、LICENSE、`msix` 标记、图标及生成清单 |
| `LucidDesk-<包版本>-windows-x64-unsigned.msix` | 未指定证书时的包 |
| `LucidDesk-<包版本>-windows-x64.msix` | 指定证书并完成签名的包 |
| 对应 `.sha256` | 最终包文件的 SHA256 |
| `README.md` | 本说明的副本 |

清单当前声明 x64、`Windows.FullTrustApplication` 和 `runFullTrust`。清单最低系统版本为 `10.0.17763.0`，这是部署声明，不代表所有满足版本号的系统都完成了桌面组件验收。包内资源由脚本明确挑选，不会直接复制输入目录中的所有文件。

## 桌面组件加载

MSIX 打包脚本会在 EXE 同目录生成空文件 `msix`；普通安装包和便携包不包含该标记。应用无标记时保持原来的同目录 DLL 加载，不查询包身份或创建组件缓存。

标记必须是普通文件；目录或重解析点会报错，内容不参与模式判定。有标记时查询 Windows 包身份：如果系统明确返回无包身份，则按普通包处理，直接加载 EXE 同目录 DLL，不创建组件缓存；有包身份时通过 `ApplicationData.Current.LocalFolder` 获取实际 LocalState 目录。查询失败或无法获取包数据目录时仍报告错误，不静默回退。标记用于选择部署方式，不用于证明商店来源或进行安全认证。

| EXE 旁的标记与身份状态 | DLL 来源 |
| --- | --- |
| 没有 `msix` 文件 | EXE 同目录；不查询包身份 |
| 有合法标记，系统明确返回无包身份 | EXE 同目录；不创建缓存 |
| 有合法标记且有包身份 | 包内 DLL 部署到该包的 LocalState 缓存 |
| 标记异常、身份查询失败或包数据路径不可用 | 报错，不静默切换来源 |

判断依据是运行进程的包身份，不是 EXE 文件名，也不靠匹配某个 WindowsApps 路径。添加标记不会给普通 EXE 创建包身份。

包内 DLL 按完整 SHA256 部署到 `LocalState\DesktopComponent\<SHA256>\luciddesk_explorer.dll`。每次连接前比对副本内容，缺失或损坏时使用临时文件原子替换；部署锁避免并发写入，验证后的文件句柄保持到加载完成，阻止期间写入和删除。旧哈希目录只尝试删除其中已释放的 DLL，不递归删除未知内容或跟随重解析点；占用时保留并在下次连接时重试。现有 Explorer 旧组件检查继续阻止新旧组件混用。

`LocalState` 由 `ApplicationData.Current.LocalFolder` 返回，代码不硬编码安装目录或包目录名。`<SHA256>` 是 DLL 内容哈希，不是应用版本号；相同 DLL 内容复用目录，不同内容分别部署。

缓存内容不匹配或缺失时尝试修复；权限错误、异常路径或部署锁被占用时返回错误，不保证任意损坏都能自动修复。旧副本清理是尽力执行，失败不会递归删除整个缓存目录，也不会强制卸载正在使用的 DLL。

普通版无需复制 DLL。MSIX 仍由系统安装、更新和卸载，应用不自行触发更新；LocalState 缓存随包卸载清理。

## 打包与签名

本项目打包脚本按 Store 包版本规则校验：四段数字，各段不超过 65535，第一段不能为 0，第四段必须为 0。默认映射为 `(应用主版本 + 1).次版本.修订版本.0`：应用 `0.15.1` 对应包版本 `1.15.1.0`，`0.16.0` 对应 `1.16.0.0`，应用 `1.0.0` 对应 `2.0.0.0`，保持升级顺序。应用自身版本不变。可通过 `-PackageVersion` 显式指定符合要求的 Store 包版本；给现有用户发布更新时，应大于已发布的适用包版本。文件名使用包版本，`build.json` 保留应用版本。

```powershell
.\tools\package-msix.ps1 -SourcePath '<普通包解压目录>'
```

| 参数 | 说明 |
| --- | --- |
| `-SourcePath` | 必填，普通生产包解压目录 |
| `-IdentityName` | 包身份名称，默认 `Yuchen95.LucidDesk` |
| `-Publisher` | 清单发布者；签名时必须与证书 Subject 完全一致 |
| `-PublisherDisplayName` | 显示的发布者名称，默认 `Yuchen95` |
| `-PackageVersion` | 覆盖默认映射的四段包版本，不改应用版本 |
| `-CertificateThumbprint` | 当前用户个人证书存储中的证书指纹；不提供时生成未签名包 |

默认生成使用配置身份的 `-unsigned.msix`，不能直接双击安装。脚本不会查询已发布版本；手动指定包版本后，需要自行保持后续发布的版本递增顺序。

有签名证书时，指定当前用户个人证书存储中的证书指纹和与证书 Subject 完全相同的发布者：

```powershell
.\tools\package-msix.ps1 -SourcePath '<普通包解压目录>' -Publisher 'CN=Your Publisher' -CertificateThumbprint '<证书指纹>'
```

脚本要求签名证书具有私钥，并在签名后运行 `signtool verify /pa`；签名完成但信任验证失败仍视为失败。脚本不会生成证书或修改证书信任。侧载安装要求签名证书被目标设备信任；自签名证书仅适合测试。Microsoft Store 发布需要将 `IdentityName`、`Publisher`、`PublisherDisplayName` 替换为 Partner Center 分配的身份。

默认使用配置的 Partner Center 身份：包名 `Yuchen95.LucidDesk`，发布者 `CN=407B0E68-BE57-40C1-908A-6AD24F037975`，显示名称 `Yuchen95`。Store 提交包可以保持未签名，由 Microsoft Store 在发布时签名；未签名包不能直接双击安装。应用以 full-trust Win32 方式运行。包清单验证不等于安装及运行测试通过；尤其需要验证 Explorer 桌面 DLL 的加载、退出释放，以及 MSIX 升级、卸载和用户数据行为。商店分发和目标系统的验收要求见[验证与兼容边界](development/validation.md)。

参考：[MakeAppx](https://learn.microsoft.com/en-us/windows/msix/package/create-app-package-with-makeappx-tool)、[MSIX 签名](https://learn.microsoft.com/en-us/windows/msix/package/signing-package-overview)。

## 侧载安装与检查

先准备由目标设备信任的证书签名的包，再执行安装。不要将未签名包改名为普通 `.msix` 来代替签名。

```powershell
Add-AppxPackage -Path '<已签名包的完整路径>'
Get-AppxPackage -Name 'Yuchen95.LucidDesk' |
    Select-Object Name, Version, PackageFullName, InstallLocation, Status
```

自定义包名时替换查询中的名称。查询结果只能证明部署状态，还需实际启动应用，检查桌面分组、Explorer 加载的 DLL 路径及退出后的释放。测试使用的证书、身份和版本应记录在报告中；临时测试信任由测试流程负责清理。

## 更新与卸载

更新由 Windows 的包部署机制处理。旧主程序退出并释放 Explorer 中的桌面组件后，新版启动时根据包内 DLL 的哈希准备匹配副本；相同内容复用同一目录，不覆盖正在使用的不同版本。主程序保留旧组件占用检查，组件尚未释放时等待重连。

使用 PowerShell 侧载更新时，普通 `Add-AppxPackage -Path <新版包>` 在应用占用包资源时可能返回 `0x80073D02`。`-ForceTargetApplicationShutdown` 可关闭目标应用后更新；此命令不负责重新启动 LucidDesk。App Installer 和 Microsoft Store 的更新交互应分别验收。

侧载时可分别验证两种更新方式：

```powershell
# 普通更新：应用占用资源时可能被部署服务拒绝
Add-AppxPackage -Path '<新版已签名包>'

# 允许关闭目标应用后更新；完成后手动启动应用验证
Add-AppxPackage -Path '<新版已签名包>' -ForceTargetApplicationShutdown
```

只提高包版本而保持相同 EXE/DLL，只能验证部署流程及缓存复用。要验证组件切换，使用 DLL 内容确实不同的新包，并核对新加载路径、哈希及旧组件释放情况。

卸载由 Windows 清理包注册及 LocalState 中的组件缓存。应用配置与工作区的数据规则见[存储说明](development/storage.md)，不要把 DLL 缓存等同于全部用户数据。

## 验证

组件单元测试：

```powershell
cargo test -p luciddesk --bin luciddesk desktop_component::tests --locked --offline
```

覆盖普通包不查询身份、标记存在但无身份时的回退、查询错误不回退、SHA256、损坏修复、占用副本保留和并发部署。这些测试不能证明真实包身份、签名信任或 WindowsApps 内运行已通过。

安装验证必须使用实际部署到 WindowsApps 的可信签名包，检查 Explorer 加载路径、退出释放、运行中更新、损坏缓存修复和卸载清理。仅注册展开目录不能替代安装验证。平台与商店分发的限制见[验证与兼容边界](development/validation.md)。

## 常见问题

| 现象 | 优先核对 |
| --- | --- |
| 输入校验失败 | 是否使用普通生产包，EXE/DLL 是否与 `build.json` 配套 |
| 签名失败 | 当前用户证书存储、私钥、指纹与 Publisher 精确匹配 |
| 签名后信任验证失败 | 打包机器的信任链；目标机器也需独立满足信任要求 |
| 安装成功但桌面组件未连接 | 标记、真实包身份、LocalState 路径、缓存内容与 Explorer 旧组件占用 |
| 更新返回 `0x80073D02` | 目标应用是否仍运行或占用包资源 |
| 旧哈希目录仍存在 | DLL 是否仍被占用，目录内是否含未知文件；不应强制删除正在使用的组件 |

反馈问题时记录应用版本和包版本、包身份、Windows 构建号、部署命令与错误码、实际 DLL 加载路径。把打包校验、侧载测试、App Installer 和商店分发结果分别记录，不互相替代。

## 登录自启

设置入口与普通版共用「设置 → 通用」。程序同时确认真实包身份和程序旁的有效 `msix` 标记后，使用 `Windows.ApplicationModel.StartupTask`，不创建 Run 自启项。仅有标记但没有包身份的解压副本走普通版逻辑；有包身份却缺少或无法读取标记时，显示无法确认，不降级写入 Run。

包清单声明稳定任务 ID `LucidDeskStartup`，默认 `Enabled="false"`。用户主动开启后调用 `RequestEnableAsync`，关闭调用 `Disable`；状态读取与修改在后台进行。Task Manager/Windows 启动设置禁用后的 `DisabledByUser` 以及策略控制的状态均只读显示，引导用户到系统设置，不覆盖系统选择。任务 ID 在更新中保持一致，卸载由 Windows 清理包登记。

验证应区分清单校验和真实部署：MakeAppx 校验通过不能替代受信任签名包安装后对启用、停用、任务管理器禁用、更新保留状态及卸载的测试。


## CLI 与 Agent Skill

包内包含同版本的 `luciddesk-cli.exe`、`cli.md`、`protocol.schema.json` 和 `skills/luciddesk-control/SKILL.md`。主程序运行后，在程序目录执行 `./luciddesk-cli.exe status --json` 验证连接。`./luciddesk-cli.exe skill show` 离线显示配套 Skill，Agent 可使用 `skill show --json` 获取结构化结果。

需要让 Agent 自动发现 Skill 时，将完整 `skills/luciddesk-control` 文件夹复制到该 Agent 配置的技能目录；若已有同名技能，先比较内容，保留本地定制。安装程序不会修改 Agent 的配置、技能目录或系统 PATH。程序移动或升级后，使用新目录中的 CLI 与 Skill。操作命令和恢复规则见 `cli.md`。
