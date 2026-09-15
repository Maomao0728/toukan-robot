# 投刊机器人

新版桌面端投刊工具。目标是替代旧版 Streamlit 投刊助手，但旧项目保持不动，作为功能蓝图和回退版本。

## 当前状态

已完成第一批基础骨架：

- Tauri + Rust + Web 前端项目结构。
- `D:\投刊机器人\data` 集中数据目录。
- 旧版功能审计文档。
- Rust 核心模块边界。
- SQLite schema/migration 初版。
- AI 输入质量判断，保留“只填题目可推荐但提示信息不足”。
- RAG chunk 生成规则初版。
- 前端 6 个主页面 + 设置诊断页骨架。
- 醒目提示组件：普通、重要、风险。
- 基础测试。

## 常用命令

```powershell
$env:Path='D:\Node.js;C:\Users\Administrator\.cargo\bin;C:\Program Files\Git\cmd;' + $env:Path
cd D:\投刊机器人\frontend
npm run build
cd D:\投刊机器人
cargo check
cargo test
```

## 重要约束

- 旧版源码和桌面打包版不删除、不覆盖。
- 新版所有数据都放在 `D:\投刊机器人\data`。
- 后续迁移旧功能时必须逐项对照 `docs\legacy_audit.md`。
