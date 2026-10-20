import { expect, test } from "@playwright/test";

/**
 * The decision-assistance surfaces.
 *
 * Both checks are behavioural: the settings section must render its
 * contract, and the send preview must show the real bytes and default to
 * "skip". No snapshot baselines are added — the settings screen is
 * captured by the existing `pages.spec.ts` route.
 */

test("决策辅助设置小节展示模式、场景开关与显式外发说明", async ({ page }) => {
  await page.goto("/ui-demo/settings");
  await page.getByRole("button", { name: "决策辅助" }).click();

  await expect(page.getByTestId("decision-settings")).toBeVisible();
  // The outbound notice is part of the contract: the user must be told
  // that this feature sends data to a third party.
  await expect(page.getByTestId("decision-settings")).toContainText("TypeSafe");
  // The service source is a first-class choice: one at a time, never an
  // automatic fallback to the other.
  await expect(page.getByTestId("decision-provider-typeSafe")).toHaveAttribute(
    "aria-checked",
    "true",
  );
  await expect(page.getByTestId("decision-provider-openRouter")).toBeVisible();
  await expect(page.getByTestId("decision-mode-off")).toHaveAttribute("aria-checked", "true");
  await expect(page.getByTestId("decision-mode-observe")).toBeVisible();
  await expect(page.getByTestId("decision-mode-assist")).toBeVisible();
  await expect(page.getByTestId("decision-model")).toHaveValue("jev-1.13.0");
  // The scenarios start off; a saved credential never turns them on.
  await expect(page.getByTestId("decision-credential-save")).toBeDisabled();
  await expect(page.getByTestId("decision-credential-clear")).toBeDisabled();
});

test("切换服务来源会换掉模型并各自记住密钥", async ({ page }) => {
  await page.goto("/ui-demo/settings");
  await page.getByRole("button", { name: "决策辅助" }).click();

  await page.getByTestId("decision-provider-openRouter").click();
  await expect(page.getByTestId("decision-provider-openRouter")).toHaveAttribute(
    "aria-checked",
    "true",
  );
  // The model id means something different per provider, so the default
  // follows the provider.
  await expect(page.getByTestId("decision-model")).toHaveValue("typesafe/jev-1.14");

  await page.getByTestId("decision-provider-typeSafe").click();
  await expect(page.getByTestId("decision-model")).toHaveValue("jev-1.13.0");
});

test("发送预览展示完整请求内容并默认聚焦到跳过", async ({ page }) => {
  await page.goto("/ui-demo/conversation?decision-preview=demo");
  const preview = page.getByTestId("decision-preview");
  await expect(preview).toBeVisible();

  // The destination and the model are visible before anything is sent.
  await expect(page.getByTestId("decision-preview-destination")).toHaveText(
    "https://api.typesafe.ai/v1/systemone",
  );
  await expect(page.getByTestId("decision-preview-model")).toHaveText("jev-1.13.0");
  // The *actual* bytes, not a summary.
  await expect(page.getByTestId("decision-preview-body")).toContainText("把重复的校验逻辑合并一下");
  await expect(page.getByTestId("decision-preview-questions")).toContainText("task_type");

  // The safe choice holds the focus.
  await expect(page.getByTestId("decision-preview-skip")).toBeFocused();
  await expect(page.getByTestId("decision-preview-approve")).toBeVisible();
});
