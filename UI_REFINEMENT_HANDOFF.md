# Block UI 紧凑界面交付与历史交接

更新日期：2026-11-05。用户随后要求“ulw 继续工作”，原暂停状态已被恢复执行取代。

**最新验收见本文件末尾“恢复交付验收”。下文暂停、失败和未验证记录均为历史快照，保留用于追溯，不能覆盖最新证据。** 原生证据总目录为 `target/ui-refinement-20261105/`；不要删除 Delta 归档、研究源码或原生证据。

## 目标与约束

- 原始任务：归档并移除 Delta 全部子工作区；深入研究优秀 UI 框架、桌面应用，逐页检查布局、交互衔接和微动效；实施改进并保留原生证据。
- 用户中途明确修正：“所有页面大小我希望能和Home统一，所以所有的交互，所有的布局都不是绝对死守，你可以根据需要彻底大改。”
- 当前设计目标：全部页面，包括独立崩溃帮助，采用 **360×720 逻辑内容尺寸**。原生外框可能包含阴影和装饰，不等于内容尺寸。
- 集合采用单栏列表→详情→编辑；返回需要保留父级、查询、选择、滚动、草稿和焦点。任务向导和固定操作区适配窄窗。
- 必须在 Linux ARM64 验证实际原生行为和微动效。macOS 截图或测试不能替代这一证据。
- 保留游戏、配置、源安装和安全凭据边界；未进行真实认证或启动游戏。不得把截图、模型调用或机械录制成功当作行为通过。
- 不推送、不发布。当前没有新提交，工作树修改保留原样。

## 仓库与文件入口

- 仓库：`/Users/zeithrold/Documents/Develop/Native/Block`
- 暂停时实际 HEAD：`51eecb4`。
- Delta 归档：`target/delta-archive/README.md`、`cleanup-receipt.json` 及 `children/`。
- 研究、截图、录像和检查日志：`target/ui-refinement-20261105/`。
- 设计：`target/ui-refinement-20261105/research/compact-design.md`。
- 研究：同目录下 `frameworks.md`、`desktop-apps.md`、`surface-review.md`、`crash-compact-audit.md`。
- 早期完整过程便笺：`/var/folders/9h/tzfvkgtn37l50rjh3c2s57vw0000gn/T/ulw-20261105.XXXXXX.md.H3863cME6J`。它包含历史状态；以下交接优先描述暂停时最新情况。
- **不要将 `target/delta-archive` 当 Cargo 缓存删除。** 研究和原生证据也应保留。

## 已完成

### 归档与研究

- 盘点 55 个 Delta 子目录，其中 54 个非空；归档 74,599 个文件、4,566,708,424 字节非构建内容。
- 对源文件与压缩归档逐项比对，验证自包含 Git bundle 和恢复；再次比对后移除了 55 个原始子目录。清理回执记录剩余子目录为空。
- 有用的源码、脏文件、补丁、历史和忽略的证据已保留；近期可复用构建缓存移至 `target/ui-refinement-build`。
- 完成框架与设计系统研究、五个桌面应用源码研究、全页面/旅程/动效盘点和紧凑设计决策。
- 完成原 74 个场景的浅色/深色原生基线采集及联系表检查。基线中严格断言失败有记录，未豁免。

### 已落入工作树的实现

- Router 增加类型化页面/记录、稳定入口 ID、父级栈，以及认证和任务的返回位置。
- 统一窗口初始化尺寸，移除导航时的宽窗伸缩；共享 body/footer 分区替代永久侧栏/详情并排。
- 重排集合、编辑、认证/provider、首次使用、创建/导入、Settings/About 和 CrashView 的窄窗布局。
- 编辑器退出确认记录原始入口及退出动作。Save and leave 关联操作完成结果；失败保留草稿，不凭“是否脏”推断成功。
- 通用后台成功事件不再清空任意表单；保存和创建完成使用对应操作、入口和草稿身份。
- 收敛单栏页面动效；修复 motion layer 缺少 flex 约束导致的滚动区高度问题。
- 修复路径展示的横向截断，展示层增加换行机会，实际路径数据不变。
- 测试驱动固定当前窗口，不再搜索其他测试根并误投输入；保留实际鼠标、滚轮和按键测试。
- 以上仍属于未交付的集成修改，尚未通过最终完整门禁。

## 最新现场：先从这里恢复

### 1. 已证明并局部验证的焦点问题

Linux 原生场景里，编辑 Instance 名称为 `Native QA draft`，Alt+Left 打开退出确认，Escape 保留了草稿，但没有返回名称输入框焦点。

原因已通过源码和失败测试证明：`confirm_editor_exit_if_needed` 在弹层栈为空时仍调用 `Overlays::clear`，后者会强制聚焦页面 fallback，使确认框记住错误的返回焦点。

修改：

- `crates/block-ui/src/navigation/actions.rs`：只有已有弹层时才 clear。
- 新文件 `crates/block-ui/src/games/instance_editor_tests.rs`：增加真实输入框焦点回归；同时用 Result 替换其中一个测试的 expect。

证据：

- `editor-exit-focus-before-fix.log`：回归确实失败。
- `editor-exit-focus-after-fix.log`：当时 compact_tests **3 passed，0 failed**。
- 该通过记录只覆盖当时的测试版本。随后测试扩展了原生关闭/应用退出入口，不能据此宣布最新版本通过。

### 2. 最新发现：原生关闭和应用退出遗漏编辑器守卫

原 `allow_native_window_close` 和 `allow_application_quit` 各自维护部分退出守卫，遗漏新编辑器退出确认。

已扩展同一个回归，依次覆盖：

1. Alt+Left；
2. `allow_native_window_close`；
3. `allow_application_quit`。

每项要求：拒绝直接退出、确认框保存准确的原始退出动作、Escape 返回原输入框且草稿保留。

`native-editor-exit-before-fix.log` 的最新运行已成功编译，并在原生关闭断言处失败：

```text
a native close must retain the dirty editor
test result: FAILED. 0 passed; 1 failed; 596 filtered out
```

**暂停前最后一笔产品修改，尚未编译或测试：**

`navigation/actions.rs` 提取 `intercept_action`，将 UI 动作原有完整守卫链供 `handle`、原生关闭、应用退出共用。包括认证保存锁、输入法组合、动作可用性、整合包/游戏/Settings/编辑器退出守卫。

恢复后第一步应验证这笔修改，不要先堆叠其他改动：

```sh
TMPDIR=/tmp/block-ulw.QrFU3Q \
CARGO_TARGET_DIR=target/ui-refinement-build \
cargo test --locked -p block-ui \
  instance_editor::compact_tests::dirty_editor_prompt_escape_restores_the_actual_text_input_focus \
  -- --exact --nocapture
```

还需要核对共用动作可用性守卫后，后台工作/游戏运行时允许关闭启动器的既有契约。

### 3. 原生动效回归：本地已修，远程仍是旧版本

- Linux `step-001` 实际从已滚动的 Games 详情打开 Instance settings。
- 录像显示 outgoing 详情在淡出前跳回顶部，**动效判定失败**，尽管录制工具机械通过。
- 原因两处：缓存快照早于滚轮/文本更新；另一个无 flex 约束的 inert 包装使冻结滚动偏移被布局钳为 0。
- 本地增加 `capture_motion_origin`，在接受动作和非空引擎消息批次前刷新匹配的已渲染身份；去掉多余包装，将按键捕获放到已约束的 motion layer。
- `outgoing-scroll-constrained-layer.log`：**9 个 navigation 测试通过**，保留布局前后偏移断言。
- **远程二进制尚不包含修复。必须重建并重跑相同原生录像，不能凭单测宣布动效通过。**

### 4. 新场景集已合入，未完成原生验证

最近子任务 `st_01a1ab06` 已结束，无后台实现子任务。

只修改了：

- `runtime/scenarios.rs`
- `games/modpack_import.rs`
- `feedback/crash_view.rs`

原 74 个场景保留，当前注册 **98 个唯一场景**。`accounts-providers` 改为实际 provider 列表。

新增 24 个：

```text
provider-detail
provider-edit
profile-editor-root
profile-editor-data
profile-editor-launch
profile-editor-appearance
instance-editor-exit-dirty
profile-editor-exit-dirty
provider-editor-exit-dirty
games-missing-record
games-detail-long-path
setup-overview-draft
wizard-configuration-invalid
setup-import-long-destination
modpack-choose
modpack-inspecting
modpack-review
modpack-blocked
modpack-stopped
modpack-installing
modpack-complete
crash-overview
crash-logs
crash-long-metadata
```

子任务只报告三文件 rustfmt 检查通过，未运行构建/测试/GUI。父任务集成时发现两个不存在的 `Window::set_title`，已改成编译器提示的 `set_window_title`，之后上述退出回归已成功编译。仍未完成场景截图、可见锚点、滚动和交互验证。

需更新截图尺寸/内容契约和历史 review registry；不要删除历史对照或捏造用户接受。

## 检查状态

- 最近一次完整 UI 套件：`compact-ui-tests-7.log`，**594 passed，1 failed**，总计 595。
- 当时唯一失败为旧测试期待脏编辑器 Cancel 立即退出；迁移为确认草稿保留、实际点击 Discard，原最终断言保留，单独回归通过。
- 后续增加动效和焦点回归，当前测试总数为 597；**最新完整套件尚未运行**。
- `compact-clippy-1.log`：严格 Clippy 仍有 **14 项 lib、22 项 test 诊断**。包括旧宽窗机制、局部样式和测试 panic/expect；未解决完，不得压制。
- 已知编译 warning 至少有：
  - `lib.rs` 的 `window_geometry::initialize_window_size` 不必要限定；
  - `creation_saved` 的未使用 `cx`。
- NavigationBack 在 `available_while_busy` 中似乎未列入；源码已确认缺项，但**尚未新增复现测试或修复**。应检查后台任务期间实际 Back 是否可用，不要直接宣称已验证缺陷。
- 每记录详情滚动保存/恢复、准确返回焦点仍需审计，尚未完成。
- 最终 `just check`、当前源码的正式 app build、Python 全套、diff check、自审均未完成。
- 历史 Python 基线 19 项有 1 项 stale review mapping 失败，需按最终场景集处理。
- LSP 多次超时，曾报告 rust-analyzer/Biome 不可用。最新 apply_patch 仍提示 30 秒 LSP daemon timeout；不等于编译通过或失败，以真实检查为准。

最终门禁来自 justfile，并应保留 --locked：

```sh
just check
cargo build --locked -p block-app --bin block
python3 -B -m unittest discover -s scripts -p 'test_*.py'
GIT_MASTER=1 git diff --check
```

## 原生证据与环境

### macOS

- 独立 CrashView 曾以实际 PID/window 捕获 360×720 完整画面：`native-macos/crash-overview.png`。
- Cua 报告 screen_locked，拒绝输入；没有绕过或解锁。
- 所有权明确的 Mac crash QA 进程已停止，Cua `block-ui-ulw` 会话已结束。
- 独立 CrashView 的实际搜索、来源选择、导出取消、键盘遍历和 Return to profile 尚未验证。

### Linux ARM64

- 任务私有根：`/tmp/block-ui-ulw.QEidnN`。未覆盖共享 `~/block-src`。
- 桌面最后确认：KDE Wayland/XWayland，session 3 active、LockedHint=no。
- 环境最后确认：

```text
DISPLAY=:1
XAUTHORITY=/run/user/1000/xauth_rdOoqm
XDG_RUNTIME_DIR=/run/user/1000
DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus
```

- 恢复后必须重新确认会话、锁屏、PID、窗口和命令所有权；这些标识可能过期。
- SSH 从项目 `.env` 的 `LINUX_ARM64_VM_SSH` 读取，避免输出其他变量或复制凭据。
- 最后构建源包：`linux-source-compact-1.tar`。
- 源包 SHA256：`67c0e0f90fc1b419c4476b3180f77c94d027fcfb34c75a53b245ea019f9412ad`
- 远程旧二进制 SHA256：`f873757dae7fd5c76e95314c3879ae0b188d685e1f1fc0b0ec81a1b1cabb591d`
- 此旧二进制不含最新滚动/焦点/共用退出守卫/98 场景等修改。
- 源码传输必须 `COPYFILE_DISABLE=1`、`tar --no-xattrs`；之前 AppleDouble `._*.json` 污染 locales 导致大量编译错误，已诊断修复。
- SSH 非登录 PATH 需加入 `$HOME/.cargo/bin`。

暂停前最后观察到的原生应用：

```text
PID 902719
X11 window 67108865
/tmp/block-ui-ulw.QEidnN/bin/block --scenario games-detail --theme light
当前页 Instance settings，名称 Native QA draft，无确认框
```

**暂停时保留该应用和合成草稿，没有关闭或删除远程数据。**

原生逻辑尺寸已由实际 paint trace 确认为 360×720；xdotool 外框 402×762 包含装饰/阴影。

- trace：远程 `evidence/linux-games-journey-1.jsonl`。
- 启动使用 `BLOCK_FRAME_MOTION_TRACE` 和 `BLOCK_FRAME_MOTION_MANUAL=1`；MANUAL 阻止自动 UiAction 重放，录制的是实际 OS 输入。
- 有界驱动：本地 `target/ui-refinement-20261105/run_native_case.py`。
- case：同目录 `cases/`；已取回证据 `native-linux/step-001`、`step-002`、`step-003`。
- 驱动验证 PID/可执行文件/窗口/未锁屏，提前记录每个 xdotool 参数，录制等待首帧，输入后等待真实 paint，截图并停止录像。机械 PASS 仍标记 visual=PENDING。
- 所有已启动的逐步录像器已结束。首个 120 秒广域录像也结束；不能据此声称动画帧率或延迟达标。

最新原生关闭尝试：

- `step-004-native-close-prompt` 使用 `xdotool key --clearmodifiers alt+F4`。
- 没有等到新 paint，驱动超时；随后只读探针确认应用仍活着、窗口仍 active、画面仍是编辑器。
- 已查看 `linux-after-close-probe-failure.png`，无退出确认框。因此此尝试**不是原生关闭守卫通过证据**。
- VM xdotool 为 `3.20160805.1`，help 只有 windowclose/windowkill，没有 windowquit。
- `xdotool windowclose` 会销毁窗口，不适合证明应用的 graceful close/草稿确认；早期预检已经观察到窗口消失但进程仍活着。不要混用或使用 windowkill 替代。
- 下一次需要可证实触发实际原生关闭协议的办法；尚未实施。不要重试不明的 Alt+F4 并宣称通过。

## 暂停时后台与资源

- 最新测试监视器 bash_118 已失败结束；没有为最后的共用守卫修改启动新检查。
- 全部实现子任务均已结束。
- 只读远程进程生命周期监视器 bash_96 已按暂停要求停止；`bash_output` 确认为 `killed exit_code: 1`。
- 停止工具曾报状态文件删除 EPERM：
  `/Users/zeithrold/.omo/agent/sessions/--Users-zeithrold-Documents-Develop-Native-Block--/extensions/terminal/state/mon_AH6BVQ3FY2JXSXVA`
  随后确实收到 watcher killed 事件。监视器不是应用本身。元数据清除未证实，恢复时留意只读监视器是否重现。
- 本机私有临时根：`/tmp/block-ulw.QrFU3Q`，含 node/bun 缓存与 `crash-qa` 合成数据。
- 远程私有临时根：`/tmp/block-ui-ulw.QEidnN`，含旧应用、源码、构建缓存、输入日志和原生证据。
- 暂停不是收尾清理；上述资源尚未删除。恢复后先保全证据、验证所有权，再处理资源。

## 任务账本

暂停时 **9/19 完成**。保留原状态，未把未验证项目标为完成。

已完成：

1. 盘点 Delta 全部 子工作区 独有 内容 与 活动
2. 归档 Delta 有用 内容 并 验证 可恢复性
3. 移除 已验证 归档 的 Delta 全部 子工作区
4. 深入 学习 优秀 UI 框架 与 设计系统
5. 深入 研究 优秀 桌面 应用 行为 与 源码
6. 逐页 审视 布局 交互 流程 与 微动效
7. 汇总 研究 证据 并 确定 UI 改进 方案
8. 采集 当前 全部 页面 状态 的 原生 基线
9. 建立 有界 原生 输入 录像 截屏 驱动

未完成，恢复顺序：

1. **实现 已确定 UI 改进 并 逐项 观察 渲染**（暂停时进行中）：先验证最后共用退出守卫，再处理剩余源问题。
2. 统一 全部 页面 为 Home 尺寸 并 验证
3. 重组 集合 列表 详情 编辑 与 返回 导航
4. 重排 全部 向导 首次使用 与 导入 页面
5. 适配 关于 崩溃帮助 弹层 与 紧凑 操作
6. 调整 单栏 导航 微动效 并 覆盖 中断 行为
7. 验证 Linux ARM64 原生 页面 流程 与 微动效
8. 运行 相关 测试 完整 检查 并 自审 证据
9. 清理 本次 临时 目录 缓存 与 QA 资源
10. 交付 归档 研究 实现 与 原生 验证 结果

## 工作树边界与恢复注意

暂停时 git status 实际显示约 46 个已修改文件，集中于：

- `crates/block-app/src/main.rs`
- `crates/block-ui/locales/{accounts,common,games}.json`
- `crates/block-ui/src/accounts/`
- `crates/block-ui/src/controls/`
- `crates/block-ui/src/feedback/crash_view.rs`
- `crates/block-ui/src/games/`
- `crates/block-ui/src/navigation/`
- `crates/block-ui/src/overlays/`
- `crates/block-ui/src/runtime/`
- `crates/block-ui/src/settings/`
- `crates/block-ui/src/setup/`
- `crates/block-ui/src/lib.rs`
- `scripts/snap-native`

另有未跟踪新测试 `crates/block-ui/src/games/instance_editor_tests.rs` 和本交接文件。证据主要位于忽略的 target 目录，git status 不会完整列出。

不要 reset、checkout、stash 或回退这些修改来获得干净状态。恢复时以实际源码和 git diff 为准；历史便笺里的“当前 pending monitor”多数已经结束。不要重复已通过且未被后续修改影响的检查；最终源码变化后的必要门禁和原生验证仍须执行。

**本文件保存完成即停止；只有用户要求恢复后才继续。**

## 恢复后的证据

- 已收到继续执行完整目标的指令；原范围和验收条件保持不变。
- `native-editor-exit-after-fix.log`：最后共用守卫修改的聚焦回归已执行并通过，1 passed、596 filtered out。覆盖 Back、原生关闭、应用退出的确认/准确退出动作/Escape 焦点与草稿保留。
- 此为测试窗口证据；Linux 旧二进制尚未更新，不能据此宣布最新原生交互或动效通过。

## 再次停止时的最新状态（优先于以上历史快照）

用户明确要求停止 Goal。已停止后续实现、测试与原生输入；没有提交、推送或完成声明，也没有把未完成目标标为完成或 blocked。

### 已完成的最后一次检查

- `bash_120 / mon_416AHGD4VG1ESNWE` 已正常结束，退出码 0。
- 精确命令为聚焦的 `instance_editor::compact_tests::dirty_editor_prompt_escape_restores_the_actual_text_input_focus` 回归。
- 结果：1 passed、596 filtered out；另一个测试目标 0 passed、3 filtered out。
- 日志：`target/ui-refinement-20261105/native-editor-exit-after-fix.log`。
- 后续清理改动未再编译或测试，因此此结果不能代表停止时整个工作树通过。

### 最后一批 Rust 清理已写入，但未验证

`apply_patch` 已返回以下六个文件更新成功；后续诊断均遇到 LSP daemon 超时。没有因中断重复执行这批补丁。

1. `crates/block-ui/src/lib.rs`
   - 去除 `initialize_window_size` 不必要的模块限定。
   - `creation_saved` 移除未使用的 Context 参数。
   - 删除无调用者的 `instance()` 私有方法。
2. `crates/block-ui/src/runtime/events.rs`
   - 同步 `creation_saved` 调用参数。
3. `crates/block-ui/src/controls/components.rs`
   - 删除无调用者的旧宽窗 `StepperStep` / `stepper`。
4. `crates/block-ui/src/accounts/providers.rs`
   - 删除无调用者的 `draft_id` 和已被 ContentSlots 取代的 `provider_editor` 包装。
5. `crates/block-ui/src/navigation/frame_motion.rs`
   - 删除无调用者的 `is_home_frame` 和生产未使用的 `active` 包装。
   - 原有六处测试读取同一个 `running.get()` 观测值，断言含义保留。
6. `crates/block-ui/src/navigation/shell.rs`
   - 合并 Escape 中 Instance/Profile 编辑器相同的 Back 分支。

**这批清理尚无编译、Clippy、完整测试或原生结果。** 下一次若获准恢复，应先检查实际 diff 并运行受影响检查，不要假定已经绿色。先前计划的动作/焦点收尾清理、WizardSection 清理、busy Back 回归和逐记录滚动修复尚未实施。

### 截图契约子任务已取消，保留了部分改动

- 子任务：`st_01a1ab4a`，截图契约与历史 review registry 迁移。
- 停止时实际状态：`paused -> cancelled`，终态 `cancelled`、资源状态 `disposed`。不会继续执行。
- 子任务日志声称迁移前基线为 19 个测试、2 个失败；这仅是子任务报告，父任务没有复核。
- 子任务已开始修改，取消不等于回滚。停止时实际 `git status --short` 新增显示以下文件已修改：
  - `docs/visual-review-cases.json`
  - `scripts/capture_assertions.py`
  - `scripts/review_cases.py`
  - `scripts/scenario_contracts.py`
  - `scripts/test_capture_assertions.py`
  - `scripts/test_scenario_contracts.py`
- `scripts/test_review_cases.py` 未显示新增修改。
- 没有最终完成报告；迁移、历史行处理、断言强度和 Python 测试结果均未获父任务验收。应作为部分实现接手，不能声明 98 场景契约已完成。
- 恢复时由主任务直接检查并接手这些文件，不恢复已取消的子任务，不覆盖现有修改。

### 资源与剩余工作

- 本次恢复未重新启动 Linux 应用、录像器或原生输入；远程现场仍以上文“最后观察到”记录为准，未重新核实其存活情况。
- 原先只读生命周期监视器 bash_96 已停止；本次聚焦测试 bash_120 已结束；截图契约子任务已取消。
- 任务私有根、构建缓存、研究、归档和原生证据均保留，没有进行收尾删除。
- 总账仍是 **9/19 完成**，十项未完成任务未被标为通过。
- 交接文件和 `instance_editor_tests.rs` 仍未跟踪；所有源码/脚本修改仍留在工作树，没有创建提交。

**停止要求优先于旧 Goal 的继续执行、保持活跃或最终验收要求。当前只交接，不继续推进 Goal。**

### 待办调度已撤销

为执行用户的停止要求，原先十项未完成待办现已逐项标记为 `dropped`，没有开放或进行中的待办。它们没有完成，不能把工具汇总的 “19/19 done” 当作交付完成：实际为 **9 项完成、10 项因用户停止而撤销调度**。

上文十项未完成工作及其证据缺口仍保留，供用户明确授权恢复时重建待办。本次仅更新调度状态和本交接文件，没有继续实现、验证、原生输入或清理资源。

## 恢复交付验收

原范围已恢复实施：统一 360×720 内容窗口，集合列表→详情→编辑，任务向导、首次使用、导入、Settings/About、独立 CrashView 和紧凑弹层。未改动游戏监督进程、认证凭据存储或玩家数据策略。

- 修复已滚动 outgoing 快照跳顶；按导航入口持有详情滚动，并保留逐记录重开位置。
- 共用退出守卫覆盖 Back、原生关闭与应用 Quit；确认取消后保留草稿和实际输入焦点。
- 修复快速连续 Back 的焦点空窗、分类按钮焦点被延迟回调抢走、服务编辑首字段缺失焦点和返回触发按钮身份丢失。
- 修复 HTTP 同意文字、认证错误、账户分组标题及整合包标题在窄窗中的真实问题。
- 独立启动暴露场景二次 resize 掩盖的尺寸问题；共享初始化改为根视图首帧后执行一次，移除场景专用尺寸补偿。独立崩溃页和返回后的普通启动器均实测内容 360×720。

最终检查：

| 检查 | 结果与日志 |
| --- | --- |
| `CARGO_TARGET_DIR=target/ui-refinement-build just check` | PASS；`resumed-check-9.log`，含格式、全工作区检查、严格 Clippy、测试；UI 599 项及框架 3 项通过 |
| `cargo build --locked -p block-app --bin block` | macOS ARM64 PASS：`resumed-build-11.log`；Linux ARM64 PASS：`resumed-linux-build-11.log` |
| Python 全套 | 23 项 PASS：`resumed-python-11.log` |
| 脚本执行入口 | `scripts/review_cases.py --help`、`scripts/snap-native --help` PASS；自审恢复两者原有执行权限 |
| 差异自审 | 守卫、异步完成归属、路由、滚动快照、焦点、布局、测试及脚本逐组检查；78 条历史注册记录保留，新增 98 条当前契约不继承旧语义对照结论 |

原生证据：

- `native-linux/current-visual-review.md`：98 个场景的浅色/深色共 196 张初始截图均已查看；替换和真实滚动补充截图保留来源。
- `native-linux/combined-content-assertions.json`：196/196 内容断言通过；这不代替交互验收。
- `native-linux/current-journey/review.md`：真实滚轮、Back/Escape、输入、原生关闭、Quit、查询/行焦点返回、后台 busy Back、服务按钮、向导草稿及独立 CrashView 搜索与返回。
- `native-linux/toast-traces/review.md`：普通/减少动态 × 浅色/深色四组轨迹分析与画面检查通过。滑出使用产品程序回放，不声称验证了实体手势。
- `native-linux/evidence-final.tar.gz`：远程全部证据和无凭据合成夹具归档；两端 SHA256 均为 `7735b2625e42575afe3e49e16e2ad4190a49bdc12bc8cdd024c482b55ee05706`。
- 最终 Linux 源码包 `linux-source-resumed-11.tar` SHA256：`69e25997abe5af44a29ffb48369aa405fec038c895a6445c24d6017e66262b16`；二进制 SHA256：`7afe07d7b67fb9ae1dac8b979a3d2138fe20be8306277720c6090188f6c30402`。后续仅恢复脚本执行位及更新交接文档，产品代码未变。

限制如实保留：LSP 的 rust-analyzer、basedpyright、Biome 不可用或超时，以实际编译/Clippy/测试为依据。XWayland 全屏录制为黑帧，不能证明跨窗口连续动效；CrashView 逐窗口录像、实际返回动作日志、新窗口归属和稳定目标截图证明搜索及返回行为。未进行真实 Microsoft 认证或游戏玩法验收，也不把 Linux ARM64 结果推广到其他平台。依赖 `block v0.1.6` 的未来兼容提示保留。

实现提交：`d7b65da` — `Unify launcher pages in a compact navigation frame`。界面、相关测试和当前场景契约相互依赖，因此作为一个可构建增量提交；本文件单独记录验收。没有推送或发布。

所有自有 QA 应用已退出，两处远程临时根和两处本地临时根均已清理。回执为 `resumed-remote-cleanup.log`、`resumed-inherited-cleanup.log`、`resumed-current-cleanup.log`。共享 Cargo target、Delta 归档、研究克隆、原生证据和持久便笺保留。交付索引：`target/ui-refinement-20261105/DELIVERY.md`。
