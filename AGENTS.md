# Nexus — Agent 指南

本文件是**项目级**约定，配合用户级 `~/.pi/agent/AGENTS.md`（环境总览）一起看。
面向在本仓库改代码的 AI agent，重点是「哪些事会踩坑、哪些不变量不能破」。


---

## 项目概览

**Nexus** —— Windows 桌面下载管理器。UI 用 GPUI-CE（纯 Rust GPU 框架），下载由独立的
`aria2c.exe` 完成，Nexus 自己拉起进程、用 JSON-RPC 驱动、随 app 一起退出。

一个原生 GUI 可执行文件，无 Electron、无内置浏览器、无常驻 daemon。

当前状态：下载闭环可用 + 设置页 + i18n + SQLite 历史/日志。**没有**：日志浏览器 UI、
BT 选项、限速、代理、多选、拖入 `.torrent`、自动更新。

## 常用命令

```sh
cargo run                  # 需要 resources/aria2c.exe，见下
cargo test                 # 15 个测试；aria2 用例会真的起一个 aria2c 下载
cargo clippy --all-targets # 必须零 warning
cargo fmt
```

- **不要用 `python`/`python3`**（Store 占位符）。临时脚本用 `uv run python`，
  例如查数据库：`uv run python -c "import sqlite3; ..."`（stdlib 自带 sqlite3）。
- `cargo test` 会真的花几秒编译/运行 aria2 集成测试，属正常，不要为了「跑快点」去掉它们。

## 环境要点

| 事项 | 说明 |
|---|---|
| `resources/aria2c.exe` | **必需且不入库**（`resources/` 被 gitignore）。缺失时 app 会显示横幅提示，不会静默失败 |
| 引擎查找顺序 | exe 同目录 → `<项目>/resources/` → `PATH` |
| Cargo registry | rsproxy.cn 镜像 |
| 子系统 | `#![windows_subsystem = "windows"]`：**不弹控制台** |
| 崩溃日志 | `./nexus.log`（进程工作目录，panic hook 写入，追加式） |
| 用户数据 | `./`（进程工作目录）：`nexus.db`、`nexus.log`、`state.json.imported`。便携式，不写 profile |
| 屏幕 | 200% DPI |

### 强制工作方式

- **不要自己截图。** 截图由用户完成，放在 `app-screenshot/`（已 gitignore）。
  需要看图时，读用户给的图；可以用 `uv run --with pillow --with numpy python` 放大/量像素。
- **不要用 stdout 调试。** GUI 子系统没有 stderr，`println!` 基本看不到。
  要么写测试，要么用工作目录的 `nexus.log`。
- 改完必须 `cargo fmt && cargo clippy --all-targets && cargo test` 三件套。

- **用脚本整段重写 `.rs` 后，先 `touch` 一下再信编辑器里的报错。** rust-analyzer 的
  `cargo check` 结果是落在 `target/flycheck0/stdout` 的，它靠文件监听触发重跑；
  外部进程（脚本、`sed -i`、格式化工具）一次性覆写文件时监听有时会漏，于是编辑器会
  **继续回放改写到一半时的旧错误**，而那些符号在磁盘上早已不存在。
  判断方法：`rg <报错的符号> src/`。找不到就是陈的。
  解法：`rm -rf target/flycheck0 && touch src/*.rs src/ui/*.rs`，或重启 rust-analyzer 服务。
  不要因为这种幽灵报错去改已经对的代码 —— 以 `cargo check` 的输出为准。

## 架构地图

```
src/
  main.rs       窗口创建、bootstrap、panic hook、mod 声明
  aria2.rs      aria2c 子进程生命周期 + JSON-RPC 客户端（含集成测试）
  state.rs      NexusApp：任务表、筛选、轮询、写穿、迁移（最大的文件）
  model.rs      Task/Status、名称推导、格式化（纯函数，好测）
  store.rs      SQLite：schema、历史、事件日志、设置读写
  settings.rs   持久化偏好：主题/语言/字体/下载目录
  i18n.rs       全部界面文案，每种语言一个 struct
  assets.rs     include_bytes! 嵌入图标
  ui/
    mod.rs        Nexus 自己的控件（库里没有的那些）：IconButton / Chip / ProgressBar /
                  StatusBadge / icon() / CONNECTIONS / speed_options
    root.rs       根视图：标题栏、命令栏、筛选行、列表、新建下载弹窗、确认弹窗
    settings.rs   设置卡片（模态浮层，不占整页；主页永远只显示下载内容）
    task_row.rs   单个下载行
assets/icons/   单色 SVG（GPUI 会当 alpha mask 上色，见下）
resources/      放你自己的 aria2c.exe
STYLE.md        本项目怎么用 Nexus-look（规范本身在库仓库的同名文件里）
```

## 必须遵守的不变量

改代码前先确认没有破坏这些。每一条都是有意设计的，不是随手写的。

1. **所有状态变更都走 `NexusApp` 的方法。**
   持久化和 `cx.notify()` 集中在一处；视图层永远不直接改 engine 或 store，
   而是调用方法 → 方法里 off-thread 派发 RPC → 回调里把结果折回状态。

2. **内存里的 `Vec<Task>` 是 UI 的唯一数据源，SQLite 只写穿、只在启动时读。**
   任何一帧里都不允许查询数据库。

3. **持久化要节流。** 状态变化立刻写；字节数由 `PROGRESS_FLUSH`（3s）控制。
   轮询是 500ms，别在 tick 里无条件写库。

4. **`seq` 是稳定身份。** aria2 的 `gid` 是临时的（重启即失效），
   listener 里一律用 `seq`，不要用 gid 做 key。

5. **任务只由 UI 创建。** 轮询里发现不认识的 gid 要忽略，不要凭空建行。

6. **文案一律进 `i18n.rs`。** 两种语言都要写；视图里不许出现字面量字符串。
   `Strings` 是 struct 不是 map —— 漏翻译会编译失败，这正是它的目的。

7. **颜色一律来自 `Theme`。** 视图里不许出现颜色字面量。
   `Theme` 是 GPUI Global，在 `NexusApp::new` 里按设置安装，切换时用 `cx.set_global`。

8. **`icon(path, size, tint)` 的 tint 是必填参数。** 见下面「GPUI-CE 坑」第 1 条。

9. **数据库必须能优雅降级。** `Store` 的 `conn` 是 `Option`：打不开时 app 照常跑，
   只是不记录，并在设置页显示原因。不要在 `open` 失败时 panic 或让窗口打不开。

10. **弹出的卡片只有一种骨架：`nexus_look::Modal`。**
    设置、新建下载、确认删除全都是 `Modal::new(id, 标题).block(..).action(..)` —— 它自己就是
    元素（`#[derive(IntoElement)]`），直接 `.child(..)` 挂上去，**没有 `.build(theme, window)`**。
    遮罩、卡片宽度（`nexus_look::modal_width`）、圆角、内边距、底色（`theme.surface`）、标题、
    可滚动主体与渐隐、底部分隔线和按钮行——全部由它决定。
    **视图里不得再出现 `.occlude()` / `.rounded(px(16.0))` / `.bg(theme.scrim)` / `stop_propagation()`
    / `justify_end()` 这类手写的卡片外壳。** 需要新的差异时，给 `Modal` 加参数
    （如 `.narrow(nexus_look::layout::MODAL_W_NARROW)`、`.scrolling()`），不要另写一份。
    每张卡片必须至少有一个退出的 `.action(..)` 按钮；不要加关闭叉、不要点遮罩关闭
    （理由见 `STYLE.md` 第 6 节）。

11. **同一个控件只有一处定义。** 想写第二遍之前先去库里找：`nexus_look` 有 `Modal` /
    `Row` / `SettingGroup` / `Segmented` / `Button` / `IconButton` / `TextInput` / `TitleBar` /
    `Toast` / `hint` / `divider` / `card`；没有的（`IconButton` 的旧版 / `Chip` / `ProgressBar` /
    `StatusBadge` / `icon()` / `CONNECTIONS` / `speed_options`）才在 `ui/mod.rs` —— 那些是
    任务行专用的形状，不是可以再抄一份的理由。**弹窗里的一律用库的**：
    `Modal`（卡片骨架）+ `Row`（弹层里的每一行，设置和新建下载共用）+ `SettingGroup`（组头）。
    判据很简单：同一段布局代码出现第二遍就是 bug，不论两处当下多像。

12. **能从 `tasks` 算出来的东西不要存成字段。** 存下来的派生值只会在它自己的更新时机上刷新，
    于是某个入口忘了刷新就留下一个过期数字。「暂停之后还显示速度」就是这么来的：
    速度合计曾经是 `NexusApp` 上的字段，只由**轮询成功**时刷新，而 `pause()` 只清零了那一行 ——
    下一次 tick 之前顶部一直挂着旧速度。现在它在渲染时现算（`model::total_speed`、`NexusApp::speed`）。
    `aria2.getGlobalStat` 也因此整个不用了（见 README 的 Engine notes）。
    加任何「汇总值」之前先问：能不能从这个列表算出来？能就别存。

13. **设置页是草稿式的，只有保存才落盘。** 打开设置时 `open_settings()` 把 `settings` 克隆成
    `draft`；面板里所有控件改的是草稿，渲染一律读 `active_settings()`（开着取草稿，否则取已保存）。
    只有 `commit_settings()` 才把草稿并回 `settings`、**同步**写一次库（`Store::update_settings`，
    一个事务）、把引擎全局推给 aria2；`cancel_settings()` 只是丢掉草稿，再把已保存的主题装回。
    主题 / 语言 / 字体因为渲染直接读 `active_settings()`，所以天然是实时预览，取消即回退。
    因此：**设置控件里不要直接改 `self.settings`，要通过 `active_settings()`/`self.draft`**；
    也不要为设置变更去设 `dirty`（那是给任务行的）。杀掉进程最坏只丢掉没保存的编辑，
    库永远是上一次保存的完整状态。

## GPUI-CE 坑（写 UI 前必读）

详细分析、源码行号和最小复现都在 **`gs-issue.md`**，那里是给 skill 作者的完整报告。
这里只列「会咬到你」的结论：

1. **元素样式是自包含的，不从父级继承。**`compute_style_internal` 从 `Style::default()`
   起步。所以：
   - `text_color` 设在包裹层**不会**给子 `Svg` 上色，而 `Svg` 自己没有颜色时**整个 draw
     被跳过** —— 症状是「位置留着、什么都没有」。`icon()` 强制传色就是为了堵这个。
   - `Window::text_style()` 在 `render` 期间返回的是**窗口默认样式**，不是继承来的。
     量文字宽度（如光标滚动）必须自己拼 `Font`，见 `input_font`。
   - 悬停变色也不能继承，要么挂在元素自己身上，要么用命名 group。
     注意 `group`/`group_hover` 收的是 `Into<SharedString>`，**不是 `ElementId`**。

2. **`.on_click` 需要 `.id()`。** 否则报「no method named on_click」。

3. **原生文件对话框是内置的**：`cx.prompt_for_paths(...)` / `prompt_for_new_path(...)`。
   Windows 后端就是 `IFileOpenDialog`。**不要为了选文件夹加 `rfd` 之类的依赖。**

4. **遮罩层**：`.occlude()` + `.absolute().top_0().left_0().size_full()`，
   **没有 `inset_0()`**；卡片上要 `.on_click(|_, _, cx| cx.stop_propagation())`，
   否则点按钮会同时算作点了遮罩。

5. **`impl Trait` 作参数时，返回值不能写 `+ use<>`**（编译报
   「must mention all type parameters」）。要么去掉 `use<>`，要么显式具名泛型。

6. **`Styled` 没有 `opacity()`**，只有 `ColorExt::opacity`（作用于颜色）。

7. **`WindowControlArea::Max` 系统自己会切换最大/还原**（映射到 `HTMAXBUTTON`），
   只需要按 `window.is_maximized()` 换图标。
   另外 `Drag` 区域必须是窗口按钮的**兄弟节点**，不能是祖先（否则按钮收不到点击）。

8. 没有内置文本输入框 —— **全 app 的输入框都是 `nexus_look::TextInput`**，库里的唯一实现：
   `id` + `track_focus` + chrome + 光标/选区/placeholder + 点击定位 + `handle_input` 全在里面，
   调用方漏不了项（漏 `track_focus` 会静默丢键，见 `gs-issue.md` #5）；底层缓冲和 UTF-16 桥
   收在库的 `text_edit::TextEdit`。Nexus 自己不拼输入框，也**不持有 `TextEdit` / 裸
   `FocusHandle`**（老的 `ui/text_field.rs` 已删）。
   - Enter / Escape 接 `.on_submit(..)` / `.on_dismiss(..)`：`on_submit` 拿得到文本，
     所以 `cx.listener` 就能造；**`on_dismiss` 的签名是 `Fn(&mut Window, &mut App)`，
     没有事件参数，`cx.listener` 造不出来** —— 用 `NexusApp::dismissal(cx, |this, window, cx| ..)`。
   - 根视图的 `handle_keys` 只在 `NexusApp::field_focused(window, cx)` 为假时才处理按键；
     否则同一个 Enter 会把同一个动作跑两遍。

**发现新的 GPUI-CE 或 skill 问题时，追加到 `gs-issue.md`**（带源码行号 + 最小复现）。
用户会定期把它交给 skill 作者，所以要写成「对方能直接照改」的形式。

## aria2 坑

结论写在 `README.md` 的 “Engine notes”。最容易忘的三条：

1. **`system.multicall` 的 token 要放在每个子调用里**，放顶层会得到
   `HTTP 400 "The parameter at 0 has wrong type."`
2. **JSON-RPC 报错走 HTTP 400**，客户端若把 4xx 当错误就会吞掉真实原因
   （已设 `http_status_as_error(false)`）。
3. **必须 `--no-conf=true`**，否则读用户自己的 `aria2.conf`（Motrix 就在跑一个自己的
   aria2，端口 16800）。我们用随机端口 + 每次启动随机 secret + `--stop-with-process`。
4. 子进程用 `CREATE_NO_WINDOW` 启动，避免闪控制台。

## 数据库约定

- 文件：进程工作目录下的 `nexus.db`（便携式，不写 `%APPDATA%`），WAL 模式。
- 三张表：`downloads`（当前队列 + 历史，就地更新）、`events`（**只追加**的审计日志）、
  `settings`（键值，每项偏好一行：`theme`/`language`/`font`/`download_dir`；旧版写的
  `ui` JSON 行会在启动时一次性拆到这些键）。
- 时间戳是 **ISO-8601 UTC 文本**（`2026-10-03T11:52:19Z`），列声明为 `TIMESTAMP`。
  SQLite 没有日期类型，只能存三种东西；在它们中间选文本，是因为它在任何客户端里都读得出是个时间，
  而且 `strftime` 能认——在 `TIMESTAMP` 的标题下塞一个整数是比那个整数本身更大的谎。
  换算全部在 SQL 里（`store.rs` 的 `stamp()` / `stamp_or_null()` / `unix_seconds()`），Rust 侧
  仍然是 `i64` 纪元秒，边界就是这几条语句。用 `stamp_or_null` 的地方不能改成 `stamp`：
  「没有值」本身有意义（没下完的任务没有 `finished_at`，填成「现在」就是给一个从未完成的传输盖章）。
- **`events.seq` 是真外键**（`REFERENCES downloads(seq)`），可为 NULL——引擎级事件属于任何下载。
  注意：**这个 build 的外键是默认开的**，`libsqlite3-sys` 编译时带了
  `-DSQLITE_DEFAULT_FOREIGN_KEYS=1`（见它 `build.rs:153`）。所以别以为「没写 pragma 就没约束」——
  重建表时必须**显式 `PRAGMA foreign_keys = OFF`**：`DROP TABLE downloads` 会先做一次隐式
  `DELETE FROM`，而旧 `events` 还指着那些行，一下就违反约束、整个 `connect()` 失败、库静默降级。
  这个坑真踩过（v4→v5 迁移就是这么炸的）。
- **时间戳列有 `CHECK ... GLOB`，不是只靠声明。** SQLite 的声明类型只是 affinity，它会一声不响地把
  整数存进 `TIMESTAMP` 列——`mark_deleted` 就漏过 `updated_at = now_unix()`，你才在客户端里看到一个数字。
  加了约束之后，写错类型会**整条语句失败**而不是写进坏数据。所以测「值对不对」不够，要断言
  **写确实生效了**（`is_deleted = 1`），否则语句被拒时类型检查照样通过。
- **`downloads` 只用软删除，永远不 `DELETE`。** 从列表里移除一个任务是
  `UPDATE downloads SET is_deleted = 1`（见 `Store::mark_deleted`）。`seq` 是这个 app 的稳定身份，
  `events` 靠它编号，硬删会把「发生了什么」的账目断掉。所以：
  `load_downloads()` 带 `WHERE is_deleted = 0`，而 `Store::highest_seq()` **不带**这个过滤——
  隐藏的行仍然占着自己的号，否则重启后 `seq` 会被重新发放，新任务的日志会挂在旧故事上。
  `upsert_download` 的 `ON CONFLICT` 里带 `is_deleted = 0` 作为安全网。
  注意：**软删除只针对数据库，文件照旧真删**（`discard_download`），这条没变过。
- **改 schema 时**：递增 `store.rs` 的 `SCHEMA_VERSION`，并在 `connect()` 里加幂等修复语句
  （已有先例：把迁移行的 `created_at = 0` 修成 `updated_at`）。两种手法看改什么：
  - **加列用 `add_column()`** —— SQLite 没有 `ADD COLUMN IF NOT EXISTS`，它先查 `pragma_table_info`，
    所以每次启动跑都安全。
  - **改类型 / 加约束只能重建表**（`rebuild()`）——`ALTER TABLE` 做不到。靠**声明的类型**判断
    文件是哪一代（`declared_type()`），而不是 `user_version`：这样空文件不会被白重建一遍。
    重建把外键临时关着（靠 `connect()` 里把它排在 `PRAGMA foreign_keys = ON` 之前），
    因为其中一步就是修那些永远满足不了新外键的行——旧版本硬删过 `downloads`，日志里就会剩下
    指向不存在行的 `seq`；这些事件保留位置、只把 `seq` 置空（见 `an_event_whose_download_is_gone_keeps_its_place`）。
- **测试绝不碰真实数据库**：用 `std::env::temp_dir()` + pid 造独立目录（见 `store.rs`
  的 `scratch()`），不引入 `tempfile`。
- 事件类型用 `Event` 枚举 + `as_str()`，`Event::for_status(from, to)` 负责「从哪到哪算什么事」——
  新增状态转移时改这里，不要在各处硬编码字符串。

## 代码风格

- **注释写「为什么」，不写「什么」。**现有代码里的注释都在解释设计取舍
  （为什么兄弟节点、为什么节流、为什么 pre-populate `seen`），保持这个水准。
  不要写 `// 增加计数` 这种复述代码的注释。
- 每个文件顶部有一句模块级 doc 说明职责，新增文件照做。
- 函数尽量小、职责单一；视图函数一屏内能读完最好。
- 错误处理：面向用户的失败要转成 `warn(...)`（6 秒自动消失）或 banner；
  纯装饰性操作（如清理残留记录）失败可以忽略，但要写明「失败无所谓」。
- 用户可见的错误信息要带上底层原因，不要吞掉成「失败了」。

## 禁止事项

- ❌ 不要自己截图（用户负责），不要为了截图往项目根目录扔临时脚本/图片。
- ❌ 不要在视图里硬编码文案或颜色。
- ❌ **不要手写模态卡片的外壳**（遮罩/宽度/圆角/底色/底部按钮行）——用 `ui::Modal`。
- ❌ **不要把同一段布局写两遍**：先找 `ui/mod.rs`，没有就提上去。
- ❌ 不要在模态里加右上角关闭叉，也不要点遮罩关闭（见 `STYLE.md` 第 6 节）。
- ❌ 不要为选文件/选目录引入第三方 GUI 依赖。
- ❌ 不要在每一帧或每个轮询 tick 里读写数据库。
- ❌ 不要用 gid 当任务身份。
- ❌ 不要在 `Store::open` 失败时 panic。
- ❌ 不要把「删用户的文件」写成静默失败。`let _ = std::fs::remove_file(..)` 的结果是文件还在、
  界面上什么都没解释，而这正是按钮承诺过不会发生的事。要么重试（Windows 上 antivirus /
  句柄释放都要时间，见 `discard_download`），要么把底层错误报给用户。
- ❌ 不要把 `resources/aria2c.exe` 加进 git（已被 gitignore）。
- ❌ 不要删 `state.json.imported` —— 那是用户从旧版迁移过来的备份。
