# Moon·月 · 桌宠引擎

角色正式名：暗月。

暗月是一款 Windows 桌宠，支持本地互动、轻养成、小小琴盘和可选的 API 交流，也能显示 Kimi Code 的工作状态。

交流资料可浏览提炼后的事实与往事，查看来源并修正或删除。记忆在交流设置中独立开启，后台提炼使用聊天服务商并产生额外用量，首次使用本机检索时需下载中文模型。详见[记忆管理](docs/记忆底层重整-实现与验证-20261103.md)。

交流设置支持[多组API连接](docs/多服务商API-v2.md)：常用服务商预设或手填地址，查询模型并一键保存、Key首尾对照、可排序的自动备选；原有两家凭据兼容保留。

## 运行

在 Windows x64 上开发，需要 Node.js、Rust MSVC 工具链、Visual Studio C++ Build Tools 和 WebView2。

```bash
npm install
npm run dev        # 开发模式（Tauri）
```

如需生成可分享的 ZIP，请按[构建 Windows 便携包](docs/便携试玩打包.md)运行打包脚本。

## 接入 Kimi Code（hooks 数据源）

将示例中的 `C:/path/to/pet-engine` 换成仓库的绝对路径，再将片段加入 `~/.kimi-code/config.toml`。

```toml
[[hooks]]
event = "SessionStart"
command = "py \"C:/path/to/pet-engine/adapters/hook.py\" session_start"

[[hooks]]
event = "SessionEnd"
command = "py \"C:/path/to/pet-engine/adapters/hook.py\" session_end"

[[hooks]]
event = "TurnStarted"
command = "py \"C:/path/to/pet-engine/adapters/hook.py\" turn_started"

[[hooks]]
event = "Stop"
command = "py \"C:/path/to/pet-engine/adapters/hook.py\" stop"

[[hooks]]
event = "Interrupt"
command = "py \"C:/path/to/pet-engine/adapters/hook.py\" interrupt"

[[hooks]]
event = "TaskStarted"
command = "py \"C:/path/to/pet-engine/adapters/hook.py\" task_started"

[[hooks]]
event = "SubagentStart"
command = "py \"C:/path/to/pet-engine/adapters/hook.py\" subagent_start"

[[hooks]]
event = "SubagentStop"
command = "py \"C:/path/to/pet-engine/adapters/hook.py\" subagent_stop"

[[hooks]]
event = "SessionHeartbeat"
command = "py \"C:/path/to/pet-engine/adapters/hook.py\" heartbeat"
```

wire.jsonl 由引擎自动定位（`$KIMI_CODE_HOME/sessions/*/​*/agents/main/wire.jsonl` 取最近修改），无需配置。


## 小小琴盘

右键 → 小小琴盘 → 启用琴盘声音，可离线自由弹奏或练习光流、地鼠；“听她弹”先示范一遍。高音 QWERTYU、中音 ASDFGHJ、低音 ZXCVBNM，也可点琴键。练习失焦会暂停，继续有两拍视觉预备。琴盘通过记录独立保存在程序旁的`琴盘记录.json`；不使用养成金币。

琴盘支持半音、同拍和弦与辅助声部。项目随附 7 首示例曲目，陪演动作使用长笛动画，声音来自钢琴采样。详见[音源署名](app/audio/piano/ATTRIBUTION.txt)。

## 跟她说话

双击桌宠打开固定高度的输入条，之后可拖动前端圆球自由摆放。点击圆球收起或恢复，Esc或再次双击隐藏；Enter发送、Shift+Enter换行，长内容在框内滚动，不保存草稿。右键或托盘的「交流设置」可添加多组API连接、查询模型和设置备用连接；历史及停止、重试、清空也在交流资料中。

回复由网络流式接收，完整检查后显示在对白气泡，可展开或滚动阅读；生成时可以明确停止，隐藏或最小化输入不停止回复。投喂、开始打工、领取奖励可在限频内补充一句API回应。聊天和有限桌宠状态会发给所选服务商并按其规则计费；自动备选开启时，相同上下文可能发送给备用服务商。未配置时离线功能仍可使用。

长期记忆在交流设置中独立开启，默认关闭。开启后后台会把一批真实交流和生活事件发送给所选聊天服务商整理，产生额外用量。向量检索在本机，首次需要下载中文模型。“她的记忆”可浏览来源、修正内容，也可选择仅删除提炼项、停止使用来源或一并删除关联原话。详情见[记忆管理](docs/记忆底层重整-实现与验证-20261103.md)及[暗月人设与交流防护](docs/暗月人设与交流防护-v1.md)。

输入手势和交流调度见[输入气泡与对白](docs/输入气泡-v1.md)。连接配置与凭据说明见[配置多组 API 连接](docs/多服务商API-v2.md)。

## 贡献与致谢

- **Kimi** 负责原画设计与初始架构。
- **Codex** 参与后续功能实现、维护和验证。
