# Knowledge P0 implementation contract

基线：3ae318d28779d214fb13014069093cf55a43a97a。唯一分支 feat/knowledge-base，独立于主目录的待办任务。只用已有依赖，离线验证。本文列实施中的接口；UI 与 Rust 使用 camelCase JSON。

P0：项目/个人库、Markdown/TXT/笔记/成果文本导入、不可变正文版本、标题/行号分块、中文全文与精确检索、显式会话绑定、发送前资料预览与远程授权、可点击引用、停用/删除与作用域隔离。无向量/OCR/后台同步，界面明确关键词模式。

## Types

```text
KnowledgeBase { id, name, description, projectPath: string|null, createdAt, version }
BaseDraft { name, description, projectPath: string|null }
KnowledgeDocument { id, baseId, title, sourceKind, sourceRef: string|null, status: ready|disabled|quarantined,
  verified: bool, version, revisionId, contentHash, chunkCount, createdAt, updatedAt }
ImportDraft { title, body, sourceKind: note|file|result|answer, sourceRef: string|null, verified: bool }
KnowledgeChunk { id, revisionId, heading, lineStart, lineEnd, body }
DocumentDetail { document: KnowledgeDocument, body, chunks: KnowledgeChunk[] }
SearchQuery { baseIds: string[], projectPath: string|null, text, limit, budgetTokens }
KnowledgeHit { baseId, documentId, revisionId, chunkId, title, heading, lineStart, lineEnd, quote, channels: string[] }
KnowledgeSelection { baseIds: string[], strict: bool }
TurnKnowledge { sessionId, turnSeq, hits: KnowledgeHit[] }
```

数据库 crate API `KnowledgeStore`：`open(state_dir)`、`list_bases(project_path)`、`create_base(draft)`、`list_documents(base_id, project_path)`、`import_text(base_id, project_path, draft)`、`inspect_document(id, project_path)`、`set_enabled(id, project_path, expected_version, enabled)`、`delete_document(id, project_path, expected_version)`、`search(query)`、`get_bindings(session_id, project_path)`、`set_bindings(session_id, project_path, selection)`、`record_citations(session_id, turn_seq, project_path, hits)`、`read_citations(session_id, turn_seq, project_path)`。Rust getters return Result; nonexistent or unauthorized resource must fail without leaking body. Read citations may return historical revision for disabled documents, but must reject deleted resources and inaccessible bases.

Project paths are canonicalized by the desktop service. `list_bases(project_path)` returns personal bases plus bases for that project; a personal-only request returns only personal bases. Personal bases are searched only if explicitly present in baseIds. No retrieval cache or background workers in P0. Source files are read solely after native picker selection; confirmation binds the canonical path, exact text digest, KB and scope. Reads reject leaf symlinks, check the opened file identity, and confirmation rejects redirected parent paths. Arbitrary front-end absolute paths are never read. Text is UTF-8, at most 256 KiB / 128 Ki Unicode characters; preview counts use the real chunker.

## Desktop IPC / frontend API

```text
knowledge_list_bases({projectPath})
knowledge_create_base({draft})
knowledge_list_documents({baseId, projectPath})
knowledge_inspect_document({id, projectPath})
knowledge_import_text({baseId, projectPath, draft})
knowledge_pick_import({baseId, projectPath}) -> ImportPreview[]
knowledge_confirm_import({token}) -> KnowledgeDocument
knowledge_set_enabled({id, projectPath, expectedVersion, enabled})
knowledge_delete_document({id, projectPath, expectedVersion})
knowledge_search({query}) -> KnowledgeHit[]
knowledge_get_bindings({sessionId, projectPath}) -> KnowledgeSelection
knowledge_set_bindings({sessionId, projectPath, selection})
knowledge_prepare_turn({sessionId, projectPath, selection, text}) -> PreparedTurn
knowledge_turn_citations({sessionId, turnSeq}) -> KnowledgeHit[]
```

`ImportPreview { token, title, bodyPreview, characterCount, chunkCount }`; preview confirms a snapshot, no external parsing.
`PreparedTurn { token, hits, remote: bool, providerLabel: string|null, strict: bool }`; token binds question, scope, selection, actual destination and provider readiness. The caller displays all exact `hits.quote` text when remote and requires explicit send confirmation. `send_message` gets optional `knowledgeToken` and `knowledgeRemoteApproved`; tokens expire after five minutes, are single-use and are revalidated before request construction. Existing sessions must persist their selected bindings before preparation; subsequent unbinding invalidates the token and never restores the old selection. Draft tokens can only be consumed while creating a new session. No selected bases means no knowledge context.

`TurnDto.knowledgeUsed` is emitted from trusted `knowledge.used` session-log markers. Desktop and CLI share a public history filter which omits these turns even after disabling/deleting sources or losing the separate catalog. Knowledge turns bypass ordinary Memory recall and its outbox. The Agent receives validated JSON quotes in the user observation, fixed trust/citation rules in the system prompt, no provider/persona model fallback, and a live app/binding/source guard around model dispatch and compaction. Strict empty retrieval terminates before calling models/tools. Without a usable provider the Agent returns labelled excerpts rather than claiming AI generation. Model POST requests do not follow redirects; WebSearch separately approves its exact query and DuckDuckGo destination.

Errors use stable codes: invalid_input, not_found, scope_denied, storage_unavailable, revision_conflict, sensitive_content, unsupported_format, preview_expired, source_changed, knowledge_remote_approval_required, knowledge_unavailable. UI owns Chinese error wording.

The frontend supplies typed wrappers, deterministic browser fixtures, management and binding/citation UI. The desktop bridge owns picker grants, project/session authority and exact-send approval. `kodo-knowledge` owns the independent local catalog. `kodo-agent` owns trusted dispatch guards and model/tool behavior. Core/shell preserve turn provenance across both desktop and CLI history replay.
