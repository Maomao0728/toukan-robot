# 开发状态

## 已验证

- 前端构建：通过。
- Rust/Tauri 检查：通过。
- Rust 测试：4 passed。

## 已建立模块

- paths：统一目录管理。
- db：SQLite schema 和 schema_version。
- recommendation：输入质量判断、本地兜底推荐占位。
- rag_index：profile/scope/experience/articles/guidelines chunk 生成。
- ai_clients：API Key 清洗、掩码、错误提示基础逻辑。
- web_search：联网查询词生成。
- backup_restore：数据库备份占位。
- diagnostics：诊断摘要。
- settings：隐私设置和 UI 提示。
- legacy_import：旧版迁移报告占位。

## 下一阶段

1. 正式迁移旧版数据库读取和 Excel 数据导入。
2. 实现期刊雷达真实筛选表格。
3. 实现收藏库和投稿记录的增删改查。
4. 接入 OpenRouter/xFastAPI 真实请求。
5. 实现 FTS5/BM25 和 embedding 缓存。
