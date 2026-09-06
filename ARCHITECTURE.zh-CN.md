# APEIR Kernel 架构

## 设计目标

APEIR Kernel 管理异构工作负载的完整生命周期，但不承载产品界面和具体业务逻辑。NKI 是唯一对外控制边界，Journal 是持久状态的权威来源，Provider 只负责执行，不能拥有策略或状态转换权。

```text
APEIR 发行版 / CLI / SDK
          |
          | NKI 请求与响应
          v
+------------------------------------------------+
| APEIR Kernel                                   |
| 请求校验 | 准入 | 安全 | 调度 | 资源租约       |
| 执行生命周期 | Journal | 恢复 | Receipt | Effect |
+------------------------------------------------+
          |
          | Provider 进程协议
          v
模型适配器 / 算法 / 工具 / 设备适配器
          |
          v
操作系统 / 本地服务 / 远程服务 / 物理设备
```

客户端不能直接打开 Journal、构造内核 Provider、修改租约或伪造 Commit。发行版可以展示已提交状态，但这些投影不能取代内核权威状态。

## 执行生命周期

1. NKI 校验帧格式、协议版本、请求身份、会话令牌、期限和载荷结构。
2. 准入层检查能力、安全约束、资源可行性和可用量。
3. SchedulerCore 选择符合条件的执行目标并记录决策轨迹。
4. 资源管理器签发带 fencing token 的租约。
5. 执行核心先持久化 Intent，再调用不可信 Provider。
6. Provider 在受限的子进程中处理单次请求。
7. 内核规范化结果并持久化 Receipt。
8. Commit 使结果成为权威状态，终态清理负责释放租约。

取消、超时、关闭和重启均复用同一生命周期。恢复只能重新进入满足交付语义要求的操作；如果 Receipt 已持久化但尚未 Commit，恢复过程不会再次调用 Provider。

## 权威与持久状态

| 领域 | 权威组件 | 持久表示 |
| --- | --- | --- |
| 工作负载和操作生命周期 | 执行核心 | 追加式 Journal 记录 |
| 准入和能力授权 | 安全与准入模块 | Decision 与 Receipt |
| 调度选择 | SchedulerCore | DecisionTrace |
| 资源所有权 | LeaseManager | Lease 与 fencing 记录 |
| Provider 输出 | 规范化后的执行核心 | OperationReceipt 与 Commit |
| Effect 生命周期 | Effect authority | Intent、Receipt、Commit、补偿记录 |
| 声明式资产 | 控制目录 | 带 generation 校验的版本记录 |

内存映射、队列、指标和健康状态均为派生数据，可以根据持久记录重建，不能成为第二套事实来源。

## 信任边界

可信计算基包括 NKI 校验、准入与安全决策、SchedulerCore、租约约束、Journal 解释、执行生命周期、Effect 校验和恢复。Provider、扩展、模型、工具、设备适配器、用户输入和界面均视为不可信组件。

Provider 进程隔离用于故障控制，不等同于操作系统安全沙箱。守护进程只监听回环地址；文件系统隔离、网络策略、安全凭据存储和系统账户隔离仍由部署环境负责。

## 契约与兼容性

`spec/contracts/v1` 定义内核执行语义，`spec/open-runtime/v1` 定义可移植扩展声明。对于可能改变行为的拼写错误，契约校验采用未知字段拒绝策略。

APEIR 品牌不改变 v1 契约身份。0.1 版本继续接受 `nous.*.v1`、`NOUS_*`、持久化 Receipt 与 ID，以及兼容命令。破坏 Wire 或持久状态兼容性的调整必须使用新协议版本，并提供协商、转换测试和迁移说明。

## 依赖约束

`scripts/architecture_check.py` 检查工作区依赖。控制权只能按以下方向流动：

```text
应用 -> 客户端 SDK -> NKI -> 内核权威模块 -> Provider 契约
```

具体领域框架、云 SDK、桌面代码、机器人组件和设备栈不能成为可信决策核心与状态核心的依赖。

详细文档见[内核边界](docs/architecture/KERNEL_BOUNDARY.md)、[执行路径](docs/architecture/EXECUTION_PATH.md)、[状态所有权](docs/architecture/STATE_OWNERSHIP.md)、[调度架构](docs/architecture/SCHEDULER_ARCHITECTURE.md)和[安全模型](docs/security/SECURITY_MODEL.md)。
