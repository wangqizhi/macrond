# macOS 易用定时任务工具（Rust）PRD

## 1. 文档信息
- 版本: v1.0
- 状态: Draft（可实现）
- 目标平台: macOS
- 目标用户: 希望替代 `crontab`、并通过 CLI/TUI 管理任务的开发者或运维人员

## 2. 背景与目标
传统 `crontab` 在以下方面不够友好:
- 配置依赖 cron 表达式，不易上手
- 缺乏统一的任务状态查询与执行记录查看
- 不便于以结构化配置文件管理多个任务

本项目目标:
- 提供兼容 cron 的调度能力
- 提供更易用的“简化调度配置”
- 提供后台守护进程 + CLI + TUI 的统一体验
- 提供清晰的文本日志与基本可观测能力

## 3. 范围
## 3.1 V1 范围（In Scope）
- 支持多任务（多个 job）
- 支持两种调度定义:
  - cron 表达式
  - 简化配置（时间 + 重复规则: 每日/每周/每月）
- 支持执行脚本或可执行程序
- 守护进程模式（`daemon` 子命令）
- CLI 命令:
  - `start`
  - `stop`
  - `status`
  - `list`
  - `logs`
  - `run`（手动触发一次）
- TUI（V1）:
  - 新增 job
  - 编辑 job
  - 删除 job
  - 查看最近执行记录
- 配置文件:
  - 目录: `./jobs/*.json`
  - 支持热加载（新增/修改/删除）
- 日志:
  - 目录: `./logs/`
  - 文本格式
  - 按天切分
  - 保留 30 天

## 3.2 说明
- 本次按 V1 范围完整实现，不单独定义 Out of Scope。

## 4. 关键约束与默认策略
- 时区: 使用 macOS 系统本地时区
- 运行身份: 使用当前启动程序的用户身份
- 同一 job 并发策略: 允许并行执行（不做互斥）
- 超时控制: 每个 job 可配置超时，默认 1 小时
- 失败处理: 不重试，仅记录日志

## 5. 用户故事
1. 作为用户，我可以通过 `jobs/*.json` 增加定时任务，服务自动热加载生效。
2. 作为用户，我既可以写 cron，也可以在 TUI 里通过“时间 + 重复规则”创建任务。
3. 作为用户，我能用 `list/status` 查看任务数量、运行状态和任务详情。
4. 作为用户，我能用 `run <job_id>` 立即手动触发某任务。
5. 作为用户，我能在 `logs/` 查看成功/失败和执行耗时等记录。

## 6. 功能需求
## 6.1 Job 配置模型（JSON）
每个 job 一个 JSON 文件，文件名建议 `job_id.json`。

示例（cron）:
```json
{
  "id": "backup_db",
  "name": "Backup Database",
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
      "PATH": "/usr/local/bin:/usr/bin:/bin",
      "API_KEY": "xxx"
    }
  },
  "timeout_seconds": 3600
}
```

示例（简化规则）:
```json
{
  "id": "daily_report",
  "name": "Daily Report",
  "enabled": true,
  "schedule": {
    "type": "simple",
    "time": "09:30",
    "repeat": "daily"
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

字段约束:
- `id`: 必填，全局唯一，建议 `[a-zA-Z0-9_-]+`
- `name`: 必填，展示名称
- `enabled`: 可选，默认 `true`
- `schedule.type`: `cron | simple`
- `schedule.expression`: 当 `type=cron` 必填
- `schedule.time`: 当 `type=simple` 必填，格式 `HH:MM`（24h）
- `schedule.repeat`: 当 `type=simple` 必填，取值:
  - `daily`
  - `weekly`（需扩展 `weekday`）
  - `monthly`（需扩展 `day`）
- `command.program`: 必填，程序或脚本解释器路径
- `command.args`: 可选，参数数组
- `command.working_dir`: 可选，默认进程当前目录
- `command.env`: 可选，键值对
- `timeout_seconds`: 可选，默认 `3600`

补充字段（simple 扩展）:
- 当 `repeat=weekly` 时增加 `weekday`（`1-7`，1=周一）
- 当 `repeat=monthly` 时增加 `day`（`1-31`）

## 6.2 调度与执行
- 调度器每秒级检查到期任务（精度满足分钟级任务）
- cron 与 simple 统一转换为“下一次触发时间”进行调度
- 任务触发后创建独立执行实例，不阻塞同 job 下一次触发
- 每次执行记录:
  - `run_id`
  - `job_id`
  - 触发时间、开始时间、结束时间
  - 退出码
  - 状态（`success` / `failed` / `timeout`）
  - stderr 摘要（可选）

## 6.3 超时与终止
- 达到 `timeout_seconds` 时标记 `timeout`
- 尝试终止子进程（先温和终止，再强制终止）
- 将超时结果写入日志

## 6.4 热加载
- 监听 `./jobs/` 目录文件变化（新增/修改/删除）
- 变化后重新解析对应 job:
  - 新增: 注册新任务
  - 修改: 替换任务定义（后续触发按新配置）
  - 删除: 注销任务（不再调度）
- 非法配置文件:
  - 不使 daemon 崩溃
  - 记录错误日志
  - CLI/TUI 可见该错误（至少在 `status/logs` 可查）

## 6.5 日志
- 路径:
  - 守护进程日志: `./logs/daemon-YYYY-MM-DD.log`
  - 任务执行日志: `./logs/job-YYYY-MM-DD.log`
- 格式: 文本单行（可读优先）
- 最少字段:
  - 时间戳（本地时区）
  - 级别（INFO/WARN/ERROR）
  - job_id/run_id（任务日志）
  - 事件（start/success/failed/timeout/reload）
  - 简述
- 保留策略:
  - 每日切分
  - 自动删除超过 30 天日志

## 6.6 CLI 需求
命令语义（建议）:
- `start`: 启动后台守护进程（若已运行则提示）
- `stop`: 停止守护进程
- `status`: 显示 daemon 状态、已加载任务数、最近 reload 状态
- `list`: 列出任务（id、enabled、schedule、next_run、last_run_result）
- `logs [--job <id>] [--tail N]`: 查看日志（全局或指定 job）
- `run <job_id>`: 手动触发一次任务执行
- `daemon`: 前台运行守护进程（供 `start` 内部或调试用）

约束:
- 命令返回码需可脚本化（0 成功，非 0 失败）
- 错误输出统一到 stderr

## 6.7 TUI 需求（V1）
- 页面能力:
  - 任务列表
  - 新增任务（支持 cron/simple 两种方式）
  - 编辑任务
  - 删除任务
  - 查看最近执行记录
- 写入方式:
  - TUI 编辑后写回 `./jobs/<id>.json`
  - 由热加载机制自动生效
- 输入校验:
  - cron 表达式基本合法性
  - `time/repeat/weekday/day` 取值合法
  - `id` 唯一性校验

## 7. 非功能需求
- 稳定性:
  - 非法 job 配置不应导致主进程退出
- 性能:
  - 支持至少 500 个 job 常驻
  - 空闲 CPU 占用保持较低（目标 < 5%，参考值）
- 可维护性:
  - 核心模块分层清晰（配置、调度、执行、日志、CLI、TUI）
- 可测试性:
  - 核心调度逻辑、配置解析、超时处理需有单元测试

## 8. 架构建议（实现导向）
- `scheduler`: 计算下一次触发、管理触发循环
- `executor`: 启动子进程、采集结果、处理超时
- `config`: 读取 `jobs/*.json`、校验、热加载
- `state`: 保存运行态（任务快照、最近执行结果）
- `cli`: 命令入口与 daemon 控制
- `tui`: 交互界面（读写 job 文件 + 查看执行记录）
- `logging`: 文本日志输出与清理策略

## 9. 验收标准（V1）
1. 在 `jobs/` 新增合法 job 文件后，60 秒内可在 `list` 中看到并按计划执行。
2. 修改 job 调度后，新配置生效，后续触发按新时间执行。
3. 删除 job 文件后，该任务不再触发。
4. `run <job_id>` 能立即触发并产生日志记录。
5. 超时任务会被标记为 `timeout` 并写入日志。
6. `status/list/logs` 能反映 daemon 与任务执行实际状态。
7. `logs/` 按天切分，超过 30 天日志会被清理。
8. TUI 可完成新增/编辑/删除任务，并查看最近执行记录。

## 10. 里程碑建议
1. M1: 配置模型 + 调度器 + 执行器（无 TUI）
2. M2: 热加载 + CLI 全命令 + 日志轮转/清理
3. M3: TUI（新增/编辑/删除/记录查看）
4. M4: 稳定性测试与发布准备
