# Harness Memory 与 Retrieval

Memory、Retrieval、Embedding 和 Rerank 在 Harness 中都是可替换插件能力。

## 数据链路

```text
Document → parser/chunker → embedding → index
Query → retrieval → rerank → context
```

Provider 和 Consumer 通过稳定 Seam 交互。外部向量库或图存储不可用时，应返回明确错误，不能静默切换到进程内存并继续声明生产成功。

需要持久化的记忆必须定义租户、来源、生命周期、删除和备份策略；检索结果应记录必要的来源标识，便于审计和重现。
