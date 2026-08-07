# Roadmap: GPTEasy v0.1

## Overview

本路线图交付一个 Windows x64 垂直闭环，而不是恢复旧 88 项水平分层计划。每个阶段都产生可运行、可验证的增量；旧阶段执行记录已归档，不继承任何完成状态。

## Phases

- [ ] **Phase 1: 可执行基础与当前 Codex 合同** - 从空源码建立 Windows x64 Tauri/Rust/React 骨架，冻结目标 Codex 配置与本地状态合同。
- [ ] **Phase 2: 已验证供应商闭环** - 用户可以在真实设置页完成模型发现、完整验证、保存、编辑和删除。
- [ ] **Phase 3: 当前用户 Codex 安全切换** - 用户可以明确接管、切换模式、恢复配置，并在崩溃或外部修改后保持可解释状态。
- [ ] **Phase 4: 托盘与运行时一致性** - 桌面 Codex、CLI、待重启、托盘和窗口生命周期形成同一工作流。
- [ ] **Phase 5: Windows x64 验收与交付** - 自动化故障矩阵、真实 UAT、无障碍和当前用户安装包通过。

## Phase Details

### Phase 1: 可执行基础与当前 Codex 合同

**Goal**: 在不复用旧源码的前提下，建立可运行骨架和后续写入所依赖的最小可信底座。

**Requirements**: ARCH-01, ARCH-02, ARCH-03, STATE-01, STATE-02, STATE-03, STATE-04

**Success Criteria**:

1. 新建的 Tauri 2 + Rust + TypeScript/React 应用只面向 Windows x64 当前用户，React 无文件、SQLite、网络或进程权限。
2. 对当前支持的 Codex 版本重新验证默认用户路径、配置字段、供应商凭据载体、OpenAI 登录检测和桌面/CLI 共同读取行为；不满足时停止后续实现。
3. 最小 SQLite schema、追加迁移、三份一致备份、历史 fixture、未来 schema 拒写和恢复错误页通过真实文件测试。
4. 环境实际状态与 SQLite 最后应用证据分离，启动协调不会写 Codex 配置。

**Plans**: TBD

### Phase 2: 已验证供应商闭环

**Goal**: 用户可以从空目录创建真正兼容 Codex 的供应商，未验证输入不会成为持久状态。

**Depends on**: Phase 1

**Requirements**: PROV-01, PROV-02, PROV-03, PROV-04, PROV-05, PROV-06, PROV-07, UI-01

**Success Criteria**:

1. “供应商”页可以新增、获取模型、选择默认模型并逐步完成 Responses 流式工具调用验证。
2. 安全 URL、禁止重定向、SSE 分片/断流、严格工具参数和 nonce 回传由模拟服务覆盖。
3. 验证失败、取消或离页后 SQLite 和 Codex 环境无变化；成功后只有明确保存才进入目录。
4. 非当前供应商保存、当前供应商保存并应用、仅改名和删除限制分别符合供应商生命周期。

**Plans**: TBD

### Phase 3: 当前用户 Codex 安全切换

**Goal**: 用户可以在不丢失已有 Codex 设置的前提下切换供应商或 OpenAI 登录模式，并撤销最近一次修改。

**Depends on**: Phase 2

**Requirements**: CONF-01, CONF-02, CONF-03, CONF-04, CONF-05, CONF-06, MODE-01, UI-02

**Success Criteria**:

1. 配置不存在时只有明确切换才创建；有效外部配置展示替换范围并确认接管，其他设置保持。
2. 唯一管理区块、完整工件指纹、五份备份、同目录暂存、平台替换和复读校验通过真实 Windows 文件测试。
3. 单个未完成操作在所有提交边界崩溃后只收敛到完整旧状态、完整新状态或管理冲突。
4. OpenAI 登录切换、外部注销警告和恢复上次配置都不读取、保存或删除 OpenAI 令牌。

**Plans**: TBD

### Phase 4: 托盘与运行时一致性

**Goal**: 用户可以从设置或托盘安全切换，并准确理解运行中 Codex 是否仍需重启。

**Depends on**: Phase 3

**Requirements**: RUN-01, RUN-02, RUN-03, RUN-04, RUN-05, UI-03, UI-04

**Success Criteria**:

1. 桌面 Codex 和 Codex CLI 进程识别覆盖真实进程、路径、父子关系、PID 复用和访问受限场景。
2. 正常托盘切换直接执行；运行、未知、无消费者和 OpenAI 模式返回场景使用一次合并确认。
3. 应用不控制消费者进程；待重启只在可靠确认旧消费者退出后自动清除。
4. 两页设置窗口、托盘驻留、首次关闭通知、简体中文、系统主题和无障碍合同通过。

**Plans**: TBD

### Phase 5: Windows x64 验收与交付

**Goal**: 用可重复自动化和真实环境共同证明首个闭环可验收，并产出当前用户安装包。

**Depends on**: Phase 4

**Requirements**: REL-01, REL-02, REL-03, REL-04

**Success Criteria**:

1. 两供应商、四种初始配置、验证失败、并发修改、数据库恢复和多工件崩溃矩阵全部通过。
2. API Key canary 证明数据库和明确凭据载体之外的日志、错误、通知、截图辅助和测试输出不含完整 Key。
3. 一次性 Windows x64 当前用户环境中，真实标准供应商、真实 Codex CLI 和桌面 Codex完成端到端 UAT。
4. 未签名当前用户安装包完成安装、启动、覆盖安装、卸载和数据保留验收；正式对外发布前另行满足 Authenticode。

**Plans**: TBD

## Coverage

| Phase | Requirement Count |
|-------|-------------------|
| 1. 可执行基础与当前 Codex 合同 | 7 |
| 2. 已验证供应商闭环 | 8 |
| 3. 当前用户 Codex 安全切换 | 8 |
| 4. 托盘与运行时一致性 | 7 |
| 5. Windows x64 验收与交付 | 4 |
| **Total** | **34** |

## Progress

| Phase | Plans Complete | Status |
|-------|----------------|--------|
| 1 | 0/TBD | Ready to plan |
| 2 | 0/TBD | Not started |
| 3 | 0/TBD | Not started |
| 4 | 0/TBD | Not started |
| 5 | 0/TBD | Not started |
