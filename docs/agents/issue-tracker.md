# Issue tracker: GitHub

本仓库的问题和 PRD 存放在 GitHub Issues：

- Repository: `yinshaohua/GPTEasy`
- Git remote: `git@github.com:yinshaohua/GPTEasy.git`
- CLI: `gh`

所有操作均使用 `gh` CLI。在已正确配置 Git remote 的仓库中，可由 `gh` 自动识别仓库；否则使用 `--repo yinshaohua/GPTEasy` 显式指定。

## Conventions

- **创建 Issue**：`gh issue create --repo yinshaohua/GPTEasy --title "..." --body "..."`
- **读取 Issue**：`gh issue view <number> --repo yinshaohua/GPTEasy --comments`，同时获取标签；需要筛选数据时使用 `jq`
- **列出 Issue**：`gh issue list --repo yinshaohua/GPTEasy --state open --json number,title,body,labels,comments --jq '[.[] | {number, title, body, labels: [.labels[].name], comments: [.comments[].body]}]'`，并按需要添加 `--label` 和 `--state` 过滤条件
- **评论 Issue**：`gh issue comment <number> --repo yinshaohua/GPTEasy --body "..."`
- **添加标签**：`gh issue edit <number> --repo yinshaohua/GPTEasy --add-label "..."`
- **移除标签**：`gh issue edit <number> --repo yinshaohua/GPTEasy --remove-label "..."`
- **关闭 Issue**：`gh issue close <number> --repo yinshaohua/GPTEasy --comment "..."`

多行 Issue 正文或评论应使用 heredoc，避免转义和换行损坏。

## Pull requests as a triage surface

**PRs as a request surface: no.**

如果以后需要将外部 Pull Request 作为功能请求纳入 triage 队列，可将上面的值改为 `yes`。`triage` 技能会读取该标志。

启用后，Pull Request 使用与 Issue 相同的标签和状态，并使用相应的 `gh pr` 命令：

- **读取 PR**：`gh pr view <number> --repo yinshaohua/GPTEasy --comments`
- **读取 PR diff**：`gh pr diff <number> --repo yinshaohua/GPTEasy`
- **列出用于 triage 的外部 PR**：使用 `gh pr list` 获取 `authorAssociation`，仅保留 `CONTRIBUTOR`、`FIRST_TIME_CONTRIBUTOR` 或 `NONE`；排除 `OWNER`、`MEMBER` 和 `COLLABORATOR`
- **评论、添加标签或关闭**：使用 `gh pr comment`、`gh pr edit` 和 `gh pr close`

GitHub 的 Issue 和 Pull Request 共用编号空间，因此 `#42` 可能是 Issue，也可能是 PR。先运行 `gh pr view 42`，失败时再运行 `gh issue view 42`。

## When a skill says "publish to the issue tracker"

在 `yinshaohua/GPTEasy` 中创建 GitHub Issue。

## When a skill says "fetch the relevant ticket"

运行：

`gh issue view <number> --repo yinshaohua/GPTEasy --comments`

## Wayfinding operations

供 `/wayfinder` 使用。Map 是一个总 Issue，child ticket 是其子 Issue。

- **Map**：使用标签 `wayfinder:map` 的单个 Issue，正文包含 Notes、Decisions-so-far 和 Fog。使用 `gh issue create --repo yinshaohua/GPTEasy --label wayfinder:map`
- **Child ticket**：作为 GitHub sub-issue 链接至 Map。若仓库未启用 sub-issues，则将 child 添加到 Map 正文的任务列表，并在 child 正文顶部写入 `Part of #<map>`。标签采用 `wayfinder:<type>`，其中 type 为 `research`、`prototype`、`grilling` 或 `task`
- **Blocking**：优先使用 GitHub 原生 Issue dependencies。通过 `gh api --method POST repos/yinshaohua/GPTEasy/issues/<child>/dependencies/blocked_by -F issue_id=<blocker-db-id>` 添加依赖。这里的 `blocker-db-id` 是数据库数字 ID，可使用 `gh api repos/yinshaohua/GPTEasy/issues/<n> --jq .id` 获取，不是 Issue 编号或 `node_id`
- **依赖降级方案**：如果原生 dependencies 不可用，在 child 正文顶部写入 `Blocked by: #<n>, #<n>`
- **Frontier query**：列出 Map 的未关闭 child，排除仍有未关闭 blocker 或已有 assignee 的 Issue，并按 Map 中的顺序选择第一个
- **Claim**：运行 `gh issue edit <n> --repo yinshaohua/GPTEasy --add-assignee @me`；这是会话的第一次写操作
- **Resolve**：先评论答案，再关闭 Issue，最后向 Map 的 Decisions-so-far 添加上下文指针和链接
