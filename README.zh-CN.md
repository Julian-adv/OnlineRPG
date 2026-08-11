# 开放式 MMORPG

[English](README.md) | [日本語](README.ja.md)

一个平等对待 AI 智能体和人类玩家的 MMORPG。

智能体与人类连接到同一个世界，遵循相同规则行动，并且彼此互动时不作区分。智能体不会获得特权 API，而是通过与人类玩家相同的接口参与游戏。

**立即游玩：[openmmo.to.nexus](https://openmmo.to.nexus)** — 使用 Google 登录，即刻进入游戏。

> 本项目由个人独立开发，并采用氛围编程（vibe coding）。素材由 AI 生成、程序化创建和从互联网获取的内容混合组成。[欢迎提交 PR！](#贡献)

## 功能

- **智能体与人类平等**：智能体和人类玩家使用完全相同的 WebSocket 协议，不存在特权 API 或独立端点。服务器无法区分二者，因此人类能做的任何行为，智能体也能做（反之亦然）。
- **实时多人游戏**：通过 WebSocket 实时同步玩家
- **3D 环境**：基于 Three.js 的斜俯视 3D 游戏世界
- **点光源火把**：火把投射具有衰减效果和阴影的实时点光源

![带有火把照明的夜间场景](doc/images/gameplay-night.png)

- **建筑与住宅**：模块化木框架建筑，支持逐房间遮挡和 L 形屋顶连接
  - 支持 2 层、3 层和 4 层的多层建筑
  - 可交互开关的门窗
  - 可自定义墙壁、屋顶和地板的纹理/材质
  - 可摆放家具（例如床），并可在游戏世界中互动（睡觉/使用）

![带床的玩家自建木框架房屋](doc/images/gameplay-housing.png)
- **昼夜循环**：模拟日照时间变化，包括太阳、天空和环境光照的动态变化
  - 昼夜长度随行星轨道位置变化（季节性的长昼与长夜）
- **双月系统**：两个卫星拥有各自独立的轨道和月相模拟

![展示太阳、行星和双月的天体轨道面板](doc/images/gameplay-orbits.png)
- **程序化世界**：地形、河流、海岸线和生物群系均完全由程序生成
  - 广阔的 32 公里 × 32 公里世界
  - 程序化生成河流，包含冲刷形成的河道与辫状分流
  - 连接各地聚落的程序化道路网络
  - 道路跨越河流时自动放置桥梁
  - 随阵风摇曳的动态草地和植被
  - 带动画效果的海浪（Gerstner）与流动河面波纹
  - 河流入海处形成具有分支分流和河口淡咸水交融效果的三角洲

![程序化生成的世界地图](doc/images/gameplay-worldmap.png)

![自动放置在程序化河流上的木桥](doc/images/gameplay-bridge.png)

![带有辫状分流和沙洲的入海河流三角洲](doc/images/gameplay-delta.png)

- **内置地图编辑器**：用于塑造游戏世界的内置工具
  - 地形笔刷（道路、平整、高度绘制），支持实时编辑
  - 放置建筑、道具和植被等对象，并提供预览
  - 通过矩形区域绘制城镇（禁止生成）和每个区域的怪物生成区

![启用高度笔刷的游戏内地图编辑器](doc/images/gameplay-map-editor.png)

- **基于属性的战斗系统**：采用 NetHack/D&D 风格，由服务器权威判定战斗结果
  - 六项经典属性（STR、DEX、CON、INT、WIS、CHA），取值范围为 3–18
  - 创建角色时采用 4d6 去掉最低值的掷骰方式，并包含职业修正和 72 点重新分配
  - 所有伤害、命中和结果判定均由服务器处理

![包含属性、纸娃娃式装备和物品栏的角色面板](doc/images/gameplay-character-sheet.png)

- **物品栏与装备**：受负重限制的物品栏，以及完整的纸娃娃式装备系统
  - 十一个装备槽：头部、主手、副手、胸部、耳部、颈部、腰带、裤子、靴子和两个戒指槽
  - 拾取物品时会检查单件重量，因此沉重配装需要真正作出取舍
- **掉落物品**：物品可以丢到游戏世界中，并由任何人拾取
  - 地面物品会保留在掉落位置，并显示对应模型
  - 感知楼层：掉在房屋二楼的物品只能从该楼层拾取（支持多层住宅）
  - 拾取操作会检查距离并在服务器端以原子方式执行，以防止物品复制
- **AI 生成的背景音乐**：约 50 首背景音乐由 [Suno](https://suno.com) 和 [Google Flow Music](https://labs.google/fx/tools/music-fx) 生成
  - 采用受 Ultima 启发的中世纪奇幻配器，包括鲁特琴、竖笛、竖琴、弦乐、打击乐和铜管乐
  - 环境音乐与战斗音乐分别组成曲库；进入战斗时交叉淡入战斗音乐，短暂延续后再淡回环境音乐
- **聊天系统**：实时聊天功能
- **玩家移动**：使用鼠标/键盘控制角色

## 文档

- [开发日志](doc/devlog/README.md)

**世界与地形**
- [世界构建](doc/WORLD_BUILDING.md)
- [地图与地形设计](doc/MAP_DESIGN.md)
- [地形生成](doc/TERRAIN_GENERATION.md)
- [河流系统](doc/RIVER_SYSTEM.md)
- [水体系统](doc/WATER_SYSTEM.md)
- [植被系统](doc/VEGETATION_SYSTEM.md)
- [区域系统](doc/ZONE_SYSTEM.md)
- [Splatmap v2](doc/SPLATMAP_V2.md)

**游戏系统**
- [住宅系统](doc/HOUSING_SYSTEM.md)
- [战斗](doc/COMBAT.md)
- [NPC 与怪物 AI](doc/NPC_MONSTER_AI.md)
- [动画](doc/ANIMATION.md)

**引擎与性能**
- [运行时性能](doc/RUNTIME_PERFORMANCE.md)
- [加载优化](doc/LOADING_OPTIMIZATION.md)

**素材与智能体**
- [素材](doc/ASSETS.md)
- [智能体客户端](doc/AGENT_CLIENT.md)

## 架构

- **客户端**：Svelte 组件化 UI + 通过 Threlte 集成 Three.js
- **服务器**：使用广播通道管理游戏状态的 Rust 异步服务器
- **通信**：通过 WebSocket 进行实时双向通信

## 技术栈

**客户端：**
- Svelte + TypeScript
- Three.js (Threlte) + WebGPU
- Vite

**智能体客户端：**
- Rust
- Tokio + tokio-tungstenite（WebSocket）
- Axum（本地旁观面板）

**服务器：**
- Rust
- Tokio（异步运行时）
- tokio-tungstenite（WebSocket）
- Axum（地形 REST API）
- serde（JSON 序列化）

## 开发环境设置

### 1. 前置要求

- **Rust 与 Cargo**：[安装 Rust](https://rustup.rs/)
- **Node.js 与 npm**：[安装 Node.js](https://nodejs.org/)
- **（推荐）cargo-watch**：用于在服务器代码变化时自动重启。
  ```bash
  cargo install cargo-watch
  ```

### 2. 端口分配

| 端口  | 服务                          |
|-------|----------------------------------|
| 10004 | 客户端（Vite 开发服务器）                |
| 10005 | GLB 编辑器                       |
| 10006 | 服务器 WebSocket（绑定到 127.0.0.1；开发环境通过 Vite 代理访问，生产环境通过 nginx 访问） |
| 10007 | 服务器地形/住宅/NPC API（绑定到 127.0.0.1；写入操作需要身份验证） |

> 两个服务器端口默认只监听回环地址（`--bind` / `--api-bind`）。只有在需要直接为其他机器上的客户端提供服务时，才应传入 `--bind 0.0.0.0`；该方式没有 TLS，也没有前置代理。

> **代理规则：**Vite 开发服务器会自动将 `/ws` 代理到 `ws://localhost:10006`，并将 `/api`（所有 REST 端点）代理到 `http://localhost:10007`（参见 `client/vite.config.ts`）。

### 3. 运行服务器

本项目组织为 **Cargo 工作区**。共享 Rust crate（`shared/`）供服务器、通过 WASM 运行的客户端以及智能体客户端共同使用。游戏源数据位于 `data-src/`，并会在 Cargo 构建期间转换为 `data/` 中生成的 JSON。若只想在服务器 crate（`server/`）、共享 crate 或源数据变化时重新构建服务器，请在**根目录**运行以下监视命令。

```bash
cargo watch -w server -w shared -w data-src -x "run -p onlinerpg-server"
```

服务器默认监听 10006 端口。地形/住宅/NPC REST API 会自动在 10007 端口（游戏端口 + 1）启动，并绑定到 127.0.0.1（可用 `--api-bind` 覆盖）。读取操作公开；写入操作（PUT/POST/DELETE）需要 bearer token：可以是 NPC token（用于本地脚本），也可以是电子邮件已列入 `ADMIN_EMAILS` / `--admin-emails`（逗号分隔）的 Google ID token；地图编辑器会自动发送已登录用户的 token。

WebSocket 和地形 API 的代理由 Vite 开发服务器处理（参见 `client/vite.config.ts`），因此不需要单独运行 socat 或 SSL 代理。

**Google 登录**：浏览器登录使用 Google OAuth。请将同一个 Web 客户端 ID 传给服务器（`GOOGLE_CLIENT_ID` 环境变量 / `--google-client-id`）和客户端（`VITE_GOOGLE_CLIENT_ID`，参见第 4 步）。若不提供，服务器仍可运行，但会拒绝浏览器登录。NPC/机器人 token 会在首次运行时自动生成到 `data/npc_token`；可通过 `NPC_AUTH_TOKEN` / `--npc-token` 覆盖（至少 16 个字符）。

在他人机器上运行的智能体客户端会使用自己的 Google 账号通过设备流程登录，这需要第二个类型为“TV and Limited Input”的 OAuth 客户端（无头客户端不能使用 Web 客户端）。请通过 `GOOGLE_CLI_CLIENT_ID` / `--google-cli-client-id` 传入该客户端 ID；服务器会接受来自任一客户端的 token。参见 [doc/REMOTE_AGENT_CLIENT.md](doc/REMOTE_AGENT_CLIENT.md)。


### 4. 运行客户端

```bash
cd client
cp .env.example .env.local   # then set VITE_GOOGLE_CLIENT_ID (required for login)
npm install
npm run dev -- --port 10004
```

### 5. 运行智能体客户端

编辑 `agent-client/data/config.toml`，设置正确的端口号，然后运行：

```bash
cd agent-client
cargo watch -i "data/prompts/memory/" -x run
```

### 6. 共享代码变化时自动重新构建 WASM（推荐）
若希望 `shared` 库中的 Rust 代码变化立即反映到浏览器中，请在单独的终端运行以下命令：

```bash
# Run from the root directory
cargo watch -w shared -s "npm run build:wasm --prefix client"
```

### 7. 运行 GLB 编辑器

```bash
cd tools/glb-editor
npm install
npm run dev -- --port 10005
```

## 生产环境部署

生产环境使用 systemd 单元（`tools/systemd/`）运行两个二进制文件，并由 `/var/www/openmmo` 静态提供客户端构建产物。

| 单元 | 二进制文件 | Syslog 标识符 |
|------|--------|-------------------|
| `openmmo-server` | `onlinerpg-server` | `openmmo` |
| `openmmo-agent-client` | `agent-client` | `openmmo-agent` |

请在生产主机上运行 `tools/deploy-prod.sh` 进行部署。该脚本会拉取 master、构建两个二进制文件和客户端包、发布静态文件，然后重启两个单元。

服务器会优雅处理 systemd 的 `SIGTERM`：它会向已连接的玩家显示重启通知，关闭监听器和周期性任务，等待正在进行的批量保存完成，持久化所有已连接角色、物品栏和世界时钟，然后退出。`systemctl restart` 会等待该排空过程完成，再启动新的二进制文件。

管理员角色可以使用 `/notice <message>` 手动显示相同的横幅（这是游戏内实时横幅，并非由 `data/announcements/` 提供的登录界面公告）。不带消息的 `/notice` 会清除横幅；横幅生效期间进入游戏的玩家会在加入时收到它。

通过 SSH 运行时，请将进程与会话分离，避免连接断开导致构建在中途终止：

```bash
ssh prod 'setsid nohup bash ~/work/OnlineRPG/tools/deploy-prod.sh > ~/deploy-latest.log 2>&1 < /dev/null &'
ssh prod 'tail -f ~/deploy-latest.log'   # follow; ends at "==> deployed <commit>"
```

前台运行时若连接丢失，会浪费整个构建过程，但绝不会造成部署一半的状态：脚本会先完成全部构建，仅在最后才更新在线文件（使用 `rsync` 同步到 Web 根目录，然后重启服务），因此在此之前中断会保留相互匹配的旧客户端包和旧服务器进程。

### 日志

两个单元都将日志写入 journald（`StandardOutput=journal`），不会生成单独的日志文件。

```bash
journalctl -u openmmo-server -f              # follow the game server
journalctl -u openmmo-agent-client -f        # follow the NPC agent client
journalctl -u openmmo-server -n 200 --no-pager   # recent history
journalctl -u openmmo-server --since "1 hour ago" -p err   # errors only
```

两个二进制文件都使用 `RUST_LOG` 配置日志订阅器；当该值未设置或无法解析时，默认使用 `info`。各项操作的详细信息（怪物生成/消失、战斗掷骰/生命值/经验值计算）使用 `debug` 级别，以避免高负载时产生过多日志；如需查看，请提高日志级别：

```bash
sudo systemctl edit openmmo-server   # or write /etc/openmmo/server.env
# [Service]
# Environment=RUST_LOG=debug
sudo systemctl restart openmmo-server
```

每个单元都会读取 `EnvironmentFile=-/etc/openmmo/{server,agent-client}.env`，因此可以在该文件中设置 `RUST_LOG`。请注意，systemd 单元不会继承 shell 环境；在终端中导出 `RUST_LOG` 只会影响由该终端直接启动的二进制文件。

移动警告（目标移动被拒绝、路径点队列已满、移动受阻）会特意保持在 `warn` 级别：它们在服务器和客户端的步进检查不一致时触发，因此代表错误信号，而不是正常游戏过程。

## 贡献

欢迎贡献！关于如何选择任务、CI 运行的检查项以及 PR 约定，请参阅 [CONTRIBUTING.md](CONTRIBUTING.md)。当你首次提交 Pull Request 时，机器人会要求你在 PR 中留言签署[贡献者许可协议](CLA.md)。签署 CLA 后你的贡献才能被合并。

如需贡献二进制资产（3D 模型、音乐、音效），请参阅 [doc/ASSETS.md](doc/ASSETS.md) — 它们通过 Hugging Face 数据集 PR 提交，而不是 git。
