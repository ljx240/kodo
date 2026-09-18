# Kodo Design System

## 1. Product character

Kodo is a coding-agent workstation.

It should feel closer to a restrained IDE than to:
- a SaaS dashboard;
- a chat consumer app;
- a project-management tool.

Priority order:

1. conversation readability;
2. agent execution visibility;
3. information density;
4. fast project/session switching;
5. minimal chrome;
6. visual polish.

The UI must stay visually quiet.  
The content — conversation, commands, files, results — is the product.

---

## 2. Primary visual language

### Theme

V1 primary theme: **Light**

Use neutral whites and cool grays with one restrained blue accent.

```text
Canvas                #FFFFFF
Sidebar                #FAFBFC
Secondary surface      #F7F8FA
Hover                  #F3F6FA
Selected               #EEF5FF
Border                 #E5E9F0
Border strong          #D7DEE8

Text primary           #172033
Text secondary         #667085
Text muted             #98A2B3

Accent                 #2563EB
Accent hover           #1D4ED8
Success                #16A34A
Warning                #D97706
Error                  #DC2626
```

Do not use gradients in normal application chrome.

### Typography

Use system UI font first.

```text
UI font:
-apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif

Code:
"SFMono-Regular", Consolas, "Liberation Mono", monospace
```

Base sizes:

```text
Primary UI text        13px
Secondary text         12px
Micro metadata         11px
Conversation body      14px
Code / commands        12px
Page title             18px
Section title          13px / 600
```

Typical line height:

```text
UI                     1.35
Conversation            1.55
Code                    1.45
```

Do not enlarge text to make the UI feel “premium”.  
Kodo uses compact IDE density.

---

## 3. Density

Primary reference viewport:

```text
1586 × 992
```

Secondary validation:

```text
1440 × 900
1920 × 1080
2560 × 1440
```

At 1586×992, the user should simultaneously see:
- project tree;
- multiple nested conversations;
- several agent trace items;
- final response;
- composer;
- optional Inspector content.

No oversized empty hero areas.

---

## 4. Shape language

Radius:

```text
Input / small control       5px
Panel                       6px
Popover                     8px
```

Avoid frequent 12–20px rounded cards.

Borders:
- 1px subtle separators;
- use boundaries more often than shadows.

Shadows:
- only floating menus, dialogs and transient overlays;
- no shadow around every content group.

---

## 5. Spacing

Base scale:

```text
2 / 4 / 6 / 8 / 12 / 16 / 20 / 24
```

Default compact control height:

```text
Small control        28px
Default control      32px
Toolbar              44px
Composer min         54px
```

Conversation content should use whitespace for hierarchy, but avoid card-per-message layouts.

---

## 6. Icons

Use one consistent line-icon family.

Rules:
- 14–16px in dense rows;
- 16–18px in primary navigation;
- filled icons only for strong status emphasis;
- tool types may have semantic icons;
- do not mix multiple icon styles.

---

## 7. Agent trace visual language

Agent work is shown inline inside an assistant reply.

A trace item has:

```text
status | action | short detail | metadata / duration | disclosure
```

Examples:

```text
✓ Thinking          分析需求，制定检查计划                    12s
✓ Search codebase   搜索 k2k / router / heartbeat             8s
✓ Read file         source/db/source.table                    14s
✓ Run command       cargo check                               18s
✓ Call model        claude-3-5-sonnet    12.4k → 2.1k        24s
```

Rules:
- trace is readable at a glance;
- long outputs stay collapsed by default;
- active item may expand;
- failed item remains expanded;
- completed output should not dominate the conversation.

---

## 8. Visual hierarchy

The hierarchy is:

```text
Project / conversation context
        ↓
User request
        ↓
Agent execution trace
        ↓
Final answer
        ↓
Changed files
        ↓
Composer
```

Inspector is secondary and must never visually overpower the conversation.

---

## 9. Minimalism rules

Do not add visual elements just because there is empty space.

Prefer:
- separators over containers;
- inline metadata over separate cards;
- compact rows over tile grids;
- nested tree navigation over duplicate sidebars;
- progressive disclosure over permanently visible detail.

Kodo should look calm even when it contains a lot of information.
