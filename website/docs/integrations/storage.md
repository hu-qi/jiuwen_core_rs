# 存储与 Checkpoint

openJiuwen 的外部存储可用于 Session、Checkpoint、对象、向量、图数据和消息持久化。

## 选择后端

| 数据 | 常见后端 |
| --- | --- |
| Session / Checkpoint | Redis、SQLite、PostgreSQL、GaussDB |
| 向量检索 | Elasticsearch、Milvus、Chroma、PGVector |
| 对象存储 | OBS 或兼容对象存储 |
| 消息 | Pulsar |

## 接入要求

- 使用独立 namespace 隔离环境和租户。
- 明确序列化格式和版本。
- 验证并发写入、事务和过期策略。
- 测试断线、重连和部分写入。
- 为备份、恢复和数据删除提供操作步骤。
- 后端不可用时返回明确错误，不静默降级到进程内存。

恢复测试必须从真实持久化状态重新启动运行时，不能只在同一进程中读取缓存。
