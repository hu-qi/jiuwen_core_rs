# Harness 状态与生命周期

Harness 的状态边界由 Context、Session、Plugin 和 Effect 共同决定。

## 生命周期

```text
create Context → mount Profile → resolve services → invoke → cancel/complete → drop Effects
```

Context 只在当前运行时范围内提供服务解析。Session 和 Checkpoint 插件负责跨进程保存可恢复数据；Effect 负责撤销本次挂载产生的注册。

## 必须保持的状态区分

- 正常完成
- 用户取消
- 超时
- 结构化失败
- 可恢复中断
- 关闭清理

插件关闭时必须终止自己创建的后台任务、子进程和 Transport，不能只删除 Registry 中的服务。
