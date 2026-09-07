# 运维指南

生产运行需要同时管理模型服务、持久化后端、工具权限、观测和恢复流程。

## 上线清单

- 使用受控密钥注入 API Key、数据库密码和 OAuth 凭据。
- 使用真实 Provider 完成一次端到端调用。
- 验证 Session、Checkpoint 的写入、恢复和损坏处理。
- 验证取消、超时、重连、优雅关闭和后台任务清理。
- 配置 Trace、日志、指标和敏感字段脱敏。
- 明确数据保留、备份、恢复和删除策略。
- 检查生产 Profile 没有 Mock Provider。

## 部署顺序

1. 部署并验证基础存储。
2. 部署模型和协议依赖。
3. 运行结构化健康检查。
4. 启动 Agent Runtime。
5. 执行最小业务场景。
6. 观察错误率、延迟、资源和 Trace。

## 相关参考

- [外部集成](/integrations/)
- [配置与环境变量](/reference/configuration)
- [可观测性](/integrations/observability)
- [能力与兼容性](/reference/compatibility)
