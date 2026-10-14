/** Frozen, local-browser-only Knowledge fixtures for visual development. */

import type {
  DocumentDetail,
  KnowledgeBase,
  KnowledgeDocument,
  KnowledgeHit,
} from "../knowledgeApi";

const stamp = Math.floor(Date.parse("2026-09-30T09:12:00Z") / 1000);

export const DEMO_KNOWLEDGE_BASES: KnowledgeBase[] = [
  {
    id: "kb-personal-demo",
    name: "个人资料",
    description: "个人整理的说明与工作笔记",
    projectPath: null,
    createdAt: stamp,
    version: 1,
  },
  {
    id: "kb-kodo-demo",
    name: "Kodo 项目资料",
    description: "仅用于本地界面演示的项目范围资料",
    projectPath: "/demo/kodo",
    createdAt: stamp,
    version: 1,
  },
];

export const DEMO_KNOWLEDGE_PROJECTS = [
  { path: "/demo/kodo", name: "Kodo 示例项目" },
];

const compatibilityBody = `# 接口兼容约定

## 旧客户端
字段迁移期间保留旧客户端仍在使用的字段，并为新增字段提供明确默认值。
对仍在使用的请求，服务端应兼容旧字段并补充新旧字段的校验。

## 验收
覆盖新旧客户端的读写路径，并记录兼容窗口结束的条件。
涉及破坏性变更时，先确认调用方已经完成升级。`;

const releaseBody = `# 发布复核笔记

发布前检查变更范围、兼容性和回滚方式。
对未运行的验证命令应明确标注，不能把计划中的验证写成已通过。

## 复核结论
保留可恢复的原文快照，后续更新需要重新检查内容。`;

const projectBody = `# 项目本地约定

每个功能任务使用唯一工作分支，并在修改现有文件前刷新磁盘内容。
提交前检查暂存差异和工作区状态。`;

function makeChunk(
  id: string,
  revisionId: string,
  heading: string,
  lineStart: number,
  lineEnd: number,
  body: string,
) {
  return { id, revisionId, heading, lineStart, lineEnd, body };
}

export const DEMO_KNOWLEDGE_DOCUMENTS: KnowledgeDocument[] = [
  {
    id: "doc-compat-demo",
    baseId: "kb-personal-demo",
    title: "接口兼容约定.md",
    sourceKind: "file",
    sourceRef: "接口兼容约定.md",
    status: "ready",
    verified: true,
    version: 3,
    revisionId: "rev-compat-demo-3",
    contentHash: "demo-sha256-compat-3",
    chunkCount: 2,
    createdAt: stamp,
    updatedAt: stamp,
  },
  {
    id: "doc-release-demo",
    baseId: "kb-personal-demo",
    title: "发布复核笔记",
    sourceKind: "note",
    sourceRef: null,
    status: "ready",
    verified: false,
    version: 1,
    revisionId: "rev-release-demo-1",
    contentHash: "demo-sha256-release-1",
    chunkCount: 2,
    createdAt: stamp,
    updatedAt: stamp,
  },
  {
    id: "doc-project-demo",
    baseId: "kb-kodo-demo",
    title: "项目本地约定.md",
    sourceKind: "file",
    sourceRef: "项目本地约定.md",
    status: "ready",
    verified: true,
    version: 1,
    revisionId: "rev-project-demo-1",
    contentHash: "demo-sha256-project-1",
    chunkCount: 1,
    createdAt: stamp,
    updatedAt: stamp,
  },
];

export const DEMO_KNOWLEDGE_DETAILS: DocumentDetail[] = [
  {
    document: DEMO_KNOWLEDGE_DOCUMENTS[0],
    body: compatibilityBody,
    chunks: [
      makeChunk("chunk-compat-demo-1", "rev-compat-demo-3", "旧客户端", 3, 5, "## 旧客户端\n字段迁移期间保留旧客户端仍在使用的字段，并为新增字段提供明确默认值。\n对仍在使用的请求，服务端应兼容旧字段并补充新旧字段的校验。"),
      makeChunk("chunk-compat-demo-2", "rev-compat-demo-3", "验收", 7, 9, "## 验收\n覆盖新旧客户端的读写路径，并记录兼容窗口结束的条件。\n涉及破坏性变更时，先确认调用方已经完成升级。"),
    ],
  },
  {
    document: DEMO_KNOWLEDGE_DOCUMENTS[1],
    body: releaseBody,
    chunks: [
      makeChunk("chunk-release-demo-1", "rev-release-demo-1", "正文", 1, 5, "# 发布复核笔记\n\n发布前检查变更范围、兼容性和回滚方式。\n对未运行的验证命令应明确标注，不能把计划中的验证写成已通过。"),
      makeChunk("chunk-release-demo-2", "rev-release-demo-1", "复核结论", 6, 7, "## 复核结论\n保留可恢复的原文快照，后续更新需要重新检查内容。"),
    ],
  },
  {
    document: DEMO_KNOWLEDGE_DOCUMENTS[2],
    body: projectBody,
    chunks: [
      makeChunk("chunk-project-demo-1", "rev-project-demo-1", "项目本地约定", 1, 4, "# 项目本地约定\n\n每个功能任务使用唯一工作分支，并在修改现有文件前刷新磁盘内容。\n提交前检查暂存差异和工作区状态。"),
    ],
  },
];

export const DEMO_CONVERSATION_KNOWLEDGE_HITS: KnowledgeHit[] = [
  {
    baseId: "kb-personal-demo",
    documentId: "doc-compat-demo",
    revisionId: "rev-compat-demo-3",
    chunkId: "chunk-compat-demo-1",
    title: "接口兼容约定.md",
    heading: "旧客户端",
    lineStart: 3,
    lineEnd: 5,
    quote: "## 旧客户端\n字段迁移期间保留旧客户端仍在使用的字段，并为新增字段提供明确默认值。\n对仍在使用的请求，服务端应兼容旧字段并补充新旧字段的校验。",
    channels: ["fts", "exact"],
  },
];

export const DEMO_KNOWLEDGE_FILE_PREVIEW = {
  title: "接口验收清单.md",
  body: `# 接口验收清单\n\n## 兼容检查\n确认旧客户端仍能读取兼容字段。\n记录升级条件和废弃时间。`,
};
