import { expect, test } from "@playwright/test";
import { stubShell } from "./shell";

type Grid = { cols: number; rows: number };
type Calls = { opened: Grid[]; resized: Array<Grid & { id: string }> };
type DesktopWindow = Window & {
  __TAURI_INTERNALS__: {
    invoke: (command: string, args: Grid & { id: string }) => Promise<unknown>;
  };
  terminalCalls: Calls;
};

test("terminal forwards only grid changes to the PTY", async ({ page }) => {
  await stubShell(page);
  await page.goto("/ui-demo/conversation");
  await page.evaluate(() => {
    const desktop = window as unknown as DesktopWindow;
    const shell = desktop.__TAURI_INTERNALS__;
    const invoke = shell.invoke;
    const calls: Calls = { opened: [], resized: [] };
    desktop.terminalCalls = calls;
    shell.invoke = async (command, args) => {
      if (command === "terminal_open") {
        calls.opened.push({ cols: args.cols, rows: args.rows });
        return `shell-${calls.opened.length}`;
      }
      if (command === "terminal_resize") {
        calls.resized.push(args);
        return;
      }
      return invoke(command, args);
    };
  });

  const readCalls = () =>
    page.evaluate(async () => {
      // Include the ResizeObserver callback and the next frame it schedules.
      await new Promise<void>((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
      );
      return (window as unknown as DesktopWindow).terminalCalls;
    });

  await page.getByTestId("topbar-terminal").click();
  await expect.poll(async () => (await readCalls()).opened).toHaveLength(1);
  const { opened: [initial], resized } = await readCalls();
  expect(resized).toEqual([]);

  const tabs = page.getByTestId("terminal-tab");
  await page.getByTestId("terminal-new").click();
  await expect.poll(async () => (await readCalls()).opened).toHaveLength(2);
  await tabs.nth(0).click();
  await expect(tabs.nth(0)).toHaveAttribute("aria-selected", "true");
  expect(await readCalls()).toEqual({ opened: [initial, initial], resized: [] });

  const grip = page.getByRole("separator", { name: "拖动调整终端高度" });
  await grip.press("ArrowUp");
  await expect.poll(async () => (await readCalls()).resized).toEqual([
    { id: "shell-1", cols: initial.cols, rows: expect.any(Number) },
  ]);

  await tabs.nth(1).click();
  await expect.poll(async () => (await readCalls()).resized).toHaveLength(2);
  const { resized: [first, second] } = await readCalls();
  expect(first.rows).toBeGreaterThan(initial.rows);
  expect(second).toEqual({ ...first, id: "shell-2" });
});
