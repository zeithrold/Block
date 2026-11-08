# Block 逐场景评审状态

更新日期：2026-11-08

建档基线：`036cc5fef32789aa59da1900d4f42f6cdaa7437e`  
场景契约：初始化场景为 `direct-setup-20261106`，其余为 `compact-ui-20261105`；360×720 逻辑内容尺寸。

本文件记录本轮用户逐场景评审的进度、问题和确认结论。每次只推进一个场景；历史截图检查、自动化断言、测试和 CI 通过均不自动转为用户验收通过。

## 当前进度

| 总场景 | 待评审 | 评审中 | 待修复 | 待复核 | 已通过 | 阻塞 |
| --- | --- | --- | --- | --- | --- | --- |
| 97 | 88 | 0 | 0 | 8 | 1 | 0 |

- 当前验收结论：仅 Welcome 页面（当前 `setup-overview` 首次设置总览）已通过，见 [R016](#r016)；其他页面及变体尚未验收通过。
- 建议下一场景：`setup-account`，等待用户开始评审。
- 当前评审记录：[R001：移除独立欢迎页](#r001)、[R002：首设卡片导航提示](#r002)、[R003：语言抽屉直接选择](#r003)、[R004：语言英文副标题与搜索](#r004)、[R005：语言选中标记](#r005)。
- 已退役场景：`setup-welcome`；其历史证据保留，不计入当前 97 项。
- 统计按场景计数，浅色与深色不重复计数。每次更新场景状态时同步本表。

## 状态与记录规则

| 状态 | 含义 |
| --- | --- |
| 待评审 | 尚未开始本轮评审。 |
| 评审中 | 正在查看或操作，主题、交互或动效仍有项目未检查。 |
| 待修复 | 已确认具体问题，记录复现方式、预期与实际结果。 |
| 待复核 | 修复已落地，或已评审的相关行为发生变化，需要回到原场景验证。 |
| 已通过 | 本次约定范围检查完成，关联问题已闭环，并有用户明确确认。 |
| 阻塞 | 缺少必要环境、数据或操作条件；记录原因和恢复条件，不记为通过。 |

通常按“待评审 → 评审中 → 待修复 → 待复核 → 已通过”推进；没有问题时可由“评审中”直接进入“已通过”。同一个场景只保留一个当前状态，历次结论追加到评审记录，不能覆盖旧问题。

每个场景检查以下内容，并在记录中区分“已看”“已操作”“未验证”和“不适用”：

- **布局与文案**：浅色、深色；信息层级、长文本、滚动到底、固定操作区、窄窗边界。
- **操作衔接**：入口、主操作、返回、取消、关闭、保存；草稿、选中项、滚动和焦点是否按预期保留。
- **反馈与恢复**：等待、忙碌、失败、重试、确认与操作结果是否清楚，是否能继续完成任务。
- **动效与输入**：进入、退出、连续操作和中断；普通动态与减少动态；鼠标、键盘与焦点恢复。

记录实际平台、CPU 架构、显示缩放和代码/二进制来源。“已通过”只覆盖记录中的范围，不推广到未检查的平台。UI 修改的原生验证须包含 Linux ARM64；其他平台按实际影响补充。

源码变更后，只把受影响的已通过场景转为“待复核”，保留未受影响的证据。部分主题通过、截图通过或单个平台通过，都不应掩盖尚未完成的约定检查。

## 清单来源与已有证据

- 场景标识以 [SCENARIOS](../crates/block-ui/src/runtime/scenarios.rs) 为准；本轮覆盖当前全部 97 项。
- [场景注册表](visual-review-cases.json) 中的 97 条 `lifecycle = current` 记录对应本轮清单；其余 78 条历史/退役记录不混入当前进度。这里的用户验收状态与注册表的语义对照状态分别记录。
- [截图契约](../scripts/scenario_contracts.py) 定义当前场景的可见内容断言，不能代替布局与交互判断。
- [已有交付记录](../UI_REFINEMENT_HANDOFF.md) 和 [本地证据索引](../target/ui-refinement-20261105/DELIVERY.md) 可复用；[Linux ARM64 视觉记录](../target/ui-refinement-20261105/native-linux/current-visual-review.md) 包含替换截图与滚动补充说明，选图时需同时核对来源。
- `target/` 下的截图、录像和日志未随源码上传。上述本地链接在没有证据目录的检出中不可用，不能把链接存在当作已查看证据。

这些场景是无网络的合成状态。真实登录、游戏安装/启动/退出、重新打开等跨页流程，需要在相关场景记录中单独写明实际验证结果；完成 97 行不自动等于完整产品或 Release 验收。

## 场景清单

以下按评审主题分组，组内可逐项推进。“评审重点”是检查提示，不是已经发现的问题；记录栏在首次评审后链接到本文件下方的对应记录。

### 首次使用与已有游戏导入（9）

| 场景 ID | 评审重点 | 状态 | 问题 / 记录 |
| --- | --- | --- | --- |
| `setup-overview` | 首次直达总览、标题右侧当前语言、账户优先与返回 Home | 已通过 | [R001](#r001)、[R002](#r002)、[R003](#r003)、[R004](#r004)、[R005](#r005)、[R006](#r006)、[R007](#r007)、[R013](#r013)、[R014](#r014)、[R015](#r015)、[R016](#r016) |
| `setup-overview-draft` | 未完成草稿的继续入口与滚动可达性 | 待复核 | [R014](#r014) |
| `setup-overview-ready` | 任务完成状态与进入 Home | 待复核 | [R014](#r014) |
| `setup-game-choice` | 推荐、自定义与导入的选择含义 | 待复核 | [R008](#r008)、[R013](#r013) |
| `setup-account` | 首次添加账户的入口和退出路径 | 待复核 | [R013](#r013) |
| `setup-game` | 首次创建游戏、保存与后续准备 | 待复核 | [R013](#r013) |
| `setup-import-sources` | 来源识别、复制说明与导入入口 | 待复核 | [R008](#r008)、[R009](#r009)、[R010](#r010)、[R014](#r014) |
| `setup-import-list` | 多项选择、数量反馈与导入操作 | 待复核 | [R008](#r008)、[R009](#r009)、[R010](#r010)、[R011](#r011)、[R012](#r012)、[R014](#r014) |
| `setup-import-long-destination` | 长目标路径、滚动与固定操作区 | 待复核 | [R008](#r008)、[R009](#r009)、[R010](#r010)、[R011](#r011)、[R012](#r012)、[R014](#r014) |

### Home（4）

| 场景 ID | 评审重点 | 状态 | 问题 / 记录 |
| --- | --- | --- | --- |
| `home-empty` | 空状态与下一步操作 | 待评审 | — |
| `home-ready` | 当前游戏、账户与启动操作 | 待评审 | — |
| `home-downloading` | 下载进度、停止与排队启动的区别 | 待评审 | — |
| `home-error` | 失败原因、重试与切换目标 | 待评审 | — |

### Home 选择器与下载详情（4）

| 场景 ID | 评审重点 | 状态 | 问题 / 记录 |
| --- | --- | --- | --- |
| `home-profile-drawer` | 游戏/配置选择、搜索与返回焦点 | 待评审 | — |
| `home-account-drawer` | 账户选择、身份区分与管理入口 | 待评审 | — |
| `home-downloads-drawer` | 下载任务、进度说明与滚动 | 待评审 | — |
| `download-files` | 资源详情、状态与返回下载列表 | 待评审 | — |

### 游戏列表与详情（9）

| 场景 ID | 评审重点 | 状态 | 问题 / 记录 |
| --- | --- | --- | --- |
| `games-bundled` | 普通模式的游戏列表、搜索与创建 | 待评审 | — |
| `games-separated` | 分离模式的实例/配置关系与导航 | 待评审 | — |
| `games-detail` | 详情层级、操作入口与当前游戏状态 | 待评审 | — |
| `games-detail-long-path` | 长路径换行、完整信息与滚动 | 待评审 | — |
| `games-missing-record` | 记录缺失说明与返回列表 | 待评审 | — |
| `games-instance-settings` | 实例设置、编辑焦点与保存/取消 | 待评审 | — |
| `remove-confirmation` | 删除对象、后果说明与文件保留 | 待评审 | — |
| `separate-confirmation` | 单向模式转换的含义与确认 | 待评审 | — |
| `games-help` | 实例/配置解释与设置入口 | 待评审 | — |

### 游戏与配置编辑（6）

| 场景 ID | 评审重点 | 状态 | 问题 / 记录 |
| --- | --- | --- | --- |
| `profile-editor-root` | 名称、服务器与编辑分区入口 | 待评审 | — |
| `profile-editor-data` | 数据策略、默认账户与应用到草稿 | 待评审 | — |
| `profile-editor-launch` | Java、内存、窗口与继承关系 | 待评审 | — |
| `profile-editor-appearance` | 图标、颜色、预览与取消 | 待评审 | — |
| `instance-editor-exit-dirty` | 实例草稿的保存/放弃/继续编辑 | 待评审 | — |
| `profile-editor-exit-dirty` | 配置草稿的退出确认与焦点恢复 | 待评审 | — |

### 游戏创建向导（15）

| 场景 ID | 评审重点 | 状态 | 问题 / 记录 |
| --- | --- | --- | --- |
| `wizard-setup` | 游戏名称、组件入口与创建后果 | 待评审 | — |
| `wizard-version-picker` | 版本搜索、过滤、选择与返回 | 待评审 | — |
| `wizard-loader-picker` | Vanilla 与 Loader 选择 | 待评审 | — |
| `wizard-fabric-picker` | Fabric 构建列表与选择保留 | 待评审 | — |
| `wizard-quilt-picker` | Quilt 构建列表与选择保留 | 待评审 | — |
| `wizard-forge-picker` | Forge 构建列表与选择保留 | 待评审 | — |
| `wizard-neoforge-picker` | NeoForge 构建列表与选择保留 | 待评审 | — |
| `wizard-loader-loading` | 加载反馈与仍可执行的操作 | 待评审 | — |
| `wizard-loader-error` | 请求失败、重试与返回 | 待评审 | — |
| `wizard-loader-empty` | 无兼容构建的解释与下一步 | 待评审 | — |
| `wizard-loader-search-empty` | 搜索无结果与已选构建的关系 | 待评审 | — |
| `wizard-optifine` | 官方 JAR 选择、限制说明与退出 | 待评审 | — |
| `wizard-optifine-selected` | 已选版本、兼容性与移除 | 待评审 | — |
| `wizard-configuration` | 目录/Java/窗口等设置与草稿 | 待评审 | — |
| `wizard-configuration-invalid` | 无效目录的提示、定位与恢复 | 待评审 | — |

### 账户列表与详情（4）

| 场景 ID | 评审重点 | 状态 | 问题 / 记录 |
| --- | --- | --- | --- |
| `accounts-list` | 账户类型、搜索、添加与服务入口 | 待评审 | — |
| `accounts-detail` | 账户身份、认证状态与当前选择 | 待评审 | — |
| `accounts-offline-detail` | 离线身份的限制与使用操作 | 待评审 | — |
| `remove-account-confirmation` | 移除对象与不删除游戏文件的说明 | 待评审 | — |

### 认证服务管理（7）

| 场景 ID | 评审重点 | 状态 | 问题 / 记录 |
| --- | --- | --- | --- |
| `accounts-providers` | 服务列表、协议标记与导航 | 待评审 | — |
| `provider-add` | 名称/地址、识别与添加 | 待评审 | — |
| `provider-http-warning` | HTTP 提示、完整文案与明确同意 | 待评审 | — |
| `provider-detail` | 服务身份、信任目标与编辑/移除 | 待评审 | — |
| `provider-edit` | 编辑识别、保存与取消 | 待评审 | — |
| `provider-editor-exit-dirty` | 服务草稿的退出确认 | 待评审 | — |
| `remove-provider-confirmation` | 移除后果、关联账户与取消 | 待评审 | — |

### 登录与添加账户（17）

| 场景 ID | 评审重点 | 状态 | 问题 / 记录 |
| --- | --- | --- | --- |
| `auth-choose` | 认证方式、主入口与其他选项 | 待评审 | — |
| `auth-information` | 浏览器/设备码方式与开始操作 | 待评审 | — |
| `auth-waiting` | 等待浏览器、重新打开与切换方式 | 待评审 | — |
| `auth-device-preparing` | 请求设备码的等待反馈 | 待评审 | — |
| `auth-device-waiting` | 验证码、复制、打开网页与取消 | 待评审 | — |
| `auth-verifying` | 账户验证状态与取消边界 | 待评审 | — |
| `auth-complete` | 已准备身份与最终保存确认 | 待评审 | — |
| `auth-saving` | 保存期间的反馈与退出限制 | 待评审 | — |
| `auth-error` | 失败解释与实际可用的恢复操作 | 待评审 | — |
| `auth-authlib-provider-add` | 登录流程中添加服务与返回 | 待评审 | — |
| `auth-authlib-information` | 服务目标、账户/密码与信任信息 | 待评审 | — |
| `auth-authlib-http-warning` | 输入凭据前的 HTTP 提示与同意 | 待评审 | — |
| `auth-authlib-characters` | 多角色选择、身份与继续 | 待评审 | — |
| `auth-authlib-complete` | 角色/服务确认与最终保存 | 待评审 | — |
| `auth-offline-information` | 本地名称、身份说明与添加 | 待评审 | — |
| `auth-offline-error` | 名称校验、错误换行与更正 | 待评审 | — |
| `auth-offline-saving` | 保存反馈与重复提交防护 | 待评审 | — |

### Settings 与 About（10）

| 场景 ID | 评审重点 | 状态 | 问题 / 记录 |
| --- | --- | --- | --- |
| `settings-index` | 分类入口、保存状态与返回 | 待评审 | — |
| `settings-general` | 语言/外观/减少动态的预览与保存 | 待评审 | — |
| `settings-game` | 新游戏默认值、Java/JVM 与滚动 | 待评审 | — |
| `settings-profiles` | 数据策略及模式转换说明 | 待评审 | — |
| `settings-downloads` | 下载限制、代理与连接测试 | 待评审 | — |
| `settings-socks5` | SOCKS5 草稿、DNS 语义与保存 | 待评审 | — |
| `settings-invalid-proxy` | 无效代理的错误定位与恢复 | 待评审 | — |
| `settings-storage` | 有效存储路径、只读说明与长文本 | 待评审 | — |
| `settings-exit-dirty` | 保存/放弃/继续编辑及退出目的地 | 待评审 | — |
| `about` | 产品与构建身份、诊断入口及固定操作区 | 待评审 | — |

### 本地整合包导入（7）

| 场景 ID | 评审重点 | 状态 | 问题 / 记录 |
| --- | --- | --- | --- |
| `modpack-choose` | 文件选择、导入范围与取消 | 待评审 | — |
| `modpack-inspecting` | 检查期间的说明与停止 | 待评审 | — |
| `modpack-review` | 名称、目标、可选文件与安装确认 | 待评审 | — |
| `modpack-blocked` | 阻塞原因、长标题与可用操作 | 待评审 | — |
| `modpack-installing` | 安装进度、停止/取消与目标归属 | 待评审 | — |
| `modpack-stopped` | 已停止状态、保留内容与重试 | 待评审 | — |
| `modpack-complete` | 导入结果与完成后的去向 | 待评审 | — |

### 崩溃帮助（3）

| 场景 ID | 评审重点 | 状态 | 问题 / 记录 |
| --- | --- | --- | --- |
| `crash-overview` | 已知原因、建议操作、隐私与返回 | 待评审 | — |
| `crash-logs` | 日志来源、搜索、滚动与导出 | 待评审 | — |
| `crash-long-metadata` | 长游戏/配置名称与固定操作区 | 待评审 | — |

### Toast 动效（2）

| 场景 ID | 评审重点 | 状态 | 问题 / 记录 |
| --- | --- | --- | --- |
| `toast-motion` | 出入场、堆叠、暂停、关闭与中断 | 待评审 | — |
| `toast-motion-reduced` | 减少动态时的反馈、关闭与稳定终态 | 待评审 | — |

## 问题与评审记录

<a id="r001"></a>

### R001：移除独立欢迎页 / 2026-11-06

- 原场景：`setup-welcome`。欢迎页只增加一次开始点击；用户确认直接进入初始化总览。
- 设计决定：移除独立欢迎页，保留语言切换和稍后设置；账户排在游戏之前，但允许先完成任意任务。
- 实现范围：首次进入 Overview，移除 Welcome/Start/返回欢迎页及专用圆形控件；语言入口移到总览顶栏。已有存储位置、任务保存、草稿返回、导入和 Home/准备操作保留。
- 当前版本：直接初始化改动为 `7b14c7b`，语言按钮位置调整为 `58d8b0e`，存储路径一致性修复为 `d631a36`；均已推送。后续卡片提示见 R002。
- 技术验证：完成。冷启动回归已在修改前失败；完整 `just check`、586 项 UI 测试、3 项框架测试和 23 项 Python 测试通过。后续中日文微调分别通过全部 33 项初始化测试和编译翻译一致性检查。
- 原生验证：Linux ARM64 / KDE XWayland / 360×720。九个初始化场景的浅深色和滚动终态已检查；实际输入验证了语言切换与保存、账户返回、先设置游戏、草稿返回与恢复、跳过到主页和重启后保持主页。未进行真实登录、安装或游戏运行验收。
- 新界面：[中文总览](../target/first-run-direct-20261106/evidence/journey-final/final-resume-short-zh.png)、[日文总览](../target/first-run-direct-20261106/evidence/journey-final/final-resume-short-ja.png)、[浅色总览](../target/first-run-direct-20261106/after-final/setup-overview-light.png)、[深色总览](../target/first-run-direct-20261106/after-final/setup-overview-dark.png)。构建来源、复核结论及保留的失败记录见 [验证记录](../target/first-run-direct-20261106/QA.md)；这些本地证据未随源码上传。
- 卡片使用相同内边距和图标尺寸，允许多行文案自然撑高，不强制不同语言的卡片等高。
- 用户确认：已确认设计方向并授权实施；修改后的实际界面尚待用户复核。
- 本轮状态：`setup-welcome` 退役，评审由 `setup-overview` 承接，目前为“待复核”；尚未记为用户验收通过。

后续调整（用户已确认方向）：语言按钮从窗口左上角移到正文标题右侧，显示当前语言名称和下拉箭头，沿用原语言抽屉、保存与焦点恢复逻辑。按钮属于正文标题，随正文滚动。新证据存放于本地 `target/setup-language-20261106/`，不替代用户复核。

按钮调整验证：586 项 UI 测试、3 项框架测试、严格 Clippy、格式及 23 项 Python 测试通过；Linux ARM64 浅深色总览、就绪和草稿状态已查看，真实鼠标/键盘完成打开抽屉、切换中文和关闭后焦点恢复。合成场景无存储引擎，其保存失败提示不代表真实持久化失败；持久化合同由原有测试验证。详见本地 `target/setup-language-20261106/QA.md`。

<a id="r002"></a>

### R002：首设卡片导航提示 / 2026-11-07

- 场景：`setup-overview`；同时检查共享卡片的 `setup-overview-ready` 和 `setup-overview-draft`。
- 问题：账户、游戏卡片的静态外观接近信息卡，缺少明确的可进入提示。用户授权先处理可点击性，存储位置留待后续。
- 修改：两张卡片的图标行右侧加入导航箭头，沿用现有整卡按钮、悬停/按压、键盘焦点和操作入口；已保存标记、文案及卡片内边距保留。
- 版本：`9c28e4f`，已提交并推送，渲染代码仅修改 `setup/first_run.rs`。
- 自动检查：macOS ARM64 上 586 项 UI 测试、3 项框架测试、严格 Clippy 和格式检查通过，现有卡片点击、键盘顺序及紧凑布局断言均保留并通过。
- 原生证据：Linux ARM64 / KDE XWayland / 360×720 产品内容，初始、已保存和草稿状态各自的浅深色共六张截图已查看；箭头、已保存标记及文案均可见，无新增裁切或重叠。
- 实际操作：账户箭头区域悬停时整卡响应，点击进入账户设置，Escape 返回总览；键盘从语言入口依次 Tab 到账户、游戏，游戏卡片 Enter 进入游戏选择。未进行真实登录或游戏安装。
- 证据：[浅色总览](../target/setup-card-affordance-20261107/evidence/setup-overview-light.png)、[深色总览](../target/setup-card-affordance-20261107/evidence/setup-overview-dark.png)及[验证记录](../target/setup-card-affordance-20261107/QA.md)，均为本地文件，未随源码上传。
- 用户确认：实现后的实际界面仍待用户复核；场景保持“待复核”，未记为验收通过。

<a id="r003"></a>

### R003：语言抽屉直接选择 / 2026-11-07

- 场景：`setup-overview` 的语言抽屉。
- 问题：点击标题右侧 English 后，抽屉内还需要再打开 Select，增加了一层选择。用户确认改成直接列出三种语言。
- 修改：抽屉直接展示 English、简体中文、日本語，当前预览语言显示勾选及原生单选状态；每行可直接点击或用 Tab、Enter 操作。语言专属保存成功后关闭抽屉并恢复入口焦点；失败保留抽屉及提示，允许重试同一已勾选项。
- 版本：`410624d`，已提交并推送。设置页原有语言 Select 的预览、取消及保存边界保留；未调整游戏库存储位置的呈现。
- 自动检查：旧实现运行直接选择回归时两项测试失败；最终 586 项 UI 测试、3 项框架测试、严格 Clippy 和格式检查通过。现有草稿、初始化完成状态、语言持久化、背景隔离、焦点包含及恢复断言保留并通过，通用嵌套 Select 的 Escape/Tab 契约仍有真实产品弹层测试覆盖。
- 原生操作：Linux ARM64 / KDE XWayland / 360×720 产品内容；使用独立真实存储引擎，已查看浅深色三行列表，实际点击中文后保存并收起，重开显示中文勾选。注入仅限任务目录的配置解析失败后选择日语，抽屉保留提示且磁盘仍为中文；恢复配置后用 Enter 重试日语，保存成功、自动收起且入口显示键盘焦点环。
- 范围：未进行真实登录、游戏安装或其他平台 GUI 验收；任务进程与临时数据已清理，共享缓存和用户数据保留。
- 证据：[浅色抽屉](../target/setup-language-direct-20261107/evidence/language-list-light.png)、[深色抽屉](../target/setup-language-direct-20261107/evidence/language-list-dark.png)、[验证记录](../target/setup-language-direct-20261107/QA.md)，均为本地文件。
- 用户确认：设计已确认，实际实现待复核；总览仍为“待复核”，未记为验收通过。

<a id="r004"></a>

### R004：语言英文副标题与搜索 / 2026-11-07

- 场景：`setup-overview` 的语言抽屉。
- 要求：在语言自称下补充英文名称副标题，并支持搜索。
- 修改：条目显示英文名 English、Simplified Chinese、Japanese；搜索匹配自称、英文名和语言代码，不区分大小写并忽略首尾空格。提供无结果提示和清除按钮，清除后输入保持焦点，每次重新打开抽屉清空搜索。复用原生输入，查询只属于当前首设语言抽屉。
- 边界：保留勾选和单选语义、过滤后直接保存、成功收起并恢复入口焦点、失败留在抽屉且可重试；切换语言预览时同步搜索提示语言。设置页 Select、账户/游戏选择器和游戏库存储呈现均未改动。
- 版本：`6095ef2`，已提交并推送。
- 自动检查：旧实现在新增真实输入回归中因缺少搜索控件失败；最终 587 项 UI 测试、3 项框架测试、严格 Clippy 及格式检查通过。新增覆盖自称/英文名/代码、大小写、空白、无结果、清除、重开重置及搜索不选择；既有保存测试改为选择过滤后的日语结果，草稿、持久化与焦点断言保留。
- 原生证据：Linux ARM64 / KDE XWayland / 360×720 产品内容；浅深色布局已查看，实际搜索混合大小写 `jApAnEsE` 与大写代码 `ZH-HANS` 均正确过滤。失败注入仅影响独立任务配置：搜索结果选择失败后保留查询和可见提示，磁盘仍为 English；修复配置后 Enter 重试保存日语、收起并恢复入口焦点。重开确认搜索清空，实际验证日语无结果与清除，深色筛选中文后保存成功且未完成初始化。
- 范围：未运行其他平台 GUI 或全工作区 CI；自有进程、源码和测试数据已清理，共享缓存及用户数据保留。
- 证据：[浅色搜索](../target/setup-language-search-20261107/evidence/language-search-light.png)、[深色搜索](../target/setup-language-search-20261107/evidence/language-search-dark.png)、[验证记录](../target/setup-language-search-20261107/QA.md)，均为本地文件。
- 用户确认：实现待复核；总览继续保持“待复核”，未记为验收通过。

<a id="r005"></a>

### R005：语言选中标记 / 2026-11-07

- 场景：`setup-overview` 的语言选择器。
- 要求：选中 Indicator 使用实心圆圈加对勾。
- 修改：18px 实心圆搭配居中的 12px 对勾，使用主题 `primary` / `on_primary` 对比色。标记不可单独获取焦点，沿用现有整行单选状态与动作；未调整搜索、副标题或保存流程。
- 版本：基于 `6095ef2` 的未提交改动，渲染代码只修改 `navigation/shell.rs`。
- 验证：34 项首设回归、严格 Clippy 和格式检查通过；Linux ARM64 浅深色实际截图已查看，无新增裁切或重叠。实际点击浅色圆形标记区域仍保存当前语言并关闭抽屉。此纯视觉改动复用未受影响的完整包测试证据，未重新运行其他平台 GUI 或全平台 CI。
- 证据：[浅色标记](../target/setup-language-indicator-20261107/evidence/filled-indicator-light.png)、[深色标记](../target/setup-language-indicator-20261107/evidence/filled-indicator-dark.png)、[验证记录](../target/setup-language-indicator-20261107/QA.md)，均为本地文件。任务进程和临时数据已清理，保留共享缓存与用户数据。
- 用户确认：实现待复核，总览状态保持“待复核”。

<a id="r006"></a>

### R006：首设游戏提醒与确认页存储位置 / 2026-11-07

- 场景：`setup-overview`、游戏创建流程、`setup-import-sources`、`setup-import-list`、`setup-import-long-destination`、新实例整合包确认。
- 要求：发现可导入游戏时在游戏卡显示绿色 Badge，并执行此前讨论的首设调整。
- 修改：总览移除存储卡，将检测说明放入游戏流程；仅可导入且尚未保存的候选贡献绿色提醒。已知候选优先呈现导入入口，进入流程后不因迟到的扫描结果重排入口。单按钮底栏使用对称间距。
- 位置：创建、来源直接确认、候选列表和新实例整合包确认提供游戏库位置。目录选择只修改所属流程草稿，实际确认并通过预检后才保存；取消、过期请求和其他来源不接受旧回调。保留单游戏独立目录，已有游戏不迁移，缓存位置不变。默认与复制位置相同时只显示一次；保护来源而使用其他目标时另行说明实际位置。
- 自动检查：596 项 UI 测试、3 项框架测试、`cargo clippy --locked -p block-ui --all-targets --all-features -- -D warnings`、格式检查通过。多个新增回归先在旧实现失败，包括来源页直接确认。既有依赖 `block v0.1.6` 的 future-incompatibility 提示仍在。
- 原生证据：Linux ARM64 / KDE XWayland / 360×720 产品内容。浅深色总览、创建、导入来源、候选列表与长路径截图已查看；最终来源页位置入口可滚动到达，底栏固定，无重复目标路径。实际目录选择、取消、待确认预览、独立游戏目录及创建保存已操作；磁盘配置确认默认库与独立目录分别保留。
- 真实发现与复制：使用自有普通配置文件的 Linux 文件租约暂停真实读取，实际查看 Searching 状态后释放；不增加生产测试接口。等待时无绿色误报。随后 Vanilla 1.20.6 单游戏直接导入成功，世界副本存在于所选库，源目录四个文件 SHA-256 前后相同；磁盘实例目录与默认库一致，先前独立创建的 26.3 实例目录未变。未启动 Minecraft 或验证世界玩法。
- 范围与证据缺口：未运行其他平台原生 GUI、全工作区检查；整合包确认、异常及过期回调的分支以自动回归覆盖，本轮未逐个执行其原生故障操作。版本为本轮未提交改动，Linux 二进制由对应源码构建。
- 证据：[验证记录](../target/setup-overview-refinement-20261107/QA.md)、[浅色长路径](../target/setup-overview-refinement-20261107/evidence/reviewed-setup-import-long-destination-light-scrolled.png)、[深色来源确认](../target/setup-overview-refinement-20261107/evidence/reviewed-setup-import-sources-dark-scrolled.png)、[真实扫描等待](../target/setup-overview-refinement-20261107/evidence/live-searching-light.png)、[真实导入结果](../target/setup-overview-refinement-20261107/evidence/live-import-result.png)，均为本地文件。
- 用户确认：实现待复核；未将实现完成记为验收通过，其他尚未评审场景状态保留。

<a id="r007"></a>

### R007：Game 自动导入提示、Drawer 重排与 Wizard 图标 / 2026-11-07

- 当前请求：点击 Game 自动弹出导入提示 Drawer，重新设计 Drawer；Set up a game 与各 Wizard 配上易懂图标。本记录替代 R006 中“Game 只进入选择页”的入口约定，其余数据保护与延迟确认规则保留。
- 入口：先进入游戏选择任务，再打开一层导入 Drawer。复用首设已有或进行中的发现；关闭后仍在同一任务，草稿可继续。Create a new game 返回选择页，不再调用会重新弹提示的 Game 动作；晚到结果不重开提示、不重排已进入页面的入口。
- Drawer：620px 来源提示面板，标题导入图标、发现状态/错误、重试、来源图标及选中状态；当前来源的路径和诊断可查看，多候选保留直达查看入口。整合包入口与来源分组区分；空来源不显示 Import 0，提供明确创建出口。保留候选勾选、返回、结果、完整目的地及 Change、固定确认区与原始文件保护。
- 图标：共享 `wizard_heading` 使用现有24px单色 SVG，与既有20px标题和独立说明对齐。创建/实例选择用 Games，版本用 Blocks，Loader/整合包用 Layers，配置/Profile 用 Settings，OptiFine 用 Leaf；账户按连接、等待、验证、失败和完成状态分别表达。Set up a game 及其入口卡也有对应图标。创建类型标题收紧为三语言等义短标题，避免图标加入后英文孤立问号换行。
- 焦点回归：真实 Linux 操作发现点击重试后，忙碌按钮卸载/禁用会使 Escape 离开 Sheet 路径。新增操作回归先红；仅保留按钮仍失败，读取组件到 pinned gpui-base 的禁用焦点实现后，将焦点交给现有顶层 ImportGames Sheet。实际阻塞读取期间 Escape 现可关闭提示，释放后保持关闭，未加入全局键盘补丁。
- 自动检查：597项 UI测试、3项框架测试、doc0；完整 `block-ui --all-features` 测试、严格 Clippy、格式检查通过。Linux ARM64 最终构建通过。LSP 超时/工具不可用，未将其记为通过；未安装依赖或语言服务。
- 原生覆盖：Linux ARM64 / KDE XWayland，360×720产品内容。38个既有场景的双主题首屏及滚动末端152张均已查看；另外实际进入创建类型、实例选择、Profile 和整合包检查确认，均覆盖浅深色。最终来源/候选/长路径6个主题目标另行复查。标题局部验证不等于完整 Microsoft 登录或逐场景全部功能验收。
- 连续操作：有草稿时 Game 自动弹出，Escape 返回；继续草稿保留名称 Unfinished creative world 与版本1.21.1。真实空态、重试等待、关闭、晚到结果均有截图；失败恢复使用明确的无后端场景夹具。真实单游戏复制前磁盘只有 storage 配置，确认后1/1 Saved，源4文件 SHA-256不变，世界副本哈希相同，目录与缓存符合配置。仅检查自有 `.mrpack`，未安装包；检查前后游戏配置相同。
- 未扩展：既有 `wizard-configuration-invalid` 重复底部错误行在滚动末端横向裁切，字段内错误完整换行；图标修改未改变该错误渲染路径，作为相邻未改问题记录。未运行其他平台 GUI、全工作区 CI、Minecraft 启动/玩法或真实 Microsoft 登录。
- 证据：[本轮 QA](../target/wizard-import-drawer-20261107/QA.md)、[自动提示](../target/wizard-import-drawer-20261107/evidence/draft-auto-drawer.png)、[等待时 Escape 成功](../target/wizard-import-drawer-20261107/evidence/final-searching-escaped.png)、[真实导入](../target/wizard-import-drawer-20261107/evidence/final-import-saved.png)、[最终创建类型](../target/wizard-import-drawer-20261107/evidence/wizard-start-final-light.png)、[深色整合包](../target/wizard-import-drawer-20261107/evidence/pack-review-dark.png)。
- 状态：实现与所述验证完成，未提交或推送；用户待复核。保留97场景清单与既有用户验收状态，不把本轮局部验证自动记作全部场景通过。

<a id="r008"></a>

### R008：导入 Drawer 六点简化与 Wizard 入口 / 2026-11-07

- 最新要求替代 R007 的 Drawer 图标、独立重试与整合包分区：标题和可导入总数无图标；来源卡仅标题、实际数量、路径，View 在所属卡内部；右上角为刷新与 X；整合包移回 Wizard 推荐、自定义、整合包三张主卡，已有游戏导入保留次要入口。
- 未假设 `.minecraft` 只有一个版本或 Vanilla。真实自有相邻来源发现3个候选，进入列表分别显示 Vanilla1.19.4、Fabric/Minecraft1.20.1、Forge/Minecraft1.20.1。本次不修改扫描器。
- 环境与版本：对应未提交源码最终构建，Linux ARM64 / KDE XWayland，360×720产品内容。四场景浅深主题、首屏与滚动末端16张截图已查看，长路径可换行，操作区可达。
- 实际操作：卡片内View3、返回来源、刷新等待与禁用、Escape和X关闭、晚到扫描不重开、次要入口重新打开。Wizard整合包卡打开原生文件选择器，取消后仍为Wizard Modpack第2步，来源Drawer保持关闭。
- 自动检查：597 UI、3框架、15发现测试通过；严格block-ui Clippy、格式检查与最终Linux构建通过。LSP不可用；未运行完整just check、其他平台GUI、真实游戏导入/启动、CF认证下载。未将既有复制证据重算为本轮实际导入。
- 证据：[本轮QA及R008](../target/import-drawer-simplify-20261107/QA.md)、[多版本来源](../target/import-drawer-simplify-20261107/evidence/multi-source-card.png)、[候选列表](../target/import-drawer-simplify-20261107/evidence/multi-version-list.png)、[刷新等待](../target/import-drawer-simplify-20261107/evidence/multi-refresh-wait.png)、[Wizard整合包](../target/import-drawer-simplify-20261107/evidence/wizard-pack-after-picker-cancel.png)。
- CF只读研究另见 `target/curseforge-import-research-20261107.md`；研究报告不代表完整CF导入已实现。没有读取密钥或调用真实Key。
- 自有测试应用、输入设备、扫描锁及两端临时根目录已清理，日志与截图保留，未提交或推送。四个直接修改场景转为待复核；未将实现完成或自动检查通过记为用户验收通过。

<a id="r009"></a>

### R009：导入 Drawer 滚动条侵入卡片 / 2026-11-07

- 用户指出R008截图中的滚动条压入来源卡片。原因：共享滚动组件在同一视口上覆盖绘制10px轨道，而导入内容铺满该宽度，没有留轨道间距。R008关于来源卡边界的视觉检查遗漏由本记录纠正。
- 修复：仅在导入来源、候选列表和进度/结果三个视口增加16px右内边距，容纳轨道及间距；共享滚动组件、标题、底栏和选择状态不变。生产代码为 `games/import_games.rs` 三行，扩展既有Drawer几何用例，保留选择、清空和返回断言。
- 自动检查：旧布局的来源间距断言先失败；最终597 UI、3框架、doc0、严格block-ui Clippy、格式检查通过。LSP超时未记为通过；Linux ARM64最终构建通过。
- 原生检查：对应未提交源码，Linux ARM64 / KDE XWayland，360×720产品内容。来源、候选列表、长目标路径的浅深主题首屏及实际滚动末端12张均已查看；卡片边框与轨道明确分离，长路径换行、末端内容与固定操作正常。
- 范围：进度/结果页复用同一留白及UI回归，未单独执行原生故障导入；未运行全工作区检查、其他平台GUI或真实游戏导入。本次未改变发现、复制或持久化语义。
- 证据：[QA及检查日志](../target/drawer-scrollbar-gutter-20261107/QA.md)、[浅色来源](../target/drawer-scrollbar-gutter-20261107/evidence/setup-import-sources-light.png)、[深色来源](../target/drawer-scrollbar-gutter-20261107/evidence/setup-import-sources-dark.png)、[长路径末端](../target/drawer-scrollbar-gutter-20261107/evidence/setup-import-long-destination-light-end.png)。六个自有应用进程及两端临时目录已清理，未提交或推送。
- 用户确认：待复核，场景统计不变，不将自动检查或截图通过等同于用户验收。

<a id="r010"></a>

### R010：导入 Drawer 移除 Game library 的 Change / 2026-11-07

- 用户指出Block已自动判定路径，确认删除导入Drawer中的Change。本条替代R006/R007关于在导入来源与候选页提供默认库目录选择入口的约定。
- 实现：共享目录组件对 `Scope::Import` 不再渲染Change；保留路径、说明、预检错误和固定操作。自动路径判定、已保存配置优先级、来源重叠保护与复制实现不改；单游戏独立目录及其他创建流程不在本次修改范围。
- 自动回归：来源/候选页的无目录选择器断言先在旧代码失败，修复后598 UI、3框架、doc0通过；严格block-ui Clippy、格式检查及Linux ARM64构建通过。LSP超时，未记为通过。
- 原生：对应未提交源码，Linux ARM64 / KDE XWayland / 360×720产品内容。来源、候选列表、长路径的浅深主题首屏与实际滚动末端12张已全部查看：Change消失，目录与长路径完整，滚动轨道间距和固定操作保持正常。
- 范围：未运行完整just check、其他平台GUI或真实游戏导入。这里的场景目录是合成样本，未将截图宣称为本轮真实导入证据。
- 证据：[本轮QA与日志](../target/import-library-readonly-20261107/QA.md)、[浅色来源](../target/import-library-readonly-20261107/evidence/setup-import-sources-light.png)、[深色来源](../target/import-library-readonly-20261107/evidence/setup-import-sources-dark.png)、[候选列表末端](../target/import-library-readonly-20261107/evidence/setup-import-list-light-end.png)。六个应用进程及两端自有临时目录均已清理，未提交或推送。
- 用户确认：实现待复核，场景统计及其他场景状态不变。

<a id="r011"></a>

### R011：候选多选列表改为右侧Checkbox卡片 / 2026-11-07

- 用户确认候选列表改为整卡可选：左侧游戏名、Minecraft版本与Loader，右侧方形Checkbox；选中强调边框和填充标记。保留全选/清空、底部导入数量与返回选择。
- 实现：每张卡本身就是既有原生Checkbox，文字、说明和诊断纳入同一点击区域，没有外层第二个点击处理器。共享choice为空可见标签时不占用多余文字空间，卡片显式提供真实可访问名称；键盘、焦点、CheckBox角色和禁用语义继续由原生控件负责。
- 回归：扩展既有Drawer用例，旧分割线列表点击metadata不切换，因此先红；新卡片验证metadata、右侧区域和Space只切换当前项一次。保留全选、清空和返回断言，新增已保存/不支持项的禁用与勾选状态检查。
- 自动检查：599 UI、3框架、doc0、严格block-ui Clippy通过；两处纯格式差异修正后格式检查通过。LSP超时未记为通过。Linux ARM64最终构建通过。
- 原生：对应未提交源码，Linux ARM64 / KDE XWayland / 360×720产品内容。候选列表、长路径的浅深主题首屏及实际滚动末端8张均已查看；右侧方框、选中/未选边框、轨道间距、只读目标路径和固定操作正常。
- 实际交互：浅色空白内边距、版本说明、方框、Space依次切换4→3→4→3→4；清空/全选4→0→4，0时导入禁用；来源返回再进入仍选4。深色说明和方框切换4→3→4。未执行真实游戏复制或故障导入，未运行全工作区检查及其他平台GUI。
- 证据：[QA与检查日志](../target/import-candidate-cards-20261107/QA.md)、[浅色卡片](../target/import-candidate-cards-20261107/evidence/setup-import-list-light.png)、[深色卡片](../target/import-candidate-cards-20261107/evidence/setup-import-list-dark.png)、[实际整卡点击](../target/import-candidate-cards-20261107/evidence/live-card-padding-toggle.png)、[深色未选状态](../target/import-candidate-cards-20261107/evidence/live-dark-metadata-toggle.png)。六个原生应用及残留自有等待脚本、两端临时根目录已清理，未提交或推送。
- 用户确认：待复核，场景统计不变。

<a id="r012"></a>

### R012：选中计数与全选/清空合并为一行 / 2026-11-07

- 用户指出Select all独占靠右一行显得突兀，确认合并为列表顶部一行：左侧已选数/可选总数，右侧轻量全选或清空，移除计数独占一行的留白。
- 实现：当前来源的selected/eligible计数与Ghost操作同排，卡片以8px间距直接跟随。操作缩小字号但保留点击高度；三语言同步短清空与双参数计数。分母排除不支持和已保存项，与既有全选及计数规则一致。
- 回归：现有Drawer用例新增卡片紧接选择栏的几何断言，旧双行布局先红；原有整卡、键盘、禁用、全选/清空与返回断言保留，只更新Clear标签预期。599 UI、3框架、doc0、严格block-ui Clippy及格式检查通过；LSP超时/JSON Biome缺失未记为通过，未安装工具。Linux ARM64构建通过。
- 原生：对应未提交源码，Linux ARM64 / KDE XWayland / 360×720产品内容。候选与长路径两场景浅深主题首屏、滚动末端8张已查看；同排对齐、卡片、只读路径、长路径换行及固定操作正常。
- 实际操作：浅深两主题均验证4/4→清空0/4→全选4/4→单卡取消3/4；Clear/Select all、卡片选择和底部导入数量同步，0时导入禁用。原生截图为英文，其他语言复用资源/编译及自动回归；未运行全工作区检查、其他平台GUI或真实游戏导入。
- 证据：[本轮QA及日志](../target/import-selectionbar-20261107/QA.md)、[浅色选择栏](../target/import-selectionbar-20261107/evidence/setup-import-list-light.png)、[深色选择栏](../target/import-selectionbar-20261107/evidence/setup-import-list-dark.png)、[实际部分选择](../target/import-selectionbar-20261107/evidence/live-light-partial.png)。六个自有应用与两端临时目录均已清理，未提交或推送。
- 用户确认：待复核，场景统计不变。

<a id="r013"></a>

### R013：Game 入口互斥分流与全部 Wizard 纯文字标题 / 2026-11-07

- 问题：Game 点击先进入 GameChoice，再无条件打开 Import Drawer，造成两层同时出现，零候选时也出现空 Drawer。回归在旧代码上复现，扫描回调本身不会自动打开 Drawer。
- 修复：仅明确的 `Scan::Found` 从总览直接打开 Drawer，总览保持在后方；空结果、扫描中、失败、未知、仅不支持项及全部已保存时进入创建选择页。晚到发现结果不改变当前目的地。Drawer 的新建操作关闭后再进入 GameChoice，Escape 返回原总览。
- 标题专项：检查全部 12 处 `wizard_heading`，移除共享组件的 Icon 参数及标题装饰。覆盖账户所有阶段、Provider 添加、游戏来源、实例选择、Profile 创建选项、Setup、Version、Loader、OptiFine、Configuration、所有整合包阶段及首次 GameChoice；卡片、状态及返回/关闭功能图标保留。没有修改管理页的身份头像。
- 自动验证：599 UI、3 框架、doc0、严格 block-ui Clippy 与格式检查通过；保留草稿、退出确认、整合包完成、异步晚到与返回的原有断言。具体记录见 [QA](../target/wizard-entry-20261107/QA.md)。
- Linux ARM64 原生验证与资源清理记录见上述 QA；逐场景浅深主题截图仅用于标题专项，不代表真实登录、安装/游戏流程或整场景用户验收。其余尚未开展完整用户评审的 Wizard 场景仍保留“待评审”，不将标题检查扩展为全流程通过。
- 当前状态：待复核，未提交或推送。之前滚动条右移建议不在本次实现范围。

<a id="r014"></a>

### R014：选择卡、来源可见性、轨道与配置状态八项修正 / 2026-11-07

1. 语言与来源单选卡使用原生 Active Radio，语言的即选即存、再次确认当前语言关闭和搜索/焦点合同保留；候选游戏继续使用多选 Checkbox。
2. 来源卡的原生控件包住标题、路径、留白与卡片内容，整卡可选；内部 View 独立激活并阻止事件冒泡。
3. View 的箭头从翻译文本字符改成 16px 语义 SVG，与文字居中对齐。
4. 语言、来源和候选卡的选中边框为 2px，未选为 1px，补偿 padding 保持外部尺寸稳定。
5. 来源名改为 Local Minecraft folder / 本地 Minecraft 文件夹 / ローカルの Minecraft フォルダー；完整路径继续显示。
6. 实际候选数为零的来源不渲染卡片，不再显示本次不存在的 Minecraft Launcher 目标。
7. 卡片与底栏恢复左右 20px 对齐；只将中间滚动/运动区域伸入原有右边距，保留内部 pr4 与所有裁剪。默认 16px rem 下轨道区右端距面板 4px，未新增左留白或改变共享滚动组件。
8. 导入计数使用蓝色 info 角色；Account 和 Game 各自显示灰色 Not Configured 或绿色 Configured，并同步三语言文案，沿用原有就绪判断。

- 反证与自动验证：恢复旧宽度时，卡片右边界396px、底栏412px，精确重现16px不对称。完整599 UI、3框架、doc0通过；扩展后的来源用例再以360/432宽度验证，严格Clippy、格式通过。没有删除、跳过或削弱既有交互断言。
- 原生：Linux ARM64 / KDE XWayland / 360×720。六场景浅深主题首屏与滚动末端24图均已查看；语言卡另有真实浅深主题和中英切换证据。实际点击路径与边缘留白切换来源，箭头进入候选页，来源/候选轨道拖动不改变选择或拖动Drawer。
- 真实发现：自有文件系统中发现Vanilla/Fabric/Forge三项，仅显示一个有游戏的来源；蓝色3 to import、两个灰色状态正常。随后仅在自有隔离库导入三项，结果全为Saved，返回后Game绿色、Account灰色，源三份元数据逐文件cmp一致。没有验证游戏启动、完整运行文件或真实登录。
- 配置状态四种组合与三语言均由实际渲染回归覆盖；原生另验证全未配置、全已配置和仅Game已配置。更多证据、限定范围与资源清理见 [QA](../target/card-refinement-20261107/QA.md)。
- 用户确认：待复核；旧手测实例和数据保留，未提交或推送。

<a id="r015"></a>

### R015：统一大圆角卡片与紧凑 Badge / 2026-11-07

- 用户要求：配置状态 Badge 与导入计数同规格同行；来源数量改为含 chevron-right 的可点击 Badge；所有卡片统一较大圆角并增加 padding；修正 Radio 内圆偏心并检查外圆；选中边框更粗。
- 实现：全应用卡片圆角统一16px；共享卡片padding20px，小型资源/关联卡片8→12px、向导组件卡片12→16px。语言、来源和候选选中边框3px，未选中1px，通过padding补偿保持边界稳定。状态与导入计数均为12px字、16px行高、水平8px/垂直2px内边距，位于图标行。来源数量为24px高、内容宽度的原生按钮，保留整卡Radio和独立入口；单游戏来源显示静态Badge。
- 原因：16px Radio内的7px白点经过GPUI像素取整后偏心半像素，改为8px并新增真实控件几何回归。旧版原生截图放大未复现额外矩形裁切，不加入推测性偏移或裁剪豁免。语言卡片增高后原420px预算不足，Drawer改为540px。
- 自动检查：600项UI、3项框架测试、严格Clippy与格式通过。向导位置卡片的既有覆盖增加真实滚轮输入和完整可见断言，保留草稿和嵌套选择器原断言。Rust LSP不可用，未声称通过。
- 原生：Linux ARM64 / KDE XWayland / 360×720；32场景浅深色首屏/滚动截图128张完成审查，创建方式卡片遗漏圆角已补齐。帮助滚动复核发现默认flex收缩会裁切卡片正文，已保留卡片固有高度并绑定内部viewport，最终浅深色实际滚轮可读到完整第三段和当前模式。真实文件系统发现三项，状态/计数同行同高，Badge箭头进入多选页，正文单次切换到2/3；语言切换及浅深色Radio已操作查看。
- 限定与证据：圆弧未复现额外裁切；修复有实证的内圆偏心。普通Toast场景无Toast不计验收，已开启现有回放取得实际浅深色卡片。个别活动窗口抓图滞后，实际交互改用合成器完整截图。详见 [QA](../target/card-unification-20261107/QA.md)。用户待复核，未提交、未推送。
- 后续顺序调整：导入计数移至配置状态左侧，现为“3 to import → Not Configured → chevron”；44项首设测试、格式及Linux ARM64真实三项发现窗口检查通过，见 [顺序验证](../target/badge-order-20261107/QA.md)。

<a id="r016"></a>

### R016：仅 Welcome 页面验收通过 / 2026-11-08

- 用户确认原话：“首先，仅限Welcome那个页面验收通过了。”
- 对应场景：当前首次设置总览 `setup-overview`；独立旧场景 `setup-welcome` 已退役，不重新纳入清单。
- 本轮状态：仅 `setup-overview` 标记已通过。`setup-overview-draft`、`setup-overview-ready`、语言 Drawer、导入 Drawer、Account、Game 及其他页面均不由此获得验收通过。
- 确认上下文：用户查看的是 Linux ARM64 实例 `Block-Import-20261107.tHK1C7` 的最新总览，包括“3 to import → Not Configured → chevron”排列。此结论不推广到未查看的平台。
- 统计：97 项中已通过1项、待复核8项、待评审88项。没有自动开启下一页面评审。

每次评审后，在此追加对应场景记录，并更新清单中的状态和记录链接。问题按 `R001`、`R002` 连续编号；一个问题影响多个场景时，复用同一编号并列出受影响场景。

记录格式：

```markdown
### 场景 ID / 第 N 次评审 / 日期

- 版本：提交；若为未提交改动，说明改动范围与实际运行二进制来源。
- 环境：OS、CPU、显示缩放、窗口内容尺寸、语言。
- 证据：本轮浅色/深色截图；实际操作或动效录像/日志，注明来源。
- 浅色布局：待验证 / 已看；结论与发现。
- 深色布局：待验证 / 已看；结论与发现。
- 操作衔接：实际入口、操作步骤、预期、结果、返回后的草稿/焦点/滚动。
- 动效与输入：普通/减少动态、连续操作、鼠标/键盘；未验证项或不适用原因。
- 问题：R编号；复现步骤、预期与实际结果、影响范围。
- 修复与复核：修复提交、复核证据、仍未完成项。
- 用户确认：明确结论与日期；未确认时写“待确认”。
- 本轮状态：采用上方状态之一；阻塞时写恢复条件。
```
