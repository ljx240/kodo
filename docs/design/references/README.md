# Reference images

The five original design references (`01-conversation-main.png` …
`05-conversation-alt.png`) were removed from disk on 2026-09-28, and the rules
that made reference screenshots the visual source of truth were retired at the
same time.

`CLAUDE.md` now names the design documents under `docs/design/` as the sole
source of truth for the UI. Nothing should look for, load, or compare against
reference screenshots.

## What the images used to govern

Historical notes kept because they still explain existing values:

- The images predate the sidebar changes recorded in `LAYOUT.md` §2 and the
  zh-CN copy centralised in `apps/desktop/src/i18n.ts` (`DESIGN.md` §10); they
  could not be matched exactly even before removal.
- Timeline rows sit on a 60px pitch (pixel-measured from the old
  `02-response-trace.png`); `LAYOUT.md` §6's "approximately 38–42px" line is
  stale, and `pages.css` carries the same note.
- Panel sizes in the images (sidebar ~285px, Inspector ~400–406px, top bar
  ~56px) differed from `LAYOUT.md` §1 (260 / 330 / 44px); the implementation
  follows `LAYOUT.md`.

## Playwright baselines

The snapshots in
`apps/desktop/tests/visual/pages.spec.ts-snapshots/` are a separate thing and
are regenerated when a design change is deliberate. See
`docs/design/UI_ACCEPTANCE.md` §7 for what that costs.
