# Reference images

`01-conversation-main.png` … `05-conversation-alt.png` are the original design
references, used at a 1586 × 992 viewport.

## These five images are pre-change baselines

Every one of them draws the sidebar, and the sidebar has since changed by
decision:

- the account row (avatar, user name, sign-out) is gone — Kodo has no login;
- `Settings` moved from the top of the sidebar to its foot;
- `Projects` gained a `+` and each project row gained a `⋯` menu.

So all five images disagree with the current implementation in the sidebar, and
none of them can be matched exactly any more.

They were **deliberately not redrawn**. Drawing new reference images would mean
declaring our own output to be the reference, which would make the comparison
meaningless. What they still are good for:

- the composition, hierarchy and density of the main column, the Inspector and
  the Response Trace — none of which changed;
- the visual target for anything outside the sidebar.

`CLAUDE.md` says reference screenshots are the visual source of truth. For
everything the sidebar does not touch, that still holds. For the sidebar itself,
this file is the record of what changed and why, so the repo does not quietly
contradict itself.

## Known geometry differences

Three panel sizes in these images differ from what `LAYOUT.md` §1 specifies, and
the implementation follows `LAYOUT.md`:

| Panel | Reference image | `LAYOUT.md` §1 |
|---|---|---|
| Sidebar | ~285px | 260px |
| Inspector | ~400–406px | 330px |
| Top bar | ~56px | 44px |

Each is outside the tolerance in `UI_ACCEPTANCE.md` §2. Per `CLAUDE.md`, geometry
follows the documentation and appearance follows the image; these are geometry,
so the documentation wins. The difference is recorded here rather than silently
resolved.

## Regenerating baselines

The Playwright baselines in `apps/desktop/tests/visual/pages.spec.ts-snapshots/`
are a separate thing and **have** been regenerated. See
`docs/design/UI_ACCEPTANCE.md` §7 for what that costs.
