# Visual Regression

Use visual regression to make Kodo implementation measurable.

Example with Playwright:

```ts
import { test, expect } from '@playwright/test'

test.use({
  viewport: { width: 1586, height: 992 }
})

test('conversation main', async ({ page }) => {
  await page.goto('/ui-demo/conversation')
  await page.waitForLoadState('networkidle')
  await expect(page).toHaveScreenshot('conversation-main.png', {
    fullPage: true
  })
})

test('conversation inspector collapsed', async ({ page }) => {
  await page.goto('/ui-demo/conversation?inspector=closed')
  await page.waitForLoadState('networkidle')
  await expect(page).toHaveScreenshot('conversation-inspector-closed.png', {
    fullPage: true
  })
})

test('response trace', async ({ page }) => {
  await page.goto('/ui-demo/trace')
  await page.waitForLoadState('networkidle')
  await expect(page).toHaveScreenshot('response-trace.png', {
    fullPage: true
  })
})
```

Notes:
- use deterministic fixtures;
- disable non-essential animation in visual-test mode;
- freeze dates/times;
- do not call real LLM APIs;
- do not read arbitrary local project state;
- compare large geometry first before tuning screenshot thresholds.
