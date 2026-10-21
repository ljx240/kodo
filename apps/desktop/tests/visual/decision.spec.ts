import { expect, test } from "@playwright/test";

/**
 * The decision-assistance settings surface.
 *
 * The checks are behavioural: the settings section must render its source,
 * mode and scenario contract, and the outbound notice must stay visible.
 * No snapshot baselines are added — the settings screen is captured by the
 * existing `pages.spec.ts` route.
 */

test("决策辅助设置小节展示模式、场景开关与显式外发说明", async ({ page }) => {
  await page.goto("/ui-demo/settings");
  await page.getByRole("button", { name: "决策辅助" }).click();

  await expect(page.getByTestId("decision-settings")).toBeVisible();
  // The outbound notice is part of the contract: the user must be told
  // that this feature sends data to a third party.
  await expect(page.getByTestId("decision-settings")).toContainText("TypeSafe");
  await expect(page.getByTestId("decision-settings")).toContainText("腾讯 EdgeOne");
  // The service source is a first-class choice: one at a time, never an
  // automatic fallback to another.
  await expect(page.getByTestId("decision-provider-typeSafe")).toHaveAttribute(
    "aria-checked",
    "true",
  );
  await expect(page.getByTestId("decision-provider-openRouter")).toBeVisible();
  await expect(page.getByTestId("decision-provider-tencent")).toBeVisible();
  await expect(page.getByTestId("decision-mode-off")).toHaveAttribute("aria-checked", "true");
  await expect(page.getByTestId("decision-mode-observe")).toBeVisible();
  await expect(page.getByTestId("decision-mode-assist")).toBeEnabled();
  await page.getByTestId("decision-mode-assist").click();
  await expect(page.getByTestId("decision-mode-assist")).toHaveAttribute("aria-checked", "true");
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

test("腾讯来源固定使用 @makers/jev 并锁定模型输入", async ({ page }) => {
  await page.goto("/ui-demo/settings");
  await page.getByRole("button", { name: "决策辅助" }).click();

  await page.getByTestId("decision-provider-tencent").click();
  await expect(page.getByTestId("decision-provider-tencent")).toHaveAttribute(
    "aria-checked",
    "true",
  );
  // EdgeOne Makers accepts exactly one alias, so the field is not editable.
  await expect(page.getByTestId("decision-model")).toHaveValue("@makers/jev");
  await expect(page.getByTestId("decision-model")).toBeDisabled();

  // Another provider unlocks the field and restores its own default.
  await page.getByTestId("decision-provider-openRouter").click();
  await expect(page.getByTestId("decision-model")).toHaveValue("typesafe/jev-1.14");
  await expect(page.getByTestId("decision-model")).toBeEnabled();
});

test("决策辅助样式表加载：外发说明与来源选择按设计排版", async ({ page }) => {
  await page.goto("/ui-demo/settings");
  await page.getByRole("button", { name: "决策辅助" }).click();

  // These styles live in decision.css, which no component-local import can
  // guarantee: a missing stylesheet still keeps every behavioural assertion
  // green, so assert the computed layout directly.
  const settings = page.getByTestId("decision-settings");
  await expect(settings.locator(".decision-settings-notice").first()).toHaveCSS(
    "border-top-width",
    "1px",
  );

  const providers = page.getByTestId("decision-provider-typeSafe").locator("..");
  await expect(providers).toHaveCSS("display", "grid");

  const option = page.getByTestId("decision-provider-typeSafe");
  await expect(option).toHaveCSS("border-top-width", "1px");
  await expect(option).toHaveCSS("text-align", "center");
});
