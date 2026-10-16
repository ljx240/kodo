import { expect, test } from "@playwright/test";
import type { ItemDto } from "../../src/api";
import { buildTraceMap } from "../../src/traceMap/build";

test("trace map keeps plan dependencies without duplicating sequential edges", () => {
  const items = [
    { id: 1, at: 1, status: "done", duration: 1, kind: "reasoning", summary: "inspect" },
    { id: 2, at: 2, status: "done", duration: 1, kind: "search", query: "router", detail: "found" },
    { id: 3, at: 3, status: "done", duration: 1, kind: "planStep", title: "verify", triggeredBy: [1, 2, 999] },
  ] as unknown as ItemDto[];
  const layout = buildTraceMap(items);
  expect(layout.edges.map(({ fromId, toId, kind }) => ({ fromId, toId, kind }))).toEqual([
    { fromId: "item:1", toId: "item:2", kind: "sequential" },
    { fromId: "item:2", toId: "item:3", kind: "sequential" },
    { fromId: "item:1", toId: "item:3", kind: "planTrigger" },
  ]);
});

test("trace map persists its tab and opens the matching timeline row by keyboard", async ({ page }) => {
  await page.goto("/ui-demo/trace?tab=map");
  const mapTab = page.getByRole("tab", { name: "地图", exact: true });
  await expect(mapTab).toHaveAttribute("aria-selected", "true");
  await expect(page.locator(".trace-map-node")).toHaveCount(10);
  await expect(page.locator(".trace-map-edge--sequential")).toHaveCount(9);
  await expect(page.getByRole("list", { name: "图例" }).getByRole("listitem")).toHaveCount(5);
  await page.reload();
  await expect(mapTab).toHaveAttribute("aria-selected", "true");

  const node = page.locator('.trace-map-node[data-item-id="3"]');
  await node.focus();
  await node.press("Enter");
  await expect(page.getByRole("tab", { name: "时间线", exact: true })).toHaveAttribute("aria-selected", "true");
  await expect(page).toHaveURL(/\/ui-demo\/trace$/);
  await expect(page.locator('[data-item-row="3"]')).toBeInViewport();
});
