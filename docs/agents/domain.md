# Domain Docs

本文定义工程技能在探索代码库时，应如何读取本仓库的领域文档。

## Before exploring, read these

开始探索代码前，读取：

- 根目录中的 `CONTEXT.md`
- 如果根目录存在 `CONTEXT-MAP.md`，则按照其中的映射读取与当前任务相关的 `CONTEXT.md`
- `docs/adr/` 中与当前工作区域相关的架构决策记录
- 对于 multi-context 仓库，还应检查 `src/<context>/docs/adr/` 中特定上下文的架构决策

如果上述文件或目录不存在，应静默继续：

- 不要将文件缺失报告为问题
- 不要预先建议创建这些文件
- 当术语或架构决策实际得到确认时，由 `/domain-modeling` 等技能按需创建

## File structure

本仓库采用 single-context 布局：

```text
/
├── CONTEXT.md
├── docs/
│   └── adr/
│       ├── 0001-example-decision.md
│       └── 0002-another-decision.md
└── src/
```

如果项目以后演变成大型 multi-context monorepo，可改为：

```text
/
├── CONTEXT-MAP.md
├── docs/
│   └── adr/
└── src/
    ├── context-a/
    │   ├── CONTEXT.md
    │   └── docs/
    │       └── adr/
    └── context-b/
        ├── CONTEXT.md
        └── docs/
            └── adr/
```

## Use the glossary's vocabulary

输出中命名领域概念时，例如 Issue 标题、重构建议、假设或测试名称，应使用 `CONTEXT.md` 中定义的术语，不要改用领域词汇表明确排除的同义词。

如果需要的概念尚未出现在词汇表中，可能表示：

1. 正在引入项目并未使用的语言，应重新考虑；或
2. 领域文档确实存在缺口，应记录并交由 `/domain-modeling` 处理。

## Flag ADR conflicts

如果建议或实现与现有 ADR 冲突，应明确指出冲突，而不是静默覆盖。例如：

> 与 ADR-0007（事件溯源订单）冲突，但值得重新评估，因为……
