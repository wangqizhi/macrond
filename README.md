# Macrond

一个 macOS 上更易用的定时任务工具（Rust），支持：
- cron 表达式
- 简化调度（daily / weekly / monthly / everyminute / once）
- 后台 daemon
- CLI 管理
- TUI 任务管理

## 1. 环境要求
- macOS
- Rust toolchain（建议 stable）

## 2. 编译
```bash
cargo build
```

开发调试可用：
```bash
cargo run -- --help
```

打包 release 可执行文件：
```bash
cargo build --release
```

产物：
```bash
./target/release/macrond
```

## 3. 目录约定
程序以 `--base-dir` 为根目录（默认当前目录 `.`）：
- `jobs/`：任务配置（`*.json`）
- `logs/`：日志（`job-YYYY-MM-DD.log` / `daemon-YYYY-MM-DD.log`）
- `run/`：运行状态文件（pid/state/request）

## 4. 运行
### 4.1 启动 daemon
```bash
macrond start
```

### 4.2 查看状态
```bash
macrond status
```

### 4.3 停止 daemon
```bash
macrond stop
```

## 5. CLI 使用
```bash
# 列出任务
macrond list

# 查看日志（最新日志文件尾部）
macrond logs --tail 100

# 只看某个 job 的日志行
macrond logs --job <job_id> --tail 100

# 立即执行一次 job
macrond run <job_id>

# 前台运行 daemon（调试用）
macrond daemon

# 启动 TUI
macrond tui
```

如果项目不在当前目录，可传：
```bash
macrond --base-dir /path/to/project list
```

## 6. TUI 使用
进入：
```bash
macrond tui
```

首页快捷键：
- `j/k`：上下移动
- `a`：新增任务
- `e` 或 `Enter`：编辑任务
- `d`：删除任务
- `s`：切换任务启停（toggle job）
- `t`：立即测试执行当前任务并返回结果
- `S`：启动 daemon
- `X`：停止 daemon
- `r`：刷新
- `q`：退出

编辑页快捷键：
- `j/k`：字段移动
- `Enter`：编辑字段 / 切换布尔 / 弹出 repeat 选择
- `s`：保存
- `q` 或 `Esc`：返回列表（有未保存改动会二次确认）

说明：
- 新建任务默认 `enabled=false`（关闭状态）。
- 首页显示 daemon 状态（running/stopped）。
- 右侧为 `History Runs`，读取 `logs/` 最新一天的 `job-*.log`。

## 7. Job 配置（JSON）
每个任务一个文件：`jobs/<job_id>.json`

### 7.1 cron 示例
```json
{
  "id": "backup_db",
  "name": "Backup DB",
  "enabled": true,
  "schedule": {
    "type": "cron",
    "expression": "0 2 * * *"
  },
  "command": {
    "program": "/bin/bash",
    "args": ["./scripts/backup.sh"],
    "working_dir": "/Users/me/project",
    "env": {
      "PATH": "/usr/local/bin:/usr/bin:/bin"
    }
  },
  "timeout_seconds": 3600
}
```

### 7.2 simple 每分钟示例
```json
{
  "id": "ping_every_minute",
  "name": "Ping Every Minute",
  "enabled": true,
  "schedule": {
    "type": "simple",
    "repeat": "everyminute",
    "time": null,
    "weekday": null,
    "day": null,
    "once_at": null
  },
  "command": {
    "program": "/bin/echo",
    "args": ["hello"],
    "working_dir": null,
    "env": {}
  },
  "timeout_seconds": 60
}
```

### 7.3 simple 一次性示例
```json
{
  "id": "run_once_task",
  "name": "Run Once Task",
  "enabled": true,
  "schedule": {
    "type": "simple",
    "repeat": "once",
    "time": null,
    "weekday": null,
    "day": null,
    "once_at": "2026-02-12 23:30"
  },
  "command": {
    "program": "/bin/echo",
    "args": ["run once"],
    "working_dir": null,
    "env": {}
  },
  "timeout_seconds": 60
}
```

### 7.4 simple 每日示例
```json
{
  "id": "daily_report",
  "name": "Daily Report",
  "enabled": true,
  "schedule": {
    "type": "simple",
    "repeat": "daily",
    "time": "09:30",
    "weekday": null,
    "day": null,
    "once_at": null
  },
  "command": {
    "program": "/usr/bin/python3",
    "args": ["report.py"],
    "working_dir": "/Users/me/report",
    "env": {}
  },
  "timeout_seconds": 1800
}
```

## 8. 热加载
daemon 运行时会监听 `jobs/*.json` 的新增/修改/删除并自动生效。

## 9. 常见问题
### 9.1 任务启用了但不执行
先确认 daemon 已运行：
```bash
macrond status
```

### 9.2 每分钟任务没看到记录
- 确认任务是 `[on]`
- 确认 daemon 在 running
- 查看 `logs/job-YYYY-MM-DD.log`

### 9.3 `working_dir` 不填会怎样
不填时，使用 daemon 进程当前工作目录（通常是启动时的 `--base-dir`）。
