# Network Monitor

一个基于 Rust 的轻量级网络监控工具，支持 TCP、ICMP (Ping) 和 DNS 协议监控，提供 Web 界面和 Webhook 告警功能。

## 逻辑结构 (Logical Structure)

本项目采用异步并发模型（Tokio），主要由以下几个核心模块组成：

### 1. 核心监控模块 (`monitor.rs`)
- **Probe Loop**: 主循环定期遍历所有监控目标 (Target)。
- **并发探测**: 针对每个目标启动异步任务进行探测 (TCP connect, ICMP ping, DNS query)。
- **状态管理**: 使用 `DashMap` (线程安全的 HashMap) 存储所有目标的实时状态 (`MonitorStatus`)。
- **状态确认机制**: 
    - 首次启动时立即确认状态。
    - 状态变更（UP <-> DOWN）需要经过多次探测确认（防抖动）。
- **配置热重载**: 监听配置文件变化，通过 Hash 比对智能更新监控列表，避免不必要的重启。

### 2. Web 服务模块 (`web.rs` & Frontend)
- **Axum Server**: 提供 HTTP API 和静态文件服务。
- **API**:
    - `GET /api/status`: 获取当前所有监控目标的状态。
    - `GET /api/config`: 获取当前配置。
    - `POST /api/config`: 更新配置（支持前端直接修改）。
- **Frontend**: 单页应用 (SPA)，实时轮询 API 展示状态，支持深色模式 (Dark Mode)，提供配置管理界面。

### 3. 数据持久化与缓存 (`main.rs` & `config.rs`)
- **Config Persistence**: 配置文件 (`config.json`) 是单一数据源 (Source of Truth)。修改配置会自动保存到磁盘。
- **Cache System**: 
    - **Graceful Shutdown**: 程序接收到终止信号 (SIGINT/SIGTERM) 时，会将当前的监控状态（如历史延迟数据、当前状态）序列化保存到 `cache.json`。
    - **Restore**: 下次启动时优先加载缓存，恢复之前的监控上下文，避免数据断层。

### 4. 告警模块 (`alert.rs`)
- **Webhook**: 当目标状态发生确认变更时，异步发送 HTTP POST 请求到配置的 URL。
- **Template**: 支持自定义告警消息模版，支持 emoji 状态标识 (🟢/🔴)。
- **Retry**: 内置简单的错误重试和详细的日志记录（Debug 模式下）。

## 编译指南 (Build)

本项目提供了 `Makefile` 以简化多架构编译。

### 前置要求
- Rust (Cargo)
- **交叉编译工具链**:
    - x86_64 musl: 需要 `x86_64-linux-gcc`
    - AArch64 musl: 需要 `aarch64-linux-gcc`

### 常用命令

```bash
# 编译 x86_64 Musl (静态链接，单文件，使用 x86_64-linux-gcc)
make x86_64

# 编译 AArch64 Musl (静态链接，单文件，使用 aarch64-linux-gcc)
make aarch64
```

## 配置文件 (`config.json`)

```json
{
  "targets": [
    {
      "id": "uuid",
      "name": "Localhost",
      "host": "127.0.0.1",
      "protocol": "Tcp",
      "port": 80,
      "interval": 10,
      "timeout": 2
    }
  ],
  "alert_config": {
    "enabled": true,
    "webhook_url": "https://your-webhook.com",
    "message_template": "..."
  },
  "retention_days": 7
}
```

## 运行

```bash
./target/release/netwatch
```
日志级别可通过 `RUST_LOG` 环境变量控制，默认为 `info`。
