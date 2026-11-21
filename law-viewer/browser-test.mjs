// SPDX-License-Identifier: AGPL-3.0-only
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { chromium } from "playwright-core";

const url = process.argv[2] ?? "http://localhost:8789/";
const browser = await chromium.launch(
  process.env.CHROME_BIN ? { executablePath: process.env.CHROME_BIN } : {},
);
try {
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
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
  await page.getByRole("button", { name: "新版本文" }).click();
  assert.match(await page.locator("#output").textContent(), /特定医薬品/);
  await page.getByRole("button", { name: "旧版本文" }).click();
  assert.match(await page.locator("#output").textContent(), /ワクチン/);
  await page.getByRole("button", { name: "新旧対照条文", exact: true }).click();
  for (const id of ["comparison-docx", "docx"]) {
    const pending = page.waitForEvent("download");
    await page.locator(`#${id}`).click();
    const download = await pending;
    assert.match(download.suggestedFilename(), /\.docx$/);
    assert.equal((await readFile(await download.path())).subarray(0, 4).toString("hex"), "504b0304");
  }
  if (process.env.LAW_VIEWER_SCREENSHOT) {
    await page.screenshot({ path: process.env.LAW_VIEWER_SCREENSHOT });
  }
  const pdf = await page.pdf({ format: "A4" });
  assert.equal(pdf.subarray(0, 4).toString(), "%PDF");
  assert.deepEqual(errors, []);
  console.log(`Verified law search, e-Gov revisions, WASM comparison, DOCX downloads and print PDF at ${url}`);
} finally {
  await browser.close();
}
