# APEIR Kernel（无限内核）

[![CI](https://github.com/kicoyini45-blip/apeir-kernel/actions/workflows/ci.yml/badge.svg)](https://github.com/kicoyini45-blip/apeir-kernel/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

APEIR Kernel 是一个面向本地进程、模型 Provider、算法和设备适配器的执行内核。它负责判断任务是否允许执行、选择符合约束的目标、记录资源所有权，并将执行结果提交为可恢复的持久状态。

内核与 APEIR 发行版保持独立。桌面界面、模型账户、文档工具、Agent 工作流、扩展市场和行业集成通过公开契约使用内核，但不进入内核的可信计算基。

[English](README.md)

## 项目状态

APEIR Kernel 0.1 是面向开发和评估的预发布版本，部署边界为可信本地主机。Windows 10 x64 已覆盖锁定依赖的 Rust 工作区测试、守护进程与 Worker 进程测试、Python/C SDK 测试和 Micro 主机测试。其他平台和外部 Provider 仍需针对确切发布版本单独验证。

NKI 只监听本机回环地址，暂不支持远程多主机控制和恶意多租户隔离。部署前请阅读[支持矩阵](docs/reference/support-matrix.md)与[安全策略](SECURITY.md)。

## 核心能力

- 带请求身份、期限、会话令牌和结构化错误的版本化 NKI；
- 面向能力、安全、资源和执行器约束的工作负载准入；
- 确定性调度与可审计的决策轨迹；
- 受 fencing token 保护的资源租约；
- 支持完整性校验、重放和恢复的追加式 Journal；
- 在受控子进程中运行 Provider，并处理取消、超时和异常退出；
- 规范化 Receipt、幂等 Commit 与基于交付语义的恢复；
- 扩展准入、审批绑定、执行 Permit 与撤销；
- Rust、Python、C 客户端和固定容量的 Micro C 实现；
- 机器可读契约、一致性测试和发布工具。

## 执行模型

所有改变状态的请求都经过同一条权威执行路径：

```text
CLI / SDK / APEIR 发行版
           |
           v
          NKI
           |
           v
准入 -> 调度 -> 带 fencing 的租约 -> 持久化 Intent
     -> 隔离 Provider -> Receipt -> Commit -> 释放租约
```

Journal 是执行状态的持久权威来源，内存队列和索引只是可以重建的投影。Provider 不能写入 Journal、签发租约、授予能力或自行提交结果。

## 构建与测试

需要：

- Rust stable 与 Cargo；
- Python 3.10 或更高版本；
- C11 编译器；
- Windows 环境下安装包含 C++ 工作负载的 Visual Studio 2022 Build Tools。

Windows PowerShell：

```powershell
./scripts/bootstrap.ps1
./scripts/build.ps1
./scripts/test.ps1
```

Linux 等 POSIX 系统：

```sh
./scripts/bootstrap.sh
./scripts/build.sh
./scripts/test.sh
```

`Cargo.lock` 固定 Rust 依赖图。完整测试脚本覆盖 Rust 工作区、守护进程与 Worker 进程边界、Python SDK、C SDK 和 Micro 主机实现。

## 本地运行

启动守护进程与确定性参考 Worker：

```powershell
target/debug/apeird serve .local/kernel.db target/debug/nous-provider-worker
```

在另一个终端执行：

```powershell
target/debug/apeir-kernelctl doctor
target/debug/apeir-kernelctl run "hello" --backend reference
target/debug/apeir-kernelctl inspect
```

参考后端不需要模型账户或网络服务。由应用管理会话时，应在守护进程与客户端环境中设置相同、长度不少于 32 个字符的随机 `NOUS_NKI_TOKEN`。

## 仓库结构

| 路径 | 职责 |
| --- | --- |
| `crates/nous-types` | 工作负载、资源、Provider 与开放运行时契约 |
| `crates/nous-state` | Journal、完整性校验、重放与持久投影 |
| `crates/nous-security` | 身份、能力授权和安全校验 |
| `crates/nous-resource` | 资源准入、租约与 fencing |
| `crates/nous-scheduler` | 可行性检查、确定性调度与决策轨迹 |
| `crates/nous-kernel-core` | 工作负载生命周期、恢复、取消与 Provider 管理 |
| `crates/nous-nki` | NKI 封装、方法、帧格式与版本处理 |
| `crates/nous-execution-proof` | Effect 记录与执行证明数据结构 |
| `crates/nous-runtime-client` | Rust NKI 客户端 |
| `crates/nous-intelligence` | 模型、任务图、数据集与项目声明校验 |
| `crates/nous-control-plane` | 声明式控制资产的版本化目录 |
| `daemon/nousd` | `apeird` 守护进程与 `nousd` 兼容程序 |
| `providers/worker` | 参考后端与适配后端的单次进程 Host |
| `tools/nous` | `apeir-kernelctl` 与兼容 CLI |
| `spec` | 版本化 JSON 契约与兼容性声明 |
| `sdk`、`micro` | 多语言客户端与受限目标实现 |
| `tests`、`conformance` | 契约、进程、恢复与故障测试 |

## 兼容性

部分 v1 标识继续使用 `nous` 命名空间，包括 crate 名、环境变量、兼容命令、C/Python API 符号和 `nous.*.v1` Wire 标识。这些名称属于兼容边界，不代表另一个产品。详情见 [COMPATIBILITY.md](COMPATIBILITY.md)。

## 文档

- [总体架构](ARCHITECTURE.zh-CN.md)
- [文档索引](docs/README.md)
- [内核与发行版边界](docs/architecture/APEIR_PROJECT_BOUNDARIES.md)
- [执行路径](docs/architecture/EXECUTION_PATH.md)
- [状态所有权](docs/architecture/STATE_OWNERSHIP.md)
- [Provider 架构](docs/architecture/PROVIDER_ARCHITECTURE.md)
- [构建与一致性测试](docs/development/BUILD_TEST_CONFORMANCE.md)
- [贡献指南](CONTRIBUTING.md)

## 安全与许可证

安全问题请按照 [SECURITY.md](SECURITY.md) 私下报告，不要在公开 Issue 中披露。APEIR Kernel 采用 Apache License 2.0，第三方依赖说明见 [THIRD_PARTY_NOTICES](THIRD_PARTY_NOTICES)。
