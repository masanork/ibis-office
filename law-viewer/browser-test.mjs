// SPDX-License-Identifier: AGPL-3.0-only
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { chromium } from "playwright-core";

const url = process.argv[2] ?? "http://localhost:8789/";
const browser = await chromium.launch(
  process.env.CHROME_BIN ? { executablePath: process.env.CHROME_BIN } : {},
);
try {
  const page = await browser.newPage({ viewport: { width: 1920, height: 1000 } });
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.goto(url);
  try {
    await page.locator("#output table").waitFor({ timeout: 45000 });
  } catch (error) {
    throw new Error(`${await page.locator("#status").textContent()} · ${errors.join(", ")}`, { cause: error });
  }
  assert.equal(await page.locator("#law-title").textContent(), "予防接種法");
  assert.match(await page.locator("#new").inputValue(), /508AC0000000076$/);
  assert.match(await page.locator("#git-source").getAttribute("href"), /Kohei-Wada\/houritsu\/blob\//);
  assert.match(await page.locator("#git-new").getAttribute("href"), /Kohei-Wada\/houritsu\/blob\/[0-9a-f]{40}\//);
  assert.match(new URL(page.url()).searchParams.get("old"), /^323AC0000000068_/);
  assert.ok(await page.locator("#output tbody tr").count() > 2);
  const output = page.locator("#output");
  const writingMode = () => output.evaluate(el => getComputedStyle(el).writingMode);
  assert.equal(await writingMode(), "vertical-rl");
  assert.deepEqual(await page.locator("button[data-view]").evaluateAll(
    buttons => buttons.map(button => button.dataset.view)
  ), ["shinkyu", "old", "new"]);
  assert.equal(await page.locator("#memo-panel, #docx").count(), 0);
  assert.ok(await output.evaluate(el => {
    const pane = el.getBoundingClientRect();
    return pane.left < 50 && innerWidth - pane.right < 50;
  }), "Wide desktop uses almost the entire viewport for reading");
  const [amended, current] = await page.locator("#output th").evaluateAll(
    cells => cells.map(cell => ({ top: cell.getBoundingClientRect().top, text: cell.textContent })),
  );
  assert.equal(amended.text, "改正後");
  assert.equal(current.text, "現行");
  assert.ok(amended.top < current.top);
  assert.ok(await output.evaluate(el => {
    el.scrollLeft = -el.scrollWidth;
    const scrollsLeft = el.scrollLeft < 0;
    el.scrollLeft = 0;
    return scrollsLeft;
  }));
  await page.locator("#writing-mode").selectOption("horizontal");
  assert.equal(await writingMode(), "horizontal-tb");
  await page.locator("#writing-mode").selectOption("auto");
  await page.getByRole("button", { name: "新版本文" }).click();
  assert.equal(await writingMode(), "horizontal-tb");
  await page.locator("#writing-mode").selectOption("vertical");
  assert.equal(await writingMode(), "vertical-rl");
  await page.locator("#writing-mode").selectOption("auto");
  assert.match(await page.locator("#output").textContent(), /特定医薬品/);
  await page.getByRole("button", { name: "旧版本文" }).click();
  assert.match(await page.locator("#output").textContent(), /ワクチン/);
  await page.getByRole("button", { name: "新旧対照条文", exact: true }).click();
  const pendingDocx = page.waitForEvent("download");
  await page.locator("#comparison-docx").click();
  const docx = await pendingDocx;
  assert.match(docx.suggestedFilename(), /\.docx$/);
  assert.equal((await readFile(await docx.path())).subarray(0, 4).toString("hex"), "504b0304");

  const pendingJson = page.waitForEvent("download");
  await page.locator("#json").click();
  const json = await pendingJson;
  const changes = JSON.parse(await readFile(await json.path(), "utf-8"));
  assert.ok(changes.article_changes.length > 0);
  assert.equal("assistant_prompts" in changes, false);
  if (process.env.LAW_VIEWER_SCREENSHOT) {
    await page.screenshot({ path: process.env.LAW_VIEWER_SCREENSHOT });
  }
  await page.emulateMedia({ media: "print" });
  assert.equal(await writingMode(), "horizontal-tb");
  const pdf = await page.pdf({ format: "A4" });
  assert.equal(pdf.subarray(0, 4).toString(), "%PDF");
  const mobile = await browser.newPage({
    viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true,
  });
  await mobile.goto(url);
  await mobile.locator("#output table").waitFor({ timeout: 45000 });
  const mobileFits = await mobile.locator("#output").evaluate(el =>
    document.documentElement.scrollWidth <= window.innerWidth && el.scrollWidth > el.clientWidth,
  );
  assert.ok(mobileFits, "Vertical scrolling stays inside the reading pane on mobile");
  assert.deepEqual(errors, []);
  console.log(`Verified law search, e-Gov revisions, WASM comparison, full-width vertical reading, comparison export and print PDF at ${url}`);
} finally {
  await browser.close();
}
